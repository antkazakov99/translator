//! Сборка `ru_lang.rpa`: переводы `tl/<язык>`, строковые переводы, подписи говорящих, переключатель языка, шрифты.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use super::rpa::RpaArchive;
use super::rpyc_writer::{AstLayout, ScriptFormat, compile_script};
use super::script::RenpyParser;
use super::snapshot::{GAME_DIR, translation_texts};
use super::{TL_LANG, language_script, path_lock};
use crate::engines::{EngineError, FileKind, GameInfo, ModArchive, ModInput, SegmentKey, SnapshotSegment};
use crate::files::FileStore;
use crate::projects::GameRoot;

/// Имя архива мода в папке `game`.
pub(crate) const MOD_ARCHIVE_NAME: &str = "ru_lang.rpa";
/// Строковые переводы реплик и подписей, которых нет в переводе игры.
const STRINGS_FILE: &str = "translator_strings.rpy";
/// Переключатель языка для игры, где выбрать язык перевода нельзя.
const LANGUAGE_FILE: &str = "translator_language.rpy";

fn failed(message: impl Into<String>) -> EngineError {
    EngineError::Build(message.into())
}

/// Формат `.rpyc` из сведений считывания: папка игры для сборки не нужна.
fn script_format(info: &GameInfo) -> Result<ScriptFormat, EngineError> {
    let details = &info.details;
    let script_version = details.get("script_version").and_then(serde_json::Value::as_i64);
    let layout = match details.get("ast_layout").and_then(serde_json::Value::as_str) {
        Some("Python2") => Some(AstLayout::Python2),
        Some("Python3") => Some(AstLayout::Python3),
        Some("CompactNames") => Some(AstLayout::CompactNames),
        _ => None,
    };
    match (script_version, layout) {
        (Some(script_version), Some(layout)) => Ok(ScriptFormat { layout, script_version }),
        _ => Err(failed("формат скриптов игры неизвестен: считайте игру заново")),
    }
}

/// Строки файла перевода, где стоит реплика: номера с 1, их записывает считывание.
fn target_lines(segment: &SnapshotSegment) -> Vec<usize> {
    segment
        .engine_meta
        .as_ref()
        .and_then(|meta| meta.get("target_lines"))
        .and_then(serde_json::Value::as_array)
        .map(|lines| lines.iter().filter_map(serde_json::Value::as_u64).filter_map(|line| usize::try_from(line).ok()).collect())
        .unwrap_or_default()
}

/// Файлы архива и счёт переводов.
#[derive(Default)]
struct Pack {
    files: Vec<(String, Vec<u8>)>,
    written: usize,
    skipped: usize,
    left_out: Vec<String>,
}

impl Pack {
    /// Ren'Py загружает из архива только скомпилированные скрипты.
    fn compile(&mut self, format: ScriptFormat, name: &str, source: &str) -> Result<(), EngineError> {
        let compiled = format!("{}.rpyc", name.strip_suffix(".rpy").unwrap_or(name));
        let data = compile_script(format, TL_LANG, name, source).map_err(|error| failed(format!("{name}: {error}")))?;
        self.files.push((format!("tl/{TL_LANG}/{compiled}"), data));
        Ok(())
    }
}

pub(crate) fn build(input: &ModInput, store: &FileStore, out_dir: &Path) -> Result<ModArchive, EngineError> {
    let format = script_format(&input.snapshot.info)?;
    let translations: HashMap<&SegmentKey, &str> = input
        .translations
        .iter()
        .filter(|item| !item.text.trim().is_empty())
        .map(|item| (&item.key, item.text.as_str()))
        .collect();
    let tl_prefix = format!("{GAME_DIR}/tl/{TL_LANG}/");
    let game_prefix = format!("{GAME_DIR}/");
    let mut pack = Pack::default();
    // Строки `old`, которые уже переводит игра: повтор в одном языке Ren'Py считает ошибкой.
    let mut taken: HashSet<String> = HashSet::new();
    let mut strings: BTreeMap<String, String> = BTreeMap::new();

    let has_translation = input.snapshot.files.iter().any(|file| file.kind == FileKind::Translation);
    if has_translation {
        let mut by_file: HashMap<&str, Vec<&SnapshotSegment>> = HashMap::new();
        for segment in &input.snapshot.segments {
            by_file.entry(segment.file.as_str()).or_default().push(segment);
        }
        for (file, text) in translation_texts(&input.snapshot.files, store)? {
            let Some(name) = file.path.strip_prefix(&tl_prefix) else { continue };
            taken.extend(RenpyParser::string_translation_sources(&text));
            let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
            let mut changed = 0;
            for segment in by_file.get(file.path.as_str()).into_iter().flatten() {
                let Some(&translated) = translations.get(&segment.key) else { continue };
                if segment.game_text.as_deref() == Some(translated) {
                    continue;
                }
                let mut applied = false;
                for number in target_lines(segment) {
                    let Some(line) = number.checked_sub(1).and_then(|index| lines.get_mut(index)) else { continue };
                    let body = line.trim_end_matches(['\r', '\n']);
                    let ending = line[body.len()..].to_string();
                    if let Some(updated) = RenpyParser::replace_dialogue_text(body, translated) {
                        *line = format!("{updated}{ending}");
                        applied = true;
                    }
                }
                if applied {
                    changed += 1;
                } else {
                    pack.skipped += 1;
                }
            }
            if changed > 0 || !file.path.ends_with(".rpyc") {
                let source = match name.strip_suffix(".rpyc") {
                    Some(base) => format!("{base}.rpy"),
                    None => name.to_string(),
                };
                // Файл с оператором, который компилятор мода не знает, в мод не входит: остальной перевод важнее.
                match pack.compile(format, &source, &lines.concat()) {
                    Ok(()) => pack.written += changed,
                    Err(error) => {
                        // Нетронутый файл с готовой компиляцией игры идёт её копией: она лежит среди картинок перевода.
                        let compiled = format!("{}.rpyc", file.path.strip_suffix(".rpy").unwrap_or(&file.path));
                        let has_copy = input.snapshot.files.iter().any(|other| other.kind == FileKind::Asset && other.path == compiled);
                        if changed > 0 || !has_copy {
                            tracing::warn!(file = %file.path, %error, "Файл перевода не вошёл в мод");
                            pack.skipped += changed;
                            pack.left_out.push(name.to_string());
                        }
                    }
                }
            } else {
                pack.files.push((format!("tl/{TL_LANG}/{name}"), store.read(&file.hash)?));
            }
        }
        for file in input.snapshot.files.iter().filter(|file| file.kind == FileKind::Asset) {
            if let Some(name) = file.path.strip_prefix(&game_prefix) {
                pack.files.push((name.to_string(), store.read(&file.hash)?));
            }
        }
    } else {
        // Без перевода в игре реплики идут строковым переводом: он действует и на диалоги.
        for segment in &input.snapshot.segments {
            let Some(&translated) = translations.get(&segment.key) else { continue };
            // Пустой оригинал движок нашёл бы в каждой реплике и имени.
            if segment.source_text.trim().is_empty() {
                pack.skipped += 1;
                continue;
            }
            match strings.get(&segment.source_text) {
                Some(existing) if existing != translated => pack.skipped += 1,
                Some(_) => {}
                None => {
                    strings.insert(segment.source_text.clone(), translated.to_string());
                    pack.written += 1;
                }
            }
        }
    }

    for (label, translated) in &input.speaker_labels {
        if !label.trim().is_empty() && !taken.contains(label) && !strings.contains_key(label) {
            strings.insert(label.clone(), translated.clone());
        }
    }
    if !strings.is_empty() {
        pack.compile(format, STRINGS_FILE, &strings_file(&strings))?;
    }
    if pack.files.is_empty() {
        return Err(failed("в проекте нет перевода"));
    }
    let offers_language = input.snapshot.info.details.get("offers_language").and_then(serde_json::Value::as_bool).unwrap_or(false);
    if !offers_language {
        pack.compile(format, LANGUAGE_FILE, &language_script())?;
    }
    // Движок ищет файл сначала в tl/<язык>: замена действует только на языке перевода.
    for font in &input.fonts {
        let Some(name) = super::fonts::mod_path(&font.target_path) else { continue };
        pack.files.push((name, store.read(&font.hash)?));
    }

    pack.files.sort_by(|a, b| a.0.cmp(&b.0));
    pack.files.dedup_by(|a, b| a.0 == b.0);
    let path = out_dir.join(MOD_ARCHIVE_NAME);
    let lock = path_lock(&path);
    let _guard = lock.lock();
    RpaArchive::create_archive(&path, &pack.files).map_err(failed)?;
    Ok(ModArchive { path, files: pack.files.len(), written: pack.written, skipped: pack.skipped, left_out: pack.left_out })
}

/// `translate russian strings` с парами по алфавиту оригинала.
fn strings_file(strings: &BTreeMap<String, String>) -> String {
    let mut output = format!("# Создано Translator.\n\ntranslate {TL_LANG} strings:\n");
    for (original, text) in strings {
        output.push_str(&format!(
            "\n    old \"{}\"\n    new \"{}\"\n",
            RenpyParser::escape_renpy_string(original),
            RenpyParser::escape_renpy_string(text)
        ));
    }
    output
}

/// Копирует архив в `game/ru_lang.rpa` через временный файл: игра никогда не видит половину архива.
pub(crate) fn install(root: &GameRoot, archive: &ModArchive) -> Result<(), EngineError> {
    let target = root.as_path().join(GAME_DIR).join(MOD_ARCHIVE_NAME);
    let data = std::fs::read(&archive.path)?;
    let lock = path_lock(&target);
    let _guard = lock.lock();
    super::rpa::write_atomic(&target, |file| {
        use std::io::Write;
        file.write_all(&data).map_err(|error| format!("Не удалось записать {target:?}: {error}"))
    })
    .map_err(failed)
}

/// Удаляет `game/ru_lang.rpa`. Архива нет: снимать нечего, это не ошибка.
pub(crate) fn uninstall(root: &GameRoot) -> Result<(), EngineError> {
    let target = root.as_path().join(GAME_DIR).join(MOD_ARCHIVE_NAME);
    let lock = path_lock(&target);
    let _guard = lock.lock();
    match std::fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::engines::renpy::RenpyEngine;
    use crate::engines::renpy::pickle::PickleDecoder;
    use crate::engines::renpy::rpyc::RpycParser;
    use crate::engines::renpy::snapshot::tests::sample_game;
    use crate::engines::{GameEngine, GameSnapshot, GameSource, SegmentTranslation};

    fn read(root: &GameRoot, store: &FileStore) -> GameSnapshot {
        let info = RenpyEngine.detect(root).unwrap();
        RenpyEngine.read(&GameSource::Folder(root.clone()), &info, store).unwrap()
    }

    /// Записи архива: скомпилированные скрипты дерево pickle текстом, остальное как есть.
    fn archive(path: &Path) -> BTreeMap<String, String> {
        let archive = RpaArchive::open(path).unwrap();
        let size = std::fs::metadata(path).unwrap().len();
        let mut file = std::fs::File::open(path).unwrap();
        archive
            .entries
            .values()
            .map(|entry| {
                let data = entry.read_compiled(&mut file, size).unwrap();
                let text = if entry.name.ends_with(".rpyc") {
                    let slots = RpycParser::read_slots(&data).unwrap();
                    let tree = RpycParser::decompress_slot(&data, &slots[0]).unwrap();
                    format!("{:?}", PickleDecoder::new(&tree).decode().unwrap())
                } else {
                    String::from_utf8_lossy(&data).into_owned()
                };
                (entry.name.clone(), text)
            })
            .collect()
    }

    fn translate(snapshot: &GameSnapshot, pairs: &[(&str, &str)]) -> Vec<SegmentTranslation> {
        let mut used = HashSet::new();
        pairs
            .iter()
            .filter_map(|(source, text)| {
                let segment = snapshot.segments.iter().find(|segment| segment.source_text == *source && used.insert(segment.key.clone()))?;
                Some(SegmentTranslation { key: segment.key.clone(), text: text.to_string() })
            })
            .collect()
    }

    fn input(snapshot: GameSnapshot, translations: Vec<SegmentTranslation>) -> ModInput {
        ModInput {
            snapshot,
            translations,
            speaker_labels: vec![("Eileen".to_string(), "Эйлин".to_string())],
            fonts: Vec::new(),
            target_language: "ru".to_string(),
        }
    }

    #[test]
    fn game_without_translation_gets_strings_and_language_switch() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);
        let translations = translate(&snapshot, &[("Hello.", "Привет."), ("Hello.", "Здравствуй."), ("Narration.", "Рассказ.")]);
        let out = dir.path().join("exports");
        std::fs::create_dir_all(&out).unwrap();

        let built = build(&input(snapshot, translations), &store, &out).unwrap();
        assert_eq!((built.written, built.skipped), (2, 1), "второй перевод того же оригинала не вошёл");
        let files = archive(&built.path);
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            ["tl/russian/translator_language.rpyc", "tl/russian/translator_strings.rpyc"]
        );
        let strings = &files["tl/russian/translator_strings.rpyc"];
        for text in ["Hello.", "Привет.", "Narration.", "Рассказ.", "Eileen", "Эйлин"] {
            assert!(strings.contains(text), "{text} нет в строках: {strings}");
        }
        assert!(!strings.contains("Здравствуй."));
        assert_eq!(built.files, 2);
    }

    #[test]
    fn existing_translation_is_patched_by_lines_and_keeps_its_strings() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let tl = root.as_path().join(GAME_DIR).join("tl").join(TL_LANG);
        std::fs::create_dir_all(tl.join("gui")).unwrap();
        std::fs::write(
            tl.join("script.rpy"),
            "translate russian start_9a0b1c2d:\n\n    # e \"Hello.\"\n    e \"Привет.\"\n\n\
             translate russian strings:\n\n    # game/script.rpy:11\n    old \"Go left\"\n    new \"Налево\"\n",
        )
        .unwrap();
        std::fs::write(tl.join("gui").join("title.png"), b"png").unwrap();
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);
        let translations = translate(&snapshot, &[("Hello.", "Здравствуй, \"друг\"."), ("Go left", "Налево")]);
        let mut input = input(snapshot, translations);
        input.speaker_labels.push(("Go left".to_string(), "Влево".to_string()));
        let out = dir.path().join("exports");
        std::fs::create_dir_all(&out).unwrap();

        let built = build(&input, &store, &out).unwrap();
        assert_eq!((built.written, built.skipped), (1, 0), "перевод, равный переводу игры, не считается");
        let files = archive(&built.path);
        let script = &files["tl/russian/script.rpyc"];
        assert!(script.contains("Здравствуй, \\\"друг\\\".") || script.contains("Здравствуй, \"друг\"."), "{script}");
        assert!(!script.contains("Привет."));
        assert!(script.contains("Налево"));
        assert_eq!(files["tl/russian/gui/title.png"], "png", "картинки перевода идут как есть");
        let strings = &files["tl/russian/translator_strings.rpyc"];
        assert!(strings.contains("Эйлин"));
        assert!(!strings.contains("Влево"), "строку, которую переводит игра, второй раз не переводим");
    }

    #[test]
    fn install_replaces_archive_and_uninstall_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);
        let translations = translate(&snapshot, &[("Hello.", "Привет.")]);
        let out = dir.path().join("exports");
        std::fs::create_dir_all(&out).unwrap();
        let built = build(&input(snapshot, translations), &store, &out).unwrap();

        let target = root.as_path().join(GAME_DIR).join(MOD_ARCHIVE_NAME);
        install(&root, &built).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), std::fs::read(&built.path).unwrap());
        uninstall(&root).unwrap();
        assert!(!target.exists());
        uninstall(&root).unwrap();
    }

    /// Собирает мод настоящей игры из `TRANSLATOR_TEST_MOD_GAME` (папка `game`), переведя каждую реплику.
    /// Папка игры только читается. Проверяет, что компилятор мода понимает все файлы перевода игры.
    #[test]
    #[ignore]
    fn real_game_builds_with_every_line_translated() {
        let Some(game) = crate::engines::renpy::local_test_game("TRANSLATOR_TEST_MOD_GAME") else { return };
        let root = GameRoot::new(Path::new(&game).parent().unwrap()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let started = std::time::Instant::now();
        let snapshot = read(&root, &store);
        let translations: Vec<SegmentTranslation> = snapshot
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| SegmentTranslation { key: segment.key.clone(), text: format!("Перевод {index} «{}»", segment.source_text) })
            .collect();
        let lines = translations.len();
        let built = build(&input(snapshot, translations), &store, dir.path()).unwrap();
        let files = archive(&built.path);
        println!(
            "{game}: реплик {lines}, в моде {}, не вошло {}, файлов {}, без файлов {:?}, {:?}",
            built.written,
            built.skipped,
            built.files,
            built.left_out,
            started.elapsed()
        );
        assert!(built.written > 0);
        assert_eq!(files.len(), built.files);
    }

    #[test]
    fn game_compilation_stands_in_for_a_file_the_mod_cannot_compile() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let tl = root.as_path().join(GAME_DIR).join("tl").join(TL_LANG);
        std::fs::create_dir_all(&tl).unwrap();
        std::fs::write(tl.join("script.rpy"), "translate russian start_9a0b1c2d:

    # e \"Hello.\"
    e \"Привет.\"
").unwrap();
        std::fs::write(tl.join("init.rpy"), "define dict_ru = [(\"a\", \"б\")]
").unwrap();
        std::fs::write(tl.join("init.rpyc"), b"compiled by the game").unwrap();
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);
        let copy = snapshot.files.iter().find(|file| file.path.ends_with("init.rpyc")).unwrap();
        assert_eq!(copy.kind, FileKind::Asset, "актуальная компиляция перевода остаётся готовой копией");

        let translations = translate(&snapshot, &[("Hello.", "Здравствуй.")]);
        let built = build(&input(snapshot, translations), &store, dir.path()).unwrap();
        assert!(built.left_out.is_empty(), "{:?}", built.left_out);
        let archive = RpaArchive::open(&built.path).unwrap();
        let entry = &archive.entries["tl/russian/init.rpyc"];
        let size = std::fs::metadata(&built.path).unwrap().len();
        let data = entry.read_compiled(&mut std::fs::File::open(&built.path).unwrap(), size).unwrap();
        assert_eq!(data, b"compiled by the game");
        assert_eq!(built.written, 1);
    }

    #[test]
    fn nothing_to_build_and_unknown_format_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);
        let mut empty = input(snapshot, Vec::new());
        empty.speaker_labels.clear();
        assert!(build(&empty, &store, dir.path()).is_err());
        empty.snapshot.info.details = serde_json::json!({});
        assert!(script_format(&empty.snapshot.info).is_err());
    }
}
