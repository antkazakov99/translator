pub mod glossary;
pub mod graph;
pub mod parser;
pub mod pickle;
pub mod project;
pub mod rpa;
pub mod rpyc;

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

            // Игнорируем скрытые папки (.codex-backups, .editor-backups, .git), папки с бэкапами и сторонние инжекторы _mods
            if file_name.starts_with('.') || file_name == "_mods" || file_name.contains("backup") || file_name.contains("tmp") {
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

/// Проверяет наличие архивов `.rpa` в директории игры
fn has_rpa_archives(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("rpa") {
                return true;
            }
        }
    }
    false
}

/// Возвращает путь к изолированной папке кэша в AppData для конкретной игры,
/// чтобы оригинальная папка игры никогда не засорялась распакованными файлами.
pub fn get_project_cache_dir(game_dir: &Path) -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    game_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    let hash = format!("{:012x}", hasher.finish());

    let folder_name = game_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "game".to_string());

    local_app_data
        .join("TranslatorApp")
        .join("projects_cache")
        .join(format!("{}_{}", folder_name, hash))
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
    pub speaker_alias: Option<String>,
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

        // 2. Если в папке игры есть архивы .rpa — распаковываем ТОЛЬКО скрипты в изолированный AppData кэш
        let cache_dir = get_project_cache_dir(base_path);
        if has_rpa_archives(base_path) {
            let _ = rpa::RpaArchive::extract_scripts_from_game_archives(base_path, &cache_dir);
            collect_rpy_files(&cache_dir, &mut script_files);
        }

        let mut characters_map: std::collections::HashMap<String, String> = script_files
            .par_iter()
            .filter_map(|path| {
                let path_str = path.to_string_lossy();
                if !path_str.contains(r"\tl\") && !path_str.contains("/tl/") {
                    std::fs::read_to_string(path).ok().map(|content| RenpyParser::extract_speakers(&content))
                } else {
                    None
                }
            })
            .reduce(std::collections::HashMap::new, |mut acc, map| {
                acc.extend(map);
                acc
            });

        // Подставляем имя главного героя вместо шаблонных переменных вроде [player.name]
        if let Some(player_name) = characters_map.get("player").cloned() {
            for (_, v) in characters_map.iter_mut() {
                if v == "[player.name]" || v == "[player]" {
                    *v = player_name.clone();
                }
            }
        }

        let mut characters: Vec<(String, String)> = characters_map.into_iter().collect();
        characters.sort_by(|a, b| a.1.cmp(&b.1));

        // 3. Ищем файлы переводов в game/tl/russian в параллельных потоках (сверхбыстрый подсчет строк)
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
                    if total == 0 {
                        return None; // Фильтруем пустые файлы перевода без строк диалогов!
                    }
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

        // 4. Если в tl/russian пусто (новый перевод), но в кэше есть скрипты из RPA — выводим их как базу для перевода
        if files.is_empty() && cache_dir.exists() {
            let mut cached_files = Vec::new();
            collect_rpy_files(&cache_dir, &mut cached_files);

            let mut parsed_cached: Vec<TranslationFileItem> = cached_files
                .into_par_iter()
                .filter_map(|path| {
                    let content = std::fs::read_to_string(&path).ok()?;
                    let size_kb = content.len() as f64 / 1024.0;
                    let (total, translated) = RenpyParser::quick_translation_stats(&content);
                    if total == 0 {
                        return None; // Фильтруем пустые скрипты (анимации, внутренние переменные, спеллы)!
                    }
                    let relative_name = path
                        .strip_prefix(&cache_dir)
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

            for f in &parsed_cached {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_cached.sort_by(|a, b| b.size_kb.partial_cmp(&a.size_kb).unwrap_or(std::cmp::Ordering::Equal));
            files = parsed_cached;
        }

        // 5. Если в tl/russian пусто и нет RPA, но есть распакованные .rpy в папке игры
        if files.is_empty() && !script_files.is_empty() {
            let mut parsed_loose: Vec<TranslationFileItem> = script_files
                .into_par_iter()
                .filter(|p| !p.starts_with(&cache_dir))
                .filter_map(|path| {
                    let content = std::fs::read_to_string(&path).ok()?;
                    let size_kb = content.len() as f64 / 1024.0;
                    let (total, translated) = RenpyParser::quick_translation_stats(&content);
                    if total == 0 {
                        return None; // Фильтруем скрипты без диалогов
                    }
                    let relative_name = path
                        .strip_prefix(base_path)
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

            for f in &parsed_loose {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_loose.sort_by(|a, b| b.size_kb.partial_cmp(&a.size_kb).unwrap_or(std::cmp::Ordering::Equal));
            files = parsed_loose;
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

        let mut speakers_map = std::collections::HashMap::new();
        if let Some(parent) = path.parent() {
            let mut search_dir = parent.to_path_buf();
            while search_dir.parent().is_some() && (search_dir.ends_with(r"tl\russian") || search_dir.ends_with("tl/russian")) {
                search_dir = search_dir.parent().unwrap().to_path_buf();
            }
            let mut check_files = Vec::new();
            collect_rpy_files(&search_dir, &mut check_files);
            for f in check_files.into_iter().take(40) {
                if let Ok(c) = std::fs::read_to_string(&f) {
                    speakers_map.extend(RenpyParser::extract_speakers(&c));
                }
            }
        }

        let parser = RenpyParser::new().with_speakers(speakers_map);
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
                            speaker_alias: d.speaker_alias.clone(),
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
        } else if let Ok(content) = std::fs::read_to_string(&file_path) {
            // Если сам файл уже является оригинальным скриптом (например из AppData кэша)
            RenpyParser::parse_script_structure(&content)
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
        let cache_dir = get_project_cache_dir(base_path);

        let mut script_files = Vec::new();
        collect_rpy_files(base_path, &mut script_files);
        if cache_dir.exists() {
            collect_rpy_files(&cache_dir, &mut script_files);
        }

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
                        let rel_path = if orig_path.starts_with(&cache_dir) {
                            orig_path.strip_prefix(&cache_dir).unwrap_or(&orig_path)
                        } else {
                            orig_path.strip_prefix(base_path).unwrap_or(&orig_path)
                        };
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

#[tauri::command]
async fn pack_mod_rpa(
    game_dir: String,
    output_rpa_path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        let tl_dir = base_path.join(r"tl\russian");
        if !tl_dir.exists() {
            return Err("Папка перевода tl/russian не найдена в директории игры".to_string());
        }

        let out_path = if let Some(ref p) = output_rpa_path {
            PathBuf::from(p)
        } else {
            base_path.join("russian_mod.rpa")
        };

        let count = rpa::RpaArchive::pack_mod_from_folder(&tl_dir, &out_path)?;
        Ok(format!(
            "Мод успешно собран! Упаковано {} файлов в {}",
            count,
            out_path.to_string_lossy()
        ))
    })
    .await
    .map_err(|e| format!("Ошибка сборки RPA мода: {}", e))?
}

#[tauri::command]
async fn unpack_rpa(rpa_path: String, target_dir: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let src = Path::new(&rpa_path);
        let dst = Path::new(&target_dir);
        rpa::RpaArchive::extract_all(src, dst)
    })
    .await
    .map_err(|e| format!("Ошибка распаковки RPA: {}", e))?
}

#[tauri::command]
async fn clean_rpyc_cache(game_dir: String, tl_only: bool) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        rpyc::RpycParser::clean_cache(base_path, tl_only)
    })
    .await
    .map_err(|e| format!("Ошибка очистки кэша .rpyc: {}", e))?
}

#[tauri::command]
async fn decompile_rpyc_file(
    rpyc_path: String,
    output_rpy_path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&rpyc_path);
        let stmts = rpyc::RpycParser::parse_rpyc_file(path)?;
        let rpy_text = rpyc::RpycParser::decompile_to_rpy_text(&stmts);

        if let Some(ref out_p) = output_rpy_path {
            let out_path = Path::new(out_p);
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(out_path, &rpy_text)
                .map_err(|e| format!("Не удалось сохранить декомпилированный .rpy: {}", e))?;
        }

        Ok(rpy_text)
    })
    .await
    .map_err(|e| format!("Ошибка декомпиляции: {}", e))?
}

#[tauri::command]
async fn get_project_glossary(
    game_dir: String,
    scanned_characters: Option<Vec<(String, String)>>,
) -> Result<glossary::ProjectGlossary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        let mut gl = glossary::ProjectGlossary::load_for_project(base_path);
        if let Some(ref chars) = scanned_characters {
            gl.merge_with_scanned_characters(chars);
        }
        Ok(gl)
    })
    .await
    .map_err(|e| format!("Ошибка загрузки глоссария: {}", e))?
}

#[tauri::command]
async fn save_project_glossary(
    game_dir: String,
    glossary: glossary::ProjectGlossary,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = Path::new(&game_dir);
        let saved_path = glossary.save_for_project(base_path)?;
        Ok(format!("Глоссарий сохранен ({:?})", saved_path.file_name().unwrap_or_default()))
    })
    .await
    .map_err(|e| format!("Ошибка сохранения глоссария: {}", e))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            scan_game_project,
            parse_file_entries,
            find_scene_target,
            pack_mod_rpa,
            unpack_rpa,
            clean_rpyc_cache,
            decompile_rpyc_file,
            get_project_glossary,
            save_project_glossary
        ])
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

    #[test]
    fn test_rpa_and_rpyc_tauri_commands() {
        tauri::async_runtime::block_on(async {
            let game_dir = r"F:\Playnite\Games\0b588a07-1844-467f-8459-e5629642a1bf\Installed\game";
            if !Path::new(game_dir).exists() {
                return;
            }

            // 1. Тест декомпиляции celia.rpyc через команду decompile_rpyc_file
            let celia_rpyc = format!(r"{}\celia.rpyc", game_dir);
            let decompiled = decompile_rpyc_file(celia_rpyc, None).await.unwrap();
            assert!(decompiled.contains("label end_scene_nei:"));
            println!("Команда decompile_rpyc_file успешно декомпилировала celia.rpyc!");

            // 2. Тест упаковки мода в RPA архив во временный файл
            let temp_rpa = std::env::temp_dir().join("test_command_mod.rpa");
            let result_msg = pack_mod_rpa(game_dir.to_string(), Some(temp_rpa.to_string_lossy().to_string()))
                .await
                .unwrap();
            println!("Результат pack_mod_rpa: {}", result_msg);
            assert!(temp_rpa.exists(), "Архив мода должен быть создан");

            // 3. Тест распаковки созданного мода через unpack_rpa
            let temp_unpack_dir = std::env::temp_dir().join("test_command_unpacked");
            let unpacked_count = unpack_rpa(
                temp_rpa.to_string_lossy().to_string(),
                temp_unpack_dir.to_string_lossy().to_string(),
            )
            .await
            .unwrap();
            println!("Распаковано файлов мода: {}", unpacked_count);
            assert!(unpacked_count > 0);

            // Очищаем временные файлы
            let _ = std::fs::remove_file(&temp_rpa);
            let _ = std::fs::remove_dir_all(&temp_unpack_dir);
        });
    }

    #[test]
    fn test_scan_packed_game_of_whores() {
        tauri::async_runtime::block_on(async {
            let game_dir = r"F:\Games\Game of Whores\Game of Whores (0.34)\game";
            if !Path::new(game_dir).exists() {
                return;
            }

            let start = std::time::Instant::now();
            let summary = scan_game_project(game_dir.to_string()).await.unwrap();
            println!(
                "Game of Whores отсканирована за {:?}! Найдено файлов: {}, всего строк: {}, персонажей: {}",
                start.elapsed(), summary.files.len(), summary.total_lines, summary.characters.len()
            );

            assert!(!summary.files.is_empty(), "Должны быть обнаружены файлы из archive.rpa");
            assert!(summary.total_lines > 0, "Должны быть подсчитаны строки диалогов");

            for f in summary.files.iter().take(10) {
                println!("  [Файл] {} ({:.1} КБ, строк: {})", f.name, f.size_kb, f.total_lines);
            }

            // Проверяем парсинг одного из файлов
            let test_file = &summary.files[0];
            let parsed = parse_file_entries(test_file.path.clone()).await.unwrap();
            println!(
                "Скрипт {} распарсен: {} сцен, {} реплик",
                parsed.file_name, parsed.scenes.len(), parsed.total_lines
            );
            assert!(!parsed.scenes.is_empty(), "Сцены должны быть сформированы");

            // Проверяем, что в самой папке игры НЕ появилось распакованных .rpy файлов!
            assert!(!Path::new(game_dir).join("bquest1.rpy").exists(), "Архив не должен распаковываться в папку игры");
            assert!(!Path::new(game_dir).join("mgquest1.rpy").exists(), "Архив не должен распаковываться в папку игры");
            println!("Папка игры чиста: 0 файлов распаковано в корень игры!");
        });
    }

    #[test]
    fn test_scan_lust_and_power() {
        tauri::async_runtime::block_on(async {
            let game_dir = r"F:\Games\Lust and Power\Lust and Power (0.75)\game";
            if !Path::new(game_dir).exists() {
                return;
            }

            let start = std::time::Instant::now();
            let summary = scan_game_project(game_dir.to_string()).await.unwrap();
            println!(
                "Lust and Power отсканирована за {:?}! Файлов: {}, всего строк: {}, персонажей: {}",
                start.elapsed(), summary.files.len(), summary.total_lines, summary.characters.len()
            );

            let empty_count = summary.files.iter().filter(|f| f.total_lines == 0).count();
            println!("Файлов с 0 строк диалогов: {}", empty_count);
            assert_eq!(empty_count, 0, "Все пустые файлы с 0 строк диалогов должны быть отфильтрованы!");

            println!("Извлеченные персонажи (всего {}):", summary.characters.len());
            for (alias, name) in summary.characters.iter().take(25) {
                println!("  [{}] -> {}", alias, name);
            }
            assert!(!summary.characters.is_empty(), "Персонажи должны быть успешно извлечены!");
            assert!(summary.characters.iter().any(|(a, n)| a == "lis" && n == "Lisa"), "Lisa должна быть найдена");
            assert!(summary.characters.iter().any(|(a, n)| a == "emm" && n == "Emma"), "Emma должна быть найдена");
        });
    }

    #[test]
    fn test_glossary_commands() {
        tauri::async_runtime::block_on(async {
            let unique_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let temp_game_dir = std::env::temp_dir().join(format!("test_glossary_game_{}", unique_id));
            let _ = std::fs::create_dir_all(&temp_game_dir);

            let scanned = vec![
                ("lis".to_string(), "Lisa".to_string()),
                ("dh".to_string(), "The Demon Hunter".to_string()),
            ];

            let mut gl = get_project_glossary(
                temp_game_dir.to_string_lossy().to_string(),
                Some(scanned),
            ).await.unwrap();

            assert_eq!(gl.characters.len(), 2);
            let dh = gl.characters.iter_mut().find(|c| c.alias == "dh").unwrap();
            dh.translated_name = "Охотник на демонов".to_string();

            gl.terms.push(glossary::TermGlossaryEntry {
                id: "term_1".to_string(),
                source_term: "Demon Blade".to_string(),
                translated_term: "Клинок демона".to_string(),
                category: "item".to_string(),
                notes: "Реликвия".to_string(),
            });

            let save_msg = save_project_glossary(
                temp_game_dir.to_string_lossy().to_string(),
                gl,
            ).await.unwrap();
            println!("{}", save_msg);

            // Загружаем повторно и сверяем сохраненные данные
            let reloaded = get_project_glossary(
                temp_game_dir.to_string_lossy().to_string(),
                None,
            ).await.unwrap();

            assert_eq!(reloaded.characters.len(), 2);
            assert_eq!(reloaded.terms.len(), 1);
            assert_eq!(reloaded.terms[0].translated_term, "Клинок демона");

            let _ = std::fs::remove_dir_all(&temp_game_dir);
            let _ = std::fs::remove_dir_all(get_project_cache_dir(&temp_game_dir));
        });
    }
}
