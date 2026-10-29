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
fn parse_file_entries(file_path: String) -> Result<Vec<DialogueEntryDto>, String> {
    let path = Path::new(&file_path);
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

    let parser = RenpyParser::new();
    let graph = parser.parse_file(path, &content)
        .map_err(|e| format!("Ошибка парсинга: {}", e))?;

    let mut entries = Vec::new();
    for idx in graph.node_indices() {
        if let Some(ScenarioNode::Dialogue(d)) = graph.get_node(idx) {
            entries.push(DialogueEntryDto {
                id: d.id.clone(),
                line_number: d.line_number,
                speaker: d.speaker.clone(),
                original_text: d.original_text.clone(),
                translated_text: d.translated_text.clone(),
                status: format!("{:?}", d.status),
            });
        }
    }

    Ok(entries)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, scan_game_project, parse_file_entries])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
