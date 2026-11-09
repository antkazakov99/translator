//! Шрифты игры для замены. Движок ищет любой файл сначала в `tl/<язык>`, поэтому шрифт с тем же
//! именем в архиве мода заменяет исходный и в настройках, и в тегах `{font=...}`, причём только
//! для языка перевода. Файлы игры не меняются.

use std::collections::{HashMap, HashSet};

use regex::bytes::RegexBuilder;

use super::TL_LANG;
use super::rpyc::RpycParser;
use super::snapshot::GAME_DIR;
use crate::engines::{EngineError, EngineFont, FileKind, SnapshotFile};
use crate::files::FileStore;

/// Шрифты игры вне `tl/`: упоминания в скриптах и причина, по которой замена не подействует.
pub(crate) fn game_fonts(files: &[SnapshotFile], store: &FileStore) -> Result<Vec<EngineFont>, EngineError> {
    let game_prefix = format!("{GAME_DIR}/");
    let tl_prefix = format!("{GAME_DIR}/tl/{TL_LANG}/");
    let translated: HashSet<String> = files
        .iter()
        .filter(|file| file.kind == FileKind::Font)
        .filter_map(|file| file.path.strip_prefix(&tl_prefix))
        .map(str::to_lowercase)
        .collect();
    let fonts: Vec<&SnapshotFile> = files
        .iter()
        .filter(|file| file.kind == FileKind::Font && file.path.starts_with(&game_prefix) && !file.path.starts_with(&format!("{GAME_DIR}/tl/")))
        .collect();
    let mentions = count_mentions(files, store, &fonts);
    Ok(fonts
        .into_iter()
        .map(|file| {
            let relative = file.path.strip_prefix(&game_prefix).unwrap_or(&file.path);
            let locked = translated
                .contains(&relative.to_lowercase())
                .then(|| format!("В игре уже есть свой шрифт перевода tl/{TL_LANG}/{relative}: он важнее мода"));
            EngineFont { path: file.path.clone(), mentions: mentions.get(&base_name(&file.path)).copied().unwrap_or(0), locked }
        })
        .collect())
}

/// Путь шрифта в архиве мода: `tl/<язык>/` и путь относительно `game/`. `None`: шрифт не из `game/`.
pub(crate) fn mod_path(target_path: &str) -> Option<String> {
    let relative = target_path.strip_prefix(&format!("{GAME_DIR}/"))?;
    (!relative.starts_with("tl/")).then(|| format!("tl/{TL_LANG}/{relative}"))
}

/// Имя файла без папок, в нижнем регистре: так скрипты ссылаются на шрифт чаще всего.
fn base_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_lowercase()
}

/// Сколько скриптов упоминают каждый шрифт по имени файла. Имена ищутся и в скомпилированных файлах:
/// декомпилятор оставляет от экранов только связи, а шрифты задаются именно там.
/// Имя, собранное в коде, поиск не найдёт. Битый скрипт пропускается.
fn count_mentions(files: &[SnapshotFile], store: &FileStore, fonts: &[&SnapshotFile]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let mut bases: Vec<String> = fonts.iter().map(|file| base_name(&file.path)).collect::<HashSet<_>>().into_iter().collect();
    if bases.is_empty() {
        return counts;
    }
    // Длинные имена первыми: `a.ttf` не должно найтись внутри `aa.ttf`.
    bases.sort_by_key(|base| std::cmp::Reverse(base.len()));
    let pattern = bases.iter().map(|base| regex::escape(base)).collect::<Vec<_>>().join("|");
    let Ok(pattern) = RegexBuilder::new(&pattern).case_insensitive(true).build() else { return counts };

    for file in files.iter().filter(|file| matches!(file.kind, FileKind::Script | FileKind::Translation)) {
        let Ok(data) = store.read(&file.hash) else { continue };
        let text = if file.path.ends_with(".rpyc") { compiled_text(&data) } else { data };
        let found: HashSet<String> =
            pattern.find_iter(&text).map(|hit| String::from_utf8_lossy(hit.as_bytes()).to_lowercase()).collect();
        for base in found {
            *counts.entry(base).or_default() += 1;
        }
    }
    counts
}

/// Распакованные слоты `.rpyc`: строки pickle в них лежат как есть.
fn compiled_text(data: &[u8]) -> Vec<u8> {
    let Ok(slots) = RpycParser::read_slots(data) else { return Vec::new() };
    slots.iter().filter_map(|slot| RpycParser::decompress_slot(data, slot).ok()).flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(store: &FileStore, path: &str, kind: FileKind, content: &[u8]) -> SnapshotFile {
        let blob = store.put(content).unwrap();
        SnapshotFile { path: path.into(), kind, hash: blob.hash, size: blob.size }
    }

    #[test]
    fn fonts_count_mentions_and_lock_when_the_game_translates_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let files = vec![
            file(&store, "game/fonts/Main.ttf", FileKind::Font, b"main"),
            file(&store, "game/fonts/aMain.ttf", FileKind::Font, b"other"),
            file(&store, "game/Title.otf", FileKind::Font, b"title"),
            file(&store, "game/tl/russian/Title.otf", FileKind::Font, b"ru"),
            file(&store, "game/screens.rpy", FileKind::Script, b"style say_dialogue:\n    font \"fonts/main.ttf\"\n"),
            file(&store, "game/tl/russian/common.rpy", FileKind::Translation, b"define gui.text_font = \"fonts/Main.ttf\""),
        ];
        let fonts = game_fonts(&files, &store).unwrap();
        let found: Vec<(&str, usize, bool)> = fonts.iter().map(|font| (font.path.as_str(), font.mentions, font.locked.is_some())).collect();
        assert_eq!(found, [("game/fonts/Main.ttf", 2, false), ("game/fonts/aMain.ttf", 0, false), ("game/Title.otf", 0, true)]);
        assert_eq!(mod_path("game/fonts/Main.ttf").as_deref(), Some("tl/russian/fonts/Main.ttf"));
        assert_eq!(mod_path("game/tl/russian/a.ttf"), None);
        assert_eq!(mod_path("other/a.ttf"), None);
    }
}
