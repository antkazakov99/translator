//! Разбор сцены моделью перед её описанием: новые персонажи и термины по частям сцены. Находки сводятся
//! по `NameKey`, известное глоссарию, корзине терминов и подписям разобранных говорящих отсеивается, остальное сразу
//! пишется в глоссарий. Описывает новые записи потом общий путь описаний.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::repo::{CharacterRepo, LineSampleRepo, SpeakerRepo, TermRepo};
use super::entries::scene_entries;
use super::service::{give_term, merge_characters, merge_terms};
use crate::engines::ContainerKey;
use super::{
    Character, CharacterId, DiscardReason, DiscardedTerm, Form, GlossaryData, NameKey, NoteOwner, NoteRepo, Speaker, SpeakerLink, Term, TermCategory,
    TermId,
};
use crate::common::{Gender, Timestamp};
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
places, organizations, items, titles and ranks specific to this game, events and other named concepts of this game. \
An ordinary title or rank on its own (\"King\", \"Princess\", \"Sister\", \"Captain\") is a common noun, not a term. \
Skip everything under \"Known names and terms\", ordinary words, common nouns, interjections, \
and anything inside [square brackets] or {curly brackets}. \
A new spelling of a known or listed name is not skipped: an abbreviation, a short or full form, a nickname \
(\"PPF\" for \"People's Police Force\", \"Stuart\" for \"Stu\"). List it with \"same_as\". \
A rank, title or form of address before a name is not part of the name: for \"Detective-Colonel Karanovic\" \
list the title \"Detective-Colonel\" and the name \"Karanovic\" separately, and skip \"Mr.\", \"Mrs.\" and similar. \
Answer with a JSON array, or [] if there is nothing new. Each object has the fields: \
\"kind\": one of \"character\", \"location\", \"organization\", \"title\", \"item\", \"event\", \"general\"; \
\"name\": the exact spelling from the text; \
\"gender\": \"male\", \"female\" or \"unknown\", only for a character; \
\"first_name\" and \"last_name\": only for a character with a personal name, its given name and surname as written \
(\"Vera Karanovic\": \"Vera\" and \"Karanovic\"; a lone \"Karanovic\" that is a surname: only \"last_name\"); \
\"same_as\": only when this is another name of a known name or of another name in your answer, its exact spelling. \
Respond with the JSON array only.";

/// Обращения перед именем: не должность, термином не становятся.
const HONORIFICS: [&str; 10] = ["mr", "mrs", "ms", "miss", "mx", "dr", "sir", "lady", "lord", "madam"];

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
    #[serde(alias = "given_name", alias = "firstName")]
    first_name: Option<String>,
    #[serde(alias = "surname", alias = "lastName", alias = "family_name")]
    last_name: Option<String>,
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
                let parts = match kind {
                    FoundKind::Character => NameParts::check(&name, item.first_name.as_deref(), item.last_name.as_deref()),
                    FoundKind::Term(_) => None,
                };
                Some(Finding { name, kind, gender, same_as, parts })
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
            (Finding { name: finding.name.clone(), kind: finding.kind, gender: None, same_as: None, parts: None }, Vec::new())
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
    /// Имена персонажей: по ним короткая форма находит полное имя («Dimi» у «Dimitria»).
    first_names: Vec<(String, CharacterId)>,
    /// Персонажи, которые говорят или упомянуты в разобранной сцене: из нескольких похожих выбирается свой.
    present: Vec<CharacterId>,
}

/// Короткая форма имени не короче этого: «Al» у «Alice» слишком случайно.
const MIN_SHORT_NAME: usize = 3;
/// Короткая форма не длиннее этого и короче полной хотя бы на `MIN_SHORT_GAP` букв: «Eileen» и «Eileenor»
/// разные имена, а не форма одного.
const MAX_SHORT_NAME: usize = 5;
const MIN_SHORT_GAP: usize = 2;

/// Одно имя короткая форма другого без подсказки модели: «Dimi» и «Dimitria», «Stu» и «Stuart» в любом порядке.
pub fn short_and_full(one: &str, other: &str) -> bool {
    let (a, b) = (one.trim().chars().count(), other.trim().chars().count());
    let (short, long) = (a.min(b), a.max(b));
    (MIN_SHORT_NAME..=MAX_SHORT_NAME).contains(&short) && long >= short + MIN_SHORT_GAP && (is_fuller(one, other) || is_fuller(other, one))
}

impl Owners {
    /// Персонаж, чьё имя это короткая или полная форма `name`: единственный такой, иначе единственный такой
    /// среди персонажей сцены. `exclude`: сам персонаж находки.
    fn short_form_owner(&self, name: &str, exclude: Option<CharacterId>) -> Option<CharacterId> {
        let mut candidates: Vec<CharacterId> =
            self.first_names.iter().filter(|(first, id)| Some(*id) != exclude && short_and_full(name, first)).map(|(_, id)| *id).collect();
        candidates.dedup();
        match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            several => match several.iter().filter(|id| self.present.contains(id)).collect::<Vec<_>>().as_slice() {
                [only] => Some(**only),
                _ => None,
            },
        }
    }

    /// `speakers`: говорящие снимка. Подпись говорящего с одним персонажем ведёт к этому персонажу.
    fn load(glossary: &GlossaryData, speakers: &[Speaker]) -> Self {
        let mut owners = Self { by_key: HashMap::new(), term_names: Vec::new(), first_names: Vec::new(), present: Vec::new() };
        for character in &glossary.characters {
            if let (Some(id), Some(first)) = (character.id(), character.first_name()) {
                owners.first_names.push((first.to_string(), id));
            }
        }
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
        for speaker in speakers {
            let Some(label) = &speaker.label else { continue };
            let linked: Vec<CharacterId> = glossary.links.iter().filter(|link| link.speaker == speaker.key).map(|link| link.character).collect();
            if let [only] = linked.as_slice() {
                owners.add(label, Owner::Character(*only));
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
        let abbreviated = self
            .term_names
            .iter()
            .filter(|(_, id)| Some(Owner::Term(*id)) != own)
            .find(|(name, _)| abbreviates(&finding.name, name) || abbreviates(name, &finding.name))
            .map(|(_, id)| Owner::Term(*id));
        if abbreviated.is_some() || finding.kind != FoundKind::Character || finding.name.contains(char::is_whitespace) {
            return abbreviated;
        }
        let exclude = match own {
            Some(Owner::Character(id)) => Some(id),
            _ => None,
        };
        self.short_form_owner(&finding.name, exclude).map(Owner::Character)
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
            // Главным остаётся персонаж с полным именем: «Stu» уходит формой в «Stuart».
            let first = |id| -> Result<String, DbError> { Ok(characters.get(id)?.first_name().unwrap_or_default().to_string()) };
            let (from, into) = if is_fuller(&first(a)?, &first(b)?) { (b, a) } else { (a, b) };
            let name = character_name(from)?;
            merge_characters(tx, from, into)?;
            format!("{name} → {} (объединено)", character_name(into)?)
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

/// `long` это полная форма имени `short`: одно слово, длиннее и начинается с него («Stuart» у «Stu»).
pub fn is_fuller(long: &str, short: &str) -> bool {
    let (long, short) = (long.trim().to_lowercase(), short.trim().to_lowercase());
    !short.is_empty() && !long.contains(char::is_whitespace) && !short.contains(char::is_whitespace) && long.len() > short.len() && long.starts_with(&short)
}

/// Должности и звания перед именем: по ним «Detective-Colonel Karanovic» делится на должность и имя.
const RANKS: [&str; 34] = [
    "detective", "colonel", "general", "major", "captain", "lieutenant", "sergeant", "corporal", "private", "officer", "agent", "inspector",
    "commander", "admiral", "chief", "director", "president", "doctor", "professor", "nurse", "coach", "principal", "judge", "senator",
    "king", "queen", "prince", "princess", "duke", "count", "father", "sister", "brother", "deputy",
];

/// Должность или обращение перед именем: «Detective-Colonel» и «Karanovic» у «Detective-Colonel Karanovic»,
/// «Mr.» и «Lee» у «Mr. Lee». `None`: впереди нет должности или после неё не осталось имени.
pub fn title_and_name(name: &str) -> Option<(String, String)> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let titled = |word: &str| {
        word.split('-').all(|piece| {
            let piece = piece.trim_matches('.').to_lowercase();
            !piece.is_empty() && (RANKS.contains(&piece.as_str()) || HONORIFICS.contains(&piece.as_str()))
        })
    };
    let count = words.iter().take_while(|word| titled(word)).count();
    if count == 0 || count == words.len() {
        return None;
    }
    Some((words[..count].join(" "), words[count..].join(" ")))
}

/// Подпись из одного токена: «[mc]», «[player_name]». «[mc]'s mom» это роль, а не токен.
pub fn is_token_label(label: &str) -> bool {
    let label = label.trim();
    label.len() > 2 && label.starts_with('[') && label.ends_with(']') && !label[1..label.len() - 1].contains(['[', ']'])
}

/// Одно обычное звание или обращение («King», «Princess», «Sister», «Mr.»): это обычное слово, а не термин игры.
/// Его перевод очевиден, а одно написание на всю игру вредит: «Sister» бывает монахиней и сестрой. Составное
/// звание («Detective-Colonel», «Grand Duke») термином остаётся.
pub fn is_common_title(name: &str) -> bool {
    let word = name.trim().trim_matches('.').to_lowercase();
    !word.is_empty()
        && !word.contains(|c: char| c.is_whitespace() || c == '-')
        && (RANKS.contains(&word.as_str()) || HONORIFICS.contains(&word.as_str()))
}

/// Запись находок одного разбора: что уже известно, чьи это написания, что добавлено.
struct Saver<'c> {
    tx: &'c rusqlite::Connection,
    known: HashSet<NameKey>,
    /// Говорящие снимка: по подписям связанных находится персонаж.
    speakers: Vec<Speaker>,
    /// Подписи говорящих, которых пользователь ещё не разобрал: термином такая находка не становится.
    new_labels: HashSet<NameKey>,
    owners: Owners,
    added: Vec<String>,
}

impl Saver<'_> {
    fn is_known(&self, key: &NameKey) -> Result<bool, DbError> {
        Ok(self.known.contains(key) || TermRepo { conn: self.tx }.is_known(key)?)
    }

    fn reload(&mut self) -> Result<(), DbError> {
        let present = std::mem::take(&mut self.owners.present);
        self.owners = Owners::load(&GlossaryData::load(self.tx)?, &self.speakers);
        self.owners.present = present;
        Ok(())
    }

    fn save(&mut self, finding: &Finding) -> Result<(), DbError> {
        let Some(key) = NameKey::new(&finding.name) else { return Ok(()) };
        // Личное имя с частями от модели должностью не считается.
        let personal = finding.parts.as_ref().is_some_and(|parts| parts.first.is_some());
        if let (false, Some((title, name))) = (personal, title_and_name(&finding.name))
            && self.names_a_person(finding, &name)
        {
            return self.save_titled(finding, &title, &name);
        }
        // Подпись говорящего («Waitress») переводится шагом подписей, а не термином, обычное звание тоже не термин.
        if matches!(finding.kind, FoundKind::Term(_)) && (self.new_labels.contains(&key) || is_common_title(&finding.name)) {
            return Ok(());
        }
        if self.is_known(&key)? {
            // Известная запись оказалась другим именем другой записи: связь нашлась позже, записи сливаются.
            if let Some(own) = self.owners.by_key.get(&key).copied()
                && let Some(other) = self.owners.owner_of(finding, Some(own))
                && let Some(text) = merge_entries(self.tx, own, other)?
            {
                self.added.push(text);
                self.reload()?;
            }
            return Ok(());
        }
        match self.owners.owner_of(finding, None) {
            Some(Owner::Term(id)) => self.term_form(finding, id)?,
            Some(Owner::Character(id)) => self.character_form(finding, id)?,
            None => self.create(finding)?,
        }
        self.known.insert(key);
        Ok(())
    }

    /// Должность с именем это человек, а не название: находка-персонаж, находка-должность или остаток уже известен
    /// как персонаж. «General Store» у места делиться не должен.
    fn names_a_person(&self, finding: &Finding, name: &str) -> bool {
        let known_person = NameKey::new(name).and_then(|key| self.owners.by_key.get(&key)).is_some_and(|owner| matches!(owner, Owner::Character(_)));
        matches!(finding.kind, FoundKind::Character | FoundKind::Term(TermCategory::Title)) || known_person
    }

    /// Должность пишется термином-должностью (обращения нет), имя после неё как отдельная находка. Термин с тем же
    /// написанием, записанный раньше («Princess Selena»), удаляется: его название уходит в корзину.
    fn save_titled(&mut self, finding: &Finding, title: &str, name: &str) -> Result<(), DbError> {
        if let Some(Owner::Term(id)) = NameKey::new(&finding.name).and_then(|key| self.owners.by_key.get(&key)).copied() {
            let old = self.discard_term(id)?;
            let parts = if is_common_title(title) { name.to_string() } else { format!("{title} + {name}") };
            self.added.push(format!("{old} → {parts} (разделено)"));
            self.reload()?;
        }
        if !is_common_title(title)
            && let Some(key) = NameKey::new(title)
            && !self.is_known(&key)?
            && let Ok(term) = Term::new(title, TermCategory::Title)
        {
            let id = TermRepo { conn: self.tx }.save(&term)?;
            self.owners.add_term(title, id);
            self.known.insert(key);
            self.added.push(title.to_string());
        }
        let parts = finding.parts.as_ref().and_then(|parts| NameParts::check(name, parts.first.as_deref(), parts.last.as_deref()));
        let rest = Finding { name: name.to_string(), kind: FoundKind::Character, gender: finding.gender, same_as: None, parts };
        if title_and_name(name).is_some() { Ok(()) } else { self.save(&rest) }
    }

    /// Термин удаляется вместе с заметками, его название уходит в корзину как мусор: разбор его больше не предложит.
    fn discard_term(&mut self, id: TermId) -> Result<String, DbError> {
        let terms = TermRepo { conn: self.tx };
        let term = terms.get(id)?;
        terms.delete(id)?;
        NoteRepo { conn: self.tx }.delete_owner(&NoteOwner::Term(id))?;
        terms.discard(&DiscardedTerm {
            text: term.name().to_string(),
            key: term.name_key().clone(),
            reason: DiscardReason::Junk,
            discarded_at: Timestamp::now(),
        })?;
        Ok(term.name().to_string())
    }

    /// Обычные звания, записанные терминами раньше («Princess», «Sister»), уходят в корзину. Термин, которому
    /// дали перевод, остаётся: его оставили намеренно.
    fn drop_common_titles(&mut self) -> Result<(), DbError> {
        let terms = TermRepo { conn: self.tx }.list()?;
        let mut dropped = false;
        for term in terms.iter().filter(|term| term.translation().is_none() && is_common_title(term.name())) {
            let Some(id) = term.id() else { continue };
            let name = self.discard_term(id)?;
            self.added.push(format!("{name} (обычное слово, убрано)"));
            dropped = true;
        }
        if dropped {
            self.reload()?;
        }
        Ok(())
    }

    fn term_form(&mut self, finding: &Finding, id: TermId) -> Result<(), DbError> {
        let terms = TermRepo { conn: self.tx };
        let mut term = terms.get(id)?;
        if acronym(term.name()).is_some() && acronym(&finding.name).is_none() {
            // Полное название к термину-аббревиатуре: оно становится названием, аббревиатура формой со своим
            // переводом, перевод полного названия даст шаг переводов глоссария.
            let (old, translation) = (term.name().to_string(), term.translation().map(str::to_string));
            if term.rename(&finding.name).is_err() {
                return Ok(());
            }
            term.set_translation("");
            if let Ok(form) = Form::new(&old, translation.as_deref()) {
                let _ = term.add_form(form);
            }
            terms.save(&term)?;
            self.added.push(format!("{old} → {}", finding.name));
        } else {
            let Ok(form) = Form::new(&finding.name, None) else { return Ok(()) };
            if term.add_form(form).is_err() {
                return Ok(());
            }
            terms.save(&term)?;
            self.added.push(format!("{} → {}", finding.name, term.name()));
        }
        self.owners.add_term(&finding.name, id);
        Ok(())
    }

    fn character_form(&mut self, finding: &Finding, id: CharacterId) -> Result<(), DbError> {
        let characters = CharacterRepo { conn: self.tx };
        let mut character = characters.get(id)?;
        let first = character.first_name().unwrap_or_default().to_string();
        if is_fuller(&finding.name, &first) {
            // Полное имя к уменьшительному: оно становится именем, уменьшительное формой со своим переводом.
            let translated = character.translated_first_name().map(str::to_string);
            let last = character.translated_last_name().unwrap_or_default().to_string();
            character.set_first_name(&finding.name);
            character.set_translated_names("", &last);
            if let Ok(form) = Form::new(&first, translated.as_deref()) {
                let _ = character.add_form(form);
            }
            characters.save(&character)?;
            self.added.push(format!("{first} → {}", finding.name));
        } else {
            let Ok(form) = Form::new(&finding.name, None) else { return Ok(()) };
            if character.add_form(form).is_err() {
                return Ok(());
            }
            characters.save(&character)?;
            self.added.push(format!("{} → {}", finding.name, character.full_name().unwrap_or_default()));
        }
        self.owners.add(&finding.name, Owner::Character(id));
        Ok(())
    }

    /// Новая запись: персонаж с именем и фамилией по частям от модели и родом, термин с категорией.
    fn create(&mut self, finding: &Finding) -> Result<(), DbError> {
        match finding.kind {
            FoundKind::Character => {
                let mut character = match &finding.parts {
                    Some(parts) => {
                        let mut character = Character::new(parts.first.as_deref().unwrap_or_default());
                        if let Some(last) = &parts.last {
                            character.set_last_name(last);
                        }
                        character
                    }
                    None => Character::new(&finding.name),
                };
                character.set_gender(finding.gender);
                let id = CharacterRepo { conn: self.tx }.save(&character)?;
                for name in character.full_name().into_iter().chain(character.first_name().map(str::to_string)).chain(character.last_name().map(str::to_string)) {
                    self.owners.add(&name, Owner::Character(id));
                }
            }
            FoundKind::Term(category) => {
                let Ok(term) = Term::new(&finding.name, category) else { return Ok(()) };
                let id = TermRepo { conn: self.tx }.save(&term)?;
                self.owners.add_term(&finding.name, id);
            }
        }
        self.added.push(finding.name.clone());
        Ok(())
    }

    /// Термины вида «должность + имя известного персонажа», записанные раньше («Princess Selena», когда Selena уже
    /// персонаж), делятся на должность и персонажа.
    fn split_titled_terms(&mut self) -> Result<(), DbError> {
        let terms = TermRepo { conn: self.tx }.list()?;
        for term in terms.iter().filter(|term| term.category() != TermCategory::Title) {
            let Some((title, name)) = title_and_name(term.name()) else { continue };
            let person = Finding { name: term.name().to_string(), kind: FoundKind::Term(term.category()), gender: None, same_as: None, parts: None };
            if self.names_a_person(&person, &name) {
                self.save_titled(&person, &title, &name)?;
            }
        }
        Ok(())
    }

    /// Говорящие, которых пользователь ещё не разобрал, привязываются к персонажу по подписи: подпись это имя,
    /// фамилия, полное имя или форма персонажа, то же после должности («Detective-Colonel Karanovic») или короткая
    /// форма единственного имени («Dimi» у «Dimitria», при нескольких решает сцена). Связь становится основной, если
    /// у персонажа основной ещё нет. Говорящий с подписью из одного токена («[mc]», «[player_name]»), обычно герой,
    /// модели не виден: если у него есть реплики, он становится персонажем с этой подписью.
    fn link_speakers(&mut self) -> Result<(), DbError> {
        let repo = SpeakerRepo { conn: self.tx };
        let characters = CharacterRepo { conn: self.tx };
        let speaking: HashSet<_> = repo.current()?.into_iter().filter(|used| used.lines > 0).map(|used| used.speaker.key).collect();
        for speaker in repo.new_speakers()? {
            let Some(label) = speaker.label.as_deref() else { continue };
            let owner = |name: &str| NameKey::new(name).and_then(|key| self.owners.by_key.get(&key).copied());
            let found = owner(label)
                .or_else(|| title_and_name(label).and_then(|(_, name)| owner(&name)))
                .or_else(|| (!label.contains(char::is_whitespace)).then(|| self.owners.short_form_owner(label, None).map(Owner::Character)).flatten());
            let id = match found {
                Some(Owner::Character(id)) => id,
                Some(Owner::Term(_)) => continue,
                None if is_token_label(label) && speaking.contains(&speaker.key) => {
                    let id = characters.save(&Character::new(label))?;
                    repo.link(&SpeakerLink { speaker: speaker.key.clone(), character: id, is_primary: true })?;
                    self.owners.add(label, Owner::Character(id));
                    self.added.push(format!("{label} (говорящий, новый персонаж)"));
                    continue;
                }
                None => continue,
            };
            let primary = !repo.links_of(id)?.iter().any(|link| link.is_primary);
            repo.link(&SpeakerLink { speaker: speaker.key.clone(), character: id, is_primary: primary })?;
            self.added.push(format!("{label} (говорящий) → {}", characters.get(id)?.full_name().unwrap_or_default()));
        }
        Ok(())
    }

    /// Персонаж из одного слова сливается с единственным персонажем, у которого это слово фамилия или имя
    /// при фамилии («Karanovic» в «Vera Karanovic»), иначе с единственным, чьё имя это его полная или короткая
    /// форма («Dimi» и «Dimitria», при нескольких таких решает сцена). Главным остаётся полное имя.
    fn merge_single_names(&mut self) -> Result<(), DbError> {
        let characters = CharacterRepo { conn: self.tx }.list()?;
        let mut gone: HashSet<CharacterId> = HashSet::new();
        for single in &characters {
            let (Some(id), Some(first), None) = (single.id(), single.first_name(), single.last_name()) else { continue };
            if gone.contains(&id) || first.contains(char::is_whitespace) {
                continue;
            }
            let Some(key) = NameKey::new(first) else { continue };
            let alive = |other: &&Character| other.id().is_some_and(|other| other != id && !gone.contains(&other));
            let full: Vec<CharacterId> = characters
                .iter()
                .filter(alive)
                .filter(|other| other.last_name().is_some())
                .filter(|other| {
                    let same = |name: Option<&str>| name.and_then(NameKey::new).as_ref() == Some(&key);
                    same(other.last_name()) || same(other.first_name())
                })
                .filter_map(Character::id)
                .collect();
            let into = match full.as_slice() {
                [into] => Some(*into),
                [] => self.owners.short_form_owner(first, Some(id)).filter(|other| !gone.contains(other)),
                _ => None,
            };
            let Some(into) = into else { continue };
            if let Some(text) = merge_entries(self.tx, Owner::Character(id), Owner::Character(into))? {
                // Удалённым мог оказаться и второй: главным остаётся полное имя.
                let removed = if (CharacterRepo { conn: self.tx }).get(id).is_ok() { into } else { id };
                gone.insert(removed);
                self.added.push(text);
                self.reload()?;
            }
        }
        Ok(())
    }
}

/// Новые записи разбора в глоссарий: персонаж с именем, фамилией и родом, термин с категорией. Другое имя
/// известной записи или другой находки становится формой той записи, должность перед именем термином, персонаж
/// из одного слова сливается с единственным полным, неразобранный говорящий привязывается к персонажу по подписи.
/// Известное глоссарию, корзине терминов и подписям разобранных говорящих
/// пропускается. Возвращает написания добавленных, форму как «PPF → People's Police Force».
pub fn save_found(tx: &rusqlite::Connection, found: &[Finding]) -> Result<Vec<String>, DbError> {
    save_scene_found(tx, found, None)
}

/// `save_found` для находок сцены `scene`: из похожих персонажей выбирается тот, кто в ней говорит или упомянут.
pub fn save_scene_found(tx: &rusqlite::Connection, found: &[Finding], scene: Option<&ContainerKey>) -> Result<Vec<String>, DbError> {
    let known: HashSet<NameKey> = known_for_prompt(tx)?.iter().filter_map(|name| NameKey::new(name)).collect();
    let glossary = GlossaryData::load(tx)?;
    let speakers: Vec<Speaker> = SpeakerRepo { conn: tx }.current()?.into_iter().map(|used| used.speaker).collect();
    let new_labels = SpeakerRepo { conn: tx }.new_speakers()?.iter().filter_map(Speaker::label_key).collect();
    let mut owners = Owners::load(&glossary, &speakers);
    if let Some(key) = scene {
        owners.present = scene_entries(&glossary, &LineSampleRepo { conn: tx }.scene_lines(key)?).0;
    }
    let mut saver = Saver { tx, known, speakers, new_labels, owners, added: Vec::new() };
    for finding in ordered(found) {
        saver.save(finding)?;
    }
    saver.split_titled_terms()?;
    saver.drop_common_titles()?;
    saver.merge_single_names()?;
    saver.link_speakers()?;
    Ok(saver.added)
}

/// Имена, фамилии, полные имена и формы персонажей, термины и их формы и подписи разобранных говорящих текущего
/// снимка. Подписи говорящих, которых пользователь ещё не разобрал, известными не считаются: модель называет их
/// персонажами, и говорящий привязывается к найденному персонажу.
pub fn known_for_prompt(tx: &rusqlite::Connection) -> Result<Vec<String>, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let speakers = SpeakerRepo { conn: tx };
    let new: HashSet<_> = speakers.new_speakers()?.into_iter().map(|speaker| speaker.key).collect();
    let labels = speakers.current()?.into_iter().filter(|used| !new.contains(&used.speaker.key)).filter_map(|used| used.speaker.label);
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
        Finding { name: name.to_string(), kind, gender: None, same_as: None, parts: None }
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
    fn titles_split_off_the_name() {
        assert_eq!(title_and_name("Detective-Colonel Karanovic"), Some(("Detective-Colonel".to_string(), "Karanovic".to_string())));
        assert_eq!(title_and_name("Mr. Lee"), Some(("Mr.".to_string(), "Lee".to_string())));
        assert_eq!(title_and_name("Captain Jack Morgan"), Some(("Captain".to_string(), "Jack Morgan".to_string())));
        assert_eq!(title_and_name("Vera Karanovic"), None);
        assert_eq!(title_and_name("Captain"), None, "без имени после должности делить нечего");
        assert!(is_common_title("Mr.") && is_common_title("Princess") && is_common_title(" sister "));
        assert!(is_token_label("[mc]") && is_token_label(" [player_name] "));
        assert!(!is_token_label("[mc]'s mom") && !is_token_label("[]") && !is_token_label("[a] [b]") && !is_token_label("Eileen"));
        assert!(!is_common_title("Detective-Colonel") && !is_common_title("Grand Duke") && !is_common_title("Selena"));
    }

    #[test]
    fn full_given_name_is_fuller_than_its_short_form() {
        assert!(is_fuller("Stuart", "Stu"));
        assert!(is_fuller("alexander", "Alex"));
        assert!(!is_fuller("Stu", "Stuart"));
        assert!(!is_fuller("Stuart Lee", "Stu"), "полное имя из двух слов не уменьшительное");
        assert!(!is_fuller("Stu", "Stu"));
        assert!(short_and_full("Dimi", "Dimitria") && short_and_full("Stuart", "Stu"));
        assert!(!short_and_full("Eileen", "Eileenor"), "длинное имя не короткая форма");
        assert!(!short_and_full("Al", "Alice"), "слишком коротко");
        assert!(!short_and_full("Anya", "Anastasia"));
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
