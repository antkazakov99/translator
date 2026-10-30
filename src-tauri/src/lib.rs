pub mod graph;
pub mod parser;
pub mod project;

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use rayon::prelude::*;
use crate::parser::renpy::{ChoiceBlockDto, RenpyParser, SceneEdgeDto};
use crate::parser::GameParser;
use crate::graph::ScenarioNode;

/// Рекурсивно собирает все файлы `.rpy`, строго игнорируя:
/// - скрытые директории и файлы (.codex-backups, .editor-backups, .git)
/// - файлы бэкапов (*.bak, *.backup, *.tmp)
fn collect_rpy_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            // Игнорируем скрытые папки (.codex-backups, .editor-backups, .git) и папки с бэкапами
            if file_name.starts_with('.') || file_name.contains("backup") || file_name.contains("tmp") {
                continue;
            }

            if path.is_dir() {
                collect_rpy_files(&path, files);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rpy") {
                // Исключаем любые бэкапы вида *.bak или *.bak.*
                if !file_name.ends_with(".bak") && !file_name.contains(".bak") {
                    files.push(path);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationFileItem {
    pub name: String,
    pub path: String,
    pub size_kb: f64,
    pub total_lines: usize,
    pub translated_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub game_name: String,
    pub characters: Vec<(String, String)>,
    pub files: Vec<TranslationFileItem>,
    pub total_lines: usize,
    pub total_translated: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueEntryDto {
    pub id: String,
    pub line_number: usize,
    pub speaker: Option<String>,
    pub original_text: String,
    pub translated_text: Option<String>,
    pub status: String,
    pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDto {
    pub id: String,
    pub name: String,
    pub total_lines: usize,
    pub translated_lines: usize,
    pub choices: Vec<ChoiceBlockDto>,
    pub jumps: Vec<String>,
    pub entries: Vec<DialogueEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFileDto {
    pub file_path: String,
    pub file_name: String,
    pub total_lines: usize,
    pub translated_lines: usize,
    pub scenes: Vec<SceneDto>,
    pub edges: Vec<SceneEdgeDto>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Привет, {}! Мост между Svelte 5 и Rust работает идеально!", name)
}

#[tauri::command]
async fn scan_game_project(game_dir: String) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        if !base_path.exists() {
            return Err(format!("Папка игры не найдена: {}", game_dir));
        }

        // 1. Ищем персонажей в оригинальных скриптах игры в параллельных потоках (Rayon)
        let mut script_files = Vec::new();
        collect_rpy_files(base_path, &mut script_files);

        let characters_map: std::collections::HashMap<String, String> = script_files
            .into_par_iter()
            .filter_map(|path| {
                let path_str = path.to_string_lossy();
                if !path_str.contains(r"\tl\") && !path_str.contains("/tl/") {
                    std::fs::read_to_string(&path).ok().map(|content| RenpyParser::extract_speakers(&content))
                } else {
                    None
                }
            })
            .reduce(std::collections::HashMap::new, |mut acc, map| {
                acc.extend(map);
                acc
            });

        let mut characters: Vec<(String, String)> = characters_map.into_iter().collect();
        characters.sort_by(|a, b| a.1.cmp(&b.1));

        // 2. Ищем файлы переводов в game/tl/russian в параллельных потоках (сверхбыстрый подсчет строк)
        let tl_dir = base_path.join(r"tl\russian");
        let mut files = Vec::new();
        let mut grand_total = 0;
        let mut grand_translated = 0;

        if tl_dir.exists() {
            let mut tl_files = Vec::new();
            collect_rpy_files(&tl_dir, &mut tl_files);

            let mut parsed_files: Vec<TranslationFileItem> = tl_files
                .into_par_iter()
                .filter_map(|path| {
                    let content = std::fs::read_to_string(&path).ok()?;
                    let size_kb = content.len() as f64 / 1024.0;
                    let (total, translated) = RenpyParser::quick_translation_stats(&content);
                    let relative_name = path
                        .strip_prefix(&tl_dir)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| path.file_name().unwrap_or_default().to_string_lossy().to_string());

                    Some(TranslationFileItem {
                        name: relative_name,
                        path: path.to_string_lossy().to_string(),
                        size_kb,
                        total_lines: total,
                        translated_lines: translated,
                    })
                })
                .collect();

            for f in &parsed_files {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_files.sort_by(|a, b| b.size_kb.partial_cmp(&a.size_kb).unwrap_or(std::cmp::Ordering::Equal));
            files = parsed_files;
        }

        Ok(ProjectSummary {
            game_name: base_path.file_name().unwrap_or_default().to_string_lossy().to_string(),
            characters,
            files,
            total_lines: grand_total,
            total_translated: grand_translated,
        })
    })
    .await
    .map_err(|e| format!("Ошибка задачи сканирования: {}", e))?
}

#[tauri::command]
async fn parse_file_entries(file_path: String) -> Result<ParsedFileDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

        let parser = RenpyParser::new();
        let graph = parser.parse_file(path, &content)
            .map_err(|e| format!("Ошибка парсинга: {}", e))?;

        let mut scenes: Vec<SceneDto> = Vec::new();
        let mut file_total = 0;
        let mut file_translated = 0;
        let mut current_scene_name = "Общие диалоги".to_string();

        for idx in graph.node_indices() {
            if let Some(node) = graph.get_node(idx) {
                match node {
                    ScenarioNode::Label(label_name) => {
                        current_scene_name = label_name.clone();
                    }
                    ScenarioNode::Dialogue(d) => {
                        let is_trans = d.translated_text.is_some() && !d.translated_text.as_ref().unwrap().trim().is_empty();
                        file_total += 1;
                        if is_trans {
                            file_translated += 1;
                        }

                        // Для строк интерфейса сцена всегда одна: "Интерфейс и меню (strings)"
                        let target_scene_name = if d.speaker.as_deref() == Some("UI / Choice") {
                            "Интерфейс и меню (strings)".to_string()
                        } else {
                            current_scene_name.clone()
                        };

                        // Находим существующую сцену с таким именем (чтобы реплики склеивались в одну сцену)
                        let scene_idx = if let Some(pos) = scenes.iter().position(|s| s.name == target_scene_name) {
                            pos
                        } else {
                            let new_id = format!("scene_{}", scenes.len());
                            scenes.push(SceneDto {
                                id: new_id,
                                name: target_scene_name.clone(),
                                total_lines: 0,
                                translated_lines: 0,
                                choices: Vec::new(),
                                jumps: Vec::new(),
                                entries: Vec::new(),
                            });
                            scenes.len() - 1
                        };

                        let scene = &mut scenes[scene_idx];
                        scene.total_lines += 1;
                        if is_trans {
                            scene.translated_lines += 1;
                        }

                        scene.entries.push(DialogueEntryDto {
                            id: d.id.clone(),
                            line_number: d.line_number,
                            speaker: d.speaker.clone(),
                            original_text: d.original_text.clone(),
                            translated_text: d.translated_text.clone(),
                            status: format!("{:?}", d.status),
                            scene_name: target_scene_name,
                        });
                    }
                    _ => {}
                }
            }
        }

        // Извлекаем граф ветвлений, выборов и переходов из оригинального скрипта игры
        let orig_path_str = file_path
            .replace(r"tl\russian\", "")
            .replace("tl/russian/", "");
        let orig_path = Path::new(&orig_path_str);

        let script_struct = if orig_path.exists() {
            if let Ok(orig_content) = std::fs::read_to_string(orig_path) {
                RenpyParser::parse_script_structure(&orig_content)
            } else {
                Default::default()
            }
        } else {
            Default::default()
        };

        // Привязываем найденные выборы (меню) и переходы к каждой сцене
        for scene in &mut scenes {
            if let Some(choices) = script_struct.scene_choices.get(&scene.name) {
                scene.choices = choices.clone();
            }
            if let Some(jumps) = script_struct.scene_jumps.get(&scene.name) {
                scene.jumps = jumps.iter().map(|(target, _)| target.clone()).collect();
            }
        }

        let mut edges = script_struct.edges;
        // Если переходов в файле не обнаружено, связываем сцены последовательно
        if edges.is_empty() && scenes.len() > 1 {
            for i in 0..scenes.len() - 1 {
                if scenes[i].name != "Интерфейс и меню (strings)" && scenes[i + 1].name != "Интерфейс и меню (strings)" {
                    edges.push(SceneEdgeDto {
                        from: scenes[i].name.clone(),
                        to: scenes[i + 1].name.clone(),
                        kind: "linear".to_string(),
                        condition: None,
                    });
                }
            }
        }

        Ok(ParsedFileDto {
            file_path: file_path.clone(),
            file_name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
            total_lines: file_total,
            translated_lines: file_translated,
            scenes,
            edges,
        })
    })
    .await
    .map_err(|e| format!("Ошибка задачи парсинга: {}", e))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTargetResult {
    pub file_name: String,
    pub short_name: String,
    pub file_path: String,
    pub original_path: String,
    pub line_number: usize,
    pub scene_name: String,
}

#[tauri::command]
async fn find_scene_target(
    game_dir: String,
    target_name: String,
) -> Result<Option<SceneTargetResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        if !base_path.exists() {
            return Err(format!("Путь к игре не существует: {}", game_dir));
        }

        let tl_dir = base_path.join(r"tl\russian");

        let mut script_files = Vec::new();
        collect_rpy_files(base_path, &mut script_files);

        // 1. Сначала ищем в оригинальных скриптах игры (быстрее всего)
        let orig_scripts: Vec<_> = script_files
            .iter()
            .filter(|p| {
                let s = p.to_string_lossy();
                !s.contains(r"\tl\") && !s.contains("/tl/")
            })
            .cloned()
            .collect();

        let pattern_label_colon = format!("label {}:", target_name);
        let pattern_label_space = format!("label {} ", target_name);
        let pattern_screen_colon = format!("screen {}:", target_name);
        let pattern_screen_paren = format!("screen {}(", target_name);

        let found = orig_scripts.into_par_iter().find_map_any(|orig_path| {
            if let Ok(content) = std::fs::read_to_string(&orig_path) {
                for (idx, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(&pattern_label_colon)
                        || trimmed.starts_with(&pattern_label_space)
                        || trimmed.starts_with(&pattern_screen_colon)
                        || trimmed.starts_with(&pattern_screen_paren)
                    {
                        let rel_path = orig_path.strip_prefix(base_path).unwrap_or(&orig_path);
                        let tl_path = tl_dir.join(rel_path);
                        let resolved_tl_path = if tl_path.exists() {
                            tl_path.to_string_lossy().to_string()
                        } else {
                            orig_path.to_string_lossy().to_string()
                        };

                        let file_name = rel_path.to_string_lossy().replace('\\', "/");
                        let short_name = orig_path.file_name().unwrap_or_default().to_string_lossy().to_string();

                        return Some(SceneTargetResult {
                            file_name,
                            short_name,
                            file_path: resolved_tl_path,
                            original_path: orig_path.to_string_lossy().to_string(),
                            line_number: idx + 1,
                            scene_name: target_name.clone(),
                        });
                    }
                }
            }
            None
        });

        if found.is_some() {
            return Ok(found);
        }

        // 2. Если не нашли в оригинале, ищем в tl/russian (по блокам translate russian <target>:)
        let tl_scripts: Vec<_> = script_files
            .into_iter()
            .filter(|p| {
                let s = p.to_string_lossy();
                s.contains(r"\tl\") || s.contains("/tl/")
            })
            .collect();

        let tl_prefix_hash = format!("translate russian {}_", target_name);
        let tl_prefix_colon = format!("translate russian {}:", target_name);

        let tl_found = tl_scripts.into_par_iter().find_map_any(|tl_path| {
            if let Ok(content) = std::fs::read_to_string(&tl_path) {
                for (idx, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(&tl_prefix_hash) || trimmed.starts_with(&tl_prefix_colon) {
                        let rel_path = tl_path.strip_prefix(&tl_dir).unwrap_or(&tl_path);
                        let file_name = rel_path.to_string_lossy().replace('\\', "/");
                        let short_name = tl_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let orig_path = base_path.join(rel_path);

                        return Some(SceneTargetResult {
                            file_name,
                            short_name,
                            file_path: tl_path.to_string_lossy().to_string(),
                            original_path: orig_path.to_string_lossy().to_string(),
                            line_number: idx + 1,
                            scene_name: target_name.clone(),
                        });
                    }
                }
            }
            None
        });

        Ok(tl_found)
    })
    .await
    .map_err(|e| format!("Ошибка задачи поиска сцены: {}", e))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, scan_game_project, parse_file_entries, find_scene_target])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_and_parse_story_rpy() {
        tauri::async_runtime::block_on(async {
            let game_dir = r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game";
            if Path::new(game_dir).exists() {
                let summary = scan_game_project(game_dir.to_string()).await.unwrap();
                println!("Найдены файлы ({} шт):", summary.files.len());
                // Проверяем, что в списке нет бэкапов!
                for f in &summary.files {
                    assert!(!f.name.contains(".bak"), "Файл бэкапа не должен попадать в проект: {}", f.name);
                    assert!(!f.path.contains(".codex-backups"), "Папка .codex-backups не должна сканироваться: {}", f.path);
                }

                let story_path = format!(r"{}\tl\russian\story.rpy", game_dir);
                if Path::new(&story_path).exists() {
                    let parsed = parse_file_entries(story_path).await.unwrap();
                    println!("story.rpy распарсен: {} сцен, всего строк: {}, переведено: {}", 
                        parsed.scenes.len(), parsed.total_lines, parsed.translated_lines);

                    for (i, s) in parsed.scenes.iter().enumerate().take(10) {
                        println!("Сцена #{}: {} ({} строк, переведено: {})", i, s.name, s.total_lines, s.translated_lines);
                    }

                    // Проверяем, что intro и vera_intro не фрагментированы на одиночные строки
                    let intro_scene = parsed.scenes.iter().find(|s| s.name == "intro");
                    assert!(intro_scene.is_some(), "Сцена intro должна существовать");
                    let intro_lines = intro_scene.unwrap().total_lines;
                    assert!(intro_lines >= 50, "Сцена intro должна содержать не менее 50 реплик, а не по 1 строке! Найдено: {}", intro_lines);

                    // Проверяем, что "Интерфейс и меню (strings)" содержит дедуплицированные строки
                    let ui_scene = parsed.scenes.iter().find(|s| s.name == "Интерфейс и меню (strings)");
                    assert!(ui_scene.is_some(), "Сцена UI строк должна существовать");
                    let ui_lines = ui_scene.unwrap().total_lines;
                    println!("Уникальных строк UI в story.rpy: {}", ui_lines);
                    assert!(ui_lines < 66, "Строки UI должны быть дедуплицированы, а не 66 дубликатов! Найдено: {}", ui_lines);
                    // Проверяем поиск целевой сцены "house" (должна найтись в script.rpy)
                    let found_house = find_scene_target(game_dir.to_string(), "house".to_string()).await.unwrap();
                    assert!(found_house.is_some(), "Целевая сцена 'house' должна быть найдена в проекте");
                    let house_loc = found_house.unwrap();
                    println!("Найдена сцена 'house': файл {}, строка {}", house_loc.file_name, house_loc.line_number);
                    assert_eq!(house_loc.file_name, "script.rpy", "Сцена 'house' должна находиться в script.rpy");

                    // Проверяем поиск сцены "PPF" в подпапке "script/locations/PPF HQ.rpy"
                    let found_ppf = find_scene_target(game_dir.to_string(), "PPF".to_string()).await.unwrap();
                    assert!(found_ppf.is_some(), "Сцена 'PPF' должна быть найдена");
                    let ppf_loc = found_ppf.unwrap();
                    println!("Найдена сцена 'PPF': file_name={}, short_name={}, line={}", ppf_loc.file_name, ppf_loc.short_name, ppf_loc.line_number);
                    assert_eq!(ppf_loc.file_name, "script/locations/PPF HQ.rpy");
                    assert_eq!(ppf_loc.short_name, "PPF HQ.rpy");
                    assert!(ppf_loc.file_path.contains(r"tl\russian\script\locations\PPF HQ.rpy"));
                }
            }
        });
    }
}
