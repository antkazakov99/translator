//! Считывание игры Ren'Py в снимок. Сначала файлы попадают в `FileStore` как есть,
//! потом снимок собирается только из `FileStore`: так же он пересобирается без папки игры.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use serde::Serialize;

use super::graph::{DialogueData, ScenarioNode};
use super::rpa::{self, RpaArchive};
use super::rpyc::RpycParser;
use super::scenes::{choice_scenes, natural_key, scene_edges, story_order};
use super::script::{OUTSIDE_LABELS, RenpyParser, ScriptStructure};
use super::{TL_LANG, collect_game_files};
use crate::engines::{
    ContainerKey, EngineError, FileKind, GameInfo, GameSnapshot, SegmentKey, SnapshotContainer, SnapshotFile,
    SnapshotEdge, SnapshotSegment, SnapshotSpeaker, SourceText, SpeakerKey,
};
use crate::files::{ContentHash, FileStore};
use crate::projects::GameRoot;

/// Папка скриптов и ресурсов внутри корня игры.
pub(crate) const GAME_DIR: &str = "game";

/// Самый большой файл, который берётся из архива в память.
const MAX_ARCHIVED_FILE: u64 = 256 * 1024 * 1024;

const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc"];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg", "avif"];

/// Подпись реплик интерфейса в разборе файла перевода.
const UI_CHOICE: &str = "UI / Choice";

fn extension(name: &str) -> Option<String> {
    let file = name.rsplit('/').next()?;
    let (_, ext) = file.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

fn is_script(name: &str) -> bool {
    matches!(extension(name).as_deref(), Some("rpy" | "rpyc"))
}

/// Имя скрипта без расширения: так сравниваются его исходник и компиляция.
fn script_base(name: &str) -> &str {
    name.strip_suffix(".rpyc").or_else(|| name.strip_suffix(".rpy")).unwrap_or(name)
}

/// Что из папки `game` нужно проекту. Имя относительно неё, через `/`. Переводы на другие языки не нужны.
fn classify(name: &str) -> Option<FileKind> {
    let ext = extension(name)?;
    let ext = ext.as_str();
    if name.starts_with("tl/") {
        rpa::translation_file_name(name)?;
        return match ext {
            "rpy" | "rpyc" => Some(FileKind::Translation),
            _ if FONT_EXTENSIONS.contains(&ext) => Some(FileKind::Font),
            _ if IMAGE_EXTENSIONS.contains(&ext) => Some(FileKind::Asset),
            _ => None,
        };
    }
    match ext {
        "rpy" | "rpyc" => Some(FileKind::Script),
        _ if FONT_EXTENSIONS.contains(&ext) => Some(FileKind::Font),
        _ => None,
    }
}

fn format_error(path: impl Into<String>) -> impl FnOnce(String) -> EngineError {
    let path = path.into();
    move |reason| EngineError::Format { path, reason }
}

/// Файлы игры в `FileStore` и сведения о них. Как в движке, файл на диске важнее архива,
/// архивы перебираются с конца алфавита, а `.rpyc` рядом со своим `.rpy` не нужен. Исключение: актуальная
/// компиляция файла перевода на диске остаётся готовой копией (`Asset`): если сборка мода не сможет
/// скомпилировать нетронутый файл сама, в мод пойдёт эта копия.
pub(crate) fn collect_folder(root: &GameRoot, store: &FileStore) -> Result<Vec<SnapshotFile>, EngineError> {
    let game = root.as_path().join(GAME_DIR);
    let mut chosen: BTreeMap<String, SnapshotFile> = BTreeMap::new();
    let mut disk_scripts = HashSet::new();
    let mut modified: HashMap<String, std::time::SystemTime> = HashMap::new();

    for path in collect_game_files(&game) {
        let Ok(relative) = path.strip_prefix(&game) else { continue };
        let name = relative.to_string_lossy().replace('\\', "/");
        let Some(kind) = classify(&name) else { continue };
        if let Ok(time) = std::fs::metadata(&path).and_then(|meta| meta.modified()) {
            modified.insert(name.clone(), time);
        }
        let blob = store.put(File::open(&path)?)?;
        if is_script(&name) {
            disk_scripts.insert(script_base(&name).to_string());
        }
        let file = SnapshotFile { path: format!("{GAME_DIR}/{name}"), kind, hash: blob.hash, size: blob.size };
        chosen.insert(name, file);
    }

    for archive_path in rpa::game_archives(&game).map_err(format_error(game.to_string_lossy()))?.iter().rev() {
        let archive_name = archive_path.to_string_lossy().into_owned();
        let archive = RpaArchive::open(archive_path).map_err(format_error(&archive_name))?;
        let mut file = File::open(archive_path)?;
        let archive_size = file.metadata()?.len();
        let mut entries: Vec<_> = archive.entries.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, entry) in entries {
            let name = name.replace('\\', "/");
            let Some(kind) = classify(&name) else { continue };
            if chosen.contains_key(&name) || (is_script(&name) && disk_scripts.contains(script_base(&name))) {
                continue;
            }
            let data = entry.read_up_to(&mut file, archive_size, MAX_ARCHIVED_FILE).map_err(format_error(&archive_name))?;
            let blob = store.put(data.as_slice())?;
            let file = SnapshotFile { path: format!("{GAME_DIR}/{name}"), kind, hash: blob.hash, size: blob.size };
            chosen.insert(name, file);
        }
    }

    let sources: HashSet<String> =
        chosen.keys().filter_map(|name| name.strip_suffix(".rpy")).map(str::to_string).collect();
    let current = |base: &str| match (modified.get(&format!("{base}.rpyc")), modified.get(&format!("{base}.rpy"))) {
        (Some(compiled), Some(source)) => compiled >= source,
        _ => false,
    };
    for (name, file) in &mut chosen {
        if file.kind == FileKind::Translation
            && let Some(base) = name.strip_suffix(".rpyc")
            && sources.contains(base)
            && current(base)
        {
            file.kind = FileKind::Asset;
        }
    }
    chosen.retain(|name, file| file.kind == FileKind::Asset || name.strip_suffix(".rpyc").is_none_or(|base| !sources.contains(base)));
    Ok(chosen.into_values().collect())
}

/// Текст файла без BOM. Битые байты заменяются, а не роняют считывание.
fn decode_text(data: &[u8]) -> String {
    let data = data.strip_prefix("\u{feff}".as_bytes()).unwrap_or(data);
    String::from_utf8_lossy(data).into_owned()
}

/// Ключ реплики. Он не зависит от строки в файле, поэтому переживает правки игры выше по тексту.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
enum LineKey {
    /// Реплика блока `translate`: `id` Ren'Py вычисляет сам и он уникален во всей игре.
    Block { id: String, repeat: usize },
    /// Строка блока `translate strings`: уникальна только в своём файле.
    Strings { file: String, old: String },
    /// Реплика скрипта в метке. Метка уникальна во всей игре.
    Script { label: String, text: String, repeat: usize },
    /// Реплика скрипта вне меток.
    Loose { file: String, text: String, repeat: usize },
}

impl LineKey {
    /// Реплики идут в порядке файла: `repeats` различает одинаковые строки одного блока или метки.
    fn for_line(file: &str, line: &DialogueData, repeats: &mut HashMap<LineKey, usize>) -> Self {
        let first = match &line.block_id {
            None => return Self::Strings { file: file.to_string(), old: line.original_text.clone() },
            Some(block) if !line.target_lines.is_empty() => Self::Block { id: block.clone(), repeat: 0 },
            Some(block) if block == OUTSIDE_LABELS => {
                Self::Loose { file: file.to_string(), text: line.original_text.clone(), repeat: 0 }
            }
            Some(block) => Self::Script { label: block.clone(), text: line.original_text.clone(), repeat: 0 },
        };
        let seen = repeats.entry(first.clone()).or_default();
        let repeat = *seen;
        *seen += 1;
        match first {
            Self::Block { id, .. } => Self::Block { id, repeat },
            Self::Script { label, text, .. } => Self::Script { label, text, repeat },
            Self::Loose { file, text, .. } => Self::Loose { file, text, repeat },
            strings => strings,
        }
    }
}

/// Код персонажа это имя переменной, всё остальное подпись-строка.
fn is_code(alias: &str) -> bool {
    let mut chars = alias.chars();
    chars.next().is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Есть ли в подписи слова вне переменных: `????` и `[mc] & [ne]` говорящими не считаются.
fn has_words(label: &str) -> bool {
    let mut depth = 0;
    label.chars().any(|c| match c {
        '[' => {
            depth += 1;
            false
        }
        ']' => {
            depth = (depth - 1).max(0);
            false
        }
        _ => depth == 0 && c.is_alphabetic(),
    })
}

fn speaker_key(line: &DialogueData) -> Option<(SpeakerKey, bool)> {
    let alias = line.speaker_alias.as_deref().filter(|alias| !alias.is_empty())?;
    if line.speaker.as_deref() == Some(UI_CHOICE) {
        return None;
    }
    if is_code(alias) {
        SpeakerKey::new(format!("code:{alias}")).ok().map(|key| (key, true))
    } else {
        let label = RenpyParser::unescape_renpy_string(alias);
        SpeakerKey::new(format!("label:{label}")).ok().map(|key| (key, false))
    }
}

/// Скрипт в тексте: `.rpy` как есть, `.rpyc` после декомпиляции.
pub(super) struct ScriptText<'a> {
    pub(super) file: &'a SnapshotFile,
    pub(super) text: String,
}

/// Собирает снимок из файлов в `FileStore`. Папка игры здесь уже не нужна.
/// Тексты скриптов оригинала в порядке файлов: `.rpy` как есть, `.rpyc` декомпилированный.
/// С `originals` заодно собирает реплики оригинала из `.rpyc`: по ним восстанавливается перевод в `.rpyc`.
pub(super) fn script_texts<'a>(
    files: &'a [SnapshotFile],
    store: &FileStore,
    mut originals: Option<&mut HashMap<String, String>>,
) -> Result<Vec<ScriptText<'a>>, EngineError> {
    let mut scripts: Vec<&SnapshotFile> = files.iter().filter(|f| f.kind == FileKind::Script).collect();
    scripts.sort_by_cached_key(|f| natural_key(&f.path));
    let mut texts = Vec::new();
    for file in scripts {
        let data = store.read(&file.hash)?;
        if !file.path.ends_with(".rpyc") {
            texts.push(ScriptText { file, text: decode_text(&data) });
            continue;
        }
        if let Some(originals) = originals.as_deref_mut()
            && let Err(error) = RpycParser::collect_original_dialogue(&data, originals)
        {
            tracing::warn!(file = %file.path, %error, "Реплики оригинала не прочитаны");
        }
        match RpycParser::parse_rpyc_bytes(&data) {
            Ok(statements) => texts.push(ScriptText { file, text: RpycParser::decompile_to_rpy_text(&statements) }),
            Err(error) => tracing::warn!(file = %file.path, %error, "RPYC не декомпилирован"),
        }
    }
    Ok(texts)
}

/// Переходы между метками по скриптам снимка: то же, что `GameSnapshot::edges`, без разбора реплик.
pub(crate) fn read_edges(files: &[SnapshotFile], store: &FileStore) -> Result<Vec<SnapshotEdge>, EngineError> {
    let texts = script_texts(files, store, None)?;
    let structures: Vec<ScriptStructure> = texts.iter().map(|s| RenpyParser::parse_script_structure(&s.text)).collect();
    Ok(scene_edges(&structures))
}

/// Файлы перевода снимка в порядке имён и их текст: `.rpy` как есть, `.rpyc` декомпилирован по репликам
/// оригинала. Номера строк реплик (`target_lines`) считаются по этому тексту, поэтому сборка мода берёт его отсюда же.
pub(crate) fn translation_texts<'a>(
    files: &'a [SnapshotFile],
    store: &FileStore,
) -> Result<Vec<(&'a SnapshotFile, String)>, EngineError> {
    let (texts, _) = translations_and_scripts(files, store)?;
    Ok(texts)
}

/// Файлы перевода с текстом и скрипты оригинала с текстом.
#[allow(clippy::type_complexity)]
fn translations_and_scripts<'a>(
    files: &'a [SnapshotFile],
    store: &FileStore,
) -> Result<(Vec<(&'a SnapshotFile, String)>, Vec<ScriptText<'a>>), EngineError> {
    let mut translations: Vec<&SnapshotFile> =
        files.iter().filter(|f| f.kind == FileKind::Translation && is_script(&f.path)).collect();
    translations.sort_by_cached_key(|f| natural_key(&f.path));

    // Файл перевода в .rpyc восстанавливается по репликам оригинала, найденным по id.
    let needs_originals = translations.iter().any(|f| f.path.ends_with(".rpyc"));
    let mut originals = HashMap::new();
    let texts = script_texts(files, store, needs_originals.then_some(&mut originals))?;
    let mut sources = Vec::new();
    for file in translations {
        let data = store.read(&file.hash)?;
        let text = if file.path.ends_with(".rpyc") {
            match RpycParser::decompile_translation(&data, &originals) {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(file = %file.path, %error, "Файл перевода не декомпилирован");
                    continue;
                }
            }
        } else {
            decode_text(&data)
        };
        sources.push((file, text));
    }
    Ok((sources, texts))
}

/// Текст одного файла для просмотра: тот же, по которому считаны номера строк реплик.
pub(crate) fn source_text(file: &SnapshotFile, files: &[SnapshotFile], store: &FileStore) -> Result<SourceText, EngineError> {
    let unreadable = |reason: &str| EngineError::Format { path: file.path.clone(), reason: reason.to_string() };
    if !matches!(file.kind, FileKind::Script | FileKind::Translation) || !is_script(&file.path) {
        return Err(unreadable("это не скрипт"));
    }
    if !file.path.ends_with(".rpyc") {
        return Ok(SourceText { text: decode_text(&store.read(&file.hash)?), decompiled: false });
    }
    let data = store.read(&file.hash)?;
    let text = if file.kind == FileKind::Script {
        RpycParser::decompile_to_rpy_text(&RpycParser::parse_rpyc_bytes(&data).map_err(|error| unreadable(&error.to_string()))?)
    } else {
        // Перевод в .rpyc восстанавливается по репликам оригинала, как при считывании.
        // Текст скриптов здесь не нужен: из скомпилированных берутся только реплики.
        let mut originals = HashMap::new();
        for script in files.iter().filter(|f| f.kind == FileKind::Script && f.path.ends_with(".rpyc")) {
            if let Err(error) = RpycParser::collect_original_dialogue(&store.read(&script.hash)?, &mut originals) {
                tracing::warn!(file = %script.path, %error, "Реплики оригинала не прочитаны");
            }
        }
        RpycParser::decompile_translation(&data, &originals).map_err(|error| unreadable(&error.to_string()))?
    };
    Ok(SourceText { text, decompiled: true })
}

pub(crate) fn analyze(files: Vec<SnapshotFile>, store: &FileStore, info: &GameInfo) -> Result<GameSnapshot, EngineError> {
    let (translated, texts) = translations_and_scripts(&files, store)?;

    let mut codes: HashMap<String, String> = HashMap::new();
    for script in &texts {
        codes.extend(RenpyParser::extract_speakers(&script.text));
    }
    if let Some(player) = codes.get("player").cloned() {
        for name in codes.values_mut() {
            if name == "[player.name]" || name == "[player]" {
                *name = player.clone();
            }
        }
    }

    let structures: Vec<ScriptStructure> = texts.iter().map(|s| RenpyParser::parse_script_structure(&s.text)).collect();
    let order = story_order(&structures);
    let edges = scene_edges(&structures);
    let mut containers = Vec::new();
    let mut container_keys = HashSet::new();
    let mut choices: HashMap<String, String> = HashMap::new();
    for (script, structure) in texts.iter().zip(&structures) {
        for label in &structure.labels {
            let Ok(key) = ContainerKey::new(format!("label:{}", label.name)) else { continue };
            if container_keys.insert(key.clone()) {
                containers.push(SnapshotContainer {
                    key,
                    parent: None,
                    kind: "scene".to_string(),
                    file: Some(script.file.path.clone()),
                    story_order: order.get(&label.name).map(|&rank| rank as i64),
                });
            }
        }
        let mut screens: Vec<&String> = structure.screens.keys().collect();
        screens.sort();
        for name in screens {
            let Ok(key) = ContainerKey::new(format!("screen:{name}")) else { continue };
            if container_keys.insert(key.clone()) {
                containers.push(SnapshotContainer {
                    key,
                    parent: None,
                    kind: "screen".to_string(),
                    file: Some(script.file.path.clone()),
                    story_order: None,
                });
            }
        }
        for (text, scene) in choice_scenes(structure) {
            choices.entry(text).or_insert(scene);
        }
    }

    let actions = [format!("Language(\"{TL_LANG}\")"), format!("Language('{TL_LANG}')")];
    let offers_language = texts.iter().any(|s| actions.iter().any(|action| s.text.contains(action)));

    // Есть перевод в игре: реплики берутся из него вместе с готовым текстом, иначе из скриптов.
    let has_translation = files.iter().any(|f| f.kind == FileKind::Translation && is_script(&f.path));
    let sources: Vec<ScriptText> = if has_translation {
        translated.into_iter().map(|(file, text)| ScriptText { file, text }).collect()
    } else {
        texts
    };

    let parser = RenpyParser::new().with_speakers(codes.clone());
    let mut segments = Vec::new();
    let mut segment_keys = HashSet::new();
    let mut used_speakers: BTreeMap<SpeakerKey, bool> = BTreeMap::new();
    let mut positions: HashMap<String, i64> = HashMap::new();
    for source in &sources {
        let path = &source.file.path;
        let graph = match parser.parse_file(Path::new(path), &source.text) {
            Ok(graph) => graph,
            Err(error) => {
                tracing::warn!(file = %path, %error, "Файл не разобран");
                continue;
            }
        };
        let mut repeats = HashMap::new();
        let mut label: Option<String> = None;
        for index in graph.node_indices() {
            match graph.get_node(index) {
                Some(ScenarioNode::Label(name)) => label = (name != OUTSIDE_LABELS).then(|| name.clone()),
                Some(ScenarioNode::Dialogue(line)) => {
                    let scene = if line.speaker.as_deref() == Some(UI_CHOICE) {
                        choices.get(line.original_text.trim()).cloned()
                    } else {
                        label.clone()
                    };
                    let key = serde_json::to_string(&LineKey::for_line(path, line, &mut repeats))
                        .map_err(|error| format_error(path.as_str())(error.to_string()))?;
                    let Ok(key) = SegmentKey::new(key) else { continue };
                    if !segment_keys.insert(key.clone()) {
                        tracing::warn!(file = %path, line = line.line_number, "Повтор ключа реплики пропущен");
                        continue;
                    }
                    let container = scene.and_then(|name| ContainerKey::new(format!("label:{name}")).ok());
                    if let Some(key) = &container
                        && container_keys.insert(key.clone())
                    {
                        containers.push(SnapshotContainer {
                            key: key.clone(),
                            parent: None,
                            kind: "scene".to_string(),
                            file: None,
                            story_order: None,
                        });
                    }
                    let speaker = speaker_key(line);
                    if let Some((key, is_code)) = &speaker {
                        used_speakers.insert(key.clone(), *is_code);
                    }
                    let place = container.as_ref().map_or_else(|| format!("file:{path}"), |key| key.to_string());
                    let position = positions.entry(place).or_default();
                    *position += 1;
                    let meta = (!line.target_lines.is_empty()).then(|| serde_json::json!({ "target_lines": line.target_lines }));
                    segments.push(SnapshotSegment {
                        key,
                        container,
                        file: path.clone(),
                        speaker: speaker.map(|(key, _)| key),
                        source_hash: ContentHash::of(line.original_text.as_bytes()).as_str().to_string(),
                        source_text: line.original_text.clone(),
                        game_text: line.translated_text.clone().filter(|text| !text.trim().is_empty()),
                        location: Some(line.line_number.to_string()),
                        story_order: Some(*position),
                        engine_meta: meta,
                    });
                }
                _ => {}
            }
        }
    }

    let mut speakers: BTreeMap<SpeakerKey, SnapshotSpeaker> = BTreeMap::new();
    for (alias, name) in &codes {
        if let Ok(key) = SpeakerKey::new(format!("code:{alias}")) {
            speakers.insert(key.clone(), SnapshotSpeaker { key, kind: "code".to_string(), label: Some(name.clone()) });
        }
    }
    for (key, is_code) in used_speakers {
        if speakers.contains_key(&key) {
            continue;
        }
        if is_code {
            speakers.insert(key.clone(), SnapshotSpeaker { key, kind: "code".to_string(), label: None });
        } else if let Some(label) = key.as_str().strip_prefix("label:").filter(|label| has_words(label)) {
            let label = label.to_string();
            speakers.insert(key.clone(), SnapshotSpeaker { key, kind: "label".to_string(), label: Some(label) });
        }
    }

    let mut info = info.clone();
    if let Some(details) = info.details.as_object_mut() {
        details.insert("offers_language".to_string(), offers_language.into());
    }
    Ok(GameSnapshot { info, files, containers, segments, speakers: speakers.into_values().collect(), edges })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engines::{GameEngine, GameSource, renpy::RenpyEngine};

    const SCRIPT: &str = r#"define e = Character("Eileen")
define mc = Character("[player]")

label start:
    e "Hello."
    "Narration."
    "Radio Chatter" "Static."
    "????" "Who?"
    e "Hello."
    menu:
        "Go left":
            jump left
    jump left

label left:
    e "Left side."
    return
"#;

    /// Игра Ren'Py в папке `root`: движок, скрипт, шрифт и картинка, которая проекту не нужна.
    pub(crate) fn sample_game(root: &Path) -> GameRoot {
        let engine = root.join("renpy");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join("__init__.py"), "script_version = 5003000\n").unwrap();
        std::fs::write(engine.join("vc_version.py"), "version = '8.1.3.23091805'\n").unwrap();
        let game = root.join(GAME_DIR);
        std::fs::create_dir_all(game.join("fonts")).unwrap();
        std::fs::create_dir_all(game.join("images")).unwrap();
        std::fs::write(game.join("script.rpy"), SCRIPT).unwrap();
        std::fs::write(game.join("fonts").join("Main.ttf"), b"font").unwrap();
        std::fs::write(game.join("images").join("bg.png"), b"png").unwrap();
        GameRoot::new(root).unwrap()
    }

    fn read(root: &GameRoot, store: &FileStore) -> GameSnapshot {
        let info = RenpyEngine.detect(root).expect("игра Ren'Py");
        RenpyEngine.read(&GameSource::Folder(root.clone()), &info, store).unwrap()
    }

    fn texts(snapshot: &GameSnapshot) -> Vec<&str> {
        snapshot.segments.iter().map(|segment| segment.source_text.as_str()).collect()
    }

    #[test]
    fn detects_engine_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(dir.path());
        let info = RenpyEngine.detect(&root).unwrap();
        assert_eq!(info.version.as_deref(), Some("8.1.3.23091805"));
        assert_eq!(info.details["script_version"], 5003000);
        assert!(RenpyEngine.detect(&GameRoot::new(&dir.path().join(GAME_DIR)).unwrap()).is_none());
    }

    #[test]
    fn reads_scenes_lines_and_speakers_from_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);

        let paths: Vec<&str> = snapshot.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["game/fonts/Main.ttf", "game/script.rpy"]);
        assert!(snapshot.files.iter().all(|file| store.contains(&file.hash)));

        let start = snapshot.containers.iter().find(|c| c.key.as_str() == "label:start").unwrap();
        let left = snapshot.containers.iter().find(|c| c.key.as_str() == "label:left").unwrap();
        assert!(start.story_order < left.story_order);

        let hellos: Vec<_> = snapshot.segments.iter().filter(|s| s.source_text == "Hello.").collect();
        assert_eq!(hellos.len(), 2);
        assert_ne!(hellos[0].key, hellos[1].key);
        assert_eq!(hellos[0].speaker.as_ref().unwrap().as_str(), "code:e");
        assert!(hellos[0].story_order < hellos[1].story_order);
        let narration = snapshot.segments.iter().find(|s| s.source_text == "Narration.").unwrap();
        assert!(narration.speaker.is_none());
        assert_eq!(narration.container.as_ref().unwrap().as_str(), "label:start");

        let speakers: Vec<(&str, Option<&str>)> =
            snapshot.speakers.iter().map(|s| (s.key.as_str(), s.label.as_deref())).collect();
        assert!(speakers.contains(&("code:e", Some("Eileen"))));
        assert!(speakers.contains(&("label:Radio Chatter", Some("Radio Chatter"))));
        assert!(!speakers.iter().any(|(key, _)| key.contains("????")), "подпись без слов не говорящий");
        assert_eq!(snapshot.info.details["offers_language"], false);
    }

    #[test]
    fn stored_files_give_the_same_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let first = read(&root, &store);
        std::fs::remove_dir_all(root.as_path()).unwrap();

        let again = RenpyEngine.read(&GameSource::Stored(first.files.clone()), &first.info, &store).unwrap();
        let keys = |s: &GameSnapshot| s.segments.iter().map(|segment| segment.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&first), keys(&again));
        assert_eq!(first.containers.len(), again.containers.len());
    }

    #[test]
    fn disk_wins_over_archive_and_archive_adds_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let archived = [
            ("script.rpy".to_string(), b"label start:\n    e \"Archived copy.\"\n".to_vec()),
            ("extra.rpy".to_string(), b"label extra:\n    e \"From archive.\"\n".to_vec()),
        ];
        RpaArchive::create_archive(&root.as_path().join(GAME_DIR).join("archive.rpa"), &archived).unwrap();
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);

        let texts = texts(&snapshot);
        assert!(texts.contains(&"From archive."));
        assert!(!texts.contains(&"Archived copy."));
        assert!(snapshot.files.iter().any(|file| file.path == "game/extra.rpy"));
    }

    #[test]
    fn existing_translation_gives_lines_with_game_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = sample_game(&dir.path().join("game-root"));
        let tl = root.as_path().join(GAME_DIR).join("tl").join(TL_LANG);
        std::fs::create_dir_all(&tl).unwrap();
        std::fs::write(
            tl.join("script.rpy"),
            "translate russian start_9a0b1c2d:\n\n    # e \"Hello.\"\n    e \"Привет.\"\n\n\
             translate russian strings:\n\n    # game/script.rpy:11\n    old \"Go left\"\n    new \"Налево\"\n",
        )
        .unwrap();
        let store = FileStore::create(&dir.path().join("project")).unwrap();
        let snapshot = read(&root, &store);

        let hello = snapshot.segments.iter().find(|s| s.source_text == "Hello.").unwrap();
        assert_eq!(hello.game_text.as_deref(), Some("Привет."));
        assert!(hello.key.as_str().contains("Block"));
        assert_eq!(hello.container.as_ref().unwrap().as_str(), "label:start");
        let choice = snapshot.segments.iter().find(|s| s.source_text == "Go left").unwrap();
        assert_eq!(choice.game_text.as_deref(), Some("Налево"));
        assert!(choice.key.as_str().contains("game/tl/russian/script.rpy"), "у строк old в ключе путь файла");
        assert!(snapshot.files.iter().any(|file| file.kind == FileKind::Translation));
    }

    /// Считывает настоящую игру по папке `game` из переменной окружения и печатает итог.
    fn read_real_game(env_var: &str) -> Option<GameSnapshot> {
        let game = crate::engines::renpy::local_test_game(env_var)?;
        let root = GameRoot::new(Path::new(&game).parent()?).unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(store_dir.path()).unwrap();
        let started = std::time::Instant::now();
        let snapshot = read(&root, &store);
        let translated = snapshot.segments.iter().filter(|s| s.game_text.is_some()).count();
        println!(
            "{env_var}: {:?}, файлов {}, сцен {}, реплик {} (с переводом {translated}), говорящих {}, {:?}",
            snapshot.info.version,
            snapshot.files.len(),
            snapshot.containers.len(),
            snapshot.segments.len(),
            snapshot.speakers.len(),
            started.elapsed(),
        );
        Some(snapshot)
    }

    #[test]
    #[ignore = "Читает настоящую игру: TRANSLATOR_TEST_RPA_GAME"]
    fn reads_real_game_from_archives() {
        let Some(snapshot) = read_real_game(crate::engines::renpy::TEST_RPA_GAME) else { return };
        assert!(snapshot.segments.len() > 1000);
        assert!(snapshot.containers.len() > 10);
        assert!(snapshot.speakers.iter().any(|s| s.kind == "code"));
    }

    #[test]
    #[ignore = "Читает настоящую игру: TRANSLATOR_TEST_PACKED_TL_GAME"]
    fn reads_real_translation_from_archives() {
        let Some(snapshot) = read_real_game(crate::engines::renpy::TEST_PACKED_TL_GAME) else { return };
        let translated = snapshot.segments.iter().filter(|s| s.game_text.is_some()).count();
        assert!(translated > 1000, "перевод из архива: {translated}");
        assert!(snapshot.files.iter().any(|file| file.kind == FileKind::Translation && file.path.ends_with(".rpyc")));
    }

    #[test]
    #[ignore = "Читает настоящую игру: TRANSLATOR_TEST_TL_DIR"]
    fn reads_real_translation_from_disk() {
        let Some(tl) = crate::engines::renpy::local_test_game(crate::engines::renpy::TEST_TL_DIR) else { return };
        let game = Path::new(&tl).parent().and_then(Path::parent).unwrap();
        // SAFETY: тест один в процессе пишет переменную, которую читает только он сам.
        unsafe { std::env::set_var("TRANSLATOR_TEST_TL_GAME", game) };
        let Some(snapshot) = read_real_game("TRANSLATOR_TEST_TL_GAME") else { return };
        assert!(snapshot.segments.iter().filter(|s| s.game_text.is_some()).count() > 1000);
    }

    #[test]
    fn classifies_by_place_and_extension() {
        assert_eq!(classify("script.rpy"), Some(FileKind::Script));
        assert_eq!(classify("chapters/day1.RPYC"), Some(FileKind::Script));
        assert_eq!(classify("fonts/Main.TTF"), Some(FileKind::Font));
        assert_eq!(classify("images/bg.png"), None);
        assert_eq!(classify("tl/russian/script.rpy"), Some(FileKind::Translation));
        assert_eq!(classify("tl/russian/gui/title.png"), Some(FileKind::Asset));
        assert_eq!(classify("tl/russian/fonts/ru.otf"), Some(FileKind::Font));
        assert_eq!(classify("tl/french/script.rpy"), None);
    }

    #[test]
    fn speaker_labels_need_words() {
        assert!(has_words("Radio Chatter"));
        assert!(!has_words("????"));
        assert!(!has_words("[mc] & [ne]"));
        assert!(has_words("[mc]'s mom"));
        assert!(is_code("e") && is_code("mc.name") && !is_code("Radio Chatter"));
    }
}
