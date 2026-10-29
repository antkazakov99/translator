pub mod graph;
pub mod parser;
pub mod project;

use std::path::Path;
use serde::{Deserialize, Serialize};
use crate::parser::renpy::RenpyParser;
use crate::parser::GameParser;
use crate::graph::ScenarioNode;

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
    pub entries: Vec<DialogueEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFileDto {
    pub file_path: String,
    pub file_name: String,
    pub total_lines: usize,
    pub translated_lines: usize,
    pub scenes: Vec<SceneDto>,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Привет, {}! Мост между Svelte 5 и Rust работает идеально!", name)
}

#[tauri::command]
fn scan_game_project(game_dir: String) -> Result<ProjectSummary, String> {
    let base_path = Path::new(&game_dir);
    if !base_path.exists() {
        return Err(format!("Папка игры не найдена: {}", game_dir));
    }

    // 1. Ищем персонажей в оригинальных скриптах
    let mut characters_map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(base_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpy") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    characters_map.extend(RenpyParser::extract_speakers(&content));
                }
            }
        }
    }

    let mut characters: Vec<(String, String)> = characters_map.into_iter().collect();
    characters.sort_by(|a, b| a.1.cmp(&b.1));

    // 2. Ищем файлы переводов в game/tl/russian
    let tl_dir = base_path.join(r"tl\russian");
    let mut files = Vec::new();
    let mut grand_total = 0;
    let mut grand_translated = 0;

    let parser = RenpyParser::new();

    if tl_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&tl_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rpy") {
                    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let size_kb = content.len() as f64 / 1024.0;
                        let (total, translated) = if let Ok(graph) = parser.parse_file(&path, &content) {
                            graph.translation_stats()
                        } else {
                            (0, 0)
                        };

                        grand_total += total;
                        grand_translated += translated;

                        files.push(TranslationFileItem {
                            name,
                            path: path.to_string_lossy().to_string(),
                            size_kb,
                            total_lines: total,
                            translated_lines: translated,
                        });
                    }
                }
            }
        }
    }

    files.sort_by(|a, b| b.size_kb.partial_cmp(&a.size_kb).unwrap_or(std::cmp::Ordering::Equal));

    Ok(ProjectSummary {
        game_name: base_path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        characters,
        files,
        total_lines: grand_total,
        total_translated: grand_translated,
    })
}

#[tauri::command]
fn parse_file_entries(file_path: String) -> Result<ParsedFileDto, String> {
    let path = Path::new(&file_path);
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

    let parser = RenpyParser::new();
    let graph = parser.parse_file(path, &content)
        .map_err(|e| format!("Ошибка парсинга: {}", e))?;

    let mut scenes: Vec<SceneDto> = Vec::new();
    let mut current_scene = SceneDto {
        id: "default_scene".to_string(),
        name: "Общие реплики".to_string(),
        total_lines: 0,
        translated_lines: 0,
        entries: Vec::new(),
    };

    let mut file_total = 0;
    let mut file_translated = 0;

    for idx in graph.node_indices() {
        if let Some(node) = graph.get_node(idx) {
            match node {
                ScenarioNode::Label(label_name) => {
                    // Если в текущей сцене уже были реплики, сохраняем её
                    if !current_scene.entries.is_empty() {
                        scenes.push(current_scene);
                    }
                    current_scene = SceneDto {
                        id: format!("scene_{}", label_name),
                        name: label_name.clone(),
                        total_lines: 0,
                        translated_lines: 0,
                        entries: Vec::new(),
                    };
                }
                ScenarioNode::Dialogue(d) => {
                    let is_trans = d.translated_text.is_some() && !d.translated_text.as_ref().unwrap().trim().is_empty();
                    
                    file_total += 1;
                    current_scene.total_lines += 1;
                    if is_trans {
                        file_translated += 1;
                        current_scene.translated_lines += 1;
                    }

                    // Если встретили реплики интерфейса, сгруппируем их красиво
                    if d.speaker.as_deref() == Some("UI / Choice") && current_scene.id == "default_scene" {
                        current_scene.name = "Интерфейс и выборы (strings)".to_string();
                    }

                    current_scene.entries.push(DialogueEntryDto {
                        id: d.id.clone(),
                        line_number: d.line_number,
                        speaker: d.speaker.clone(),
                        original_text: d.original_text.clone(),
                        translated_text: d.translated_text.clone(),
                        status: format!("{:?}", d.status),
                        scene_name: current_scene.name.clone(),
                    });
                }
                _ => {}
            }
        }
    }

    if !current_scene.entries.is_empty() {
        scenes.push(current_scene);
    }

    Ok(ParsedFileDto {
        file_path: file_path.clone(),
        file_name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        total_lines: file_total,
        translated_lines: file_translated,
        scenes,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, scan_game_project, parse_file_entries])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
