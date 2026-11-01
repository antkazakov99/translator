use crate::get_project_cache_dir;
use crate::rpa::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Запись, вошедшая в другого персонажа: автор игры завёл для одного героя несколько кодов.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MergedCharacter {
    pub alias: String,
    pub original_name: String,
}

/// Другая форма той же записи со своим переводом: сокращение термина («PPF») или
/// обращение к персонажу («Miss Granger»). Пустой перевод значит перевод самой записи.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NameForm {
    pub source: String,
    pub translated: String,
}

/// Запись персонажа в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CharacterGlossaryEntry {
    pub alias: String,           // Код/алиас в Ren'Py ("lis", "emm", "me"). Персонаж, которого только упоминают, вместо кода получает внутренний номер с "~"
    pub original_name: String,   // Оригинальное имя в игре ("Lisa", "Emma", "Jack")
    pub translated_name: String, // Перевод имени на русский ("Лиза", "Эмма", "Джек")
    pub gender: String,          // "male" (М) | "female" (Ж) | "unknown" (—)
    pub notes: String,           // Описание персонажа
    #[serde(default)]
    pub speech: String,          // Манера речи (напр. "дерзко, со сленгом, на 'ты'")
    #[serde(default)]
    pub excluded_scenes: Vec<String>, // Сцены, которые не передаются модели
    #[serde(default)]
    pub merged: Vec<MergedCharacter>, // Другие коды этого же персонажа
    #[serde(default)]
    pub excluded_notes: Vec<String>, // Заметки частей, которые не идут в итоговое описание
    #[serde(default)]
    pub forms: Vec<NameForm>, // Другие имена и обращения ("Miss Granger")
    #[serde(default)]
    pub first_name: String, // Каноничное имя ("Lisa"). Вместе с фамилией это подпись записи, а реплики и запросы к модели идут с `original_name`
    #[serde(default)]
    pub last_name: String, // Каноничная фамилия ("Smith")
    #[serde(default)]
    pub translated_first_name: String,
    #[serde(default)]
    pub translated_last_name: String,
    #[serde(default, skip_serializing)]
    pub card_name: String, // Прежняя подпись записи: читается только для переноса в имя и фамилию
}

impl CharacterGlossaryEntry {
    pub fn aliases(&self) -> Vec<&str> {
        std::iter::once(self.alias.as_str())
            .chain(self.merged.iter().map(|m| m.alias.as_str()))
            .collect()
    }

    pub fn names(&self) -> Vec<&str> {
        std::iter::once(self.original_name.as_str())
            .chain(self.merged.iter().map(|m| m.original_name.as_str()))
            .chain(self.forms.iter().map(|form| form.source.as_str()))
            .chain(
                [self.first_name.trim(), self.last_name.trim()]
                    .into_iter()
                    .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(self.original_name.trim())),
            )
            .collect()
    }

    /// Имя по умолчанию это имя из игры с его переводом. Фамилию вносит пользователь.
    fn default_first_name(&mut self) {
        if self.first_name.trim().is_empty() {
            self.first_name = self.original_name.trim().to_string();
        }
        if self.translated_first_name.trim().is_empty() && self.first_name.eq_ignore_ascii_case(self.original_name.trim()) {
            self.translated_first_name = self.translated_name.trim().to_string();
        }
    }

    pub fn has_alias(&self, alias: &str) -> bool {
        self.aliases().iter().any(|own| own.eq_ignore_ascii_case(alias))
    }

    /// Раскладывает прежнюю подпись на имя и фамилию. Разделить можно только два слова,
    /// остальное целиком становится именем: фамилию из него выделит пользователь.
    fn adopt_card_name(&mut self) {
        let card = std::mem::take(&mut self.card_name);
        let words: Vec<&str> = card.split_whitespace().collect();
        if words.is_empty() || !self.first_name.is_empty() || !self.last_name.is_empty() {
            return;
        }
        match words[..] {
            [first, last] => {
                self.first_name = first.to_string();
                self.last_name = last.to_string();
            }
            _ => self.first_name = words.join(" "),
        }
        // Форма с тем же написанием теперь отдельное поле
        let (first_name, last_name) = (self.first_name.clone(), self.last_name.clone());
        self.forms.retain(|form| {
            let source = form.source.trim();
            let own = if source.eq_ignore_ascii_case(&first_name) {
                &mut self.translated_first_name
            } else if source.eq_ignore_ascii_case(&last_name) {
                &mut self.translated_last_name
            } else {
                return true;
            };
            if !form.translated.trim().is_empty() {
                own.clone_from(&form.translated);
            }
            false
        });
    }
}

/// Запись термина, локации или звания в глоссарии проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TermGlossaryEntry {
    pub id: String,
    pub source_term: String, // Исходный термин ("Demon Hunter", "PPF HQ", "Ancient Rune")
    pub translated_term: String, // Перевод ("Охотник на демонов", "Штаб ППФ", "Древняя руна")
    pub category: String,    // "general" | "location" | "organization" | "title" | "item"
    pub notes: String,       // Контекст / комментарий
    #[serde(default)]
    pub forms: Vec<NameForm>, // Сокращения и короткие формы ("PPF", "Academy")
    #[serde(default)]
    pub excluded_scenes: Vec<String>, // Сцены, которые не передаются модели
    #[serde(default)]
    pub excluded_notes: Vec<String>, // Заметки частей, которые не идут в итоговое описание
}

impl TermGlossaryEntry {
    pub fn names(&self) -> Vec<String> {
        std::iter::once(&self.source_term)
            .chain(self.forms.iter().map(|form| &form.source))
            .cloned()
            .collect()
    }

    /// Чьи заметки: в общем файле они не должны совпасть с заметками персонажа.
    pub fn note_owner(&self) -> String {
        format!("term:{}", self.id)
    }
}

/// Подстановка в репликах вроде `[landlady]`
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TokenGlossaryEntry {
    pub name: String, // Имя переменной без скобок и флагов ("landlady", "friend.name")
    pub note: String, // Что она значит: при переводе токен останется в тексте, и модели нужно это знать
    #[serde(default)]
    pub value: String, // Что подставить вместо токена при чтении сцен ("мать")
}

/// Единый глоссарий проекта
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectGlossary {
    pub characters: Vec<CharacterGlossaryEntry>,
    pub terms: Vec<TermGlossaryEntry>,
    #[serde(default)]
    pub tokens: Vec<TokenGlossaryEntry>,
    #[serde(default)]
    pub removed_characters: Vec<MergedCharacter>, // Персонажи из скриптов, которых удалил пользователь: скан не возвращает их
    #[serde(default)]
    pub discarded_terms: Vec<String>, // Названия из архива, которые пользователь счёл мусором ("Oh God")
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
            return Ok(glossary.upgraded());
        }

        Ok(Self::default())
    }

    fn read_from(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Не удалось прочитать глоссарий {:?}: {}", path, e))?;
        serde_json::from_str(&content)
            .map(Self::upgraded)
            .map_err(|e| format!("Глоссарий {:?} повреждён: {}", path, e))
    }

    fn upgraded(mut self) -> Self {
        for character in &mut self.characters {
            character.adopt_card_name();
            character.default_first_name();
        }
        self
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

    /// Дополняет глоссарий записями другого проекта и возвращает их число. Заполненные
    /// здесь записи важнее, а пустые могли появиться сами при сканировании.
    pub fn merge_from(&mut self, other: Self) -> usize {
        let mut merged = 0;
        for character in other.characters {
            let own = self
                .characters
                .iter_mut()
                .find(|own| own.has_alias(&character.alias));
            match own {
                Some(own) if own.translated_name.trim().is_empty() => {
                    if !character.translated_name.trim().is_empty() {
                        let own_merged = std::mem::take(&mut own.merged);
                        *own = character;
                        own.merged.extend(own_merged);
                        merged += 1;
                    }
                }
                Some(_) => {}
                None => {
                    self.characters.push(character);
                    merged += 1;
                }
            }
        }
        for term in other.terms {
            let own = self
                .terms
                .iter_mut()
                .find(|own| own.source_term.eq_ignore_ascii_case(&term.source_term));
            match own {
                Some(own) if own.translated_term.trim().is_empty() => {
                    if !term.translated_term.trim().is_empty() {
                        *own = term;
                        merged += 1;
                    }
                }
                Some(_) => {}
                None => {
                    self.terms.push(term);
                    merged += 1;
                }
            }
        }
        for token in other.tokens {
            if !self.tokens.iter().any(|own| own.name == token.name) {
                self.tokens.push(token);
                merged += 1;
            }
        }
        for removed in other.removed_characters {
            let known = self.is_removed(&removed.alias) || self.characters.iter().any(|own| own.has_alias(&removed.alias));
            if !known {
                self.removed_characters.push(removed);
            }
        }
        for term in other.discarded_terms {
            if !self.discarded_terms.iter().any(|own| own.eq_ignore_ascii_case(&term)) {
                self.discarded_terms.push(term);
            }
        }
        merged
    }

    fn is_removed(&self, alias: &str) -> bool {
        self.removed_characters.iter().any(|removed| removed.alias.eq_ignore_ascii_case(alias))
    }

    /// Объединяет сохраненный глоссарий с новыми персонажами, найденными при сканировании
    pub fn merge_with_scanned_characters(&mut self, scanned: &[(String, String)]) {
        for (alias, name) in scanned {
            if !self.is_removed(alias)
                && !self
                    .characters
                    .iter()
                    .any(|c| c.has_alias(alias))
            {
                let gender = auto_detect_gender(name);
                self.characters.push(CharacterGlossaryEntry {
                    alias: alias.clone(),
                    original_name: name.clone(),
                    translated_name: String::new(),
                    gender,
                    first_name: name.trim().to_string(),
                    ..Default::default()
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
    fn test_discarded_terms_are_optional_and_survive_a_transfer() {
        let old: ProjectGlossary = serde_json::from_str(r#"{"characters": [], "terms": []}"#).unwrap();
        assert!(old.discarded_terms.is_empty());

        let mut own = ProjectGlossary { discarded_terms: vec!["Oh God".to_string()], ..Default::default() };
        let other = ProjectGlossary { discarded_terms: vec!["oh god".to_string(), "Yes Master".to_string()], ..Default::default() };
        own.merge_from(other);
        assert_eq!(own.discarded_terms, ["Oh God", "Yes Master"]);
    }

    #[test]
    fn test_removed_character_does_not_come_back_with_a_scan() {
        let old: ProjectGlossary = serde_json::from_str(r#"{"characters": [], "terms": []}"#).unwrap();
        assert!(old.removed_characters.is_empty());

        let removed = MergedCharacter { alias: "var".to_string(), original_name: "Var".to_string() };
        let mut glossary = ProjectGlossary { removed_characters: vec![removed.clone()], ..Default::default() };
        glossary.merge_with_scanned_characters(&[
            ("VAR".to_string(), "Var".to_string()),
            ("lis".to_string(), "Lisa".to_string()),
        ]);
        let aliases: Vec<_> = glossary.characters.iter().map(|c| c.alias.as_str()).collect();
        assert_eq!(aliases, ["lis"]);

        // Удалённый в другой копии персонаж остаётся удалённым, а тот, что здесь есть, никуда не девается
        let other = ProjectGlossary {
            removed_characters: vec![
                MergedCharacter { alias: "lis".to_string(), original_name: "Lisa".to_string() },
                MergedCharacter { alias: "old".to_string(), original_name: "Old".to_string() },
                removed,
            ],
            ..Default::default()
        };
        glossary.merge_from(other);
        let removed: Vec<_> = glossary.removed_characters.iter().map(|r| r.alias.as_str()).collect();
        assert_eq!(removed, ["var", "old"]);
    }

    #[test]
    fn test_canonical_name_is_optional_and_empty_one_is_not_a_name() {
        let old: CharacterGlossaryEntry = serde_json::from_str(
            r#"{"alias": "lis", "original_name": "Lisa", "translated_name": "", "gender": "female", "notes": ""}"#,
        )
        .unwrap();
        assert_eq!(old.names(), ["Lisa"]);

        let named = CharacterGlossaryEntry { first_name: " Lisa ".to_string(), last_name: "Smith".to_string(), ..old };
        assert_eq!(named.names(), ["Lisa", "Smith"]);
    }

    #[test]
    fn test_first_name_defaults_to_the_name_from_the_game() {
        let glossary = with_card_name("", Vec::new());
        let lisa = &glossary.characters[0];
        assert_eq!((lisa.first_name.as_str(), lisa.translated_first_name.as_str()), ("Lisa", "Лиза"));
        assert_eq!(lisa.last_name, "");

        let mut scanned = ProjectGlossary::default();
        scanned.merge_with_scanned_characters(&[("em".to_string(), "Emma".to_string())]);
        assert_eq!(scanned.characters[0].first_name, "Emma");
    }

    fn with_card_name(card_name: &str, forms: Vec<NameForm>) -> ProjectGlossary {
        let lisa = CharacterGlossaryEntry {
            alias: "lis".to_string(),
            original_name: "Lisa".to_string(),
            translated_name: "Лиза".to_string(),
            card_name: card_name.to_string(),
            forms,
            ..Default::default()
        };
        ProjectGlossary { characters: vec![lisa], ..Default::default() }.upgraded()
    }

    #[test]
    fn test_card_name_of_two_words_becomes_first_and_last_name() {
        let form = |source: &str, translated: &str| NameForm { source: source.to_string(), translated: translated.to_string() };
        let glossary = with_card_name(" Lisa  Smith ", vec![form("smith", "Смит"), form("Miss Smith", "Мисс Смит")]);
        let lisa = &glossary.characters[0];

        assert_eq!((lisa.first_name.as_str(), lisa.last_name.as_str()), ("Lisa", "Smith"));
        assert_eq!((lisa.translated_first_name.as_str(), lisa.translated_last_name.as_str()), ("Лиза", "Смит"));
        assert_eq!(lisa.forms, [form("Miss Smith", "Мисс Смит")]);
        assert!(!serde_json::to_string(&glossary).unwrap().contains("card_name"));
    }

    #[test]
    fn test_other_card_names_stay_in_the_first_name() {
        let glossary = with_card_name("Mary Jane Watson", Vec::new());
        assert_eq!(glossary.characters[0].first_name, "Mary Jane Watson");
        assert_eq!(glossary.characters[0].last_name, "");
        assert_eq!(glossary.characters[0].translated_first_name, "");
    }

    #[test]
    fn test_tokens_are_optional_and_own_note_wins() {
        let old: ProjectGlossary = serde_json::from_str(r#"{"characters": [], "terms": []}"#).unwrap();
        assert!(old.tokens.is_empty());

        let token = |name: &str, note: &str| TokenGlossaryEntry { name: name.to_string(), note: note.to_string(), ..Default::default() };
        let mut own = ProjectGlossary { tokens: vec![token("landlady", "своё")], ..Default::default() };
        let other = ProjectGlossary { tokens: vec![token("landlady", "чужое"), token("nick", "прозвище")], ..Default::default() };
        assert_eq!(own.merge_from(other), 1);
        assert_eq!(own.tokens, [token("landlady", "своё"), token("nick", "прозвище")]);
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

        // Вошедший в другого персонажа код скан не возвращает отдельной записью
        glossary.characters[0].merged.push(MergedCharacter {
            alias: "merged_code".to_string(),
            original_name: "Twin".to_string(),
        });
        glossary.merge_with_scanned_characters(&[("merged_code".to_string(), "Twin".to_string())]);
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
