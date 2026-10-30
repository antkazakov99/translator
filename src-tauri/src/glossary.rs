use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::get_project_cache_dir;

/// Запись персонажа в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CharacterGlossaryEntry {
    pub alias: String,             // Код/алиас в Ren'Py (например "lis", "emm", "me")
    pub original_name: String,     // Оригинальное имя в игре ("Lisa", "Emma", "Jack")
    pub translated_name: String,   // Перевод имени на русский ("Лиза", "Эмма", "Джек")
    pub gender: String,            // "male" (М) | "female" (Ж) | "unknown" (—)
    pub notes: String,             // Заметки для перевода (напр. "обращение на 'ты', дерзкая")
}

/// Запись термина, локации или звания в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TermGlossaryEntry {
    pub id: String,
    pub source_term: String,       // Исходный термин ("Demon Hunter", "PPF HQ", "Ancient Rune")
    pub translated_term: String,   // Перевод ("Охотник на демонов", "Штаб ППФ", "Древняя руна")
    pub category: String,          // "general" | "location" | "title" | "item"
    pub notes: String,             // Контекст / комментарий
}

/// Единый глоссарий проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectGlossary {
    pub characters: Vec<CharacterGlossaryEntry>,
    pub terms: Vec<TermGlossaryEntry>,
}

impl ProjectGlossary {
    /// Загружает глоссарий для игры: сначала проверяет папку игры, затем кэш в AppData
    pub fn load_for_project(game_dir: &Path) -> Self {
        let game_file = game_dir.join("translator_glossary.json");
        if game_file.exists() {
            if let Ok(content) = fs::read_to_string(&game_file) {
                if let Ok(glossary) = serde_json::from_str::<ProjectGlossary>(&content) {
                    return glossary;
                }
            }
        }

        let cache_file = get_project_cache_dir(game_dir).join("glossary.json");
        if cache_file.exists() {
            if let Ok(content) = fs::read_to_string(&cache_file) {
                if let Ok(glossary) = serde_json::from_str::<ProjectGlossary>(&content) {
                    return glossary;
                }
            }
        }

        Self::default()
    }

    /// Сохраняет глоссарий: пытается записать в папку игры и всегда сохраняет в изолированный кэш AppData
    pub fn save_for_project(&self, game_dir: &Path) -> Result<PathBuf, String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Ошибка сериализации глоссария: {}", e))?;

        // 1. Всегда гарантированно пишем в изолированный кэш
        let cache_dir = get_project_cache_dir(game_dir);
        let _ = fs::create_dir_all(&cache_dir);
        let cache_file = cache_dir.join("glossary.json");
        fs::write(&cache_file, &json)
            .map_err(|e| format!("Не удалось сохранить глоссарий в кэш {:?}: {}", cache_file, e))?;

        // 2. Если папка игры доступна для записи — сохраняем и туда (удобно для git и передачи коллегам)
        let game_file = game_dir.join("translator_glossary.json");
        let target = if fs::write(&game_file, &json).is_ok() {
            game_file
        } else {
            cache_file
        };

        Ok(target)
    }

    /// Объединяет сохраненный глоссарий с новыми персонажами, найденными при сканировании
    pub fn merge_with_scanned_characters(&mut self, scanned: &[(String, String)]) {
        for (alias, name) in scanned {
            if !self.characters.iter().any(|c| c.alias.eq_ignore_ascii_case(alias)) {
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
        self.characters.sort_by(|a, b| a.original_name.cmp(&b.original_name));
    }
}

/// Автоматическое первичное определение пола по англоязычным маркерам в имени
fn auto_detect_gender(name: &str) -> String {
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).collect();

    for w in &words {
        match *w {
            "girl" | "woman" | "maid" | "waitress" | "lady" | "queen" | "princess" | "sister" | "mother" | "daughter" => {
                return "female".to_string();
            }
            "boy" | "man" | "guy" | "king" | "prince" | "brother" | "father" | "son" | "hunter" | "bartender" | "guard" => {
                return "male".to_string();
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
    fn test_glossary_serialization_and_merge() {
        let mut glossary = ProjectGlossary::default();
        let scanned = vec![
            ("lis".to_string(), "Lisa".to_string()),
            ("wg".to_string(), "The warrior girl".to_string()),
            ("m".to_string(), "The man".to_string()),
        ];

        glossary.merge_with_scanned_characters(&scanned);
        assert_eq!(glossary.characters.len(), 3);

        let wg = glossary.characters.iter().find(|c| c.alias == "wg").unwrap();
        assert_eq!(wg.gender, "female");

        let m = glossary.characters.iter().find(|c| c.alias == "m").unwrap();
        assert_eq!(m.gender, "male");

        let json = serde_json::to_string_pretty(&glossary).unwrap();
        assert!(json.contains("warrior girl"));
    }
}
