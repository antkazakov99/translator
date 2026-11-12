//! Разбор сцены моделью перед её описанием: запрос по части сцены и чтение ответа. Модель называет новых
//! персонажей и термины, другое имя известной записи (`same_as`), известное имя у другой сущности
//! (`other_than`), двух персонажей сцены как одного человека (`same_person`) и отвечает на вопросы о спорных
//! упоминаниях (`check`). Что из находки получится в глоссарии, решает `resolve.rs`.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::entries::{character_names, term_names};
use super::mentions::{Casings, Checks, doubt, is_sure, mentioned, occurrences};
use super::service::ExcludeOwner;
use super::{CharacterId, GlossaryData, NameKey, TermCategory};
use crate::common::Gender;
use crate::llm::{ChatRequest, LlmTarget};
use crate::translation::answer::json_array;
use crate::translation::text::mentions;

const ANSWER_TOKENS: u32 = 1024;
/// Предел строки известных имён: в неё идут только упомянутые в части.
const MAX_KNOWN_CHARS: usize = 2000;

const SYSTEM: &str = "You read a part of a visual novel script and list the proper names and game-specific terms \
that a translator into Russian must keep consistent across the game. \
The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
List characters (people and creatures with a name, including those who are only mentioned). \
A character is someone who acts, speaks or is spoken about. A title used as the name of one person \
(\"The Fairy Queen\") is a character: list it without the article. \
A word for an act, an object or a menu choice is never a character, even when it stands alone on a line. \
Also list terms: \
places, organizations, items, titles and ranks specific to this game, events and other named concepts of this game. \
An ordinary title or rank on its own (\"King\", \"Princess\", \"Sister\", \"Captain\") is a common noun, not a term. \
Skip everything under \"Known names and terms\", ordinary words, common nouns, interjections, \
and anything inside [square brackets] or {curly brackets}. \
A new spelling of a known or listed name is not skipped: an abbreviation, a short or full form, a nickname, \
a synonym (\"PPF\" for \"People's Police Force\", \"Stuart\" for \"Stu\", \"Fey Court\" for \"Fairy Court\"). \
List it with \"same_as\". \
\"Characters of this scene\" lists known characters with ids like C3. When a name you list is another name \
of one of them (a surname, a first name, a nickname, an alias or a false name they go by), \
set \"same_as\" to that id (\"C3\"). \
When the text shows that two listed characters are one person, add {\"kind\": \"same_person\", \"ids\": [\"C3\", \"C7\"]}. \
Family members often share a surname: do not join them. \
A known name that this text uses for someone or something else (another person with the same name or surname, \
a place named like a person) is not skipped either: list it with \"other_than\" set to the id or the known name \
it must not be confused with. \
A rank, title or form of address before a name is not part of the name: for \"Detective-Colonel Karanovic\" \
list the title \"Detective-Colonel\" and the name \"Karanovic\" separately, and skip \"Mr.\", \"Mrs.\" and similar. \
When the request has \"Known names to check\", each question Q1, Q2... quotes a line with a known name. \
Answer it in one more object {\"kind\": \"check\", \"yes\": [\"Q1\"], \"no\": [\"Q2\"]}: \"yes\" when the line \
uses that known name (that person, place, thing or concept of the game), \"no\" when it is an ordinary word \
or someone or something else. A name shared by several entries is asked once per entry, with that entry in parentheses. \
A checked name is not new: do not list it as a character or term. \
Answer with a JSON array, or [] if there is nothing new. Each object has the fields: \
\"kind\": one of \"character\", \"location\", \"organization\", \"title\", \"item\", \"event\", \"general\"; \
\"name\": the exact spelling from the text; \
\"gender\": \"male\", \"female\" or \"unknown\", only for a character; \
\"first_name\" and \"last_name\": only for a character with a personal name, its given name and surname as written \
(\"Vera Karanovic\": \"Vera\" and \"Karanovic\"; a lone \"Karanovic\" that is a surname: only \"last_name\"); \
\"same_as\": only when this is another name of a known name or of another name in your answer, its exact spelling \
or id; \
\"other_than\": only for a known name used for someone or something else, the id or name of the known one. \
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
    /// Чьё это другое имя по словам модели: написание известной записи или другой находки.
    pub same_as: Option<String>,
    /// С кем находку нельзя путать по словам модели: известное написание или номер персонажа сцены. Так модель
    /// называет другую сущность с известным написанием.
    pub other_than: Option<String>,
    /// Имя и фамилия персонажа по словам модели. `None`: не личное имя или части не сошлись с написанием.
    pub parts: Option<NameParts>,
}

/// Личное имя персонажа по частям. Хотя бы одна часть есть.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameParts {
    pub first: Option<String>,
    pub last: Option<String>,
}

impl NameParts {
    /// Части годятся, только если вместе дают написание находки: «Vera» и «Karanovic» у «Vera Karanovic»,
    /// одна фамилия «Karanovic» у «Karanovic». Имя из одного слова без частей это просто имя.
    pub fn check(name: &str, first: Option<&str>, last: Option<&str>) -> Option<Self> {
        let clean = |part: Option<&str>| part.map(str::trim).filter(|part| !part.is_empty()).map(str::to_string);
        let (first, last) = (clean(first), clean(last));
        let joined = [first.as_deref(), last.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        if joined.is_empty() || NameKey::new(&joined) != NameKey::new(name) {
            return None;
        }
        Some(Self { first, last })
    }
}

/// Запись ответа. Поля под разными именами: модели пишут их по привычке.
#[derive(Debug, Default, Deserialize)]
struct AnswerItem {
    #[serde(alias = "type", alias = "category")]
    kind: Option<String>,
    #[serde(alias = "term", alias = "text")]
    name: Option<String>,
    gender: Option<String>,
    #[serde(alias = "alias_of", alias = "form_of", alias = "sameAs")]
    same_as: Option<String>,
    #[serde(alias = "not", alias = "different_from", alias = "otherThan")]
    other_than: Option<String>,
    #[serde(alias = "given_name", alias = "firstName")]
    first_name: Option<String>,
    #[serde(alias = "surname", alias = "lastName", alias = "family_name")]
    last_name: Option<String>,
    /// У записи `same_person`: номера персонажей сцены.
    #[serde(default)]
    ids: Vec<serde_json::Value>,
    /// У записи `check`: номера вопросов, где известная запись упомянута.
    #[serde(default, alias = "used")]
    yes: Vec<serde_json::Value>,
    /// У записи `check`: номера вопросов, где это обычное слово.
    #[serde(default, alias = "unused", alias = "not_used")]
    no: Vec<serde_json::Value>,
}

/// Вопросов о спорных упоминаниях в одном запросе разбора не больше этого: длинный список модель читает хуже.
const MAX_CHECKS: usize = 20;
/// Реплика в вопросе не длиннее этого.
const MAX_CHECK_LINE: usize = 300;

/// Спорное упоминание известной записи в части: модель отвечает, запись это или обычное слово.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub owner: ExcludeOwner,
    /// Написание записи, о котором спрашивается.
    pub name: String,
    /// Реплика строкой «Говорящий: текст», как в части.
    pub line: String,
    /// Отпечаток оригинала реплики: по нему пишется ответ.
    pub hash: String,
    /// Запись, о которой вопрос, если у написания несколько владельцев: «Vera Karanovic» у «Karanovic»,
    /// а при одинаковых именах вид записи («character», «term»).
    pub entry: Option<String>,
    doubt: u8,
}

/// Запись глоссария для вопросов: чья, написания и уже полученные ответы модели.
pub struct Asked<'a> {
    pub owner: ExcludeOwner,
    /// Главное имя записи: полное имя персонажа, название термина.
    pub label: String,
    pub names: Vec<String>,
    pub checks: Option<&'a Checks>,
}

impl<'a> Asked<'a> {
    /// Персонажи и термины глоссария с их написаниями и ответами.
    pub fn all(glossary: &'a GlossaryData) -> Vec<Self> {
        let characters = glossary.characters.iter().filter_map(|character| {
            let id = character.id()?;
            Some(Self {
                owner: ExcludeOwner::Character(id),
                label: character.full_name().unwrap_or_default(),
                names: character_names(character),
                checks: glossary.character_checks.get(&id),
            })
        });
        let terms = glossary.terms.iter().filter_map(|term| {
            let id = term.id()?;
            Some(Self { owner: ExcludeOwner::Term(id), label: term.name().to_string(), names: term_names(term), checks: glossary.term_checks.get(&id) })
        });
        characters.chain(terms).filter(|entry| !entry.names.is_empty()).collect()
    }
}

/// Реплика сцены для вопросов: строка как в части, текст с подстановками и отпечаток оригинала.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartLine {
    pub script: String,
    pub text: String,
    pub hash: String,
}

/// Вопросы о спорных упоминаниях известных записей в части. Без вопроса остаются реплики, где у записи есть
/// бесспорное упоминание или уже есть ответ модели. Написание с несколькими владельцами (фамилия семьи, город
/// и человек с одним именем) бесспорным не бывает: вопрос задаётся для каждой записи и называет её.
/// Первыми идут точные и необычные написания, обычные слова последними, не больше `MAX_CHECKS`.
pub fn questions(part: &str, lines: &[PartLine], entries: &[Asked], casings: &mut Casings) -> Vec<Question> {
    let by_script: HashMap<&str, &PartLine> = lines.iter().rev().map(|line| (line.script.as_str(), line)).collect();
    let mut owners: HashMap<NameKey, usize> = HashMap::new();
    for key in entries.iter().flat_map(|entry| entry.names.iter().filter_map(|name| NameKey::new(name)).collect::<HashSet<_>>()) {
        *owners.entry(key).or_default() += 1;
    }
    let shared = |name: &str| NameKey::new(name).is_some_and(|key| owners.get(&key).is_some_and(|count| *count > 1));
    let mut asked: Vec<Question> = Vec::new();
    for script in part.lines() {
        let Some(line) = by_script.get(script) else { continue };
        for entry in entries {
            if entry.checks.is_some_and(|checks| checks.contains_key(&line.hash))
                || asked.iter().any(|question| question.owner == entry.owner && question.hash == line.hash)
            {
                continue;
            }
            let found: Vec<(&str, &str)> =
                entry.names.iter().flat_map(|name| occurrences(&line.text, name).into_iter().map(move |found| (found, name.as_str()))).collect();
            if found.is_empty() || found.iter().any(|(found, name)| !shared(name) && is_sure(found, name, casings)) {
                continue;
            }
            let Some((doubt, name)) = found.iter().map(|(found, name)| (doubt(found, name), *name)).min_by_key(|(doubt, _)| *doubt) else { continue };
            let named = shared(name).then(|| match entry.owner {
                _ if NameKey::new(name) != NameKey::new(&entry.label) => entry.label.clone(),
                ExcludeOwner::Character(_) => "character".to_string(),
                ExcludeOwner::Term(_) => "term".to_string(),
            });
            let mut quoted: String = line.script.chars().take(MAX_CHECK_LINE).collect();
            if line.script.chars().count() > MAX_CHECK_LINE {
                quoted.push('…');
            }
            asked.push(Question { owner: entry.owner, name: name.to_string(), line: quoted, hash: line.hash.clone(), entry: named, doubt });
        }
    }
    asked.sort_by_key(|question| question.doubt);
    asked.truncate(MAX_CHECKS);
    asked
}

/// Персонажи сцены в запросе разбора не больше этого: длинный список модель читает хуже.
const MAX_CAST: usize = 40;

/// Известный персонаж сцены в запросе разбора: модель ссылается на него номером `C<id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastMember {
    pub id: CharacterId,
    /// Полное имя, имя, фамилия и формы.
    pub names: Vec<String>,
    pub gender: Option<Gender>,
}

/// Номер персонажа в запросе: «C3».
pub fn cast_ref(id: CharacterId) -> String {
    format!("C{}", id.get())
}

/// Персонаж по номеру из ответа модели: «C3», «c3», «3».
pub fn parse_cast_ref(text: &str) -> Option<CharacterId> {
    let text = text.trim();
    let digits = text.strip_prefix(['C', 'c']).unwrap_or(text);
    digits.parse::<i64>().ok().filter(|id| *id > 0).map(CharacterId::from_row)
}

/// Запрос разбора одной части.
pub struct AnalysisPrompt;

impl AnalysisPrompt {
    /// `known`: имена и названия глоссария. В запрос идут только упомянутые в части. `cast`: персонажи сцены
    /// с номерами, по которым модель связывает находки и называет одного человека под двумя записями.
    /// `questions`: спорные упоминания известных записей, о которых модель отвечает записью `check`.
    pub fn build(part: &str, known: &[String], cast: &[CastMember], questions: &[Question], target: &LlmTarget) -> ChatRequest {
        let mut size = 0;
        let mut seen = HashSet::new();
        let named: Vec<&str> = known
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty() && mentioned(part, "", &[name], None))
            .filter(|name| seen.insert(name.to_lowercase()))
            .take_while(|name| {
                size += name.chars().count() + 2;
                size <= MAX_KNOWN_CHARS
            })
            .collect();
        let known = if named.is_empty() { String::new() } else { format!("Known names and terms: {}\n\n", named.join(", ")) };
        let cast: String = cast
            .iter()
            .filter(|member| !member.names.is_empty())
            .take(MAX_CAST)
            .map(|member| {
                let gender = match member.gender {
                    Some(Gender::Male) => " (male)",
                    Some(Gender::Female) => " (female)",
                    None => "",
                };
                format!("{}: {}{gender}\n", cast_ref(member.id), member.names.join(", "))
            })
            .collect();
        let cast = if cast.is_empty() { String::new() } else { format!("Characters of this scene:\n{cast}\n") };
        ChatRequest {
            system: SYSTEM.to_string(),
            user: format!("{known}{cast}Script:\n\n{part}\n{}New names and terms (JSON array):", checks_block(questions)),
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
                if item.kind.as_deref().is_some_and(|kind| ["same_person", "check"].iter().any(|own| kind.trim().eq_ignore_ascii_case(own))) {
                    return None;
                }
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
                let same_as = item
                    .same_as
                    .as_deref()
                    .map(|text| text.trim().trim_matches(|c: char| "\"'«»“”".contains(c)).trim().to_string())
                    .filter(|text| !text.is_empty() && NameKey::new(text) != NameKey::new(&name));
                let other_than = item
                    .other_than
                    .as_deref()
                    .map(|text| text.trim().trim_matches(|c: char| "\"'«»“”".contains(c)).trim().to_string())
                    .filter(|text| !text.is_empty());
                let parts = match kind {
                    FoundKind::Character => NameParts::check(&name, item.first_name.as_deref(), item.last_name.as_deref()),
                    FoundKind::Term(_) => None,
                };
                Some(Finding { name, kind, gender, same_as, other_than, parts })
            })
            .collect()
    }

    /// Ответы на вопросы `questions` из записи `check`: номер вопроса с нуля и «да». Номер вне вопросов и вопрос
    /// с обоими ответами отбрасываются.
    pub fn checks(text: &str, questions: usize) -> Vec<(usize, bool)> {
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json_array(text)) else { return Vec::new() };
        let number = |value: &serde_json::Value| -> Option<usize> {
            let number = match value {
                serde_json::Value::String(text) => {
                    let text = text.trim();
                    text.strip_prefix(['Q', 'q']).unwrap_or(text).parse::<usize>().ok()?
                }
                serde_json::Value::Number(number) => usize::try_from(number.as_u64()?).ok()?,
                _ => return None,
            };
            number.checked_sub(1).filter(|index| *index < questions)
        };
        let mut answers: Vec<(usize, bool)> = Vec::new();
        let mut both: Vec<usize> = Vec::new();
        for item in values.into_iter().filter_map(|value| serde_json::from_value::<AnswerItem>(value).ok()) {
            if !item.kind.as_deref().is_some_and(|kind| kind.trim().eq_ignore_ascii_case("check")) {
                continue;
            }
            for (list, used) in [(&item.yes, true), (&item.no, false)] {
                for index in list.iter().filter_map(number) {
                    match answers.iter().find(|(known, _)| *known == index) {
                        Some((_, earlier)) if *earlier != used => both.push(index),
                        Some(_) => {}
                        None => answers.push((index, used)),
                    }
                }
            }
        }
        answers.retain(|(index, _)| !both.contains(index));
        answers
    }

    /// Пары персонажей, которых модель назвала одним человеком (`same_person`), по номерам из запроса.
    /// Номер не из `cast` отбрасывается. Записи с тремя номерами дают пары первого с каждым.
    pub fn same_people(text: &str, cast: &[CharacterId]) -> Vec<(CharacterId, CharacterId)> {
        let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json_array(text)) else { return Vec::new() };
        let mut pairs = Vec::new();
        for item in values.into_iter().filter_map(|value| serde_json::from_value::<AnswerItem>(value).ok()) {
            let kind = item.kind.unwrap_or_default().trim().to_lowercase().replace([' ', '-'], "_");
            if kind != "same_person" {
                continue;
            }
            let mut ids: Vec<CharacterId> = item
                .ids
                .iter()
                .filter_map(|value| match value {
                    serde_json::Value::String(text) => parse_cast_ref(text),
                    serde_json::Value::Number(number) => number.as_i64().filter(|id| *id > 0).map(CharacterId::from_row),
                    _ => None,
                })
                .filter(|id| cast.contains(id))
                .collect();
            ids.dedup();
            let Some((first, rest)) = ids.split_first() else { continue };
            for other in rest {
                let pair = (*first.min(other), *first.max(other));
                if pair.0 != pair.1 && !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
        }
        pairs
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
            (Finding { name: finding.name.clone(), kind: finding.kind, gender: None, same_as: None, other_than: None, parts: None }, Vec::new())
        });
        match kinds.iter_mut().find(|(kind, _)| *kind == finding.kind) {
            Some((_, count)) => *count += 1,
            None => kinds.push((finding.kind, 1)),
        }
        if merged.gender.is_none() {
            merged.gender = finding.gender;
        }
        if merged.same_as.is_none() {
            merged.same_as = finding.same_as;
        }
        if merged.other_than.is_none() {
            merged.other_than = finding.other_than;
        }
        if merged.parts.is_none() {
            merged.parts = finding.parts;
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

/// Блок вопросов в запрос разбора. Пустой, если спрашивать нечего.
fn checks_block(questions: &[Question]) -> String {
    if questions.is_empty() {
        return String::new();
    }
    let lines: String = questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let entry = question.entry.as_deref().map(|entry| format!(" ({entry})")).unwrap_or_default();
            format!("Q{} {}{entry}: {}\n", index + 1, question.name, serde_json::to_string(&question.line).unwrap_or_default())
        })
        .collect();
    format!("Known names to check. For each, is it used here as that name, or is it an ordinary word?\n{lines}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::TermId;

    fn finding(name: &str, kind: FoundKind) -> Finding {
        Finding { name: name.to_string(), kind, gender: None, same_as: None, other_than: None, parts: None }
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
    fn prompt_numbers_the_cast_and_reads_same_person_pairs() {
        let target = crate::translation::prompt::tests::target();
        let id = CharacterId::from_row;
        let cast = [
            CastMember { id: id(3), names: vec!["Dimitria".to_string(), "Dimi".to_string()], gender: Some(Gender::Female) },
            CastMember { id: id(7), names: vec!["Vellnikov".to_string()], gender: None },
        ];
        let request = AnalysisPrompt::build("Hilde: Chief Vellnikov.\n", &[], &cast, &[], &target);
        assert!(request.user.starts_with("Characters of this scene:\nC3: Dimitria, Dimi (female)\nC7: Vellnikov\n\nScript:"), "{}", request.user);

        let answer = r#"[{"kind": "same_person", "ids": ["C7", "C3"]}, {"kind": "same_person", "ids": ["C3", "C99"]}, {"kind": "character", "name": "Chief", "same_as": "C7"}]"#;
        assert_eq!(AnalysisPrompt::same_people(answer, &[id(3), id(7)]), [(id(3), id(7))], "номер не из сцены отброшен");
        let found = AnalysisPrompt::accept(answer, "Hilde: Chief Vellnikov.\n");
        assert_eq!(found.len(), 1, "same_person не находка");
        assert_eq!(found[0].same_as.as_deref(), Some("C7"));
        assert_eq!(parse_cast_ref(" c12 "), Some(id(12)));
        assert_eq!(parse_cast_ref("Dimitria"), None);
    }

    #[test]
    fn prompt_lists_only_known_names_from_the_part() {
        let target = crate::translation::prompt::tests::target();
        let known = vec!["Eileen".to_string(), "Lis".to_string(), "Nowhere".to_string(), "eileen".to_string()];
        let request = AnalysisPrompt::build("Eileen: Hi, Lis.\n", &known, &[], &[], &target);
        assert!(request.user.starts_with("Known names and terms: Eileen, Lis\n\nScript:\n\nEileen: Hi, Lis.\n"), "{}", request.user);
        assert!(AnalysisPrompt::build("Narration.\n", &known, &[], &[], &target).user.starts_with("Script:"));
    }

    #[test]
    fn known_names_follow_spelling() {
        let target = crate::translation::prompt::tests::target();
        let known = vec!["MInD".to_string(), "Hope".to_string()];
        let request = AnalysisPrompt::build("Lis: Never mind, I hope so.\n", &known, &[], &[], &target);
        assert!(request.user.starts_with("Script:"), "обычные слова не подсказываются известными: {}", request.user);
    }

    fn part_line(script: &str, hash: &str) -> PartLine {
        let text = script.split_once(": ").map_or(script, |(_, text)| text);
        PartLine { script: script.to_string(), text: text.to_string(), hash: hash.to_string() }
    }

    fn asked<'a>(owner: ExcludeOwner, label: &str, names: &[&str], checks: Option<&'a Checks>) -> Asked<'a> {
        Asked { owner, label: label.to_string(), names: names.iter().map(|name| name.to_string()).collect(), checks }
    }

    #[test]
    fn questions_cover_only_doubtful_mentions() {
        let lines = [
            part_line("Hope: Hope you're right.", "h1"),
            part_line("Lis: Never mind, MInD knows.", "h2"),
            part_line("Lis: Never mind.", "h3"),
            part_line("Lis: Karanovic and MIND!", "h4"),
            part_line("Lis: Hope, wait!", "h5"),
        ];
        let part: String = lines.iter().map(|line| format!("{}\n", line.script)).collect();
        let mut casings = Casings::new(["I hope so.", "Never mind.", "Karanovic came."].map(String::from));
        let answered: Checks = [("h5".to_string(), true)].into();
        let hope = ExcludeOwner::Character(CharacterId::from_row(1));
        let mind = ExcludeOwner::Term(TermId::from_row(2));
        let karanovic = ExcludeOwner::Character(CharacterId::from_row(3));
        let entries = [asked(hope, "Hope", &["Hope"], Some(&answered)), asked(mind, "MInD", &["MInD"], None), asked(karanovic, "Karanovic", &["Karanovic"], None)];
        let found: Vec<(ExcludeOwner, String)> =
            questions(&part, &lines, &entries, &mut casings).into_iter().map(|question| (question.owner, question.hash)).collect();
        let expected = [(hope, "h1"), (mind, "h4"), (mind, "h3")].map(|(owner, hash)| (owner, hash.to_string()));
        assert_eq!(found, expected, "бесспорное не спрашивается, отвеченное тоже, обычные слова последними");
    }

    #[test]
    fn a_name_shared_by_entries_is_asked_for_each_of_them() {
        let lines = [part_line("Lis: Karanovic is late.", "k1"), part_line("Lis: Vera Karanovic called.", "k2"), part_line("Lis: Welcome to Hope.", "k3")];
        let part: String = lines.iter().map(|line| format!("{}\n", line.script)).collect();
        let mut casings = Casings::new(["Karanovic is late.", "Welcome to Hope."].map(String::from));
        let vera = ExcludeOwner::Character(CharacterId::from_row(1));
        let ivan = ExcludeOwner::Character(CharacterId::from_row(2));
        let girl = ExcludeOwner::Character(CharacterId::from_row(3));
        let town = ExcludeOwner::Term(TermId::from_row(4));
        let entries = [
            asked(vera, "Vera Karanovic", &["Vera Karanovic", "Vera", "Karanovic"], None),
            asked(ivan, "Ivan Karanovic", &["Ivan Karanovic", "Ivan", "Karanovic"], None),
            asked(girl, "Hope", &["Hope"], None),
            asked(town, "Hope", &["Hope"], None),
        ];
        let found: Vec<(ExcludeOwner, String, Option<String>)> =
            questions(&part, &lines, &entries, &mut casings).into_iter().map(|question| (question.owner, question.hash, question.entry)).collect();
        let name = |text: &str| Some(text.to_string());
        assert_eq!(
            found,
            [
                (vera, "k1".to_string(), name("Vera Karanovic")),
                (ivan, "k1".to_string(), name("Ivan Karanovic")),
                (ivan, "k2".to_string(), name("Ivan Karanovic")),
                (girl, "k3".to_string(), name("character")),
                (town, "k3".to_string(), name("term")),
            ],
            "общая фамилия спрашивается у каждого, полное имя Веры бесспорно"
        );

        let target = crate::translation::prompt::tests::target();
        let all = questions(&part, &lines, &entries, &mut casings);
        let request = AnalysisPrompt::build(&part, &[], &[], &all, &target);
        assert!(request.user.contains("Q1 Karanovic (Vera Karanovic): \"Lis: Karanovic is late.\"\nQ2 Karanovic (Ivan Karanovic):"), "{}", request.user);
    }

    #[test]
    fn prompt_asks_and_reads_checks() {
        let target = crate::translation::prompt::tests::target();
        let question = Question {
            owner: ExcludeOwner::Term(TermId::from_row(2)),
            name: "MInD".to_string(),
            line: "Lis: Never mind.".to_string(),
            hash: "h".to_string(),
            entry: None,
            doubt: 2,
        };
        let request = AnalysisPrompt::build("Lis: Never mind.\n", &[], &[], &[question.clone(), question], &target);
        assert!(request.user.contains("Known names to check. For each, is it used here as that name, or is it an ordinary word?\nQ1 MInD: \"Lis: Never mind.\"\nQ2"), "{}", request.user);

        let answer = r#"[{"kind": "check", "yes": ["Q2", 3], "no": ["q1", "Q2", "Q9"]}, {"kind": "check", "used": ["Q3"]}, {"kind": "general", "name": "Never"}]"#;
        assert_eq!(AnalysisPrompt::checks(answer, 3), [(2, true), (0, false)], "противоречивый и чужой номер отброшены");
        assert!(AnalysisPrompt::accept(answer, "Lis: Never mind.\n").iter().all(|found| found.name == "Never"), "check не находка");
    }

    #[test]
    fn findings_merge_by_key_with_the_most_named_kind() {
        let location = FoundKind::Term(TermCategory::Location);
        let lis = Finding { gender: Some(Gender::Female), ..finding("Lis", FoundKind::Character) };
        let merged = merge(vec![finding("Rusty Anchor", location), lis.clone(), finding("rusty anchor", FoundKind::Character), finding("RUSTY ANCHOR", location)]);
        assert_eq!(merged, [finding("Rusty Anchor", location), lis]);

        let ppf = Finding { same_as: Some("People's Police Force".to_string()), ..finding("PPF", location) };
        assert_eq!(merge(vec![finding("PPF", location), ppf.clone()]), [ppf], "ссылка берётся у любой части");
    }

    #[test]
    fn name_parts_must_spell_the_name() {
        let parts = |name, first, last| NameParts::check(name, first, last);
        assert_eq!(parts("Vera Karanovic", Some("Vera"), Some("Karanovic")), Some(NameParts { first: Some("Vera".to_string()), last: Some("Karanovic".to_string()) }));
        assert_eq!(parts("Karanovic", None, Some("Karanovic")), Some(NameParts { first: None, last: Some("Karanovic".to_string()) }));
        assert_eq!(parts("Vera Karanovic", Some("Vera"), Some("Karan")), None, "части не дают написание");
        assert_eq!(parts("Old Bartender", None, None), None);

        let answer = r#"[{"kind": "character", "name": "Vera Karanovic", "given_name": "Vera", "surname": "Karanovic", "gender": "female"}]"#;
        let found = AnalysisPrompt::accept(answer, "Vera Karanovic: Hi.\n");
        assert_eq!(found[0].parts, parts("Vera Karanovic", Some("Vera"), Some("Karanovic")));
    }

    #[test]
    fn answer_keeps_the_name_it_belongs_to_and_the_one_it_is_not() {
        let part = "Eileen: The PPF is here, and Karanovic too.\n";
        let answer = r#"[{"kind": "organization", "name": "PPF", "same_as": "People's Police Force"}, {"kind": "character", "name": "Eileen", "alias_of": "eileen"},
            {"kind": "character", "name": "Karanovic", "last_name": "Karanovic", "other_than": "C5"}]"#;
        assert_eq!(
            AnalysisPrompt::accept(answer, part),
            [
                Finding { same_as: Some("People's Police Force".to_string()), ..finding("PPF", FoundKind::Term(TermCategory::Organization)) },
                finding("Eileen", FoundKind::Character),
                Finding {
                    other_than: Some("C5".to_string()),
                    parts: NameParts::check("Karanovic", None, Some("Karanovic")),
                    ..finding("Karanovic", FoundKind::Character)
                },
            ],
            "ссылка на само имя отброшена, other_than остаётся"
        );
    }
}
