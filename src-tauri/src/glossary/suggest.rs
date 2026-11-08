//! Предложения модели по записям глоссария: перевод имён и терминов, перевод подписей говорящих, значения токенов.
//! Промпты, разбор ответа и проверка. Ответ модели в базу сам не пишется: отмеченное в отборе записывает окно.

use std::collections::HashMap;

use serde::Deserialize;

use super::{Character, CharacterId, NameField, NameKey, TermCategory, TermId};
use crate::common::Gender;
use crate::engines::SpeakerKey;
use crate::llm::{ChatRequest, LlmTarget};
use crate::translation::Hints;
use crate::translation::answer::json_array;
use crate::translation::prompt::glossary_hints;
use crate::translation::text::{fill_tokens, markers, outside_markers};

/// Сколько реплик говорящего видит модель.
pub const SPEAKER_SAMPLES: usize = 4;
/// Сколько реплик с токеном видит модель.
pub const TOKEN_SAMPLES: usize = 8;
/// Чем заменён разбираемый токен: имени токена модель верит больше, чем репликам.
pub const TOKEN_GAP: &str = "[___]";

const TERM_ANSWER_TOKENS: u32 = 2048;
const SPEAKER_ANSWER_TOKENS: u32 = 1536;
const TOKEN_ANSWER_TOKENS: u32 = 1536;
/// Лимит ответа на первое имя пакета и на каждое следующее.
const NAME_ANSWER_TOKENS: u32 = 2048;
const NEXT_NAME_TOKENS: u32 = 512;
/// Сколько написаний имени показывает отбор.
pub const NAME_VARIANTS: usize = 3;
/// Длина пояснения к написанию: модели пишут целые отчёты.
const VARIANT_NOTE_CHARS: usize = 180;

const NAMES_SYSTEM: &str = r#"You are an expert in personal names and their established Russian spellings, preparing a translation glossary for a visual novel or RPG.
You receive every character name of one game and the names to spell in Russian.

Work in this order:
1. Read the whole list and work out which languages and countries the names come from. Names of one game usually share a setting, so use the others to settle a name that could belong to several languages.
2. Decide the language of each name on its own: a game can mix several. A name may lack diacritics: "Dzenana" is "Dženana".
3. Spell the name the way speakers of that language say it, using the established Russian form if there is one. Never read a name by English rules unless it is English. Prefer established Russian spellings over a mechanical letter substitution: "Liam" -> "Лиам", "Leon" -> "Леон", "Jelena" -> "Елена". Do not invent missing accents, letters or a different name. Answer in Russian in the nominative case, without alternative spellings or commentary.

Spelling by language:
- Serbian, Croatian, Bosnian: "lj" and "nj" are one soft sound, "j" is "й", "dž" and "đ" are "дж", "c" is "ц": "Anja" -> "Аня", "Dunja" -> "Дуня", "Ljubica" -> "Любица", "Ljiljana" -> "Лиляна", "Mirjana" -> "Мирьяна", "Jovan" -> "Йован", "Dženana" -> "Дженана".
- German: "ei" is "ай", "eu" is "ой", "j" is "й": "Heike" -> "Хайке", "Jürgen" -> "Юрген", "Hilde" -> "Хильде", "Marlene" -> "Марлен".
- Italian: "Giovanni" -> "Джованни", "Chiara" -> "Кьяра".
- French: use established Russian spellings, not a rule that all final consonants are silent: "Claire" -> "Клер", "Jacques" -> "Жак".
- Spanish: "j" is "х": "Jorge" -> "Хорхе", "Juana" -> "Хуана".
- Polish, Czech, Hungarian: "Wojciech" -> "Войцех", "Jiří" -> "Иржи", "Erzsébet" -> "Эржебет".
- Japanese: "Yuki" -> "Юки", "Shoji" -> "Сёдзи".
- English: "Chloe" -> "Хлоя", "Jack" -> "Джек", "Katy" -> "Кэти".

Roles, descriptions and abbreviations are translated by meaning: "Old Bartender" -> "Старый бармен", "Agent Lisa" -> "Агент Лиза", "MC" -> "Главный герой".
Text in [square brackets] is an in-game variable: return it unchanged.

A line may say more about the name:
- "surname of a woman" or "surname of a man": make the Russian surname agree with the gender if its language changes it: "Kowalski" of a woman -> "Ковальская", "Ivanov" of a woman -> "Иванова". Other surnames stay the same: "Karanovic" -> "Каранович".
- "form of Анна Каранович": another way to call that character. Keep that spelling for the parts they share and translate titles by meaning: "Mrs. Karanovic" -> "Миссис Каранович", "Annie" -> "Энни".

Translate exactly the requested "name", never replace it with the owner's full name.
- A different surname or alias stays different: spell it itself, not the full name of its owner.
- Preserve the exact form: "Hilda" does not become "Hilde"; "Stu" stays a short form, not "Stuart".
- Titles must keep their meaning: "Miss" -> "Мисс", "Mrs." -> "Миссис"; "Colonel" -> "полковник", "Lieutenant Colonel" -> "подполковник". Do not change rank or marital status.
- Keep shared surname spellings consistent with the supplied Russian base and the "Russian names" line, including the gender ending. Only add a female ending where the name's own language requires it; do not add one to every woman's foreign surname.
- Square-bracket variables must stay byte-for-byte unchanged. Outside variables use Cyrillic, spaces and name punctuation, not Latin letters, explanations or Markdown.

Respond ONLY with a valid JSON array, one object per name to spell, without markdown fences or explanations:
[
  {"id": 1, "origin": "сербский", "translated_name": "Аня", "variants": [{"source": "Anja", "translated": "Аня", "description": "Передача имени по сербскому произношению."}]}
]
"id" is the EXACT id as given. "origin" is the language of the name as one Russian word, or "описание" for roles and descriptions.
Provide 1 to 3 plausible spellings in "variants", each with "source" identifying the original name actually translated and a short Russian explanation (one sentence, at most 20 words). Translate only the requested name: given-name requests must never return surname variants. Do not invent alternatives merely for variety. No warnings, disclaimers, Markdown or essays in explanations. Gender alone is not a reason to invent a surname ending; require a rule of the language of use."#;

const TERMS_SYSTEM: &str = r#"You are an expert game localization specialist translating English terms of a visual novel into Russian.
Translate in-game terms, locations, factions, artifacts, ranks, items and events into authentic Russian.

For each term output:
1. "id": EXACT id as given.
2. "translation": Accurate, authentic Russian translation in the nominative case.
   Examples: "Demon Hunter" -> "Охотник на демонов", "PPF Headquarters" -> "Штаб ППФ", "Ancient Rune" -> "Древняя руна", "Silver Key" -> "Серебряный ключ".
3. "category": One of "general", "location", "organization", "title", "item", "event". Use "organization" for companies, factions, agencies, gangs, schools as institutions and other groups of people. Use "event" for wars, battles, disasters, holidays, tournaments and other things that happen at a particular time.
4. "note": What the term is, in Russian, one short sentence, or an empty string.

"example" is a line of the game where the term appears: use it to understand what the term means. Text in [square brackets] is an in-game variable: keep it unchanged.
A line starting with "Russian names:" gives the exact spelling of names. A line starting with "Known terms:" gives established translations: keep translations consistent with them.

Respond ONLY with a valid JSON array, without markdown fences or explanations:
[
  {"id": 1, "translation": "Штаб ППФ", "category": "location", "note": "Главная база ППФ"}
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

/// Имя в запросе. Одно написание одного вида и рода спрашивается один раз: ответ получают все владельцы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameItem {
    pub name: String,
    pub field: NameField,
    pub gender: Option<Gender>,
    /// Полное имя владельца по-русски: от него строится форма.
    pub base: Option<String>,
    pub owners: Vec<CharacterId>,
}

/// Написание имени с пояснением модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameVariant {
    pub translation: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSuggestion {
    pub character: CharacterId,
    pub field: NameField,
    /// Написание в игре: у формы по нему находится сама форма.
    pub source: String,
    /// Язык имени одним русским словом, как его назвала модель.
    pub origin: Option<String>,
    /// Не пусто. Первым идёт написание, которое модель выбрала сама.
    pub variants: Vec<NameVariant>,
}

/// Термин в запросе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermItem {
    pub id: TermId,
    pub name: String,
    pub category: TermCategory,
    /// Своё описание: модель его не заменяет, а читает.
    pub description: Option<String>,
    /// Реплика с термином, первая по сюжету.
    pub example: Option<String>,
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
    /// `None`: модель не назвала категорию из списка, остаётся прежняя.
    pub category: Option<TermCategory>,
    /// Только у термина без своего описания.
    pub description: Option<String>,
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
struct AnswerItem {
    id: Option<serde_json::Value>,
    #[serde(alias = "translated_term", alias = "translated", alias = "translated_name")]
    translation: Option<String>,
    category: Option<String>,
    #[serde(alias = "notes", alias = "description")]
    note: Option<String>,
    gender: Option<String>,
    value: Option<String>,
    origin: Option<String>,
    /// Написания имени значением, а не полем записи: сломанный список не теряет всей записи.
    variants: Option<serde_json::Value>,
}

/// Написание имени в ответе.
#[derive(Debug, Default, Deserialize)]
struct VariantItem {
    source: Option<String>,
    #[serde(alias = "translated", alias = "translated_name")]
    translation: Option<String>,
    #[serde(alias = "note", alias = "explanation")]
    description: Option<String>,
}

/// Записи ответа с номером пакета. Номер принимается числом и строкой, в том числе с буквой вроде `t3`.
/// Одну запись локальная модель иногда возвращает объектом вместо массива. Тогда первым массивом в тексте
/// оказывается вложенный, у его записей нет `id`.
fn answer_items(text: &str) -> Vec<(usize, AnswerItem)> {
    let values = serde_json::from_str::<Vec<serde_json::Value>>(json_array(text))
        .ok()
        .filter(|values| values.is_empty() || values.iter().any(|value| value.get("id").is_some()))
        .unwrap_or_else(|| single_object(text).into_iter().collect());
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

/// Первый объект с полем `id` в тексте.
fn single_object(text: &str) -> Option<serde_json::Value> {
    text.match_indices('{').find_map(|(start, _)| {
        let value = serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>().next()?.ok()?;
        value.get("id").is_some().then_some(value)
    })
}

/// Записи ответа по записям пакета: номер с 1, первая запись на номер побеждает.
fn by_number<'a, T>(text: &str, batch: &'a [T]) -> Vec<(&'a T, AnswerItem)> {
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
fn field(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty() && !text.contains('\n')).then(|| text.to_string())
}

/// Перевод названия годится, если он одной строкой и сохранил токены и теги оригинала.
fn usable_label(original: &str, translated: &str) -> bool {
    markers(original) == markers(translated)
}

/// Написание имени: те же токены, вне токенов и тегов только кириллица.
fn usable_name(original: &str, translated: &str) -> bool {
    usable_label(original, translated)
        && outside_markers(translated).chars().filter(|c| c.is_alphabetic()).all(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

/// Пояснение к написанию одной строкой, без разметки и не длиннее `VARIANT_NOTE_CHARS` знаков.
fn short_note(text: Option<&str>) -> Option<String> {
    let text: String = text?.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '#')).collect();
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(VARIANT_NOTE_CHARS).collect();
    (!text.is_empty()).then_some(text)
}

fn gender(text: Option<&str>) -> Option<Gender> {
    match text?.trim().to_lowercase().as_str() {
        "male" | "m" | "мужской" => Some(Gender::Male),
        "female" | "f" | "женский" => Some(Gender::Female),
        _ => None,
    }
}

/// Строка значения в запрос. Сериализация строки не падает.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// Подсказки глоссария по тексту пакета, абзацами перед списком.
fn hints_block(text: &str, hints: &Hints) -> String {
    glossary_hints(text, hints).iter().map(|hint| format!("{hint}\n\n")).collect()
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

/// Имена и фамилии без перевода. Одинаковое написание одного вида и рода становится одной записью.
pub fn name_items(characters: &[&Character]) -> Vec<NameItem> {
    let mut items: Vec<NameItem> = Vec::new();
    for character in characters {
        let Some(id) = character.id() else { continue };
        let gender = character.gender();
        let parts = [
            (NameField::First, character.first_name(), character.translated_first_name()),
            (NameField::Last, character.last_name(), character.translated_last_name()),
        ];
        for (field, name, translation) in parts {
            let Some(name) = name.filter(|_| translation.is_none()) else { continue };
            let key = NameKey::new(name);
            match items.iter_mut().find(|item| item.field == field && item.gender == gender && NameKey::new(&item.name) == key) {
                Some(item) => item.owners.push(id),
                None => items.push(NameItem { name: name.to_string(), field, gender, base: None, owners: vec![id] }),
            }
        }
    }
    items
}

/// Формы без перевода, каждая своей записью: основа у каждого владельца своя. Основа это полное имя по-русски
/// из глоссария, а поле без перевода берёт первое написание модели из `spelled`.
pub fn form_items(characters: &[&Character], spelled: &[NameSuggestion]) -> Vec<NameItem> {
    let first_spelling = |id: CharacterId, field: NameField| {
        spelled
            .iter()
            .find(|item| item.character == id && item.field == field)
            .and_then(|item| item.variants.first())
            .map(|variant| variant.translation.as_str())
    };
    let mut items = Vec::new();
    for character in characters {
        let Some(id) = character.id() else { continue };
        let first = character.translated_first_name().or_else(|| first_spelling(id, NameField::First));
        let last = character.translated_last_name().or_else(|| first_spelling(id, NameField::Last));
        let base = [first, last].into_iter().flatten().collect::<Vec<_>>().join(" ");
        for form in character.forms().iter().filter(|form| form.translation().is_none()) {
            items.push(NameItem {
                name: form.text().to_string(),
                field: NameField::Form,
                gender: character.gender(),
                base: (!base.is_empty()).then(|| base.clone()),
                owners: vec![id],
            });
        }
    }
    items
}

/// Полные имена всех персонажей без повторов: по соседним именам модель понимает происхождение имени.
pub fn cast(characters: &[Character]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    characters.iter().filter_map(Character::full_name).filter(|name| NameKey::new(name).is_some_and(|key| seen.insert(key))).collect()
}

/// Строка имени в запросе: что это за имя и от какого написания строится форма.
fn name_line(number: usize, item: &NameItem) -> String {
    let person = match item.gender {
        Some(Gender::Female) => "a woman",
        Some(Gender::Male) => "a man",
        None => "a character",
    };
    let detail = match (item.field, &item.base) {
        (NameField::First, _) if item.gender.is_none() => None,
        (NameField::First, _) => Some(format!("given name of {person}")),
        (NameField::Last, _) => Some(format!("surname of {person}")),
        (NameField::Form, Some(base)) => Some(format!("name of {person}, form of {base}")),
        (NameField::Form, None) => Some(format!("name of {person}")),
    };
    let detail = detail.map(|detail| format!(", {detail}")).unwrap_or_default();
    format!("- id: {number}, name: {}{detail}\n", quoted(&item.name))
}

/// Написания из ответа: выбор модели, потом её варианты. Вариант другого имени (`source` не тот) отбрасывается.
fn name_variants(name: &str, answer: &AnswerItem) -> Vec<NameVariant> {
    let key = NameKey::new(name);
    let listed: Vec<VariantItem> = answer
        .variants
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<VariantItem>(value.clone()).ok())
        .filter(|variant| variant.source.as_deref().is_none_or(|source| NameKey::new(source) == key))
        .collect();
    let chosen = field(answer.translation.as_deref());
    let chosen_note = listed.iter().find(|variant| field(variant.translation.as_deref()) == chosen).and_then(|variant| variant.description.as_deref());
    let candidates = std::iter::once((chosen.clone(), chosen_note))
        .chain(listed.iter().map(|variant| (field(variant.translation.as_deref()), variant.description.as_deref())));
    let mut variants: Vec<NameVariant> = Vec::new();
    for (translation, note) in candidates {
        let Some(translation) = translation.filter(|text| usable_name(name, text)) else { continue };
        if variants.len() < NAME_VARIANTS && !variants.iter().any(|known| known.translation == translation) {
            variants.push(NameVariant { translation, description: short_note(note) });
        }
    }
    variants
}

pub struct NamesPrompt;

impl NamesPrompt {
    pub fn build(batch: &[NameItem], cast: &[String], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let text: String =
            batch.iter().flat_map(|item| [Some(item.name.as_str()), item.base.as_deref()]).flatten().map(|line| format!("{line}\n")).collect();
        let cast = if cast.is_empty() { String::new() } else { format!("All names in this game: {}\n\n", cast.join(", ")) };
        let list: String = batch.iter().enumerate().map(|(index, item)| name_line(index + 1, item)).collect();
        let extra = u32::try_from(batch.len().saturating_sub(1)).unwrap_or(u32::MAX);
        ChatRequest {
            system: NAMES_SYSTEM.to_string(),
            user: format!("{}{cast}Names to spell:\n{list}\nProvide JSON array now:", hints_block(&text, hints)),
            temperature: target.settings.temperature,
            max_tokens: NAME_ANSWER_TOKENS.saturating_add(NEXT_NAME_TOKENS.saturating_mul(extra)),
        }
    }

    /// Хотя бы одно годное написание. Ответ на запись получают все её владельцы.
    pub fn accept(text: &str, batch: &[NameItem]) -> Vec<NameSuggestion> {
        by_number(text, batch)
            .into_iter()
            .flat_map(|(item, answer)| {
                let variants = name_variants(&item.name, &answer);
                let origin = field(answer.origin.as_deref()).map(|text| text.to_lowercase());
                let owners = if variants.is_empty() { &[][..] } else { &item.owners[..] };
                owners
                    .iter()
                    .map(|&character| NameSuggestion {
                        character,
                        field: item.field,
                        source: item.name.clone(),
                        origin: origin.clone(),
                        variants: variants.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

pub struct TermsPrompt;

impl TermsPrompt {
    pub fn build(batch: &[TermItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let text: String =
            batch.iter().flat_map(|term| [Some(term.name.as_str()), term.example.as_deref()]).flatten().map(|line| format!("{line}\n")).collect();
        let mut list = String::new();
        for (index, term) in batch.iter().enumerate() {
            list.push_str(&format!("- id: {}, term: {}, current_category: {}", index + 1, quoted(&term.name), quoted(term.category.as_str())));
            if let Some(description) = &term.description {
                list.push_str(&format!(", about: {}", quoted(description)));
            }
            if let Some(example) = &term.example {
                list.push_str(&format!(", example: {}", quoted(example)));
            }
            list.push('\n');
        }
        ChatRequest {
            system: TERMS_SYSTEM.to_string(),
            user: format!("{}Terms to translate into Russian:\n{list}\nProvide JSON array now:", hints_block(&text, hints)),
            temperature: target.settings.temperature,
            max_tokens: TERM_ANSWER_TOKENS,
        }
    }

    /// Перевод одной строкой с токенами оригинала. Категория только из списка, описание только термину без своего.
    pub fn accept(text: &str, batch: &[TermItem]) -> Vec<TermSuggestion> {
        by_number(text, batch)
            .into_iter()
            .filter_map(|(term, item)| {
                let translation = field(item.translation.as_deref()).filter(|text| usable_label(&term.name, text))?;
                let category = item.category.as_deref().and_then(|text| TermCategory::parse(&text.trim().to_lowercase()).ok());
                let description = field(item.note.as_deref()).filter(|_| term.description.is_none());
                Some(TermSuggestion { id: term.id, translation, category, description })
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
            max_tokens: SPEAKER_ANSWER_TOKENS,
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
            max_tokens: TOKEN_ANSWER_TOKENS,
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
            example: Some(format!("We met at the {name}.")),
        }
    }

    fn speaker(key: &str, label: &str) -> SpeakerItem {
        SpeakerItem { key: SpeakerKey::new(key).unwrap(), label: label.to_string(), lines: vec!["Hello there.".to_string()] }
    }

    #[test]
    fn terms_prompt_numbers_items_and_gives_glossary_hints() {
        let hints = Hints { terms: vec![("PPF".to_string(), "ППФ".to_string(), false)], ..Hints::default() };
        let request = TermsPrompt::build(&[term(7, "PPF Headquarters", Some("База"))], &hints, &target());
        assert!(request.user.starts_with("Known terms: PPF = ППФ"));
        assert!(request.user.contains(r#"- id: 1, term: "PPF Headquarters", current_category: "general", about: "База", example: "We met at the PPF Headquarters.""#));
    }

    #[test]
    fn terms_answer_is_checked_by_number_markers_and_own_description() {
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
                TermSuggestion {
                    id: TermId::from_row(7),
                    translation: "Серебряный ключ".to_string(),
                    category: Some(TermCategory::Item),
                    description: Some("Ключ от подвала".to_string()),
                },
                TermSuggestion { id: TermId::from_row(9), translation: "Гавань".to_string(), category: None, description: Some("Порт".to_string()) },
            ],
            "без [mc] перевод не годится, неизвестная категория пропускается, повтор номера и чужой номер отброшены"
        );
        let own = [term(8, "Room", Some("Своё"))];
        let accepted = TermsPrompt::accept(r#"[{"id": 1, "translation": "Комната", "note": "Чужое"}]"#, &own);
        assert_eq!(accepted[0].description, None, "своё описание модель не заменяет");
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

    fn character(id: i64, first: &str, last: Option<&str>, gender: Option<Gender>, forms: &[&str]) -> Character {
        let forms = forms.iter().map(|text| super::super::Form::new(text, None).unwrap()).collect();
        Character::restore(
            CharacterId::from_row(id),
            Some(first.to_string()),
            last.map(str::to_string),
            None,
            None,
            gender,
            None,
            None,
            forms,
        )
    }

    #[test]
    fn names_are_asked_once_per_spelling_and_forms_build_on_the_spelled_name() {
        let anna = character(1, "Anna", Some("Karanovic"), Some(Gender::Female), &["Mrs. Karanovic"]);
        let mut marko = character(2, "Marko", Some("Karanovic"), Some(Gender::Male), &[]);
        marko.set_translated_names("Марко", "");
        let sister = character(3, "Mila", Some("Karanovic"), Some(Gender::Female), &[]);
        let all = [anna, marko, sister];
        let chosen: Vec<&Character> = all.iter().collect();

        let names = name_items(&chosen);
        let shown: Vec<(&str, NameField, usize)> = names.iter().map(|item| (item.name.as_str(), item.field, item.owners.len())).collect();
        assert_eq!(
            shown,
            [("Anna", NameField::First, 1), ("Karanovic", NameField::Last, 2), ("Karanovic", NameField::Last, 1), ("Mila", NameField::First, 1)],
            "женская фамилия одна на двоих, мужская отдельно, переведённое имя не спрашивается"
        );

        let spelled = NamesPrompt::accept(r#"[{"id": 1, "translated_name": "Анна"}, {"id": 2, "translated_name": "Каранович"}]"#, &names[..2]);
        assert_eq!(spelled.len(), 3, "фамилию получили обе владелицы");
        let forms = form_items(&chosen, &spelled);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].base.as_deref(), Some("Анна Каранович"));

        let request = NamesPrompt::build(&[names[1].clone(), forms[0].clone()], &cast(&all), &Hints::default(), &target());
        assert!(request.user.starts_with("All names in this game: Anna Karanovic, Marko Karanovic, Mila Karanovic\n\nNames to spell:\n"), "{}", request.user);
        assert!(request.user.contains("- id: 1, name: \"Karanovic\", surname of a woman\n- id: 2, name: \"Mrs. Karanovic\", name of a woman, form of Анна Каранович\n"));
    }

    #[test]
    fn name_answer_keeps_russian_spellings_of_the_requested_name() {
        let item = NameItem {
            name: "Anja".to_string(),
            field: NameField::First,
            gender: Some(Gender::Female),
            base: None,
            owners: vec![CharacterId::from_row(4)],
        };
        let answer = r#"Ответ: {"id": "n1", "origin": "Сербский", "translated_name": "Аня", "variants": [
            {"source": "Anja", "translated": "Аня", "description": "**По-сербски** j это й.\n"},
            {"source": "Anja Petrovic", "translated": "Аня Петрович", "description": "Чужое"},
            {"translated": "Anya", "description": "Латиница"},
            {"source": "anja", "translated": "Анья", "description": "По-английски"},
            {"translated": "Ания"},
            {"translated": "Аниа"}
        ]}"#;
        let accepted = NamesPrompt::accept(answer, std::slice::from_ref(&item));
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].origin.as_deref(), Some("сербский"));
        assert_eq!(
            accepted[0].variants,
            [
                NameVariant { translation: "Аня".to_string(), description: Some("По-сербски j это й.".to_string()) },
                NameVariant { translation: "Анья".to_string(), description: Some("По-английски".to_string()) },
                NameVariant { translation: "Ания".to_string(), description: None },
            ],
            "объект вместо массива, чужое имя и латиница отброшены, не больше трёх"
        );
        let token = NameItem { name: "[mc]'s mom".to_string(), ..item };
        assert!(NamesPrompt::accept(r#"[{"id": 1, "translated_name": "Мама"}]"#, &[token]).is_empty(), "токен потерян");
    }

    #[test]
    fn spread_takes_lines_evenly() {
        let lines: Vec<usize> = (0..10).collect();
        assert_eq!(spread(&lines, 4), [0, 2, 5, 7]);
        assert_eq!(spread(&lines[..2], 4), [0, 1]);
        assert!(spread::<usize>(&[], 4).is_empty());
    }
}
