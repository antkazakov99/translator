//! Translator 2.0: проект не зависит от папки игры, данные лежат в SQLite.
//!
//! Слои и классы описаны на странице «Структура приложения» в Docmost, схемы баз в разделе «Хранилище 2.0».
//! Команды Tauri ещё не подключены. Тела с `todo!()` ждут своей очереди.

// Снять вместе с подключением команд: пока реэкспорты и параметры заглушек никто не использует.
#![allow(dead_code, unused_variables, unused_imports)]

pub mod common;
pub mod engines;
pub mod export;
pub mod files;
pub mod fonts;
pub mod glossary;
pub mod llm;
pub mod name_rules;
pub mod projects;
pub mod scan;
pub mod secrets;
pub mod state;
pub mod storage;
pub mod translation;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
