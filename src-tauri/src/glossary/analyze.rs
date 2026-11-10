//! Разбор сцены моделью перед её описанием: новые персонажи и термины по частям сцены. Находки сводятся
//! по `NameKey`, известное глоссарию, корзине терминов и подписям говорящих отсеивается, остальное сразу
//! пишется в глоссарий. Описывает новые записи потом общий путь описаний.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::repo::{CharacterRepo, LineSampleRepo, TermRepo};
use super::{Character, GlossaryData, NameKey, Term, TermCategory};
use crate::common::Gender;
use crate::llm::{ChatRequest, LlmTarget};
use crate::storage::DbError;
use crate::translation::answer::json_array;
use crate::translation::text::mentions;

const ANSWER_TOKENS: u32 = 1024;
/// Предел строки известных имён: в неё идут только упомянутые в части.
const MAX_KNOWN_CHARS: usize = 2000;

const SYSTEM: &str = "You read a part of a visual novel script and list the proper names and game-specific terms \
that a translator into Russian must keep consistent across the game. \
The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
List characters (people and creatures with a name, including those who are only mentioned) and terms: \
places, organizations, items, titles and ranks, events and other named concepts of this game. \
Skip everything under \"Known names and terms\", ordinary words, common nouns, interjections, \
and anything inside [square brackets] or {curly brackets}. \
Answer with a JSON array, or [] if there is nothing new. Each object has the fields: \
\"kind\": one of \"character\", \"location\", \"organization\", \"title\", \"item\", \"event\", \"general\"; \
\"name\": the exact spelling from the text; \
\"gender\": \"male\", \"female\" or \"unknown\", only for a character. \
Respond with the JSON array only.";

/// Что нашла модель: персонаж или термин своей категории.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoundKind {
    Character,
    Term(TermCategory),
}

impl FoundKind {
    fn parse(text: Option<&str>) -> Self {
        let text = text.unwrap_or_default().trim().to_lowercase();
        match text.as_str() {
            "character" | "person" | "name" | "people" => Self::Character,
            other => Self::Term(TermCategory::parse(other).unwrap_or(TermCategory::General)),
        }
    }
}

/// Находка одной части или сведённая по всем частям.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Написание из текста; у сведённой первое встреченное.
    pub name: String,
    /// У сведённой вид, который модель называла чаще.
    pub kind: FoundKind,
    pub gender: Option<Gender>,
}

/// Запись ответа. Поля под разными именами: модели пишут их по привычке.
#[derive(Debug, Default, Deserialize)]
struct AnswerItem {
    #[serde(alias = "type", alias = "category")]
    kind: Option<String>,
    #[serde(alias = "term", alias = "text")]
    name: Option<String>,
    gender: Option<String>,
}

/// Запрос разбора одной части.
pub struct AnalysisPrompt;

impl AnalysisPrompt {
    /// `known`: имена и названия глоссария. В запрос идут только упомянутые в части.
    pub fn build(part: &str, known: &[String], target: &LlmTarget) -> ChatRequest {
        let lower = part.to_lowercase();
        let mut size = 0;
        let mut seen = HashSet::new();
        let mentioned: Vec<&str> = known
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty() && mentions(&lower, &name.to_lowercase()))
            .filter(|name| seen.insert(name.to_lowercase()))
            .take_while(|name| {
                size += name.chars().count() + 2;
                size <= MAX_KNOWN_CHARS
            })
            .collect();
        let known = if mentioned.is_empty() { String::new() } else { format!("Known names and terms: {}\n\n", mentioned.join(", ")) };
        ChatRequest {
            system: SYSTEM.to_string(),
            user: format!("{known}Script:\n\n{part}\nNew names and terms (JSON array):"),
            temperature: target.settings.temperature,
            max_tokens: ANSWER_TOKENS,
        }
    }

    /// Находки, чьё имя есть в части целым словом без учёта регистра: выдуманное модель не добавит.
    /// Имя с токеном или тегом и повтор в ответе отбрасываются.
    pub fn accept(text: &str, part: &str) -> Vec<Finding> {
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json_array(text)) else { return Vec::new() };
        let part = part.to_lowercase();
        let mut seen = HashSet::new();
        values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<AnswerItem>(value).ok())
            .filter_map(|item| {
                let name = item.name.as_deref()?.trim().trim_matches(|c: char| "\"'«»“”".contains(c)).trim().to_string();
                if name.is_empty() || name.contains(['[', ']', '{', '}', '\n']) || !mentions(&part, &name.to_lowercase()) {
                    return None;
                }
                seen.insert(NameKey::new(&name)?).then_some(())?;
                let kind = FoundKind::parse(item.kind.as_deref());
                let gender = match kind {
                    FoundKind::Character => gender(item.gender.as_deref()),
                    FoundKind::Term(_) => None,
                };
                Some(Finding { name, kind, gender })
            })
            .collect()
    }
}

fn gender(text: Option<&str>) -> Option<Gender> {
    match text?.trim().to_lowercase().as_str() {
        "male" | "m" => Some(Gender::Male),
        "female" | "f" => Some(Gender::Female),
        _ => None,
    }
}

/// Сводит находки частей по `NameKey`: вид по большинству, при равенстве первый; род первый известный.
/// Порядок первого появления.
pub fn merge(found: Vec<Finding>) -> Vec<Finding> {
    let mut order: Vec<NameKey> = Vec::new();
    let mut entries: HashMap<NameKey, (Finding, Vec<(FoundKind, usize)>)> = HashMap::new();
    for finding in found {
        let Some(key) = NameKey::new(&finding.name) else { continue };
        let (merged, kinds) = entries.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            (Finding { name: finding.name.clone(), kind: finding.kind, gender: None }, Vec::new())
        });
        match kinds.iter_mut().find(|(kind, _)| *kind == finding.kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((finding.kind, 1)),
        }
        if merged.gender.is_none() {
            merged.gender = finding.gender;
        }
    }
    order
        .into_iter()
        .filter_map(|key| {
            let (mut merged, kinds) = entries.remove(&key)?;
            let mut best = kinds[0];
            for kind in &kinds[1..] {
                if kind.1 > best.1 {
                    best = *kind;
                }
            }
            merged.kind = best.0;
            if best.0 != FoundKind::Character {
                merged.gender = None;
            }
            Some(merged)
        })
        .collect()
}

/// Новые записи разбора в глоссарий: персонаж с родом, термин с категорией. Известное глоссарию, корзине
/// терминов и подписям говорящих пропускается. Возвращает написания добавленных.
pub fn save_found(tx: &rusqlite::Connection, found: &[Finding]) -> Result<Vec<String>, DbError> {
    let mut known: HashSet<NameKey> = known_for_prompt(tx)?.iter().filter_map(|name| NameKey::new(name)).collect();
    let terms = TermRepo { conn: tx };
    let characters = CharacterRepo { conn: tx };
    let mut added = Vec::new();
    for finding in found {
        let Some(key) = NameKey::new(&finding.name) else { continue };
        if known.contains(&key) || terms.is_known(&key)? {
            continue;
        }
        match finding.kind {
            FoundKind::Character => {
                let mut character = Character::new(&finding.name);
                character.set_gender(finding.gender);
                characters.save(&character)?;
            }
            FoundKind::Term(category) => {
                let Ok(term) = Term::new(&finding.name, category) else { continue };
                terms.save(&term)?;
            }
        }
        known.insert(key);
        added.push(finding.name.clone());
    }
    Ok(added)
}

/// Имена, фамилии, полные имена и формы персонажей, термины и их формы и подписи говорящих текущего снимка.
pub fn known_for_prompt(tx: &rusqlite::Connection) -> Result<Vec<String>, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let labels = LineSampleRepo { conn: tx }.speaker_labels()?;
    let characters = glossary.characters.iter().flat_map(|character| {
        character
            .full_name()
            .into_iter()
            .chain(character.first_name().map(str::to_string))
            .chain(character.last_name().map(str::to_string))
            .chain(character.forms().iter().map(|form| form.text().to_string()))
    });
    let terms = glossary.terms.iter().flat_map(|term| std::iter::once(term.name().to_string()).chain(term.forms().iter().map(|form| form.text().to_string())));
    Ok(characters.chain(terms).chain(labels).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(name: &str, kind: FoundKind) -> Finding {
        Finding { name: name.to_string(), kind, gender: None }
    }

    #[test]
    fn answer_keeps_only_names_from_the_part() {
        let part = "Eileen: Meet me at the Rusty Anchor.\nLis: Ask Captain Morgan about the Order.\nNarration.\n";
        let answer = r#"Вот:
```json
[
  {"kind": "location", "name": "Rusty Anchor"},
  {"kind": "character", "name": "Captain Morgan", "gender": "male"},
  {"type": "organization", "term": "the Order"},
  {"kind": "character", "name": "Ghost"},
  {"kind": "item", "name": "[mc]"},
  {"kind": "location", "name": "rusty anchor"}
]
```"#;
        assert_eq!(
            AnalysisPrompt::accept(answer, part),
            [
                finding("Rusty Anchor", FoundKind::Term(TermCategory::Location)),
                Finding { gender: Some(Gender::Male), ..finding("Captain Morgan", FoundKind::Character) },
                finding("the Order", FoundKind::Term(TermCategory::Organization)),
            ],
            "выдуманное имя, токен и повтор отброшены"
        );
        assert!(AnalysisPrompt::accept("Новых имён нет.", part).is_empty());
    }

    #[test]
    fn prompt_lists_only_known_names_from_the_part() {
        let target = crate::translation::prompt::tests::target();
        let known = vec!["Eileen".to_string(), "Lis".to_string(), "Nowhere".to_string(), "eileen".to_string()];
        let request = AnalysisPrompt::build("Eileen: Hi, Lis.\n", &known, &target);
        assert!(request.user.starts_with("Known names and terms: Eileen, Lis\n\nScript:\n\nEileen: Hi, Lis.\n"), "{}", request.user);
        assert!(AnalysisPrompt::build("Narration.\n", &known, &target).user.starts_with("Script:"));
    }

    #[test]
    fn findings_merge_by_key_with_the_most_named_kind() {
        let location = FoundKind::Term(TermCategory::Location);
        let lis = Finding { gender: Some(Gender::Female), ..finding("Lis", FoundKind::Character) };
        let merged = merge(vec![finding("Rusty Anchor", location), lis.clone(), finding("rusty anchor", FoundKind::Character), finding("RUSTY ANCHOR", location)]);
        assert_eq!(merged, [finding("Rusty Anchor", location), lis]);
    }
}
