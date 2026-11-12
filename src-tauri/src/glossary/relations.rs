//! Отношения между записями глоссария: чем одна запись похожа на другую по написанию. Отношение только
//! предлагает пару, ничего не пишет: одно ли это, решает модель вопросом (`same_entry.rs`). Все догадки
//! по строкам о связи записей живут здесь.

use super::spelling::{abbreviates, short_and_full, title_and_name, typo};
use super::{Character, CharacterId, ExcludeOwner, GlossaryData, NameKey, Term};

/// Почему скрипт считает две записи похожими. Порядок это порядок вопросов: первыми те, где связь вероятнее.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relation {
    /// Имя и фамилия порознь: «Dimitria» и «Vellnikov». Персонаж только с именем и персонаж только с фамилией
    /// в одной сцене, или два одиночных имени, которые в репликах игры стоят подряд (фамилия бывает записана
    /// именем: модель назвала её без `last_name`).
    Halves,
    /// Всё имя одной записи это имя, фамилия, полное имя или форма другой: «Karanovic» и «Vera Karanovic»,
    /// город «Hope» и девушка «Hope».
    NamePart,
    /// Короткая и полная форма имени: «Dimi» и «Dimitria».
    Short,
    /// Аббревиатура по первым буквам: «PPF» и «People's Police Force».
    Abbreviation,
    /// Термин это должность перед именем персонажа: «Princess Selena» и «Selena».
    Titled,
    /// Написания отличаются одной опечаткой: «Karanovic» и «Karanovich».
    Typo,
}

/// Пара записей без порядка: персонаж раньше термина, меньший номер раньше.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub a: ExcludeOwner,
    pub b: ExcludeOwner,
}

impl Pair {
    /// `None`: запись в паре с собой.
    pub fn new(a: ExcludeOwner, b: ExcludeOwner) -> Option<Self> {
        let rank = |owner: ExcludeOwner| match owner {
            ExcludeOwner::Character(id) => (0, id.get()),
            ExcludeOwner::Term(id) => (1, id.get()),
        };
        match rank(a).cmp(&rank(b)) {
            std::cmp::Ordering::Less => Some(Self { a, b }),
            std::cmp::Ordering::Greater => Some(Self { a: b, b: a }),
            std::cmp::Ordering::Equal => None,
        }
    }

    pub fn people(a: CharacterId, b: CharacterId) -> Option<Self> {
        Self::new(ExcludeOwner::Character(a), ExcludeOwner::Character(b))
    }

    pub fn has(&self, owner: ExcludeOwner) -> bool {
        self.a == owner || self.b == owner
    }
}

/// Пары терминов с одинаковым переводом: «Fey Court» и «Fairy Court», оба «Двор фей». Одинаковый перевод только
/// довод спросить модель: «Inn» и «Tavern» тоже могут стать «Таверной». Порядок по номерам терминов.
pub fn same_translation(terms: &[Term]) -> Vec<Pair> {
    let mut known: Vec<(NameKey, super::TermId)> =
        terms.iter().filter_map(|term| Some((term.translation().and_then(NameKey::new)?, term.id()?))).collect();
    known.sort_by_key(|(_, id)| id.get());
    let mut pairs = Vec::new();
    for (index, (translation, a)) in known.iter().enumerate() {
        for (_, b) in known[index + 1..].iter().filter(|(other, _)| other == translation) {
            pairs.extend(Pair::new(ExcludeOwner::Term(*a), ExcludeOwner::Term(*b)));
        }
    }
    pairs
}

/// Пара, которую предлагает скрипт.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Suspect {
    pub pair: Pair,
    pub why: Relation,
}

fn key(text: &str) -> Option<NameKey> {
    NameKey::new(text)
}

/// Имя из одного слова без токена.
fn one_word(name: &str) -> bool {
    !name.contains(char::is_whitespace) && !name.contains('[')
}

/// Есть ли у персонажа имя и фамилия.
fn shape(character: &Character) -> (bool, bool) {
    (character.first_name().is_some(), character.last_name().is_some())
}

/// Все написания персонажа: имя, фамилия, полное имя и формы.
fn person_keys(character: &Character) -> Vec<NameKey> {
    character.name_keys().into_iter().chain(character.forms().iter().map(|form| form.key().clone())).collect()
}

/// Название и формы термина.
fn term_names(term: &Term) -> impl Iterator<Item = &str> {
    std::iter::once(term.name()).chain(term.forms().iter().map(|form| form.text()))
}

fn genders_fit(a: &Character, b: &Character) -> bool {
    match (a.gender(), b.gender()) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

/// Одно имя из одного слова: только имя или только фамилия.
fn single(character: &Character) -> bool {
    shape(character) != (true, true) && character.full_name().is_some_and(|name| one_word(&name))
}

/// Отношение двух персонажей. `in_scene`: оба в одной разобранной сцене. `in_text`: их имена стоят подряд
/// в репликах игры («Dimitria Vellnikov»). Имя и фамилия порознь становятся парой при любом из двух, два
/// одиночных имени в одном поле только по тексту: иначе парой были бы любые два персонажа сцены.
pub fn people(a: &Character, b: &Character, in_scene: bool, in_text: bool) -> Option<Relation> {
    if !genders_fit(a, b) {
        return None;
    }
    let (Some(whole_a), Some(whole_b)) = (a.full_name(), b.full_name()) else { return None };
    if matches!((shape(a), shape(b)), ((true, false), (false, true)) | ((false, true), (true, false))) {
        let first = a.first_name().or(b.first_name()).unwrap_or_default();
        if key(&whole_a) != key(&whole_b) {
            return ((in_scene || in_text) && one_word(first)).then_some(Relation::Halves);
        }
    }
    let within = |whole: &str, other: &Character| key(whole).is_some_and(|key| person_keys(other).contains(&key));
    if within(&whole_a, b) || within(&whole_b, a) {
        return Some(Relation::NamePart);
    }
    if in_text && single(a) && single(b) {
        return Some(Relation::Halves);
    }
    let surnames_fit = match (a.last_name_key(), b.last_name_key()) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    if let (Some(first_a), Some(first_b)) = (a.first_name(), b.first_name())
        && surnames_fit
        && one_word(first_a)
        && one_word(first_b)
        && short_and_full(first_a, first_b)
    {
        return Some(Relation::Short);
    }
    let parts = |character: &'_ Character| -> Vec<String> {
        [character.first_name(), character.last_name()].into_iter().flatten().map(str::to_string).collect()
    };
    let single_typo = |single: &Character, full: &Character| shape(single) != (true, true) && parts(full).iter().any(|part| typo(&parts(single).concat(), part));
    (typo(&whole_a, &whole_b) || single_typo(a, b) || single_typo(b, a)).then_some(Relation::Typo)
}

/// Отношение двух терминов.
pub fn terms(a: &Term, b: &Term) -> Option<Relation> {
    let abbreviated = term_names(a).any(|one| term_names(b).any(|other| abbreviates(one, other) || abbreviates(other, one)));
    if abbreviated {
        return Some(Relation::Abbreviation);
    }
    typo(a.name(), b.name()).then_some(Relation::Typo)
}

/// Отношение термина и персонажа.
pub fn term_and_person(term: &Term, person: &Character) -> Option<Relation> {
    let keys = person_keys(person);
    if term_names(term).any(|name| key(name).is_some_and(|key| keys.contains(&key))) {
        return Some(Relation::NamePart);
    }
    if title_and_name(term.name()).is_some_and(|(_, name)| key(&name).is_some_and(|key| keys.contains(&key))) {
        return Some(Relation::Titled);
    }
    let whole = person.full_name()?;
    (shape(person) != (true, true) && term_names(term).any(|name| abbreviates(&whole, name))).then_some(Relation::Abbreviation)
}

/// Отношение двух записей любого вида.
pub fn between(glossary: &GlossaryData, a: ExcludeOwner, b: ExcludeOwner, in_scene: bool, in_text: bool) -> Option<Relation> {
    let person = |id: CharacterId| glossary.characters.iter().find(|character| character.id() == Some(id));
    let term = |id| glossary.terms.iter().find(|term| term.id() == Some(id));
    match (a, b) {
        (ExcludeOwner::Character(a), ExcludeOwner::Character(b)) => people(person(a)?, person(b)?, in_scene, in_text),
        (ExcludeOwner::Term(a), ExcludeOwner::Term(b)) => terms(term(a)?, term(b)?),
        (ExcludeOwner::Term(a), ExcludeOwner::Character(b)) | (ExcludeOwner::Character(b), ExcludeOwner::Term(a)) => term_and_person(term(a)?, person(b)?),
    }
}

/// Пары, которые скрипт считает похожими: хотя бы одна запись пары из `present` (говорит или упомянута
/// в разобранной сцене). `together`: пары персонажей, чьи одиночные имена стоят подряд в репликах игры.
/// Порядок: по отношению, при равном первыми пары, где в сцене обе записи.
pub fn suspects(glossary: &GlossaryData, present: &[ExcludeOwner], together: &[(CharacterId, CharacterId)]) -> Vec<Suspect> {
    let all: Vec<ExcludeOwner> = glossary
        .characters
        .iter()
        .filter_map(|character| character.id().map(ExcludeOwner::Character))
        .chain(glossary.terms.iter().filter_map(|term| term.id().map(ExcludeOwner::Term)))
        .collect();
    let mut found: Vec<(Suspect, bool)> = Vec::new();
    for here in present {
        for other in &all {
            let Some(pair) = Pair::new(*here, *other) else { continue };
            if found.iter().any(|(suspect, _)| suspect.pair == pair) {
                continue;
            }
            let both = present.contains(other);
            let adjacent = match (pair.a, pair.b) {
                (ExcludeOwner::Character(a), ExcludeOwner::Character(b)) => together.iter().any(|joined| *joined == (a, b) || *joined == (b, a)),
                _ => false,
            };
            if let Some(why) = between(glossary, pair.a, pair.b, both, adjacent) {
                found.push((Suspect { pair, why }, both));
            }
        }
    }
    found.sort_by_key(|(suspect, both)| (suspect.why, !both));
    found.into_iter().map(|(suspect, _)| suspect).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Gender;
    use crate::glossary::{Form, TermCategory, TermId};

    fn character(id: i64, first: &str, last: &str, gender: Option<Gender>) -> Character {
        let name = |text: &str| (!text.is_empty()).then(|| text.to_string());
        Character::restore(CharacterId::from_row(id), name(first), name(last), None, None, gender, None, None, Vec::new())
    }

    fn term(id: i64, name: &str, forms: &[&str]) -> Term {
        let forms = forms.iter().map(|form| Form::new(form, None).unwrap()).collect();
        Term::restore(TermId::from_row(id), name, None, TermCategory::General, None, forms).unwrap()
    }

    fn person(id: i64) -> ExcludeOwner {
        ExcludeOwner::Character(CharacterId::from_row(id))
    }

    #[test]
    fn people_relate_by_name_parts_short_forms_and_typos() {
        let female = Some(Gender::Female);
        let vera = character(1, "Vera", "Karanovic", female);
        let people = |a: &Character, b: &Character, in_scene: bool| super::people(a, b, in_scene, false);
        assert_eq!(people(&character(2, "", "Karanovic", None), &vera, false), Some(Relation::NamePart));
        assert_eq!(people(&character(2, "Vera", "", None), &vera, false), Some(Relation::NamePart));
        assert_eq!(people(&character(2, "Karanovic", "", None), &vera, false), Some(Relation::NamePart), "фамилия, записанная именем");
        assert_eq!(people(&character(2, "Ivan", "Karanovic", None), &vera, true), None, "однофамильцы с именами не пара");
        assert_eq!(people(&character(2, "", "Karanovic", Some(Gender::Male)), &vera, true), None, "другой род");
        assert_eq!(people(&character(2, "Vera", "Karanovic", None), &vera, false), Some(Relation::NamePart), "тёзки с одним полным именем");

        let dimitria = character(3, "Dimitria", "", female);
        let vellnikov = character(4, "", "Vellnikov", None);
        assert_eq!(people(&dimitria, &vellnikov, true), Some(Relation::Halves));
        assert_eq!(people(&vellnikov, &dimitria, false), None, "порознь и без общей реплики половинки не пара");
        assert_eq!(people(&character(5, "[mc]", "", None), &vellnikov, true), None, "токен не имя");

        assert_eq!(people(&character(5, "Dimi", "", None), &dimitria, false), Some(Relation::Short));
        assert_eq!(people(&character(5, "Dimi", "", None), &character(6, "Dimitria", "Vellnikov", None), false), Some(Relation::Short));
        assert_eq!(people(&character(5, "Dimi", "Lee", None), &character(6, "Dimitria", "Vellnikov", None), false), None, "разные фамилии");
        assert_eq!(people(&character(5, "Anya", "", None), &character(6, "Anastasia", "", None), false), None);

        assert_eq!(people(&character(5, "", "Karanovich", None), &vera, false), Some(Relation::Typo));
        assert_eq!(people(&character(5, "Vera", "Karanovich", None), &vera, false), Some(Relation::Typo));
        assert_eq!(people(&character(5, "Maria", "", None), &character(6, "Marta", "", None), false), None);

        // Фамилия, записанная именем: парой с именем становится только по общей реплике игры.
        let surname = character(7, "Vellnikov", "", None);
        assert_eq!(people(&dimitria, &surname, true), None, "два имени в одной сцене ещё не пара");
        assert_eq!(super::people(&dimitria, &surname, false, true), Some(Relation::Halves));
        assert_eq!(super::people(&dimitria, &vellnikov, false, true), Some(Relation::Halves));
        assert_eq!(super::people(&vera, &surname, false, true), None, "у полного имени половинок нет");
    }

    #[test]
    fn terms_relate_by_abbreviation_and_typo_and_to_people_by_title() {
        let police = term(1, "People's Police Force", &[]);
        assert_eq!(terms(&term(2, "PPF", &[]), &police), Some(Relation::Abbreviation));
        assert_eq!(terms(&police, &term(2, "Police", &["PPF"])), Some(Relation::Abbreviation), "по форме");
        assert_eq!(terms(&term(2, "Fey Court", &[]), &term(3, "Fairy Court", &[])), None);
        assert_eq!(terms(&term(2, "Truthsayer", &[]), &term(3, "Truthsayar", &[])), Some(Relation::Typo));

        let selena = character(1, "Selena", "", None);
        assert_eq!(term_and_person(&term(1, "Princess Selena", &[]), &selena), Some(Relation::Titled));
        assert_eq!(term_and_person(&term(1, "Selena", &[]), &selena), Some(Relation::NamePart), "одно написание у термина и персонажа");
        assert_eq!(term_and_person(&term(1, "General Store", &[]), &selena), None);
        assert_eq!(term_and_person(&police, &character(2, "PPF", "", None)), Some(Relation::Abbreviation));
        assert_eq!(term_and_person(&police, &character(2, "Peter", "Force", None)), None);
    }

    #[test]
    fn terms_with_one_translation_make_a_pair_to_ask_about() {
        let translated = |id: i64, name: &str, translation: &str| {
            let mut term = term(id, name, &[]);
            term.set_translation(translation);
            term
        };
        let terms = [translated(4, "Fairy Court", "двор фей"), translated(2, "Fey Court", "Двор фей"), translated(5, "Inn", "Таверна"), term(6, "Tavern", &[])];
        let pair = Pair::new(ExcludeOwner::Term(super::super::TermId::from_row(2)), ExcludeOwner::Term(super::super::TermId::from_row(4))).unwrap();
        assert_eq!(same_translation(&terms), [pair], "регистр перевода не важен, термин без перевода пары не даёт");
    }

    #[test]
    fn suspects_need_one_entry_in_the_scene() {
        let glossary = GlossaryData {
            characters: vec![
                character(1, "Dimitria", "", None),
                character(2, "", "Vellnikov", None),
                character(3, "Dimi", "", None),
                character(4, "Hilde", "", None),
                character(5, "", "Tominskova", None),
            ],
            terms: vec![term(1, "People's Police Force", &[]), term(2, "PPF", &[])],
            ..GlossaryData::default()
        };
        let pair = |a: ExcludeOwner, b: ExcludeOwner, why| Suspect { pair: Pair::new(a, b).unwrap(), why };
        let term = |id| ExcludeOwner::Term(TermId::from_row(id));

        // В сцене Dimitria и Vellnikov: половинки рядом, короткая форма вне сцены тоже пара Dimitria.
        let found = suspects(&glossary, &[person(1), person(2)], &[]);
        assert_eq!(found, [pair(person(1), person(2), Relation::Halves), pair(person(1), person(3), Relation::Short)]);

        // Hilde в сцене одна: фамилия вне сцены парой становится только по общей реплике игры.
        assert!(suspects(&glossary, &[person(4)], &[]).is_empty());
        let id = CharacterId::from_row;
        assert_eq!(suspects(&glossary, &[person(4)], &[(id(4), id(5))]), [pair(person(4), person(5), Relation::Halves)]);

        assert_eq!(suspects(&glossary, &[term(2)], &[]), [pair(term(1), term(2), Relation::Abbreviation)]);
        assert!(suspects(&glossary, &[], &[]).is_empty());
    }
}
