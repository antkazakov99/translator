//! Запись находок разбора сцены в глоссарий. Находка это утверждение «написание обозначает сущность»,
//! и решение о ней принимается в одном месте, по порядку:
//!
//! 1. Разложение на входе: титул как имя лица, должность перед именем, обычное звание, обрывок, обычное слово.
//! 2. Написание известно: пропуск. С `other_than` модели создаётся тёзка, с `same_as` на другую запись пара.
//! 3. То же слово в другом числе или с точками в аббревиатуре: форма.
//! 4. `same_as` модели о новом написании: привязка к записи, роль написания решает `character_link`.
//!    Другое написание имени или фамилии ложится формой своего вида, главное из написаний выбирает частота
//!    в игре (`service::settle_names`). Занятое поле привязку не даёт: запись создаётся, пара идёт вопросом.
//! 5. Иначе новая запись.
//!
//! Скрипт записи сам не сливает. Пара от модели сливается сразу, только если отношение видит и скрипт
//! (`relations.rs`), остальные пары задаются модели вопросом (`same_entry.rs`). Термины с одинаковым переводом
//! тоже только пара для вопроса: его задаёт шаг переводов глоссария.

use std::collections::{HashMap, HashSet};

use super::analyze::{CastMember, Finding, FoundKind, NameParts, parse_cast_ref};
use super::entries::{character_names, scene_entries};
use super::relations::{self, Pair};
use super::repo::{CharacterRepo, LineSampleRepo, MentionCheckRepo, PairCheckRepo, SpeakerRepo, TermRepo};
use super::service::{give_term, merge_characters, merge_terms, settle_names};
use super::spelling::{
    abbreviates, acronym, is_common_title, is_common_word, is_fragment, is_person_title, is_token_label, name_variant, plural_of, same_acronym,
    title_and_name, without_article,
};
use super::{Character, CharacterId, ExcludeOwner, Form, FormKind, GlossaryData, NameKey, NoteOwner, NoteRepo, Speaker, SpeakerLink, Term, TermCategory, TermId};
use crate::engines::ContainerKey;
use crate::storage::DbError;
use crate::translation::text::mentions;

/// Слово модели о паре записей: одно это (`same_as`, занятое поле) или разное (`other_than`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Claim {
    pub pair: Pair,
    pub same: bool,
}

/// Написания записей глоссария и их владельцы: термины по названию и формам, персонажи по полному имени,
/// имени, фамилии и формам, подпись говорящего с одним персонажем ведёт к нему. У написания бывает несколько
/// владельцев: фамилия семьи, город и человек с одним именем. Растёт по мере записи находок.
struct Names {
    by_key: HashMap<NameKey, Vec<ExcludeOwner>>,
    /// Названия и формы терминов: по ним ищется то же слово в другом числе.
    term_names: Vec<(String, TermId)>,
    /// Персонажи, которые говорят или упомянуты в разобранной сцене: из нескольких владельцев выбирается свой.
    present: Vec<CharacterId>,
}

impl Names {
    fn load(glossary: &GlossaryData, speakers: &[Speaker]) -> Self {
        let mut names = Self { by_key: HashMap::new(), term_names: Vec::new(), present: Vec::new() };
        for term in &glossary.terms {
            let Some(id) = term.id() else { continue };
            names.add_term(term.name(), id);
            for form in term.forms() {
                names.add_term(form.text(), id);
            }
        }
        for character in &glossary.characters {
            let Some(id) = character.id() else { continue };
            names.add_person(character, id);
        }
        for speaker in speakers {
            let Some(label) = &speaker.label else { continue };
            let linked: Vec<CharacterId> = glossary.links.iter().filter(|link| link.speaker == speaker.key).map(|link| link.character).collect();
            if let [only] = linked.as_slice() {
                names.add(label, ExcludeOwner::Character(*only));
            }
        }
        names
    }

    fn add(&mut self, name: &str, owner: ExcludeOwner) {
        for name in [name, without_article(name)] {
            if let Some(key) = NameKey::new(name) {
                let owners = self.by_key.entry(key).or_default();
                if !owners.contains(&owner) {
                    owners.push(owner);
                }
            }
        }
    }

    fn add_term(&mut self, name: &str, id: TermId) {
        self.add(name, ExcludeOwner::Term(id));
        self.term_names.push((name.to_string(), id));
    }

    fn add_person(&mut self, character: &Character, id: CharacterId) {
        let names = character
            .full_name()
            .into_iter()
            .chain(character.first_name().map(str::to_string))
            .chain(character.last_name().map(str::to_string))
            .chain(character.forms().iter().map(|form| form.text().to_string()));
        for name in names {
            self.add(&name, ExcludeOwner::Character(id));
        }
    }

    /// Владельцы написания, с артиклем или без: «The Fairy King» находит «Fairy King».
    fn owners(&self, name: &str) -> Vec<ExcludeOwner> {
        let mut owners: Vec<ExcludeOwner> = Vec::new();
        for name in [name, without_article(name)] {
            for owner in NameKey::new(name).and_then(|key| self.by_key.get(&key)).into_iter().flatten() {
                if !owners.contains(owner) {
                    owners.push(*owner);
                }
            }
        }
        owners
    }

    /// Запись, которую назвала модель: номер «C3» ведёт к персонажу сцены, написание к единственному владельцу,
    /// а при нескольких к единственному персонажу сцены среди них. `None`: записи нет или выбрать нельзя.
    fn one(&self, name: &str) -> Option<ExcludeOwner> {
        if let Some(id) = parse_cast_ref(name).filter(|_| name.trim().starts_with(['C', 'c'])) {
            return self.present.contains(&id).then_some(ExcludeOwner::Character(id));
        }
        let owners = self.owners(name);
        match owners.as_slice() {
            [] => None,
            [only] => Some(*only),
            several => {
                let here: Vec<&ExcludeOwner> =
                    several.iter().filter(|owner| matches!(owner, ExcludeOwner::Character(id) if self.present.contains(id))).collect();
                match here.as_slice() {
                    [only] => Some(**only),
                    _ => None,
                }
            }
        }
    }

    /// Термин, чьё название это то же слово: другое число («pixies» и «pixie») или аббревиатура с точками
    /// («P.P.F.» и «PPF»). Это вариант написания, а не догадка о связи.
    fn same_word(&self, finding: &Finding) -> Option<TermId> {
        let term = matches!(finding.kind, FoundKind::Term(_));
        self.term_names
            .iter()
            .find(|(name, _)| same_acronym(&finding.name, name) || (term && (plural_of(&finding.name, name) || plural_of(name, &finding.name))))
            .map(|(_, id)| *id)
    }
}

/// Сначала находки без ссылки на другую находку: тогда запись, к которой ложится форма, уже есть.
fn ordered(found: &[Finding]) -> Vec<&Finding> {
    let keys: HashSet<NameKey> = found.iter().filter_map(|finding| NameKey::new(&finding.name)).collect();
    let linked = |finding: &Finding| finding.same_as.as_deref().and_then(NameKey::new).is_some_and(|key| keys.contains(&key));
    found.iter().filter(|finding| !linked(finding)).chain(found.iter().filter(|finding| linked(finding))).collect()
}

/// Запись находок одного разбора: что уже известно, чьи это написания, что добавлено.
struct Saver<'c> {
    tx: &'c rusqlite::Connection,
    /// Написания без записи, которые разбор пропускает: подписи разобранных говорящих и уже записанное.
    known: HashSet<NameKey>,
    /// Говорящие снимка: по подписям связанных находится персонаж.
    speakers: Vec<Speaker>,
    /// Подписи говорящих, которых пользователь ещё не разобрал: термином такая находка не становится.
    new_labels: HashSet<NameKey>,
    names: Names,
    added: Vec<String>,
    claims: Vec<Claim>,
}

impl Saver<'_> {
    /// Написание известно без записи: корзина терминов или подпись разобранного говорящего.
    fn is_known(&self, key: &NameKey) -> Result<bool, DbError> {
        Ok(self.known.contains(key) || TermRepo { conn: self.tx }.is_known(key)?)
    }

    fn reload(&mut self) -> Result<(), DbError> {
        let present = std::mem::take(&mut self.names.present);
        self.names = Names::load(&GlossaryData::load(self.tx)?, &self.speakers);
        self.names.present = present;
        Ok(())
    }

    fn claim(&mut self, a: ExcludeOwner, b: ExcludeOwner, same: bool) {
        if let Some(pair) = Pair::new(a, b)
            && !self.claims.iter().any(|claim| claim.pair == pair)
        {
            self.claims.push(Claim { pair, same });
        }
    }

    fn name_of(&self, owner: ExcludeOwner) -> Result<String, DbError> {
        Ok(match owner {
            ExcludeOwner::Character(id) => CharacterRepo { conn: self.tx }.get(id)?.full_name().unwrap_or_default(),
            ExcludeOwner::Term(id) => TermRepo { conn: self.tx }.get(id)?.name().to_string(),
        })
    }

    /// Титул как имя лица («Fairy Queen») это персонаж, а не термин, имя персонажа пишется без артикля.
    fn normalized(finding: &Finding) -> Finding {
        let mut finding = finding.clone();
        if matches!(finding.kind, FoundKind::Term(TermCategory::General | TermCategory::Title)) && is_person_title(&finding.name) {
            finding.kind = FoundKind::Character;
        }
        if finding.kind == FoundKind::Character {
            finding.name = without_article(&finding.name).to_string();
        }
        finding
    }

    /// Все реплики игры, где есть name: по ним видно, имя это или обычное слово.
    fn is_common_word(&self, name: &str) -> Result<bool, DbError> {
        if name.contains(char::is_whitespace) {
            return Ok(false);
        }
        let lines: Vec<String> = LineSampleRepo { conn: self.tx }.containing(name, None)?.into_iter().map(|line| line.text).collect();
        Ok(is_common_word(name, &lines))
    }

    fn save(&mut self, finding: &Finding) -> Result<(), DbError> {
        let finding = &Self::normalized(finding);
        let Some(key) = NameKey::new(&finding.name) else { return Ok(()) };
        // Должность перед именем делится. Личное имя с частями от модели должностью не считается.
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
        let owners = self.names.owners(&finding.name);
        if !owners.is_empty() || self.is_known(&key)? {
            return self.known_spelling(finding, &owners);
        }
        if let Some(term) = self.names.same_word(finding) {
            self.term_form(finding, term)?;
            self.known.insert(key);
            return Ok(());
        }
        if let Some(owner) = finding.same_as.as_deref().and_then(|name| self.names.one(name)) {
            match owner {
                ExcludeOwner::Term(id) => self.term_form(finding, id)?,
                ExcludeOwner::Character(id) => {
                    if !self.character_link(finding, id)? {
                        // Поле занято другим значением: связь решит вопрос, а пока это отдельная запись.
                        if let Some(new) = self.create(finding)? {
                            self.added.push(finding.name.clone());
                            self.claim(new, owner, true);
                        }
                    }
                }
            }
            self.known.insert(key);
            return Ok(());
        }
        // Обрывок и обычное слово персонажем не становятся. Подпись говорящего не проверяется: он говорит.
        if finding.kind == FoundKind::Character && (is_fragment(&finding.name) || (!self.new_labels.contains(&key) && self.is_common_word(&finding.name)?)) {
            return Ok(());
        }
        if self.create(finding)?.is_some() {
            self.added.push(finding.name.clone());
        }
        self.known.insert(key);
        Ok(())
    }

    /// Написание уже известно. Без слова модели это та же запись, и находка пропускается. `other_than`: тем же
    /// написанием названа другая сущность, создаётся вторая запись. `same_as` на другую запись: две известные
    /// записи могут быть одним, пара идёт дальше.
    fn known_spelling(&mut self, finding: &Finding, owners: &[ExcludeOwner]) -> Result<(), DbError> {
        if let Some(other) = finding.other_than.as_deref() {
            let target = self.names.one(other).filter(|target| owners.contains(target)).or(match owners {
                [only] => Some(*only),
                _ => None,
            });
            return match target {
                Some(target) => self.namesake(finding, owners, target),
                None => Ok(()),
            };
        }
        // Написание бывает и у той записи, на которую указала модель («Karanovic» у «Vera Karanovic»): парой
        // ей становится единственный другой владелец.
        if let Some(other) = finding.same_as.as_deref().and_then(|name| self.names.one(name))
            && let [own] = owners.iter().filter(|owner| **owner != other).collect::<Vec<_>>().as_slice()
        {
            self.claim(**own, other, true);
        }
        Ok(())
    }

    /// Вторая запись с известным написанием: другой человек с той же фамилией, город с именем персонажа.
    /// Создаётся один раз: если у написания уже есть другая запись того же вида, это она. Название термина
    /// в глоссарии одно, поэтому термин-тёзка термина не создаётся.
    fn namesake(&mut self, finding: &Finding, owners: &[ExcludeOwner], target: ExcludeOwner) -> Result<(), DbError> {
        let same_kind = |owner: &ExcludeOwner| {
            matches!((owner, finding.kind), (ExcludeOwner::Character(_), FoundKind::Character) | (ExcludeOwner::Term(_), FoundKind::Term(_)))
        };
        let term_taken = matches!(finding.kind, FoundKind::Term(_)) && owners.iter().any(|owner| matches!(owner, ExcludeOwner::Term(_)));
        if term_taken || owners.iter().any(|owner| *owner != target && same_kind(owner)) {
            return Ok(());
        }
        let other = self.name_of(target)?;
        if let Some(new) = self.create(finding)? {
            self.added.push(format!("{} (не {other})", finding.name));
            self.claim(new, target, false);
        }
        Ok(())
    }

    /// Должность с именем это человек, а не название: находка-персонаж, находка-должность или остаток уже известен
    /// как персонаж. «General Store» у места делиться не должен.
    fn names_a_person(&self, finding: &Finding, name: &str) -> bool {
        let known_person = self.names.owners(name).iter().any(|owner| matches!(owner, ExcludeOwner::Character(_)));
        matches!(finding.kind, FoundKind::Character | FoundKind::Term(TermCategory::Title)) || known_person
    }

    /// Должность пишется термином-должностью (обращения и одного обычного звания нет), имя после неё идёт
    /// отдельной находкой тем же путём.
    fn save_titled(&mut self, finding: &Finding, title: &str, name: &str) -> Result<(), DbError> {
        if !is_common_title(title)
            && let Some(key) = NameKey::new(title)
            && self.names.owners(title).is_empty()
            && !self.is_known(&key)?
            && let Ok(term) = Term::new(title, TermCategory::Title)
        {
            let id = TermRepo { conn: self.tx }.save(&term)?;
            self.names.add_term(title, id);
            self.known.insert(key);
            self.added.push(title.to_string());
        }
        let parts = finding.parts.as_ref().and_then(|parts| NameParts::check(name, parts.first.as_deref(), parts.last.as_deref()));
        let rest = Finding {
            name: name.to_string(),
            kind: FoundKind::Character,
            gender: finding.gender,
            same_as: finding.same_as.clone(),
            other_than: finding.other_than.clone(),
            parts,
        };
        if title_and_name(name).is_some() { Ok(()) } else { self.save(&rest) }
    }

    /// Написание ложится формой термина. Полное название к термину-аббревиатуре и единственное число
    /// к множественному становятся названием, прежнее название формой со своим переводом: перевод нового
    /// названия даст шаг переводов глоссария.
    fn term_form(&mut self, finding: &Finding, id: TermId) -> Result<(), DbError> {
        let terms = TermRepo { conn: self.tx };
        let mut term = terms.get(id)?;
        let fuller = (acronym(term.name()).is_some() && acronym(&finding.name).is_none()) || plural_of(term.name(), &finding.name);
        if fuller {
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
        self.names.add_term(&finding.name, id);
        Ok(())
    }

    /// Имя и фамилия находки, когда модель их не назвала, а они выводятся из записи персонажа: полное имя
    /// с известным именем («Maja Zoric» у «Maja») или с известной фамилией у персонажа без имени, одно слово,
    /// которое приходится вариантом имени или фамилии («Alexa» у «Alexandra»), одно слово рядом с именем
    /// в репликах игры («Kralj» у «Claire»: «Claire Kralj»). `None`: вывести нечего, это другое имя человека
    /// (прозвище, титул).
    fn derived_parts(&self, finding: &Finding, character: &Character) -> Result<Option<NameParts>, DbError> {
        let same = |one: &str, other: &str| NameKey::new(one).is_some() && NameKey::new(one) == NameKey::new(other);
        let (first, last) = (character.first_name(), character.last_name());
        let parts = |first: Option<&str>, last: Option<&str>| NameParts::check(&finding.name, first, last);
        let together = |text: String| -> Result<bool, DbError> {
            let text = text.to_lowercase();
            Ok(LineSampleRepo { conn: self.tx }.containing(&text, None)?.iter().any(|line| mentions(&line.text.to_lowercase(), &text)))
        };
        let words: Vec<&str> = finding.name.split_whitespace().collect();
        Ok(match (words.as_slice(), first, last) {
            ([given, surname], Some(first), _) if same(given, first) || name_variant(given, first) => parts(Some(given), Some(surname)),
            ([given, surname], None, Some(last)) if same(surname, last) && !self.is_common_word(given)? => parts(Some(given), Some(surname)),
            ([word], Some(first), _) if name_variant(word, first) => parts(Some(word), None),
            ([word], _, Some(last)) if name_variant(word, last) => parts(None, Some(word)),
            ([word], Some(first), None) if !word.contains('[') && together(format!("{first} {word}"))? => parts(None, Some(word)),
            ([word], None, Some(last)) if !word.contains('[') && together(format!("{word} {last}"))? => parts(Some(word), None),
            _ => None,
        })
    }

    /// Чем новое написание приходится персонажу, на которого указала модель: фамилия или имя в пустое поле,
    /// полное имя к половинке, другое написание имени или фамилии формой своего вида, иначе форма (прозвище,
    /// псевдоним, титул). Какое из написаний имени главное, решает частота (`settle_names`).
    /// `false`: это имя или фамилия, а поле занято другим значением. Тогда привязки нет.
    fn character_link(&mut self, finding: &Finding, id: CharacterId) -> Result<bool, DbError> {
        let characters = CharacterRepo { conn: self.tx };
        let mut character = characters.get(id)?;
        let first = character.first_name().map(str::to_string);
        let last = character.last_name().map(str::to_string);
        let same = |one: &str, other: &str| NameKey::new(one).is_some() && NameKey::new(one) == NameKey::new(other);
        let single = |name: &str| !name.contains(char::is_whitespace);
        let parts = match &finding.parts {
            Some(parts) => Some(parts.clone()),
            None => self.derived_parts(finding, &character)?,
        };
        let (given, surname) = match &parts {
            Some(parts) => (parts.first.as_deref(), parts.last.as_deref()),
            None => (None, None),
        };
        let variant = |one: &str, other: &str| single(one) && single(other) && name_variant(one, other);
        let form = |character: &mut Character, text: &str, kind: FormKind| Form::named(text, None, kind).is_ok_and(|form| character.add_form(form).is_ok());
        let text = match (given, surname, first.as_deref(), last.as_deref()) {
            // Фамилия к персонажу с одним именем: «Vellnikov» у «Dimitria».
            (None, Some(surname), Some(_), None) => {
                character.set_last_name(surname);
                format!("{surname} → {}", character.full_name().unwrap_or_default())
            }
            // Имя к персонажу с одной фамилией.
            (Some(given), None, None, Some(_)) => {
                character.set_first_name(given);
                format!("{given} → {}", character.full_name().unwrap_or_default())
            }
            // Полное имя к персонажу с тем же именем или его вариантом: фамилия в пустое поле.
            (Some(given), Some(surname), Some(old), None) if same(old, given) || variant(old, given) => {
                if !same(old, given) {
                    form(&mut character, given, FormKind::First);
                }
                character.set_last_name(surname);
                format!("{old} → {}", finding.name)
            }
            // Полное имя к персонажу с той же фамилией: имя в пустое поле.
            (Some(given), Some(surname), None, Some(old)) if same(old, surname) => {
                character.set_first_name(given);
                format!("{old} → {}", character.full_name().unwrap_or_default())
            }
            // Другое написание имени: короткое, полное, с опечаткой.
            (Some(given), None, Some(old), _) if variant(old, given) => {
                if !form(&mut character, given, FormKind::First) {
                    return Ok(true);
                }
                format!("{given} → {}", character.full_name().unwrap_or_default())
            }
            // Другое написание фамилии.
            (None, Some(surname), _, Some(old)) if variant(old, surname) => {
                if !form(&mut character, surname, FormKind::Last) {
                    return Ok(true);
                }
                format!("{surname} → {}", character.full_name().unwrap_or_default())
            }
            // Имя или фамилия от модели, а поле занято другим.
            (Some(_), _, _, _) | (_, Some(_), _, _) => return Ok(false),
            (None, None, _, _) => {
                if !form(&mut character, &finding.name, FormKind::Other) {
                    return Ok(true);
                }
                format!("{} → {}", finding.name, character.full_name().unwrap_or_default())
            }
        };
        if character.gender().is_none() {
            character.set_gender(finding.gender);
        }
        characters.save(&character)?;
        settle_names(self.tx, id)?;
        self.added.push(text);
        self.names.add(&finding.name, ExcludeOwner::Character(id));
        self.names.add_person(&characters.get(id)?, id);
        Ok(true)
    }

    /// Новая запись: персонаж с именем и фамилией по частям от модели и родом, термин с категорией.
    /// `None`: название не годится термину.
    fn create(&mut self, finding: &Finding) -> Result<Option<ExcludeOwner>, DbError> {
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
                self.names.add_person(&character, id);
                Ok(Some(ExcludeOwner::Character(id)))
            }
            FoundKind::Term(category) => {
                let Ok(term) = Term::new(&finding.name, category) else { return Ok(None) };
                let id = TermRepo { conn: self.tx }.save(&term)?;
                self.names.add_term(&finding.name, id);
                Ok(Some(ExcludeOwner::Term(id)))
            }
        }
    }

    /// Пары от модели после записи находок. Отвеченная пара отбрасывается. Пара «одно», в которой отношение
    /// видит и скрипт, сливается сразу: два независимых довода. Остальные остаются для вопроса.
    fn settle_claims(&mut self) -> Result<Vec<Claim>, DbError> {
        let mut left = Vec::new();
        for claim in std::mem::take(&mut self.claims) {
            let Pair { a, b } = claim.pair;
            if (PairCheckRepo { conn: self.tx }).checked(a, b)? {
                continue;
            }
            let agreed = claim.same && relations::between(&GlossaryData::load(self.tx)?, a, b, true, false).is_some();
            if !agreed {
                left.push(claim);
            } else if let Some(text) = merge_pair(self.tx, claim.pair)? {
                self.added.push(text);
                self.reload()?;
            }
        }
        Ok(left)
    }

    /// Говорящие, которых пользователь ещё не разобрал, привязываются к персонажу по подписи: подпись это имя,
    /// фамилия, полное имя или форма одного персонажа (при нескольких решает сцена), то же после должности
    /// («Detective-Colonel Karanovic»). Связь становится основной, если у персонажа основной ещё нет. Говорящий
    /// с подписью из одного токена («[mc]», «[player_name]»), обычно герой, модели не виден: если у него есть
    /// реплики, он становится персонажем с этой подписью.
    fn link_speakers(&mut self) -> Result<(), DbError> {
        let repo = SpeakerRepo { conn: self.tx };
        let characters = CharacterRepo { conn: self.tx };
        let speaking: HashSet<_> = repo.current()?.into_iter().filter(|used| used.lines > 0).map(|used| used.speaker.key).collect();
        for speaker in repo.new_speakers()? {
            let Some(label) = speaker.label.as_deref() else { continue };
            let found = self.names.one(label).or_else(|| title_and_name(label).and_then(|(_, name)| self.names.one(&name)));
            let id = match found {
                Some(ExcludeOwner::Character(id)) => id,
                Some(ExcludeOwner::Term(_)) => continue,
                None if is_token_label(label) && speaking.contains(&speaker.key) && self.names.owners(label).is_empty() => {
                    let id = characters.save(&Character::new(label))?;
                    repo.link(&SpeakerLink { speaker: speaker.key.clone(), character: id, is_primary: true })?;
                    self.names.add(label, ExcludeOwner::Character(id));
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
}

fn shape(character: &Character) -> (bool, bool) {
    (character.first_name().is_some(), character.last_name().is_some())
}

/// Остаётся ли запись `one`, когда два персонажа сливаются: запись с именем и фамилией важнее половинки, иначе
/// записанная раньше. Главное имя и фамилию после слияния выбирает частота (`settle_names`), а не этот выбор.
fn keeps(one: &Character, other: &Character) -> bool {
    let full = |character: &Character| shape(character) == (true, true);
    if full(one) != full(other) {
        return full(one);
    }
    one.id().map(CharacterId::get) < other.id().map(CharacterId::get)
}

/// Персонаж `last` с одной фамилией сливается в персонажа `first` с одним именем: фамилия с переводом
/// становится фамилией `first`. Фамилия `last` бывает записана именем, тогда фамилией становится оно.
/// Возвращает строку итога: «Vellnikov → Dimitria Vellnikov (объединено)».
fn join_halves(tx: &rusqlite::Connection, first: CharacterId, last: CharacterId) -> Result<String, DbError> {
    let characters = CharacterRepo { conn: tx };
    let surname = characters.get(last)?.full_name().unwrap_or_default();
    // Слияние кладёт фамилию формой с переводом, после записи фамилии перевод формы уходит в перевод фамилии.
    merge_characters(tx, last, first)?;
    let mut person = characters.get(first)?;
    person.set_last_name(&surname);
    characters.save(&person)?;
    Ok(format!("{surname} → {} (объединено)", person.full_name().unwrap_or_default()))
}

/// Какое из двух одиночных имён стоит первым в репликах игры: «Dimitria Vellnikov» даёт имя и фамилию, даже
/// если фамилия записана именем. `None`: имена не одиночные, подряд не стоят или стоят в обоих порядках.
fn text_order(tx: &rusqlite::Connection, one: &Character, other: &Character) -> Result<Option<(CharacterId, CharacterId)>, DbError> {
    let single = |character: &Character| {
        let name = character.full_name().filter(|name| shape(character) != (true, true) && !name.contains(char::is_whitespace) && !name.contains('['))?;
        Some((character.id()?, name.to_lowercase()))
    };
    let (Some((a, first)), Some((b, second))) = (single(one), single(other)) else { return Ok(None) };
    let lines: Vec<String> = LineSampleRepo { conn: tx }.containing(&second, None)?.into_iter().map(|line| line.text.to_lowercase()).collect();
    let named = |text: &str| lines.iter().any(|line| mentions(line, text));
    Ok(match (named(&format!("{first} {second}")), named(&format!("{second} {first}"))) {
        (true, false) => Some((a, b)),
        (false, true) => Some((b, a)),
        _ => None,
    })
}

/// Два персонажа это один человек: имя и фамилия порознь соединяются («Dimitria» и «Vellnikov»: по полям,
/// а у двух одиночных имён по порядку в репликах игры), иначе одна запись сливается в другую (`keeps`), её имя
/// и формы становятся формами, главное имя и фамилию выбирает частота. `None`: одного из них уже нет.
pub fn merge_people(tx: &rusqlite::Connection, a: CharacterId, b: CharacterId) -> Result<Option<String>, DbError> {
    let characters = CharacterRepo { conn: tx };
    let (Ok(one), Ok(other)) = (characters.get(a), characters.get(b)) else { return Ok(None) };
    if a == b {
        return Ok(None);
    }
    let differ = one.full_name().as_deref().and_then(NameKey::new) != other.full_name().as_deref().and_then(NameKey::new);
    match (shape(&one), shape(&other)) {
        ((true, false), (false, true)) if differ => join_halves(tx, a, b).map(Some),
        ((false, true), (true, false)) if differ => join_halves(tx, b, a).map(Some),
        _ => {
            if differ && let Some((first, last)) = text_order(tx, &one, &other)? {
                return join_halves(tx, first, last).map(Some);
            }
            let (from, into) = if keeps(&one, &other) { (&other, &one) } else { (&one, &other) };
            let (Some(from_id), Some(into_id)) = (from.id(), into.id()) else { return Ok(None) };
            let name = from.full_name().unwrap_or_default();
            merge_characters(tx, from_id, into_id)?;
            Ok(Some(format!("{name} → {} (объединено)", characters.get(into_id)?.full_name().unwrap_or_default())))
        }
    }
}

/// Два термина это одно: главным остаётся полное название, а не аббревиатура, единственное число, иначе
/// записанный раньше. Второй ложится формой со своим переводом. `None`: одного из них уже нет.
fn merge_two_terms(tx: &rusqlite::Connection, a: TermId, b: TermId) -> Result<Option<String>, DbError> {
    let terms = TermRepo { conn: tx };
    let (Ok(one), Ok(other)) = (terms.get(a), terms.get(b)) else { return Ok(None) };
    if a == b {
        return Ok(None);
    }
    let short = |term: &Term| acronym(term.name()).is_some();
    let keep_one = if short(&one) != short(&other) {
        short(&other)
    } else if plural_of(other.name(), one.name()) {
        true
    } else if plural_of(one.name(), other.name()) {
        false
    } else {
        a.get() < b.get()
    };
    let (from, into, from_name, into_name) = if keep_one { (b, a, other.name(), one.name()) } else { (a, b, one.name(), other.name()) };
    merge_terms(tx, from, into)?;
    Ok(Some(format!("{from_name} → {into_name} (объединено)")))
}

/// Термин и персонаж это одно. Обычно термин уходит персонажу формой с переводом («Princess Selena» к «Selena»).
/// Персонаж без говорящих, чьё имя сокращает название термина («PPF» у «People's Police Force»), сам уходит
/// термину формой: модель ошиблась видом. `None`: одного из них уже нет.
fn merge_term_and_person(tx: &rusqlite::Connection, term: TermId, person: CharacterId) -> Result<Option<String>, DbError> {
    let terms = TermRepo { conn: tx };
    let characters = CharacterRepo { conn: tx };
    let (Ok(mut entry), Ok(character)) = (terms.get(term), characters.get(person)) else { return Ok(None) };
    let whole = character.full_name().unwrap_or_default();
    let names = std::iter::once(entry.name()).chain(entry.forms().iter().map(|form| form.text()));
    let abbreviation = shape(&character) != (true, true) && names.into_iter().any(|name| abbreviates(&whole, name));
    if abbreviation && (SpeakerRepo { conn: tx }).links_of(person)?.is_empty() {
        if let Ok(form) = Form::new(&whole, character.translated_full_name().as_deref()) {
            let _ = entry.add_form(form);
        }
        MentionCheckRepo { conn: tx }.copy(ExcludeOwner::Character(person), ExcludeOwner::Term(term))?;
        characters.delete(person)?;
        NoteRepo { conn: tx }.delete_owner(&NoteOwner::Character(person))?;
        terms.save(&entry)?;
        return Ok(Some(format!("{whole} → {} (объединено)", entry.name())));
    }
    let name = entry.name().to_string();
    give_term(tx, term, person)?;
    Ok(Some(format!("{name} → {} (объединено)", characters.get(person)?.full_name().unwrap_or_default())))
}

/// Две записи оказались одним: единственное слияние разбора. Возвращает строку итога «PPF → People's Police
/// Force (объединено)». `None`: одной из записей уже нет.
pub fn merge_pair(tx: &rusqlite::Connection, pair: Pair) -> Result<Option<String>, DbError> {
    match (pair.a, pair.b) {
        (ExcludeOwner::Character(a), ExcludeOwner::Character(b)) => merge_people(tx, a, b),
        (ExcludeOwner::Term(a), ExcludeOwner::Term(b)) => merge_two_terms(tx, a, b),
        (ExcludeOwner::Term(term), ExcludeOwner::Character(person)) | (ExcludeOwner::Character(person), ExcludeOwner::Term(term)) => {
            merge_term_and_person(tx, term, person)
        }
    }
}

/// `save_scene_found` без сцены: из нескольких владельцев написания выбрать некого.
pub fn save_found(tx: &rusqlite::Connection, found: &[Finding]) -> Result<Vec<String>, DbError> {
    Ok(save_scene_found(tx, found, None)?.added)
}

/// Итог записи находок сцены.
#[derive(Debug, Default)]
pub struct SceneSaved {
    /// Написания добавленных записей, форма как «PPF → People's Police Force», слияние с «(объединено)».
    pub added: Vec<String>,
    /// Пары от модели, которые скрипт не подтвердил: их решит вопрос «одно ли это».
    pub claims: Vec<Claim>,
}

/// Находки разбора сцены `scene` в глоссарий по таблице решений из описания модуля. После находок сливаются
/// пары, в которых слово модели совпало с отношением скрипта, и неразобранные говорящие привязываются
/// к персонажам по подписи.
pub fn save_scene_found(tx: &rusqlite::Connection, found: &[Finding], scene: Option<&ContainerKey>) -> Result<SceneSaved, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let all = SpeakerRepo { conn: tx }.current()?;
    let new: Vec<Speaker> = SpeakerRepo { conn: tx }.new_speakers()?;
    let new_keys: HashSet<_> = new.iter().map(|speaker| &speaker.key).collect();
    let known = all
        .iter()
        .filter(|used| !new_keys.contains(&used.speaker.key))
        .filter_map(|used| used.speaker.label.as_deref())
        .filter_map(NameKey::new)
        .collect();
    let new_labels = new
        .iter()
        .filter_map(|speaker| speaker.label.as_deref())
        .flat_map(|label| [NameKey::new(label), NameKey::new(without_article(label))])
        .flatten()
        .collect();
    let speakers: Vec<Speaker> = all.into_iter().map(|used| used.speaker).collect();
    let mut names = Names::load(&glossary, &speakers);
    if let Some(key) = scene {
        names.present = scene_entries(&glossary, &LineSampleRepo { conn: tx }.scene_lines(key)?).0;
    }
    let mut saver = Saver { tx, known, speakers, new_labels, names, added: Vec::new(), claims: Vec::new() };
    for finding in ordered(found) {
        saver.save(finding)?;
    }
    let claims = saver.settle_claims()?;
    saver.link_speakers()?;
    Ok(SceneSaved { added: saver.added, claims })
}

/// Персонажи, которые говорят в сцене или упомянуты в ней, для блока «Characters of this scene».
pub fn scene_cast(tx: &rusqlite::Connection, scene: &ContainerKey) -> Result<Vec<CastMember>, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let present = scene_entries(&glossary, &LineSampleRepo { conn: tx }.scene_lines(scene)?).0;
    Ok(glossary
        .characters
        .iter()
        .filter_map(|character| {
            let id = character.id().filter(|id| present.contains(id))?;
            Some(CastMember { id, names: character_names(character), gender: character.gender() })
        })
        .collect())
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
