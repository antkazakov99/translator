use crate::parser::renpy::RenpyParser;
use crate::{
    TL_LANG, collect_rpy_files, find_game_root, get_project_cache_dir, parse_project_file,
    path_lock, stable_hash_str, tl_relative_name,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Файл мода со строками для правок скриптов, у которых нет файла в `tl/<язык>`.
const SAVED_STRINGS_FILE: &str = "translator_saved_strings.rpy";

#[derive(Debug, Clone, Deserialize)]
pub struct TranslationEdit {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TranslationOverrides {
    source_path: String,
    entries: HashMap<String, String>,
}

fn store_path(source: &Path) -> PathBuf {
    let hash = stable_hash_str(&source.to_string_lossy().to_lowercase());
    get_project_cache_dir(&find_game_root(source))
        .join("translations")
        .join(format!("{:016x}.json", hash))
}

fn read_store(source: &Path, path: &Path) -> Result<TranslationOverrides, String> {
    if !path.exists() {
        return Ok(TranslationOverrides {
            source_path: source.to_string_lossy().into_owned(),
            entries: HashMap::new(),
        });
    }
    let bytes = fs::read(path)
        .map_err(|e| format!("Не удалось прочитать сохранённый перевод {:?}: {}", path, e))?;
    let store: TranslationOverrides = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Повреждён файл перевода {:?}: {}", path, e))?;
    if !store
        .source_path
        .eq_ignore_ascii_case(&source.to_string_lossy())
    {
        return Err(format!(
            "Файл перевода {:?} относится к другому источнику",
            path
        ));
    }
    Ok(store)
}

pub fn load(source: &Path) -> Result<HashMap<String, String>, String> {
    Ok(read_store(source, &store_path(source))?.entries)
}

/// Файлы мода, собранные из сохранённых правок.
#[derive(Debug, Default)]
pub struct ModExport {
    /// Содержимое файлов по пути относительно `tl/<язык>`.
    pub files: HashMap<String, Vec<u8>>,
    /// Правки, которые не удалось перенести в мод.
    pub skipped: usize,
}

struct SavedSource {
    source: PathBuf,
    entries: BTreeMap<String, String>,
}

fn saved_sources(game_dir: &Path) -> Result<Vec<SavedSource>, String> {
    let dir = get_project_cache_dir(game_dir).join("translations");
    let mut sources = Vec::new();
    if !dir.exists() {
        return Ok(sources);
    }
    for entry in fs::read_dir(&dir)
        .map_err(|e| format!("Не удалось прочитать папку переводов {:?}: {}", dir, e))?
    {
        let path = entry
            .map_err(|e| format!("Не удалось прочитать запись перевода: {}", e))?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|e| format!("Не удалось прочитать перевод {:?}: {}", path, e))?;
        let store: TranslationOverrides = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Повреждён файл перевода {:?}: {}", path, e))?;
        if !store.entries.is_empty() {
            sources.push(SavedSource {
                source: PathBuf::from(store.source_path),
                entries: store.entries.into_iter().collect(),
            });
        }
    }
    sources.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(sources)
}

/// Готовит файлы мода: правки файлов `tl/<язык>` подставляются в их копии, а правки
/// остальных скриптов становятся строковыми переводами, потому что идентификаторы
/// блоков `translate` вычисляет сам Ren'Py. Строковый перевод действует и на диалоги.
pub fn prepare_mod_export(game_dir: &Path, tl_dir: &Path) -> Result<ModExport, String> {
    let mut export = ModExport::default();
    let mut strings = BTreeMap::new();
    for saved in saved_sources(game_dir)? {
        match tl_relative_name(&saved.source) {
            Some(name) => patch_translation_file(&saved, name, &mut export)?,
            None => collect_string_edits(&saved, &mut strings, &mut export.skipped),
        }
    }

    // Повторный `old` в одном языке Ren'Py считает ошибкой и не запускает игру
    let translated = existing_string_sources(tl_dir);
    let before = strings.len();
    strings.retain(|original, _| !translated.contains(original));
    export.skipped += before - strings.len();
    if !strings.is_empty() {
        export
            .files
            .insert(SAVED_STRINGS_FILE.to_string(), render_strings_file(&strings));
    }
    Ok(export)
}

fn patch_translation_file(
    saved: &SavedSource,
    name: String,
    export: &mut ModExport,
) -> Result<(), String> {
    if !saved.source.is_file() {
        export.skipped += saved.entries.len();
        return Ok(());
    }
    let parsed = parse_project_file(saved.source.to_string_lossy().into_owned())?;
    let content = fs::read_to_string(&saved.source)
        .map_err(|e| format!("Не удалось прочитать {:?}: {}", saved.source, e))?;
    let mut lines: Vec<String> = content.split_inclusive('\n').map(str::to_string).collect();

    let mut applied = 0;
    for entry in parsed.scenes.iter().flat_map(|scene| &scene.entries) {
        let Some(text) = saved.entries.get(&entry.translation_key) else {
            continue;
        };
        applied += 1;
        for line_number in &entry.target_lines {
            if let Some(line) = lines.get_mut(line_number - 1)
                && let Some(updated) = RenpyParser::replace_dialogue_text(line, text)
            {
                *line = updated;
            }
        }
    }
    export.skipped += saved.entries.len() - applied;
    if applied > 0 {
        export.files.insert(name, lines.concat().into_bytes());
    }
    Ok(())
}

/// Исходный текст строки из ключа `[строка, сцена, текст, повтор]`.
fn original_text(key: &str) -> Option<String> {
    let (_, _, text, _) = serde_json::from_str::<(usize, String, String, usize)>(key).ok()?;
    Some(text)
}

fn collect_string_edits(
    saved: &SavedSource,
    strings: &mut BTreeMap<String, String>,
    skipped: &mut usize,
) {
    for (key, text) in saved.entries.iter().filter(|(_, text)| !text.trim().is_empty()) {
        let Some(original) = original_text(key) else {
            *skipped += 1;
            continue;
        };
        if strings.entry(original).or_insert_with(|| text.clone()) != text {
            *skipped += 1;
        }
    }
}

fn existing_string_sources(tl_dir: &Path) -> HashSet<String> {
    let mut files = Vec::new();
    collect_rpy_files(tl_dir, &mut files);
    files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .flat_map(|content| RenpyParser::string_translation_sources(&content))
        .collect()
}

fn render_strings_file(strings: &BTreeMap<String, String>) -> Vec<u8> {
    let mut output = format!(
        "# Создано Translator из сохранённых правок.\n\ntranslate {} strings:\n",
        TL_LANG
    );
    for (original, text) in strings {
        output.push_str(&format!(
            "\n    old \"{}\"\n    new \"{}\"\n",
            RenpyParser::escape_renpy_string(original),
            RenpyParser::escape_renpy_string(text)
        ));
    }
    output.into_bytes()
}

pub fn save(source: &Path, changes: &[TranslationEdit]) -> Result<(), String> {
    if !source.is_file() || source.extension().and_then(|e| e.to_str()) != Some("rpy") {
        return Err(format!("Исходный .rpy файл не найден: {:?}", source));
    }
    if changes.is_empty() {
        return Ok(());
    }
    save_at(source, &store_path(source), changes)
}

fn save_at(source: &Path, path: &Path, changes: &[TranslationEdit]) -> Result<(), String> {
    let file_lock = path_lock(path);
    let _guard = file_lock.lock();
    let mut store = read_store(source, path)?;
    for change in changes {
        if change.key.is_empty() {
            return Err("Пустой ключ строки перевода".to_string());
        }
        store
            .entries
            .insert(change.key.clone(), change.text.clone());
    }

    let parent = path
        .parent()
        .ok_or("Не удалось определить папку перевода")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Не удалось создать папку перевода {:?}: {}", parent, e))?;
    let temp_path = parent.join(format!(".{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|e| format!("Не удалось сериализовать перевод: {}", e))?;
    fs::write(&temp_path, bytes)
        .map_err(|e| format!("Не удалось записать перевод {:?}: {}", temp_path, e))?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "Не удалось сохранить перевод {:?}: {}",
            path, error
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_edits_outside_game_and_merges_repeated_saves() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-temp")
            .join(format!("translator_translation_{}", Uuid::new_v4()));
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        let source = game.join("story.rpy");
        let saved_path = root.join("data").join("translation.json");
        fs::write(&source, b"original source").unwrap();
        save_at(
            &source,
            &saved_path,
            &[TranslationEdit {
                key: "line-1".into(),
                text: "Первый".into(),
            }],
        )
        .unwrap();
        save_at(
            &source,
            &saved_path,
            &[TranslationEdit {
                key: "line-2".into(),
                text: "Второй".into(),
            }],
        )
        .unwrap();
        let loaded = read_store(&source, &saved_path).unwrap().entries;
        assert_eq!(loaded.get("line-1").unwrap(), "Первый");
        assert_eq!(loaded.get("line-2").unwrap(), "Второй");
        assert_eq!(fs::read(&source).unwrap(), b"original source");
        assert!(!game.join("translations").exists());
        let _ = fs::remove_dir_all(root);
    }
}
