//! Предложения модели по записям глоссария: перевод терминов, перевод подписей говорящих, значения токенов.
//! Промпты, разбор ответа и проверка. Годное пишет `GlossaryService::accept_*`: цепочка после пакета, окно после
//! отбора. Разбор ответа общий с именами (`names.rs`).

use std::collections::HashMap;

use serde::Deserialize;

use super::{TermCategory, TermId};
use crate::common::Gender;
use crate::engines::SpeakerKey;
use crate::llm::{ChatRequest, LlmTarget};
use crate::translation::Hints;
use crate::translation::answer::json_array;
use crate::translation::prompt::glossary_hints;
use crate::translation::text::{fill_tokens, markers};

/// Сколько реплик говорящего видит модель.
pub const SPEAKER_SAMPLES: usize = 4;
/// Сколько реплик с термином видит модель.
pub const TERM_SAMPLES: usize = 3;
/// Сколько реплик с токеном видит модель.
pub const TOKEN_SAMPLES: usize = 8;
/// Чем заменён разбираемый токен: имени токена модель верит больше, чем репликам.
pub const TOKEN_GAP: &str = "[___]";

/// Лимит ответа: не меньше этого и не меньше `*_ITEM_TOKENS` на запись пакета (`answer_tokens`).
const TERM_ANSWER_TOKENS: u32 = 2048;
const TERM_ITEM_TOKENS: u32 = 128;
const SPEAKER_ANSWER_TOKENS: u32 = 1536;
const SPEAKER_ITEM_TOKENS: u32 = 128;
const TOKEN_ANSWER_TOKENS: u32 = 1536;
const TOKEN_ITEM_TOKENS: u32 = 192;

const TERMS_SYSTEM: &str = r#"You are an expert game localization specialist translating English terms of a visual novel into Russian.
Translate in-game terms, locations, factions, artifacts, ranks, items and events into authentic Russian.

For each term output:
1. "id": EXACT id as given.
2. "translation": Accurate, authentic Russian translation in the nominative case.
   Examples: "Demon Hunter" -> "Охотник на демонов", "PPF Headquarters" -> "Штаб ППФ", "Ancient Rune" -> "Древняя руна", "Silver Key" -> "Серебряный ключ".
3. "category": One of "general", "location", "organization", "title", "item", "event". Use "organization" for companies, factions, agencies, gangs, schools as institutions and other groups of people. Use "event" for wars, battles, disasters, holidays, tournaments and other things that happen at a particular time.

"examples" are lines of the game where the term appears: use them to understand what the term means. "about" is what is already known about the term. Text in [square brackets] is an in-game variable: keep it unchanged.
A line starting with "Russian names:" gives the exact spelling of names: a term that contains a name uses that spelling. A line starting with "Known terms:" gives established translations: keep translations consistent with them, a term that contains a known term uses its translation.

Respond ONLY with a valid JSON array, without markdown fences or explanations:
[
  {"id": 1, "translation": "Штаб ППФ", "category": "location"}
]"#;

const SPEAKERS_SYSTEM: &str = r#"You translate speaker labels of a visual novel from English into Russian.
A speaker label is the name the game shows above a line when the speaker is not a regular character: a role ("Waitress"), a device ("Radio Chatter", "Cell phone"), a crowd ("Students") or several people at once ("Celia & Katy").

For each label output:
1. "id": EXACT id as given.
2. "translation": A short Russian label in the nominative case, written the way a Russian game would show it above the line: "Waitress" is "Официантка", "Radio Chatter" is "Радио", "Man 1" is "Мужчина 1", "???" stays "???". Keep [square bracket] variables exactly as written. Keep the capitalization style of the original: "RADIO" is "РАДИО".
3. "gender": "male", "female" or "unknown", the grammatical gender verbs must agree with when this speaker says "I". Use the lines to decide; "unknown" for a group, a device or when the lines do not settle it.

A line starting with "Russian names:" gives the exact spelling of names: use it inside labels like "Celia & Katy". A line starting with "Known terms:" gives established translations.

Respond ONLY with a valid JSON array, without markdown fences or explanations:
[
  {"id": 1, "translation": "Официантка", "gender": "female"},
  {"id": 2, "translation": "Радио", "gender": "unknown"}
]"#;

const TOKENS_SYSTEM: &str = r#"You study text variables of a visual novel that is being translated into Russian.
The game replaces each variable with a string at run time. In the lines below the variable is shown as [___].

Work out from the lines what each variable stands for. Rely only on the lines. If the lines do not show what it means, leave the field empty instead of guessing.

For each variable output:
1. "id": EXACT id as given.
2. "note": One short sentence in Russian for a translator: what the variable stands for and who it refers to.
3. "value": A Russian word or phrase in the nominative case that reads correctly in place of [___]. Empty string if the player types it in, if it changes during the game, or if the lines do not settle it.

A line starting with "Russian names:" gives the exact spelling of names. A line starting with "Known terms:" gives established translations.

Respond ONLY with a valid JSON array, without markdown fences or explanations:
[
  {"id": 1, "note": "Кем Селия приходится главному герою", "value": "мать"},
  {"id": 2, "note": "Имя главного героя, его вводит игрок", "value": ""}
]"#;

/// Термин в запросе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermItem {
    pub id: TermId,
    pub name: String,
    pub category: TermCategory,
    /// Своё описание: модель его не заменяет, а читает.
    pub description: Option<String>,
    /// Реплики с термином вразброс по игре: упоминание решает `mentions::mentioned`.
    pub examples: Vec<String>,
}

/// Говорящий в запросе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerItem {
    pub key: SpeakerKey,
    pub label: String,
    pub lines: Vec<String>,
}

/// Токен в запросе: реплики уже с `[___]` вместо него.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenItem {
    pub name: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermSuggestion {
    pub id: TermId,
    pub translation: String,
    /// Только у термина категории `general`: категорию разбора и пользователя перевод не меняет.
    /// `None`: остаётся прежняя.
    pub category: Option<TermCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerSuggestion {
    pub key: SpeakerKey,
    pub translation: String,
    pub gender: Option<Gender>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSuggestion {
    pub name: String,
    pub value: Option<String>,
    pub note: Option<String>,
}

/// Запись ответа. Поля под разными именами: модели переписывают их из примеров 1.0 и из своих привычек.
#[derive(Debug, Default, Deserialize)]
pub(super) struct AnswerItem {
    pub(super) id: Option<serde_json::Value>,
    #[serde(alias = "translated_term", alias = "translated", alias = "translated_name")]
    pub(super) translation: Option<String>,
    category: Option<String>,
    #[serde(alias = "notes", alias = "description")]
    note: Option<String>,
    gender: Option<String>,
    value: Option<String>,
    /// Написания имени значением, а не полем записи: сломанный список не теряет всей записи.
    pub(super) variants: Option<serde_json::Value>,
    /// Замечания проверки написаний имён.
    pub(super) issues: Option<serde_json::Value>,
    /// Коды языков разбора происхождения.
    pub(super) languages: Option<serde_json::Value>,
    pub(super) reason: Option<String>,
}

/// Лимит ответа пакета: не меньше `base` и не меньше `per_item` на запись. Размер пакета задаётся в настройках
/// действия, и постоянный лимит обрезал бы ответ на большой пакет.
pub(super) fn answer_tokens(base: u32, per_item: u32, items: usize) -> u32 {
    base.max(per_item.saturating_mul(u32::try_from(items).unwrap_or(u32::MAX)))
}

/// Записи ответа с номером пакета. Номер принимается числом и строкой, в том числе с буквой вроде `t3`.
/// Одну запись локальная модель иногда возвращает объектом вместо массива. Тогда первым массивом в тексте
/// оказывается вложенный, у его записей нет `id`. Обрезанный массив целиком не читается: из него берутся
/// готовые записи.
fn answer_items(text: &str) -> Vec<(usize, AnswerItem)> {
    let values = serde_json::from_str::<Vec<serde_json::Value>>(json_array(text))
        .ok()
        .filter(|values| values.is_empty() || values.iter().any(|value| value.get("id").is_some()))
        .unwrap_or_else(|| objects(text));
    values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<AnswerItem>(value).ok())
        .filter_map(|item| {
            let number = match item.id.as_ref()? {
                serde_json::Value::Number(number) => number.as_u64().map(|number| number as usize),
                serde_json::Value::String(text) => text.trim().trim_start_matches(|c: char| c.is_alphabetic()).parse().ok(),
                _ => None,
            }?;
            Some((number, item))
        })
        .collect()
}

/// Объекты с полем `id` в тексте по порядку: одна запись объектом вместо массива и записи массива, который
/// модель не дописала. Оборванная запись и вложенные в неё объекты пропускаются.
fn objects(text: &str) -> Vec<serde_json::Value> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('{') {
        let start = from + offset;
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(value)) if value.get("id").is_some() => {
                found.push(value);
                from = start + stream.byte_offset();
            }
            _ => from = start + 1,
        }
    }
    found
}

/// Записи ответа по записям пакета: номер с 1, первая запись на номер побеждает.
pub(super) fn by_number<'a, T>(text: &str, batch: &'a [T]) -> Vec<(&'a T, AnswerItem)> {
    let mut seen = vec![false; batch.len()];
    answer_items(text)
        .into_iter()
        .filter_map(|(number, item)| {
            let index = number.checked_sub(1)?;
            let entry = batch.get(index)?;
            (!std::mem::replace(&mut seen[index], true)).then_some((entry, item))
        })
        .collect()
}

/// Текст поля без пробелов по краям, одной строкой.
pub(super) fn field(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty() && !text.contains('\n')).then(|| text.to_string())
}

/// Перевод названия годится, если он одной строкой и сохранил токены и теги оригинала.
pub(super) fn usable_label(original: &str, translated: &str) -> bool {
    markers(original) == markers(translated)
}

fn gender(text: Option<&str>) -> Option<Gender> {
    match text?.trim().to_lowercase().as_str() {
        "male" | "m" | "мужской" => Some(Gender::Male),
        "female" | "f" | "женский" => Some(Gender::Female),
        _ => None,
    }
}

/// Строка значения в запрос. Сериализация строки не падает.
pub(super) fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// Подсказки глоссария по тексту пакета, абзацами перед списком.
pub(super) fn hints_block(text: &str, hints: &Hints) -> String {
    let lines: Vec<(&str, &str)> = text.lines().map(|line| (line, "")).collect();
    glossary_hints(&lines, hints).iter().map(|hint| format!("{hint}\n\n")).collect()
}

/// `count` записей равномерно по всему списку, по порядку.
pub fn spread<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    let count = count.min(items.len());
    (0..count).map(|index| items[index * items.len() / count].clone()).collect()
}

/// Реплика образца: говорящий, текст одной строкой, значения токенов подставлены.
pub fn sample_line(speaker: Option<&str>, text: &str, values: &HashMap<String, String>) -> String {
    let text = fill_tokens(&text.replace('\n', " "), values);
    match speaker {
        Some(speaker) => format!("{speaker}: {text}"),
        None => text,
    }
}

/// Реплика с разбираемым токеном: он заменён на `[___]`, остальные токены со значением подставлены.
pub fn token_line(speaker: Option<&str>, text: &str, token: &str, values: &HashMap<String, String>) -> String {
    let mut values = values.clone();
    values.insert(token.to_string(), TOKEN_GAP.to_string());
    sample_line(speaker, text, &values)
}

pub struct TermsPrompt;

impl TermsPrompt {
    pub fn build(batch: &[TermItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let text: String = batch.iter().flat_map(|term| std::iter::once(&term.name).chain(&term.examples)).map(|line| format!("{line}\n")).collect();
        let mut list = String::new();
        for (index, term) in batch.iter().enumerate() {
            list.push_str(&format!("- id: {}, term: {}, current_category: {}", index + 1, quoted(&term.name), quoted(term.category.as_str())));
            if let Some(description) = &term.description {
                list.push_str(&format!(", about: {}", quoted(description)));
            }
            if !term.examples.is_empty() {
                let examples: Vec<String> = term.examples.iter().map(|example| quoted(example)).collect();
                list.push_str(&format!(", examples: [{}]", examples.join(", ")));
            }
            list.push('\n');
        }
        ChatRequest {
            system: TERMS_SYSTEM.to_string(),
            user: format!("{}Terms to translate into Russian:\n{list}\nProvide JSON array now:", hints_block(&text, hints)),
            temperature: target.settings.temperature,
            max_tokens: answer_tokens(TERM_ANSWER_TOKENS, TERM_ITEM_TOKENS, batch.len()),
        }
    }

    /// Перевод одной строкой с токенами оригинала. Категория только из списка и только термину категории `general`.
    pub fn accept(text: &str, batch: &[TermItem]) -> Vec<TermSuggestion> {
        by_number(text, batch)
            .into_iter()
            .filter_map(|(term, item)| {
                let translation = field(item.translation.as_deref()).filter(|text| usable_label(&term.name, text))?;
                let category = item
                    .category
                    .as_deref()
                    .filter(|_| term.category == TermCategory::General)
                    .and_then(|text| TermCategory::parse(&text.trim().to_lowercase()).ok())
                    .filter(|category| *category != TermCategory::General);
                Some(TermSuggestion { id: term.id, translation, category })
            })
            .collect()
    }
}

pub struct SpeakersPrompt;

impl SpeakersPrompt {
    pub fn build(batch: &[SpeakerItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let text: String =
            batch.iter().flat_map(|speaker| std::iter::once(&speaker.label).chain(&speaker.lines)).map(|line| format!("{line}\n")).collect();
        let mut list = String::new();
        for (index, speaker) in batch.iter().enumerate() {
            list.push_str(&format!("\nid: {}\nLabel: {}\nLines:\n", index + 1, speaker.label));
            for line in &speaker.lines {
                list.push_str(&format!("{line}\n"));
            }
        }
        ChatRequest {
            system: SPEAKERS_SYSTEM.to_string(),
            user: format!("{}Speaker labels:\n{list}\nProvide JSON array now:", hints_block(&text, hints)),
            temperature: target.settings.temperature,
            max_tokens: answer_tokens(SPEAKER_ANSWER_TOKENS, SPEAKER_ITEM_TOKENS, batch.len()),
        }
    }

    /// Перевод с теми же токенами, что подпись: переменную в подписи подставит игра, без неё показался бы не тот.
    pub fn accept(text: &str, batch: &[SpeakerItem]) -> Vec<SpeakerSuggestion> {
        by_number(text, batch)
            .into_iter()
            .filter_map(|(speaker, item)| {
                let translation = field(item.translation.as_deref()).filter(|text| usable_label(&speaker.label, text))?;
                Some(SpeakerSuggestion { key: speaker.key.clone(), translation, gender: gender(item.gender.as_deref()) })
            })
            .collect()
    }
}

pub struct TokensPrompt;

impl TokensPrompt {
    pub fn build(batch: &[TokenItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let text: String = batch.iter().flat_map(|token| &token.lines).map(|line| format!("{line}\n")).collect();
        let mut list = String::new();
        for (index, token) in batch.iter().enumerate() {
            list.push_str(&format!("\nid: {}\nLines:\n", index + 1));
            for line in &token.lines {
                list.push_str(&format!("{line}\n"));
            }
        }
        ChatRequest {
            system: TOKENS_SYSTEM.to_string(),
            user: format!("{}Variables:\n{list}\nProvide JSON array now:", hints_block(&text, hints)),
            temperature: target.settings.temperature,
            max_tokens: answer_tokens(TOKEN_ANSWER_TOKENS, TOKEN_ITEM_TOKENS, batch.len()),
        }
    }

    /// Значение или пояснение, хотя бы одно. Значение с `[___]` модель списала из реплики.
    pub fn accept(text: &str, batch: &[TokenItem]) -> Vec<TokenSuggestion> {
        by_number(text, batch)
            .into_iter()
            .filter_map(|(token, item)| {
                let value = field(item.value.as_deref()).filter(|text| !text.contains(TOKEN_GAP));
                let note = field(item.note.as_deref());
                (value.is_some() || note.is_some()).then(|| TokenSuggestion { name: token.name.clone(), value, note })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::prompt::tests::target;

    fn term(id: i64, name: &str, description: Option<&str>) -> TermItem {
        TermItem {
            id: TermId::from_row(id),
            name: name.to_string(),
            category: TermCategory::General,
            description: description.map(str::to_string),
            examples: vec![format!("We met at the {name}.")],
        }
    }

    fn speaker(key: &str, label: &str) -> SpeakerItem {
        SpeakerItem { key: SpeakerKey::new(key).unwrap(), label: label.to_string(), lines: vec!["Hello there.".to_string()] }
    }

    #[test]
    fn terms_prompt_numbers_items_and_gives_glossary_hints() {
        let hints = Hints { terms: vec![("PPF".to_string(), "ППФ".to_string())], ..Hints::default() };
        let request = TermsPrompt::build(&[term(7, "PPF Headquarters", Some("База"))], &hints, &target());
        assert!(request.user.starts_with("Known terms: PPF = ППФ"));
        assert!(request.user.contains(r#"- id: 1, term: "PPF Headquarters", current_category: "general", about: "База", examples: ["We met at the PPF Headquarters."]"#));
    }

    #[test]
    fn terms_answer_is_checked_by_number_markers_and_category() {
        let batch = [term(7, "Silver Key", None), term(8, "[mc]'s Room", Some("Своё")), term(9, "Harbor", None)];
        let answer = r#"Вот ответ:
        [
          {"id": "1", "translated_term": "Серебряный ключ", "category": "Item", "notes": "Ключ от подвала"},
          {"id": 2, "translation": "Комната", "category": "location"},
          {"id": 3, "translation": "Гавань", "category": "place", "note": "Порт"},
          {"id": 3, "translation": "Порт"},
          {"id": 4, "translation": "Лишний"}
        ]"#;
        let accepted = TermsPrompt::accept(answer, &batch);
        assert_eq!(
            accepted,
            [
                TermSuggestion { id: TermId::from_row(7), translation: "Серебряный ключ".to_string(), category: Some(TermCategory::Item) },
                TermSuggestion { id: TermId::from_row(9), translation: "Гавань".to_string(), category: None },
            ],
            "без [mc] перевод не годится, неизвестная категория пропускается, повтор номера и чужой номер отброшены"
        );
        let placed = [TermItem { category: TermCategory::Location, ..term(8, "Room", None) }];
        let accepted = TermsPrompt::accept(r#"[{"id": 1, "translation": "Комната", "category": "item"}]"#, &placed);
        assert_eq!(accepted[0].category, None, "категорию разбора и пользователя перевод не меняет");
    }

    #[test]
    fn a_cut_answer_keeps_its_finished_items_and_the_limit_grows_with_the_batch() {
        let batch = [term(1, "Silver Key", None), term(2, "Harbor", None), term(3, "Old Mill", None)];
        let cut = r#"[{"id": 1, "translation": "Серебряный ключ"}, {"id": 2, "translation": "Гавань", "category": "location"}, {"id": 3, "translat"#;
        let accepted = TermsPrompt::accept(cut, &batch);
        assert_eq!(accepted.iter().map(|found| found.translation.as_str()).collect::<Vec<_>>(), ["Серебряный ключ", "Гавань"]);

        let limit = |count: usize| TermsPrompt::build(&vec![term(1, "Harbor", None); count], &Hints::default(), &target()).max_tokens;
        assert_eq!((limit(1), limit(20), limit(40)), (2048, 2560, 5120));
    }

    #[test]
    fn speaker_answer_keeps_markers_and_reads_gender() {
        let batch = [speaker("label:Waitress", "Waitress"), speaker("label:[mc]'s mom", "[mc]'s mom"), speaker("label:Radio", "Radio")];
        let answer = r#"[{"id": "s1", "translated": "Официантка", "gender": "female"},
                        {"id": 2, "translation": "Мама", "gender": "female"},
                        {"id": 3, "translation": "Радио", "gender": "unknown"}]"#;
        let accepted = SpeakersPrompt::accept(answer, &batch);
        assert_eq!(accepted.len(), 2);
        assert_eq!((accepted[0].translation.as_str(), accepted[0].gender), ("Официантка", Some(Gender::Female)));
        assert_eq!((accepted[1].key.as_str(), accepted[1].gender), ("label:Radio", None));
    }

    #[test]
    fn token_lines_hide_the_token_and_answer_needs_a_value_or_note() {
        let values = HashMap::from([("mom".to_string(), "мама".to_string())]);
        let line = token_line(Some("Celia"), "Hi, [mc!c].\nI'm [mom].", "mc", &values);
        assert_eq!(line, "Celia: Hi, [___]. I'm мама.");

        let batch = [TokenItem { name: "mc".to_string(), lines: vec![line] }, TokenItem { name: "pet".to_string(), lines: Vec::new() }];
        let answer = r#"[{"id": 1, "value": "", "note": "Имя героя"}, {"id": 2, "value": "[___]", "note": ""}]"#;
        assert_eq!(
            TokensPrompt::accept(answer, &batch),
            [TokenSuggestion { name: "mc".to_string(), value: None, note: Some("Имя героя".to_string()) }]
        );
    }

    #[test]
    fn spread_takes_lines_evenly() {
        let lines: Vec<usize> = (0..10).collect();
        assert_eq!(spread(&lines, 4), [0, 2, 5, 7]);
        assert_eq!(spread(&lines[..2], 4), [0, 1]);
        assert!(spread::<usize>(&[], 4).is_empty());
    }
}
