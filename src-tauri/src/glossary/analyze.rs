//! Разбор сцены моделью перед её описанием: новые персонажи и термины по частям сцены. Находки сводятся
//! по `NameKey`, известное глоссарию, корзине терминов и подписям говорящих отсеивается, остальное сразу
//! пишется в глоссарий. Описывает новые записи потом общий путь описаний.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::repo::{CharacterRepo, LineSampleRepo, TermRepo};
use super::service::{give_term, merge_characters, merge_terms};
use super::{Character, CharacterId, Form, GlossaryData, NameKey, Term, TermCategory, TermId};
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
A new spelling of a known or listed name is not skipped: an abbreviation, a short or full form, a nickname, \
a title with a surname (\"PPF\" for \"People's Police Force\", \"Mr. Lee\" for \"Eileen Lee\"). List it with \"same_as\". \
Answer with a JSON array, or [] if there is nothing new. Each object has the fields: \
\"kind\": one of \"character\", \"location\", \"organization\", \"title\", \"item\", \"event\", \"general\"; \
\"name\": the exact spelling from the text; \
\"gender\": \"male\", \"female\" or \"unknown\", only for a character; \
\"same_as\": only when this is another name of a known name or of another name in your answer, its exact spelling. \
Respond with the JSON array only.";

/// Служебные слова, которые аббревиатура пропускает: «People's Police Force» это PPF, «Order of the Dawn» это OD.
const MINOR_WORDS: [&str; 10] = ["of", "the", "and", "for", "a", "an", "in", "on", "at", "to"];

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
                let same_as = item
                    .same_as
                    .as_deref()
                    .map(|text| text.trim().trim_matches(|c: char| "\"'«»“”".contains(c)).trim().to_string())
                    .filter(|text| !text.is_empty() && NameKey::new(text) != NameKey::new(&name));
                Some(Finding { name, kind, gender, same_as })
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
            (Finding { name: finding.name.clone(), kind: finding.kind, gender: None, same_as: None }, Vec::new())
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

/// Аббревиатура: 2–8 заглавных букв или цифр, хотя бы две буквы, точки между ними допускаются («P.P.F.»).
/// Возвращает её без точек.
pub fn acronym(text: &str) -> Option<String> {
    let bare: String = text.trim().chars().filter(|c| *c != '.').collect();
    let letters = bare.chars().filter(|c| c.is_alphabetic()).count();
    let fits = (2..=8).contains(&bare.chars().count())
        && letters >= 2
        && bare.chars().all(|c| c.is_alphanumeric() && !c.is_lowercase());
    fits.then_some(bare)
}

/// Первые буквы значимых слов названия из нескольких слов: «People's Police Force» это «PPF».
/// `None`: слово одно, сокращать нечего.
pub fn initials(name: &str) -> Option<String> {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-')
        .filter(|word| !word.is_empty() && !MINOR_WORDS.contains(&word.to_lowercase().as_str()))
        .collect();
    if words.len() < 2 {
        return None;
    }
    words.iter().map(|word| word.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string())).collect()
}

/// Одно название сокращает другое: аббревиатура совпала с первыми буквами полного названия.
pub fn abbreviates(short: &str, long: &str) -> bool {
    match (acronym(short), initials(long)) {
        (Some(short), Some(long)) => short.to_uppercase() == long,
        _ => false,
    }
}

/// Запись глоссария, к которой ложится форма.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Character(CharacterId),
    Term(TermId),
}

/// Написания записей глоссария и их владельцы: термины по названию и формам, персонажи по полному имени,
/// имени, фамилии и формам. Растёт по мере записи находок.
struct Owners {
    by_key: HashMap<NameKey, Owner>,
    /// Названия и формы терминов: по ним ищется аббревиатура.
    term_names: Vec<(String, TermId)>,
}

impl Owners {
    fn load(glossary: &GlossaryData) -> Self {
        let mut owners = Self { by_key: HashMap::new(), term_names: Vec::new() };
        for term in &glossary.terms {
            let Some(id) = term.id() else { continue };
            owners.add_term(term.name(), id);
            for form in term.forms() {
                owners.add_term(form.text(), id);
            }
        }
        for character in &glossary.characters {
            let Some(id) = character.id() else { continue };
            let names = character
                .full_name()
                .into_iter()
                .chain(character.first_name().map(str::to_string))
                .chain(character.last_name().map(str::to_string))
                .chain(character.forms().iter().map(|form| form.text().to_string()));
            for name in names {
                owners.add(&name, Owner::Character(id));
            }
        }
        owners
    }

    fn add(&mut self, name: &str, owner: Owner) {
        if let Some(key) = NameKey::new(name) {
            self.by_key.entry(key).or_insert(owner);
        }
    }

    fn add_term(&mut self, name: &str, id: TermId) {
        self.add(name, Owner::Term(id));
        self.term_names.push((name.to_string(), id));
    }

    /// Чья это форма: запись, которую назвала модель, иначе термин, который сокращает находка или который
    /// сокращает её. `own`: запись самой находки, если она уже есть, на себя находка не ссылается.
    fn owner_of(&self, finding: &Finding, own: Option<Owner>) -> Option<Owner> {
        let named = finding.same_as.as_deref().and_then(NameKey::new).and_then(|key| self.by_key.get(&key)).copied();
        if let Some(owner) = named.filter(|owner| Some(*owner) != own) {
            return Some(owner);
        }
        self.term_names
            .iter()
            .filter(|(_, id)| Some(Owner::Term(*id)) != own)
            .find(|(name, _)| abbreviates(&finding.name, name) || abbreviates(name, &finding.name))
            .map(|(_, id)| Owner::Term(*id))
    }
}

/// Две записи глоссария оказались одной: связь нашлась позже, в другой сцене. Главной у терминов становится
/// полное название, термин уходит персонажу формой, персонаж `own` сливается в `other`. Персонаж в термин не
/// сливается. Возвращает строку итога: «PPF → People's Police Force (объединено)».
fn merge_entries(tx: &rusqlite::Connection, own: Owner, other: Owner) -> Result<Option<String>, DbError> {
    let terms = TermRepo { conn: tx };
    let characters = CharacterRepo { conn: tx };
    let character_name = |id| -> Result<String, DbError> { Ok(characters.get(id)?.full_name().unwrap_or_default()) };
    let text = match (own, other) {
        (Owner::Term(a), Owner::Term(b)) => {
            let (a_name, b_name) = (terms.get(a)?.name().to_string(), terms.get(b)?.name().to_string());
            let (from, into, from_name, into_name) =
                if acronym(&b_name).is_some() && acronym(&a_name).is_none() { (b, a, b_name, a_name) } else { (a, b, a_name, b_name) };
            merge_terms(tx, from, into)?;
            format!("{from_name} → {into_name} (объединено)")
        }
        (Owner::Term(a), Owner::Character(b)) => {
            let name = terms.get(a)?.name().to_string();
            give_term(tx, a, b)?;
            format!("{name} → {} (объединено)", character_name(b)?)
        }
        (Owner::Character(a), Owner::Character(b)) => {
            let name = character_name(a)?;
            merge_characters(tx, a, b)?;
            format!("{name} → {} (объединено)", character_name(b)?)
        }
        (Owner::Character(_), Owner::Term(_)) => return Ok(None),
    };
    Ok(Some(text))
}

/// Сначала находки без ссылки на другую находку: тогда запись, к которой ложится форма, уже есть.
fn ordered(found: &[Finding]) -> Vec<&Finding> {
    let keys: HashSet<NameKey> = found.iter().filter_map(|finding| NameKey::new(&finding.name)).collect();
    let linked = |finding: &Finding| finding.same_as.as_deref().and_then(NameKey::new).is_some_and(|key| keys.contains(&key));
    found.iter().filter(|finding| !linked(finding)).chain(found.iter().filter(|finding| linked(finding))).collect()
}

/// Новые записи разбора в глоссарий: персонаж с родом, термин с категорией. Другое имя известной записи или
/// другой находки становится формой той записи. Известное глоссарию, корзине терминов и подписям говорящих
/// пропускается. Возвращает написания добавленных, форму как «PPF → People's Police Force».
pub fn save_found(tx: &rusqlite::Connection, found: &[Finding]) -> Result<Vec<String>, DbError> {
    let mut known: HashSet<NameKey> = known_for_prompt(tx)?.iter().filter_map(|name| NameKey::new(name)).collect();
    let mut owners = Owners::load(&GlossaryData::load(tx)?);
    let terms = TermRepo { conn: tx };
    let characters = CharacterRepo { conn: tx };
    let mut added = Vec::new();
    for finding in ordered(found) {
        let Some(key) = NameKey::new(&finding.name) else { continue };
        if known.contains(&key) || terms.is_known(&key)? {
            // Известная запись оказалась другим именем другой записи: связь нашлась позже, записи сливаются.
            if let Some(own) = owners.by_key.get(&key).copied()
                && let Some(other) = owners.owner_of(finding, Some(own))
                && let Some(text) = merge_entries(tx, own, other)?
            {
                added.push(text);
                owners = Owners::load(&GlossaryData::load(tx)?);
            }
            continue;
        }
        match owners.owner_of(finding, None) {
            Some(Owner::Term(id)) => {
                let mut term = terms.get(id)?;
                if acronym(term.name()).is_some() && acronym(&finding.name).is_none() {
                    // Полное название к термину-аббревиатуре: оно становится названием, аббревиатура формой со своим
                    // переводом, перевод полного названия даст шаг переводов глоссария.
                    let (old, translation) = (term.name().to_string(), term.translation().map(str::to_string));
                    if term.rename(&finding.name).is_err() {
                        continue;
                    }
                    term.set_translation("");
                    if let Ok(form) = Form::new(&old, translation.as_deref()) {
                        let _ = term.add_form(form);
                    }
                    terms.save(&term)?;
                    owners.add_term(&finding.name, id);
                    added.push(format!("{old} → {}", finding.name));
                } else {
                    let Ok(form) = Form::new(&finding.name, None) else { continue };
                    if term.add_form(form).is_err() {
                        continue;
                    }
                    terms.save(&term)?;
                    owners.add_term(&finding.name, id);
                    added.push(format!("{} → {}", finding.name, term.name()));
                }
            }
            Some(Owner::Character(id)) => {
                let mut character = characters.get(id)?;
                let Ok(form) = Form::new(&finding.name, None) else { continue };
                if character.add_form(form).is_err() {
                    continue;
                }
                characters.save(&character)?;
                owners.add(&finding.name, Owner::Character(id));
                added.push(format!("{} → {}", finding.name, character.full_name().unwrap_or_default()));
            }
            None => match finding.kind {
                FoundKind::Character => {
                    let mut character = Character::new(&finding.name);
                    character.set_gender(finding.gender);
                    let id = characters.save(&character)?;
                    owners.add(&finding.name, Owner::Character(id));
                    added.push(finding.name.clone());
                }
                FoundKind::Term(category) => {
                    let Ok(term) = Term::new(&finding.name, category) else { continue };
                    let id = terms.save(&term)?;
                    owners.add_term(&finding.name, id);
                    added.push(finding.name.clone());
                }
            },
        }
        known.insert(key);
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
        Finding { name: name.to_string(), kind, gender: None, same_as: None }
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

        let ppf = Finding { same_as: Some("People's Police Force".to_string()), ..finding("PPF", location) };
        assert_eq!(merge(vec![finding("PPF", location), ppf.clone()]), [ppf], "ссылка берётся у любой части");
    }

    #[test]
    fn acronyms_match_initials_of_significant_words() {
        assert_eq!(acronym("P.P.F."), Some("PPF".to_string()));
        assert_eq!(acronym("Ppf"), None, "строчные буквы это не аббревиатура");
        assert_eq!(acronym("A"), None);
        assert_eq!(initials("People's Police Force"), Some("PPF".to_string()));
        assert_eq!(initials("the Order of the Dawn"), Some("OD".to_string()), "служебные слова пропущены");
        assert_eq!(initials("Rusty"), None);
        assert!(abbreviates("PPF", "People's Police Force"));
        assert!(abbreviates("ppf".to_uppercase().as_str(), "people's police force"));
        assert!(!abbreviates("PPF", "Police Force"));
        assert!(!abbreviates("People's Police Force", "PPF"), "порядок важен: короткое первым");
    }

    #[test]
    fn answer_keeps_the_name_it_belongs_to() {
        let part = "Eileen: The PPF is here.\n";
        let answer = r#"[{"kind": "organization", "name": "PPF", "same_as": "People's Police Force"}, {"kind": "character", "name": "Eileen", "alias_of": "eileen"}]"#;
        assert_eq!(
            AnalysisPrompt::accept(answer, part),
            [
                Finding { same_as: Some("People's Police Force".to_string()), ..finding("PPF", FoundKind::Term(TermCategory::Organization)) },
                finding("Eileen", FoundKind::Character),
            ],
            "ссылка на само имя отброшена"
        );
    }
}
