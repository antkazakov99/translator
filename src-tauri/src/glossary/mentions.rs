//! Упоминания записей глоссария в репликах. Имя ищется целым словом без учёта регистра, а засчитывается
//! совпадение по ответу модели для этой реплики или, пока ответа нет, по написанию: «MInD» не находит «mind»,
//! но находит «MIND!» и «MiNd».

use std::collections::{HashMap, HashSet};

/// Ответы модели по репликам одной записи: отпечаток оригинала реплики (`segments.source_hash`) и упомянута ли
/// в ней запись.
pub type Checks = HashMap<String, bool>;

/// Упомянута ли запись с написаниями `names` в тексте. `hash`: отпечаток оригинала реплики, по нему берётся ответ
/// модели; у текста не из реплики пустой. Ответ модели важнее написания.
pub fn mentioned(text: &str, hash: &str, names: &[impl AsRef<str>], checks: Option<&Checks>) -> bool {
    let lower = text.to_lowercase();
    let mut found = names
        .iter()
        .map(AsRef::as_ref)
        .flat_map(|name| occurrences_in(text, &lower, name).into_iter().map(move |found| (found, name)))
        .peekable();
    if found.peek().is_none() {
        return false;
    }
    if let Some(answer) = checks.and_then(|checks| checks.get(hash)) {
        return *answer;
    }
    found.any(|(found, name)| by_spelling(found, name))
}

/// Совпадения `name` в `text` целым словом без учёта регистра: куски текста, как они написаны.
pub fn occurrences<'t>(text: &'t str, name: &str) -> Vec<&'t str> {
    occurrences_in(text, &text.to_lowercase(), name)
}

/// `lower`: тот же текст в нижнем регистре, им отсеивается текст без имени.
fn occurrences_in<'t>(text: &'t str, lower: &str, name: &str) -> Vec<&'t str> {
    let name = name.trim();
    if name.is_empty() {
        return Vec::new();
    }
    let wanted: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    if !lower.contains(&wanted.iter().collect::<String>()) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut previous: Option<char> = None;
    for (start, current) in text.char_indices() {
        if !previous.is_some_and(char::is_alphanumeric)
            && let Some(end) = match_at(text, start, &wanted)
            && !text[end..].chars().next().is_some_and(char::is_alphanumeric)
        {
            found.push(&text[start..end]);
        }
        previous = Some(current);
    }
    found
}

/// Конец совпадения `wanted` (в нижнем регистре) с текстом от `start`. `None`: не совпало.
fn match_at(text: &str, start: usize, wanted: &[char]) -> Option<usize> {
    let mut rest = wanted;
    for (offset, current) in text[start..].char_indices() {
        if rest.is_empty() {
            return Some(start + offset);
        }
        for lower in current.to_lowercase() {
            let (first, tail) = rest.split_first()?;
            if *first != lower {
                return None;
            }
            rest = tail;
        }
    }
    rest.is_empty().then_some(text.len())
}

/// Засчитывается ли совпадение без ответа модели. Да: точное написание, оно же с заглавной первой буквой (начало
/// фразы у названия строчными), целиком заглавными и необычное написание. Нет: обычное написание другого слова,
/// строчными или с заглавной первой буквой («mind» и «Mind» у «MInD», «hope» у «Hope»).
pub fn by_spelling(found: &str, name: &str) -> bool {
    let name = name.trim();
    found == name || found == capitalized(name) || is_upper(found) || !is_plain(found)
}

/// Бесспорное совпадение, о котором модель не спрашивают: точное написание, если написание необычное или в игре
/// нет того же слова в другом регистре.
pub fn is_sure(found: &str, name: &str, casings: &mut Casings) -> bool {
    let name = name.trim();
    found == name && (is_unusual(name) || !casings.has_other(name))
}

/// Насколько совпадение похоже на запись: им упорядочиваются вопросы модели, когда их больше предела.
/// 0: точное или необычное написание, 1: целиком заглавными, 2: обычное написание другого слова.
pub fn doubt(found: &str, name: &str) -> u8 {
    let name = name.trim();
    if found == name || found == capitalized(name) || (!is_plain(found) && !is_upper(found)) {
        0
    } else if is_upper(found) {
        1
    } else {
        2
    }
}

/// Заглавная внутри слова при строчных в нём же: «MInD», «McAllister». Обычное слово так не пишется.
pub fn is_unusual(name: &str) -> bool {
    words(name).any(|word| word.chars().skip(1).any(char::is_uppercase) && word.chars().any(char::is_lowercase))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn is_upper(text: &str) -> bool {
    text.chars().any(char::is_alphabetic) && !text.chars().any(char::is_lowercase)
}

/// Каждое слово строчными или с одной заглавной первой буквой.
fn is_plain(text: &str) -> bool {
    words(text).all(|word| !word.chars().skip(1).any(char::is_uppercase))
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty())
}

/// Написания слов в репликах игры: есть ли у написания записи то же слово в другом регистре.
pub struct Casings {
    lines: Vec<String>,
    /// Слово в нижнем регистре и номера реплик с ним.
    words: HashMap<String, Vec<usize>>,
    known: HashMap<String, bool>,
}

impl Casings {
    pub fn new(texts: impl IntoIterator<Item = String>) -> Self {
        let lines: Vec<String> = texts.into_iter().collect();
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (number, line) in lines.iter().enumerate() {
            let mut seen = HashSet::new();
            for word in words(line) {
                let word = word.to_lowercase();
                if seen.insert(word.clone()) {
                    index.entry(word).or_default().push(number);
                }
            }
        }
        Self { lines, words: index, known: HashMap::new() }
    }

    /// Встречается ли `name` в репликах игры в другом регистре. Реплики-кандидаты берутся по самому редкому слову.
    pub fn has_other(&mut self, name: &str) -> bool {
        let name = name.trim();
        if let Some(known) = self.known.get(name) {
            return *known;
        }
        let rarest = words(name).map(|word| self.words.get(&word.to_lowercase()).map_or(0, Vec::len)).enumerate().min_by_key(|(_, count)| *count);
        let other = match rarest.and_then(|(index, _)| words(name).nth(index)) {
            Some(word) => self.words.get(&word.to_lowercase()).is_some_and(|numbers| {
                numbers.iter().any(|&number| occurrences(&self.lines[number], name).iter().any(|found| *found != name))
            }),
            None => false,
        };
        self.known.insert(name.to_string(), other);
        other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn occurrences_are_whole_words_in_any_case() {
        assert_eq!(occurrences("Never mind, MInD and MIND!", "MInD"), ["mind", "MInD", "MIND"]);
        assert_eq!(occurrences("Hope's here, hopeless", "hope"), ["Hope"]);
        assert_eq!(occurrences("the silver keys", "silver key"), Vec::<&str>::new());
        assert_eq!(occurrences("Дмитрия и ДМИТРИЯ", "дмитрия"), ["Дмитрия", "ДМИТРИЯ"]);
        assert!(occurrences("anything", "  ").is_empty());
    }

    #[test]
    fn spelling_decides_without_an_answer() {
        let mind = names(&["MInD"]);
        assert!(mentioned("MInD is watching.", "", &mind, None));
        assert!(mentioned("MIND IS WATCHING!", "", &mind, None), "целиком заглавными засчитывается");
        assert!(mentioned("MiNd is watching.", "", &mind, None), "игра регистрами засчитывается");
        assert!(!mentioned("Never mind.", "", &mind, None));
        assert!(!mentioned("Mind your step.", "", &mind, None));

        let hope = names(&["Hope"]);
        assert!(mentioned("Hope you're right.", "", &hope, None), "точное написание засчитывается до ответа модели");
        assert!(!mentioned("I hope so.", "", &hope, None));

        let mana = names(&["mana"]);
        assert!(mentioned("Mana flows here.", "", &mana, None), "начало фразы у названия строчными");
        assert!(!mentioned("Silver key.", "", &names(&["Silver Key"]), None));
    }

    #[test]
    fn answer_of_the_model_beats_spelling() {
        let hope = names(&["Hope"]);
        let checks: Checks = [("a".to_string(), false), ("b".to_string(), true)].into();
        assert!(!mentioned("Hope you're right.", "a", &hope, Some(&checks)));
        assert!(mentioned("hope is here", "b", &hope, Some(&checks)));
        assert!(mentioned("Hope you're right.", "c", &hope, Some(&checks)), "без ответа решает написание");
        assert!(!mentioned("Nothing here.", "b", &hope, Some(&checks)), "ответ не создаёт упоминания без совпадения");
    }

    #[test]
    fn sure_spellings_need_no_question() {
        let mut casings = Casings::new(names(&["Karanovic came.", "Never mind.", "Hope you know.", "I hope so.", "People's Police Force"]));
        assert!(is_sure("Karanovic", "Karanovic", &mut casings));
        assert!(is_sure("MInD", "MInD", &mut casings), "необычное написание");
        assert!(!is_sure("Hope", "Hope", &mut casings), "в игре есть «hope»");
        assert!(!is_sure("mind", "MInD", &mut casings));
        assert!(is_sure("People's Police Force", "People's Police Force", &mut casings));
        assert!(is_unusual("McAllister") && is_unusual("DiMiTrIa") && !is_unusual("PPF") && !is_unusual("Silver Key"));
    }

    #[test]
    fn doubt_orders_questions() {
        assert_eq!(doubt("Hope", "Hope"), 0);
        assert_eq!(doubt("MiNd", "MInD"), 0);
        assert_eq!(doubt("MIND", "MInD"), 1);
        assert_eq!(doubt("mind", "MInD"), 2);
    }
}
