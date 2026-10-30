use crate::{find_game_root, get_project_cache_dir};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

static SAVE_LOCK: Mutex<()> = Mutex::new(());

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
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.to_string_lossy().to_lowercase().hash(&mut hasher);
    get_project_cache_dir(&find_game_root(source))
        .join("translations")
        .join(format!("{:016x}.json", hasher.finish()))
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

pub fn has_saved_edits(game_dir: &Path) -> Result<bool, String> {
    let dir = get_project_cache_dir(game_dir).join("translations");
    if !dir.exists() {
        return Ok(false);
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
            return Ok(true);
        }
    }
    Ok(false)
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
    let _guard = SAVE_LOCK.lock().map_err(|e| e.to_string())?;
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
