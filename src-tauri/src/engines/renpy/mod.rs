//! Адаптер Ren'Py: архивы RPA, декомпиляция и компиляция `.rpyc`, разбор скриптов, считывание игры.

pub mod fonts;
pub mod graph;
pub mod modpack;
pub mod pickle;
pub mod rpa;
pub mod rpyc;
pub mod rpyc_writer;
pub mod scenes;
pub mod script;
pub mod snapshot;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use regex::Regex;

use super::{
    EngineError, EngineFont, EngineId, FileKind, GameEngine, GameInfo, GameSnapshot, GameSource, ModArchive, ModInput,
    SnapshotEdge, SnapshotFile, SourceText,
};
use crate::files::FileStore;
use crate::projects::GameRoot;

/// Ren'Py. Структура без полей: состояния у движка нет.
pub struct RenpyEngine;

/// Версия разбора: заменяет `CACHE_VERSION` из `rpa.rs`. Выросла: снимок пересобирается из `files/`.
const PARSER_VERSION: u32 = 1;

/// Версия сборки переходов между метками (`scenes::scene_edges`). Выросла: таблица `edges` пересобирается.
const EDGES_VERSION: u32 = 1;

/// Версия движка в `renpy/vc_version.py`: `version = '8.1.3.23091805'`.
static RE_ENGINE_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^version\s*=\s*u?['"]([^'"]+)['"]"#).expect("регулярное выражение версии"));

impl GameEngine for RenpyEngine {
    fn id(&self) -> EngineId {
        EngineId::Renpy
    }

    fn parser_version(&self) -> u32 {
        PARSER_VERSION
    }

    /// Игра Ren'Py: в корне есть `game/` и файлы движка `renpy/`. Формат `.rpyc` запоминается
    /// сразу: без папки игры мод собирается по нему.
    fn detect(&self, root: &GameRoot) -> Option<GameInfo> {
        let game = root.as_path().join(snapshot::GAME_DIR);
        let engine = root.as_path().join("renpy");
        if !game.is_dir() || !engine.is_dir() {
            return None;
        }
        let mut details = serde_json::Map::new();
        match rpyc_writer::ScriptFormat::detect(&game) {
            Ok(format) => {
                details.insert("script_version".to_string(), format.script_version.into());
                details.insert("ast_layout".to_string(), format!("{:?}", format.layout).into());
            }
            Err(error) => tracing::warn!("Формат скриптов не определён: {error}"),
        }
        let version = std::fs::read_to_string(engine.join("vc_version.py"))
            .ok()
            .and_then(|text| RE_ENGINE_VERSION.captures(&text).map(|caps| caps[1].to_string()));
        Some(GameInfo { engine: EngineId::Renpy, version, details: serde_json::Value::Object(details) })
    }

    /// Скрипты, переводы `tl/`, картинки перевода и шрифты, в том числе из `.rpa`.
    /// Ключи контейнеров и реплик уникальны во всей игре: `label:` и `screen:`, путь файла у строк `old`.
    fn read(&self, source: &GameSource, info: &GameInfo, files: &FileStore) -> Result<GameSnapshot, EngineError> {
        let collected = match source {
            GameSource::Folder(root) => snapshot::collect_folder(root, files)?,
            GameSource::Stored(stored) => stored.clone(),
        };
        snapshot::analyze(collected, files, info)
    }

    fn edges_version(&self) -> u32 {
        EDGES_VERSION
    }

    /// Переходы `jump`, `call`, по условию, из пунктов меню и через кнопки экранов. Нужны только скрипты.
    fn read_edges(&self, files: &[SnapshotFile], store: &FileStore) -> Result<Vec<SnapshotEdge>, EngineError> {
        snapshot::read_edges(files, store)
    }

    /// Файл перевода в `.rpyc` восстанавливается по репликам оригинала, найденным по `id` в скомпилированных скриптах.
    fn parse_dependencies(&self, file: &SnapshotFile, all: &[SnapshotFile]) -> Vec<String> {
        if file.kind != FileKind::Translation || !file.path.ends_with(".rpyc") {
            return Vec::new();
        }
        all.iter()
            .filter(|other| other.kind == FileKind::Script && other.path.ends_with(".rpyc"))
            .map(|other| other.path.clone())
            .collect()
    }

    /// Шрифты вне `tl/`. Замена ляжет в `tl/<язык>` мода под тем же путём.
    /// `.rpy` как есть, `.rpyc` декомпилирован тем же путём, что при считывании.
    fn source_text(&self, file: &SnapshotFile, files: &[SnapshotFile], store: &FileStore) -> Result<SourceText, EngineError> {
        snapshot::source_text(file, files, store)
    }

    fn fonts(&self, files: &[SnapshotFile], store: &FileStore) -> Result<Vec<EngineFont>, EngineError> {
        fonts::game_fonts(files, store)
    }

    /// `ru_lang.rpa`: переводы `tl/<язык>`, строковые переводы, подписи говорящих, включение языка, шрифты.
    fn build_mod(&self, input: &ModInput, files: &FileStore, out_dir: &Path) -> Result<ModArchive, EngineError> {
        modpack::build(input, files, out_dir)
    }

    /// Атомарно заменяет `game/ru_lang.rpa`, не трогая `tl/` и оригиналы.
    fn install(&self, root: &GameRoot, archive: &ModArchive) -> Result<(), EngineError> {
        modpack::install(root, archive)
    }

    fn uninstall(&self, root: &GameRoot) -> Result<(), EngineError> {
        modpack::uninstall(root)
    }
}

/// Язык перевода: имя папки `tl/<язык>` и блоков `translate <язык>`.
pub(crate) const TL_LANG: &str = "russian";

/// Стабильный FNV-1a хэш: одинаковый между запусками и версиями компилятора, в отличие от `DefaultHasher`.
pub fn stable_hash_str(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

static PATH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Блокировка на путь: сериализует запись одного файла или папки, не мешая остальным.
pub(crate) fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    PATH_LOCKS.lock().entry(path.to_path_buf()).or_default().clone()
}

/// Максимальная глубина рекурсивного обхода каталогов скриптов.
pub(crate) const MAX_SCAN_DEPTH: usize = 20;

/// Все файлы `.rpy` без скрытых папок, `_mods` и бэкапов.
#[cfg(test)]
pub(crate) fn collect_rpy_files(dir: &Path, files: &mut Vec<PathBuf>) {
    collect_files_depth(dir, Some(&["rpy"]), files, 0);
}

/// То же для исходных и скомпилированных скриптов.
pub(crate) fn collect_script_files(dir: &Path, files: &mut Vec<PathBuf>) {
    collect_files_depth(dir, Some(&["rpy", "rpyc"]), files, 0);
}

/// Все файлы папки с теми же исключениями. Что из них нужно, решает считывание.
pub(crate) fn collect_game_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_depth(dir, None, &mut files, 0);
    files.sort();
    files
}

fn collect_files_depth(dir: &Path, extensions: Option<&[&str]>, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        // Скрытые папки (.git, .editor-backups), бэкапы и сторонние инжекторы _mods.
        if file_name.starts_with('.') || file_name == "_mods" || file_name.contains("backup") || file_name.contains("tmp") {
            continue;
        }
        if path.is_dir() {
            collect_files_depth(&path, extensions, files, depth + 1);
        } else if extensions.is_none_or(|extensions| {
            path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e))
        }) && !file_name.contains(".bak")
        {
            files.push(path);
        }
    }
}

/// Игра с папкой `tl/russian` и несжатыми `.rpy` (папка `game`).
#[cfg(test)]
pub(crate) const TEST_TRANSLATED_GAME: &str = "TRANSLATOR_TEST_TRANSLATED_GAME";
/// Игра с `archive.rpa` и файлом `brothel_keyhole.rpyc` внутри (папка `game`).
#[cfg(test)]
pub(crate) const TEST_RPA_GAME: &str = "TRANSLATOR_TEST_RPA_GAME";
/// Игра с персонажами в `___sys/_vars/*.rpyc` внутри `archive.rpa` (папка `game`).
#[cfg(test)]
pub(crate) const TEST_CHARACTERS_GAME: &str = "TRANSLATOR_TEST_CHARACTERS_GAME";
/// Путь к файлу архива формата RPA-2.0.
#[cfg(test)]
pub(crate) const TEST_RPA2_ARCHIVE: &str = "TRANSLATOR_TEST_RPA2_ARCHIVE";
/// Путь к файлу архива с индексом в pickle протокола 4 или новее.
#[cfg(test)]
pub(crate) const TEST_FRAMED_ARCHIVE: &str = "TRANSLATOR_TEST_FRAMED_ARCHIVE";
/// Папка `tl/<язык>` реальной игры для проверки компилятора.
#[cfg(test)]
pub(crate) const TEST_TL_DIR: &str = "TRANSLATOR_TEST_TL_DIR";
/// Папка `game` игры с переводом внутри архивов RPA.
#[cfg(test)]
pub(crate) const TEST_PACKED_TL_GAME: &str = "TRANSLATOR_TEST_PACKED_TL_GAME";

/// Папка игры для тестов на реальных данных: `cargo test -- --ignored` с заданной переменной окружения.
/// Без неё такие тесты ничего не проверяют.
#[cfg(test)]
pub(crate) fn local_test_game(env_var: &str) -> Option<String> {
    std::env::var(env_var).ok().filter(|path| Path::new(path).exists())
}

/// Скрипт мода с переключателем языка (`translator_language.rpy`).
pub(crate) fn language_script() -> String {
    include_str!("translator_language.rpy").replace("__LANG__", TL_LANG)
}
