//! Вопрос после разбора сцены: одно ли это две записи глоссария. Пары дают разбор (слово модели, которое
//! скрипт не подтвердил: `resolve::Claim`, `same_person`) и скрипт (`relations::suspects`). Решает модель
//! по репликам и описаниям, отвеченные пары запоминаются (`PairCheckRepo`). Вопрос один для персонажей,
//! терминов и пары из термина и персонажа. Тот же вопрос задаёт шаг переводов глоссария о терминах с одинаковым
//! переводом (`translation_questions`): там реплики берутся по всей игре.

use super::entries::{character_names, scene_entries, term_names};
use super::mentions::{Checks, mentioned};
use super::relations::{self, Pair};
use super::suggest::spread;
use super::repo::{LineSampleRepo, PairCheckRepo, StoryLine};
use super::{Character, CharacterId, ExcludeOwner, GlossaryData, Term};
use crate::common::Gender;
use crate::engines::ContainerKey;
use crate::llm::{ChatRequest, LlmTarget};
use crate::storage::DbError;
use crate::translation::text::mentions;

/// Вопросов на одну сцену не больше этого: остальные пары спросит разбор следующей сцены, где есть одна из записей.
pub const MAX_QUESTIONS: usize = 6;
/// Реплик сцены в вопросе не больше этого.
const MAX_SCENE_LINES: usize = 12;
/// Реплик остальной игры, где упомянуты оба, не больше этого.
const MAX_GAME_LINES: usize = 6;
/// Реплик каждой записи в вопросе без сцены не больше этого.
const MAX_OWN_LINES: usize = 4;
/// Вопросов о терминах с одинаковым переводом за один запуск не больше этого: остальные спросит следующий.
pub const MAX_TRANSLATION_QUESTIONS: usize = 30;
/// Реплика в вопросе обрезается до этого числа символов.
const MAX_LINE_CHARS: usize = 300;
const ANSWER_TOKENS: u32 = 64;

const SYSTEM: &str = "You check the glossary of a visual novel: its characters and its terms (places, organizations, \
items, titles). One person or thing is sometimes listed twice: by first name and by surname, by a nickname or an alias, \
by a title, by an abbreviation, a short form, a synonym or a misspelling. Two different people or things also sometimes share a name. \
Decide from the lines and descriptions whether entries A and B are the same person or the same thing. \
Members of one family often share a surname: then they are different people. \
If the text gives no reason to think they are the same, answer false. \
Answer with JSON only: {\"same\": true} or {\"same\": false}.";

/// Запись в вопросе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub owner: ExcludeOwner,
    /// Вид для модели: «character» или «term, organization».
    pub kind: String,
    /// Полное имя, имя, фамилия и формы персонажа, название и формы термина.
    pub names: Vec<String>,
    pub gender: Option<Gender>,
    pub description: Option<String>,
}

impl Side {
    pub fn person(character: &Character) -> Option<Self> {
        Some(Self {
            owner: ExcludeOwner::Character(character.id()?),
            kind: "character".to_string(),
            names: character_names(character),
            gender: character.gender(),
            description: character.description().map(str::to_string),
        })
    }

    pub fn term(term: &Term) -> Option<Self> {
        Some(Self {
            owner: ExcludeOwner::Term(term.id()?),
            kind: format!("term, {}", term.category().as_str()),
            names: term_names(term),
            gender: None,
            description: term.description().map(str::to_string),
        })
    }

    fn of(glossary: &GlossaryData, owner: ExcludeOwner) -> Option<Self> {
        match owner {
            ExcludeOwner::Character(id) => glossary.characters.iter().find(|character| character.id() == Some(id)).and_then(Self::person),
            ExcludeOwner::Term(id) => glossary.terms.iter().find(|term| term.id() == Some(id)).and_then(Self::term),
        }
    }

    /// Имя для хода запуска и итога.
    pub fn label(&self) -> &str {
        self.names.first().map_or("?", String::as_str)
    }

    pub fn is_person(&self) -> bool {
        matches!(self.owner, ExcludeOwner::Character(_))
    }
}

/// Вопрос о паре с доказательствами.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub a: Side,
    pub b: Side,
    /// Реплики разобранной сцены, где упомянут кто-то из двоих.
    pub scene_lines: Vec<String>,
    /// Реплики игры, где упомянуты оба.
    pub game_lines: Vec<String>,
    /// Реплики игры, где упомянут один из двоих: у вопроса без сцены.
    pub own_lines: Vec<String>,
}

impl Question {
    pub fn pair(&self) -> Option<Pair> {
        Pair::new(self.a.owner, self.b.owner)
    }
}

fn line_text(line: &StoryLine) -> String {
    let text: String = line.text.chars().take(MAX_LINE_CHARS).collect();
    match &line.label {
        Some(label) => format!("{label}: {text}"),
        None => text,
    }
}

fn mentions_any(lower: &str, names: &[String]) -> bool {
    names.iter().any(|name| mentions(lower, &name.to_lowercase()))
}

/// Одиночное имя короче этого с другим подряд не ищется: «Al» найдётся в чём угодно.
const MIN_SINGLE_NAME: usize = 3;

/// Пары персонажей, чьи одиночные имена стоят подряд в репликах игры: «Dimitria Vellnikov» у «Dimitria»
/// и «Vellnikov». Полное имя бывает в сцене, которую разбор ещё не читал, а фамилия бывает записана именем.
/// Только пары, где хотя бы один из двоих в сцене.
fn named_together(tx: &rusqlite::Connection, glossary: &GlossaryData, present: &[CharacterId]) -> Result<Vec<(CharacterId, CharacterId)>, DbError> {
    let singles: Vec<(CharacterId, String)> = glossary
        .characters
        .iter()
        .filter(|character| character.first_name().is_some() != character.last_name().is_some())
        .filter_map(|character| Some((character.id()?, character.full_name()?.to_lowercase())))
        .filter(|(_, name)| name.chars().count() >= MIN_SINGLE_NAME && !name.contains(char::is_whitespace) && !name.contains('['))
        .collect();
    let mut pairs: Vec<(CharacterId, CharacterId)> = Vec::new();
    for (here, name) in singles.iter().filter(|(id, _)| present.contains(id)) {
        let lines: Vec<String> = LineSampleRepo { conn: tx }.containing(name, None)?.into_iter().map(|line| line.text.to_lowercase()).collect();
        for (other, other_name) in singles.iter().filter(|(id, other_name)| id != here && other_name != name) {
            let together = |text: String| lines.iter().any(|line| line.contains(&text) && mentions(line, &text));
            let joined = together(format!("{other_name} {name}")) || together(format!("{name} {other_name}"));
            if joined && !pairs.contains(&(*here, *other)) && !pairs.contains(&(*other, *here)) {
                pairs.push((*here, *other));
            }
        }
    }
    Ok(pairs)
}

/// Вопросы к модели для сцены `scene`: сначала пары от разбора (`claimed`), потом пары скрипта по порядку
/// отношений. Пары, где одной записи уже нет или про которые модель уже отвечала, пропускаются.
/// Не больше `MAX_QUESTIONS`.
pub fn questions(tx: &rusqlite::Connection, scene: &ContainerKey, claimed: &[Pair]) -> Result<Vec<Question>, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let lines = LineSampleRepo { conn: tx }.scene_lines(scene)?;
    let (people, terms) = scene_entries(&glossary, &lines);
    let present: Vec<ExcludeOwner> = people.iter().copied().map(ExcludeOwner::Character).chain(terms.into_iter().map(ExcludeOwner::Term)).collect();
    let together = named_together(tx, &glossary, &people)?;
    let suspected = relations::suspects(&glossary, &present, &together).into_iter().map(|suspect| suspect.pair);
    let checks = PairCheckRepo { conn: tx };
    let mut seen: Vec<Pair> = Vec::new();
    let mut questions = Vec::new();
    for pair in claimed.iter().copied().chain(suspected) {
        if questions.len() >= MAX_QUESTIONS {
            break;
        }
        if seen.contains(&pair) || checks.checked(pair.a, pair.b)? {
            continue;
        }
        seen.push(pair);
        let (Some(a), Some(b)) = (Side::of(&glossary, pair.a), Side::of(&glossary, pair.b)) else { continue };
        if a.names.is_empty() || b.names.is_empty() {
            continue;
        }
        let scene_lines: Vec<String> = lines
            .iter()
            .filter(|line| {
                let lower = format!("{} {}", line.label.as_deref().unwrap_or_default(), line.text).to_lowercase();
                mentions_any(&lower, &a.names) || mentions_any(&lower, &b.names)
            })
            .take(MAX_SCENE_LINES)
            .map(line_text)
            .collect();
        let game_lines = lines_with_both(tx, &a, &b, &scene_lines)?;
        questions.push(Question { a, b, scene_lines, game_lines, own_lines: Vec::new() });
    }
    Ok(questions)
}

/// Реплики игры, где упомянуты обе записи, кроме уже показанных в `shown`. Не больше `MAX_GAME_LINES`.
fn lines_with_both(tx: &rusqlite::Connection, a: &Side, b: &Side, shown: &[String]) -> Result<Vec<String>, DbError> {
    let mut found: Vec<String> = Vec::new();
    for name in &a.names {
        for line in (LineSampleRepo { conn: tx }).containing(name, Some(200))? {
            let lower = line.text.to_lowercase();
            let text: String = line.text.chars().take(MAX_LINE_CHARS).collect();
            if mentions(&lower, &name.to_lowercase()) && mentions_any(&lower, &b.names) && !found.contains(&text) && !shown.iter().any(|own| own.ends_with(&text)) {
                found.push(text);
            }
        }
        if found.len() >= MAX_GAME_LINES {
            break;
        }
    }
    found.truncate(MAX_GAME_LINES);
    Ok(found)
}

/// Реплики игры, где упомянута запись, вразброс по сюжету: упоминание решает `mentions::mentioned` с ответами
/// модели. Не больше `MAX_OWN_LINES`.
fn lines_of(tx: &rusqlite::Connection, side: &Side, checks: Option<&Checks>) -> Result<Vec<String>, DbError> {
    let mut found: Vec<String> = Vec::new();
    for name in &side.names {
        for line in (LineSampleRepo { conn: tx }).containing(name, Some(200))? {
            if !mentioned(&line.text, &line.source_hash, &side.names, checks) {
                continue;
            }
            let text: String = line.text.chars().take(MAX_LINE_CHARS).collect();
            let text = match &line.speaker {
                Some(speaker) => format!("{speaker}: {text}"),
                None => text,
            };
            if !found.contains(&text) {
                found.push(text);
            }
        }
    }
    Ok(spread(&found, MAX_OWN_LINES))
}

/// Пары терминов с одинаковым переводом, про которые модель ещё не отвечала (`relations::same_translation`).
pub fn translation_pairs(tx: &rusqlite::Connection, glossary: &GlossaryData) -> Result<Vec<Pair>, DbError> {
    let checks = PairCheckRepo { conn: tx };
    let mut pairs = Vec::new();
    for pair in relations::same_translation(&glossary.terms) {
        if !checks.checked(pair.a, pair.b)? {
            pairs.push(pair);
        }
    }
    Ok(pairs)
}

/// Вопросы о терминах с одинаковым переводом: реплики каждого термина и реплики, где названы оба, по всей игре.
/// Не больше `MAX_TRANSLATION_QUESTIONS`.
pub fn translation_questions(tx: &rusqlite::Connection) -> Result<Vec<Question>, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let mut questions = Vec::new();
    for pair in translation_pairs(tx, &glossary)?.into_iter().take(MAX_TRANSLATION_QUESTIONS) {
        let (Some(a), Some(b)) = (Side::of(&glossary, pair.a), Side::of(&glossary, pair.b)) else { continue };
        let checks = |owner: ExcludeOwner| match owner {
            ExcludeOwner::Term(id) => glossary.term_checks.get(&id),
            ExcludeOwner::Character(id) => glossary.character_checks.get(&id),
        };
        let mut own_lines = lines_of(tx, &a, checks(pair.a))?;
        for line in lines_of(tx, &b, checks(pair.b))? {
            if !own_lines.contains(&line) {
                own_lines.push(line);
            }
        }
        let game_lines = lines_with_both(tx, &a, &b, &own_lines)?;
        questions.push(Question { a, b, scene_lines: Vec::new(), game_lines, own_lines });
    }
    Ok(questions)
}

/// Запрос «одно ли это».
pub struct SameEntryPrompt;

impl SameEntryPrompt {
    pub fn build(question: &Question, target: &LlmTarget) -> ChatRequest {
        let side = |tag: &str, side: &Side| {
            let kind = if side.is_person() { String::new() } else { format!(" ({})", side.kind) };
            let gender = match side.gender {
                Some(Gender::Male) => "\nGender: male",
                Some(Gender::Female) => "\nGender: female",
                None => "",
            };
            let description = side.description.as_deref().map(|text| format!("\nDescription: {text}")).unwrap_or_default();
            format!("{tag}{kind}: {}{gender}{description}\n", side.names.join(", "))
        };
        let block = |title: &str, lines: &[String]| if lines.is_empty() { String::new() } else { format!("\n{title}:\n{}\n", lines.join("\n")) };
        let ask = if question.a.is_person() && question.b.is_person() { "Are A and B the same person?" } else { "Are A and B the same thing?" };
        let both = if question.scene_lines.is_empty() { "Lines of the game where both are named" } else { "Other lines of the game where both are named" };
        let user = format!(
            "{}{}{}{}{}\n{ask}",
            side("A", &question.a),
            side("B", &question.b),
            block("Lines of the scene", &question.scene_lines),
            block("Lines of the game where A or B is named", &question.own_lines),
            block(both, &question.game_lines),
        );
        ChatRequest { system: SYSTEM.to_string(), user, temperature: target.settings.temperature, max_tokens: ANSWER_TOKENS }
    }

    /// Ответ модели: `Some(true)` одно, `Some(false)` разное, `None` ответ не разобран.
    pub fn accept(text: &str) -> Option<bool> {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        let value: serde_json::Value = serde_json::from_str(text.get(start..=end)?).ok()?;
        match value.get("same")? {
            serde_json::Value::Bool(same) => Some(*same),
            serde_json::Value::String(text) => match text.trim().to_lowercase().as_str() {
                "true" | "yes" => Some(true),
                "false" | "no" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::TermId;

    #[test]
    fn answers_are_read_leniently() {
        assert_eq!(SameEntryPrompt::accept("{\"same\": true}"), Some(true));
        assert_eq!(SameEntryPrompt::accept("Ответ: ```json\n{\"same\": \"no\"}\n```"), Some(false));
        assert_eq!(SameEntryPrompt::accept("Не знаю."), None);
    }

    #[test]
    fn the_question_carries_names_and_lines() {
        let target = crate::translation::prompt::tests::target();
        let person = |id: i64, names: &[&str]| Side {
            owner: ExcludeOwner::Character(CharacterId::from_row(id)),
            kind: "character".to_string(),
            names: names.iter().map(|name| name.to_string()).collect(),
            gender: None,
            description: None,
        };
        let question = Question {
            a: Side { gender: Some(Gender::Female), description: Some("Head of the lab.".to_string()), ..person(3, &["Dimitria", "Dimi"]) },
            b: person(7, &["Vellnikov"]),
            scene_lines: vec!["Hilde: Chief Vellnikov. Allow me to introduce...".to_string()],
            game_lines: vec!["Dr Dimitria Vellnikov".to_string()],
            own_lines: Vec::new(),
        };
        let request = SameEntryPrompt::build(&question, &target);
        assert!(request.user.starts_with("A: Dimitria, Dimi\nGender: female\nDescription: Head of the lab.\nB: Vellnikov\n"), "{}", request.user);
        assert!(request.user.contains("Lines of the scene:\nHilde: Chief Vellnikov") && request.user.contains("both are named:\nDr Dimitria Vellnikov"));
        assert!(request.user.ends_with("Are A and B the same person?"));

        let term = |id: i64, name: &str| Side {
            owner: ExcludeOwner::Term(TermId::from_row(id)),
            kind: "term, organization".to_string(),
            names: vec![name.to_string()],
            gender: None,
            description: None,
        };
        let question =
            Question { a: term(1, "People's Police Force"), b: term(2, "PPF"), scene_lines: Vec::new(), game_lines: Vec::new(), own_lines: Vec::new() };
        let request = SameEntryPrompt::build(&question, &target);
        assert_eq!(request.user, "A (term, organization): People's Police Force\nB (term, organization): PPF\n\nAre A and B the same thing?");

        // Вопрос без сцены: реплики каждой записи по всей игре.
        let question = Question {
            a: term(1, "Fey Court"),
            b: term(2, "Fairy Court"),
            scene_lines: Vec::new(),
            game_lines: Vec::new(),
            own_lines: vec!["Selena: The Fey Court meets at dusk.".to_string(), "The Fairy Court is not far.".to_string()],
        };
        let request = SameEntryPrompt::build(&question, &target);
        assert!(request.user.contains("\nLines of the game where A or B is named:\nSelena: The Fey Court meets at dusk.\nThe Fairy Court is not far.\n"), "{}", request.user);
    }
}
