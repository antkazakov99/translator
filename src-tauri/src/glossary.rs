use crate::get_project_cache_dir;
use crate::rpa::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Запись персонажа в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CharacterGlossaryEntry {
    pub alias: String,           // Код/алиас в Ren'Py (например "lis", "emm", "me")
    pub original_name: String,   // Оригинальное имя в игре ("Lisa", "Emma", "Jack")
    pub translated_name: String, // Перевод имени на русский ("Лиза", "Эмма", "Джек")
    pub gender: String,          // "male" (М) | "female" (Ж) | "unknown" (—)
    pub notes: String,           // Заметки для перевода (напр. "обращение на 'ты', дерзкая")
}

/// Запись термина, локации или звания в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TermGlossaryEntry {
    pub id: String,
    pub source_term: String, // Исходный термин ("Demon Hunter", "PPF HQ", "Ancient Rune")
    pub translated_term: String, // Перевод ("Охотник на демонов", "Штаб ППФ", "Древняя руна")
    pub category: String,    // "general" | "location" | "title" | "item"
    pub notes: String,       // Контекст / комментарий
}

/// Единый глоссарий проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectGlossary {
    pub characters: Vec<CharacterGlossaryEntry>,
    pub terms: Vec<TermGlossaryEntry>,
}

impl ProjectGlossary {
    /// Загружает глоссарий из данных приложения. Файл в игре читается только для
    /// совместимости с версиями, которые сохраняли его туда.
    pub fn load_for_project(game_dir: &Path) -> Result<Self, String> {
        let cache_file = get_project_cache_dir(game_dir).join("glossary.json");
        if cache_file.exists() {
            return Self::read_from(&cache_file);
        }

        let game_file = game_dir.join("translator_glossary.json");
        if game_file.exists()
            && let Ok(content) = fs::read_to_string(&game_file)
            && let Ok(glossary) = serde_json::from_str::<ProjectGlossary>(&content)
        {
            return Ok(glossary);
        }

        Ok(Self::default())
    }

    fn read_from(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Не удалось прочитать глоссарий {:?}: {}", path, e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Глоссарий {:?} повреждён: {}", path, e))
    }

    /// Сохраняет глоссарий только в данные приложения, не изменяя папку игры.
    /// Повреждённый файл ещё можно восстановить вручную, поэтому он не заменяется.
    pub fn save_for_project(&self, game_dir: &Path) -> Result<PathBuf, String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Ошибка сериализации глоссария: {}", e))?;

        let cache_file = get_project_cache_dir(game_dir).join("glossary.json");
        if cache_file.exists() {
            Self::read_from(&cache_file)?;
        }
        write_atomic(&cache_file, |out| {
            out.write_all(json.as_bytes()).map_err(|e| {
                format!(
                    "Не удалось сохранить глоссарий в кэш {:?}: {}",
                    cache_file, e
                )
            })
        })?;

        Ok(cache_file)
    }

    /// Объединяет сохраненный глоссарий с новыми персонажами, найденными при сканировании
    pub fn merge_with_scanned_characters(&mut self, scanned: &[(String, String)]) {
        for (alias, name) in scanned {
            if !self
                .characters
                .iter()
                .any(|c| c.alias.eq_ignore_ascii_case(alias))
            {
                let gender = auto_detect_gender(name);
                self.characters.push(CharacterGlossaryEntry {
                    alias: alias.clone(),
                    original_name: name.clone(),
                    translated_name: String::new(),
                    gender,
                    notes: String::new(),
                });
            }
        }
        self.characters
            .sort_by(|a, b| a.original_name.cmp(&b.original_name));
    }
}

/// Автоматическое первичное определение пола по англоязычным маркерам в имени
fn auto_detect_gender(name: &str) -> String {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    for w in &words {
        match *w {
            "girl" | "woman" | "maid" | "waitress" | "lady" | "queen" | "princess" | "sister"
            | "mother" | "daughter" => {
                return "female".to_string();
            }
            "boy" | "man" | "guy" | "king" | "prince" | "brother" | "father" | "son" | "hunter"
            | "bartender" | "guard" => {
                return "male".to_string();
            }
            "creature" | "monster" | "beast" | "entity" | "ghost" | "spirit" | "slime"
            | "demon" | "shadow" | "thing" => {
                return "neuter".to_string();
            }
            _ => {}
        }
    }

    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_damaged_glossary_is_reported_and_kept() {
        let game_dir =
            std::env::temp_dir().join(format!("translator_glossary_{}", uuid::Uuid::new_v4()));
        let cache_dir = get_project_cache_dir(&game_dir);
        fs::create_dir_all(&cache_dir).unwrap();
        let file = cache_dir.join("glossary.json");
        fs::write(&file, "{\"characters\": [").unwrap();

        let error = ProjectGlossary::load_for_project(&game_dir).unwrap_err();
        assert!(error.contains("повреждён"), "{error}");
        assert!(ProjectGlossary::default().save_for_project(&game_dir).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\"characters\": [");

        fs::remove_file(&file).unwrap();
        ProjectGlossary::default().save_for_project(&game_dir).unwrap();
        assert!(ProjectGlossary::load_for_project(&game_dir).is_ok());
        let _ = fs::remove_dir_all(&cache_dir);
    }

    #[test]
    fn test_glossary_serialization_and_merge() {
        let mut glossary = ProjectGlossary::default();
        let scanned = vec![
            ("lis".to_string(), "Lisa".to_string()),
            ("wg".to_string(), "The warrior girl".to_string()),
            ("m".to_string(), "The man".to_string()),
            ("cr".to_string(), "Shadow Creature".to_string()),
        ];

        glossary.merge_with_scanned_characters(&scanned);
        assert_eq!(glossary.characters.len(), 4);

        let wg = glossary
            .characters
            .iter()
            .find(|c| c.alias == "wg")
            .unwrap();
        assert_eq!(wg.gender, "female");

        let m = glossary.characters.iter().find(|c| c.alias == "m").unwrap();
        assert_eq!(m.gender, "male");

        let cr = glossary
            .characters
            .iter()
            .find(|c| c.alias == "cr")
            .unwrap();
        assert_eq!(cr.gender, "neuter");

        let json = serde_json::to_string_pretty(&glossary).unwrap();
        assert!(json.contains("warrior girl"));
    }
}
