//! Translator 2.0: проект не зависит от папки игры, данные лежат в SQLite.
//!
//! Слои и классы описаны на странице «Структура приложения» в Docmost, схемы баз в разделе «Хранилище 2.0».
//! Подключены команды списка проектов, остальные фичи ещё с телами `todo!()`.

// Снять, когда заглушки получат реализацию: пока реэкспорты и параметры заглушек никто не использует.
#![allow(dead_code, unused_variables, unused_imports)]

pub mod common;
pub mod engines;
pub mod export;
pub mod files;
pub mod fonts;
pub mod glossary;
pub mod graph;
pub mod llm;
pub mod name_rules;
pub mod projects;
pub mod scan;
pub mod secrets;
pub mod state;
pub mod storage;
pub mod translation;

use tauri::Manager;
use tauri_specta::{ErrorHandlingMode, collect_commands};

use crate::secrets::SecretStore;
use crate::state::AppState;

/// Куда пишутся типы и вызовы команд для окна.
const BINDINGS: &str = "../src/lib/bindings.ts";

/// Команды окна. Типы TypeScript к ним генерируются из этого же списка.
fn commands() -> tauri_specta::Builder<tauri::Wry> {
    use glossary::commands::*;
    use graph::commands::*;
    use llm::commands::*;
    use projects::commands::*;
    use translation::commands::*;
    tauri_specta::Builder::<tauri::Wry>::new()
        .error_handling(ErrorHandlingMode::Throw)
        .commands(collect_commands![
            llm_settings,
            save_llm_provider,
            delete_llm_provider,
            forget_llm_key,
            save_llm_profile,
            delete_llm_profile,
            reorder_llm_profiles,
            save_llm_action,
            llm_models,
            cancel_llm_run,
            llm_run_active,
            scene_graph,
            glossary,
            speaker_to_character,
            all_speakers_to_characters,
            link_speaker,
            unlink_speaker,
            keep_as_speaker,
            save_speaker_entry,
            delete_speaker_entry,
            discard_speaker,
            restore_speaker,
            create_character,
            save_character,
            delete_character,
            take_term,
            save_term,
            delete_term,
            give_term_to_character,
            discard_term_name,
            restore_term_name,
            new_terms,
            save_token,
            suggest_names,
            name_rules,
            save_name_rule,
            save_own_name_rule,
            draft_name_rule,
            suggest_terms,
            suggest_speakers,
            suggest_tokens,
            translation_files,
            file_lines,
            save_line,
            confirm_lines,
            translate_lines,
            review_lines,
            list_projects,
            find_projects,
            create_project,
            open_project,
            close_project,
            archive_project,
            unarchive_project,
            rename_project,
            unbind_project,
            delete_project,
        ])
}

fn typescript() -> specta_typescript::Typescript {
    specta_typescript::Typescript::default()
        .header("// Сгенерировано из Rust (tauri-specta), руками не править: cargo test --lib export_bindings")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = commands();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let data_dir = app.path().app_local_data_dir()?;
            let state = AppState::start(data_dir, SecretStore::detect())?;
            app.manage(state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    /// Пересобирает `src/lib/bindings.ts`. Изменения файла видны в git: их коммитят вместе с командами.
    #[test]
    fn export_bindings() {
        super::commands().export(super::typescript(), super::BINDINGS).expect("типы для окна");
    }
}
