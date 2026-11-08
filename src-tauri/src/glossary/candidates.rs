//! Кандидаты в термины глоссария: названия по всем репликам игры (`find_candidates` из 1.0). В глоссарий их
//! берёт пользователь, поэтому мусор здесь допустим, а пропуск нет. Поиск зависит только от текста игры:
//! имена персонажей вычитает чтение (`without_names`), ведь глоссарий меняется чаще снимка.

use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use super::TermCandidate;

/// Версия сборки `term_candidates`: выросла, и таблица пересобирается без повторного считывания.
pub const TERM_CANDIDATES_VERSION: u32 = 1;

/// С такой доли слов с заглавной в середине предложения заглавная перестаёт выделять
/// названия: так пишутся все существительные в немецком.
const CAPITALIZED_NOUNS_SHARE: f64 = 0.15;
/// Меньше слов не дают судить о языке.
const MIN_WORDS_FOR_LANGUAGE: usize = 200;
const MAX_EXAMPLE_CHARS: usize = 120;
const EXAMPLE_CONTEXT_CHARS: usize = 40;

static RE_MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}|\[[^\]]*\]").unwrap());
pub(crate) static RE_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}+(?:['’]\p{L}+)?").unwrap());
static RE_ABBREVIATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\p{Lu}{2,6}\b").unwrap());
static RE_PHRASE: LazyLock<Regex> = LazyLock::new(|| {
    let word = r"\p{Lu}\p{Ll}+(?:['’]\p{Ll}+)?(?:-\p{Lu}?\p{Ll}+)?";
    Regex::new(&format!(r"\b{word}(?: (?:(?:of|the|de|la|del|von|der) )*{word})*\b")).unwrap()
});

/// Что известно о словах игры после первого прохода по строкам.
#[derive(Default)]
struct Vocabulary {
    /// Слова, которые встречаются со строчной буквы: это не названия.
    lowercase: HashSet<String>,
    /// Фразы, которые стоят в середине предложения или занимают всю строку.
    /// Заглавная там не объясняется началом предложения.
    named: HashSet<String>,
    inner_words: usize,
    inner_capitalized: usize,
}

impl Vocabulary {
    fn collect(lines: &[String]) -> Self {
        let mut vocabulary = Self::default();
        for line in lines {
            for found in RE_WORD.find_iter(line) {
                let word = found.as_str();
                let mut chars = word.chars();
                let first = chars.next().unwrap_or_default();
                if first.is_lowercase() {
                    vocabulary.lowercase.insert(word.to_string());
                }
                if !starts_sentence(&line[..found.start()]) {
                    vocabulary.inner_words += 1;
                    if first.is_uppercase() && chars.any(char::is_lowercase) {
                        vocabulary.inner_capitalized += 1;
                    }
                }
            }
        }
        for line in lines {
            for found in RE_PHRASE.find_iter(line) {
                let phrase = found.as_str();
                if phrase == line || !starts_sentence(&line[..found.start()]) {
                    vocabulary.named.insert(phrase.to_lowercase());
                } else if let Some((first, rest)) = phrase.split_once(' ')
                    && vocabulary.is_common(first)
                {
                    // «Take the Dark Book»: название стоит после первого слова предложения
                    let rest = rest.trim_start_matches(|c: char| c.is_lowercase() || c == ' ');
                    vocabulary.named.insert(rest.to_lowercase());
                }
            }
        }
        vocabulary
    }

    fn capitalizes_nouns(&self) -> bool {
        self.inner_words >= MIN_WORDS_FOR_LANGUAGE
            && self.inner_capitalized as f64 / self.inner_words as f64 >= CAPITALIZED_NOUNS_SHARE
    }

    fn is_common(&self, word: &str) -> bool {
        self.lowercase.contains(&word.to_lowercase())
    }
}

struct Found {
    term: String,
    count: usize,
    example: String,
}

/// Начинается ли предложение после этого текста. Заглавная там ничего не значит.
fn starts_sentence(before: &str) -> bool {
    let before = before.trim_end();
    before.is_empty()
        || before.ends_with(['.', '!', '?', '…', ':', ';', '"', '“', '«', '(', '-', '—'])
}

/// Междометия вроде «Hhhhh» и «Grrrllk».
pub(crate) fn is_stuttering(term: &str) -> bool {
    let letters: Vec<char> = term.to_lowercase().chars().collect();
    letters.windows(3).any(|run| run[0] == run[1] && run[1] == run[2] && run[0].is_alphabetic())
}

/// Отрезает от фразы то, что не относится к названию: «At the Academy» даёт «Academy».
fn trim_phrase<'a>(phrase: &'a str, mut at_start: bool, vocabulary: &Vocabulary) -> Vec<&'a str> {
    let mut words: Vec<&str> = phrase.split(' ').collect();
    while words.len() > 1 {
        let first = words[0];
        let sentence_word = at_start
            && vocabulary.is_common(first)
            && !vocabulary.named.contains(&words.join(" ").to_lowercase());
        if sentence_word || first == "The" || first.starts_with(char::is_lowercase) {
            words.remove(0);
            at_start = false;
        } else {
            break;
        }
    }
    words
}

/// Название из найденной фразы или `None`, если фраза им не выглядит.
fn normalize(
    phrase: &str,
    whole_line: bool,
    at_start: bool,
    skip_single_words: bool,
    vocabulary: &Vocabulary,
) -> Option<String> {
    let at_start = at_start && !whole_line;
    let words = trim_phrase(phrase, at_start, vocabulary);
    let trimmed = words.len() < phrase.split(' ').count();
    if is_stuttering(&words.join(" ")) {
        return None;
    }
    // Притяжательная форма: «Miss Granger's» и «Miss Granger» это один термин
    let joined = words.join(" ");
    let phrase = joined.strip_suffix("'s").or_else(|| joined.strip_suffix("’s")).unwrap_or(&joined);
    if words.len() > 1 {
        return Some(phrase.to_string());
    }
    let word = phrase;
    let unclear = skip_single_words || (at_start && !trimmed) || word.chars().count() < 3;
    (!unclear && !vocabulary.is_common(word))
        .then(|| word.to_string())
}

/// Строка игры вокруг найденного термина.
fn excerpt(line: &str, position: usize) -> String {
    let skip = line[..position].chars().count().saturating_sub(EXAMPLE_CONTEXT_CHARS);
    line.chars().skip(skip).take(MAX_EXAMPLE_CHARS).collect()
}

/// Расставляет названия от самых частых, а равные по числу упоминаний по алфавиту.
fn rank(found: HashMap<String, Found>) -> Vec<TermCandidate> {
    let mut ranked: Vec<TermCandidate> = found
        .into_values()
        .map(|entry| TermCandidate { text: entry.term, occurrences: u32::try_from(entry.count).unwrap_or(u32::MAX), example: entry.example })
        .collect();
    ranked.sort_by_cached_key(|candidate| (std::cmp::Reverse(candidate.occurrences), candidate.text.to_lowercase()));
    ranked
}

/// Похоже ли слово заглавными на сокращение. Такими словами чаще всего кричат, но обычное
/// слово игра пишет и строчными.
fn is_abbreviation(word: &str, vocabulary: &Vocabulary) -> bool {
    !vocabulary.is_common(word) && !is_stuttering(word)
}

/// Убирает кандидатов, у которых каждое слово это слово имени персонажа или подписи говорящего: «Mr Sato».
/// `names` в нижнем регистре.
pub fn without_names(candidates: Vec<TermCandidate>, names: &HashSet<String>) -> Vec<TermCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            let text = candidate.text.strip_suffix("'s").or_else(|| candidate.text.strip_suffix("’s")).unwrap_or(&candidate.text);
            !RE_WORD.find_iter(text).all(|word| names.contains(&word.as_str().to_lowercase()))
        })
        .collect()
}

/// Слова имён в нижнем регистре: по ним `without_names` узнаёт имя.
pub fn name_words<'a>(names: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    names.into_iter().flat_map(|name| RE_WORD.find_iter(name).map(|word| word.as_str().to_lowercase()).collect::<Vec<_>>()).collect()
}

/// Ищет названия: фразы с заглавной в репликах, сокращения и строки, целиком похожие на название.
/// Самые частые первыми. Если заглавная в языке игры пишется у всех существительных, одиночные слова не берутся.
pub fn find_candidates(texts: &[String]) -> Vec<TermCandidate> {
    let mut seen = HashSet::new();
    let lines: Vec<String> = texts
        .iter()
        .map(|text| RE_MARKUP.replace_all(text, "").replace('\n', " ").trim().to_string())
        .filter(|line| seen.insert(line.clone()))
        .collect();
    let vocabulary = Vocabulary::collect(&lines);
    let skip_single_words = vocabulary.capitalizes_nouns();

    let mut found: HashMap<String, Found> = HashMap::new();
    for line in &lines {
        let mut in_line = HashSet::new();
        let mut record = |term: String, position: usize, whole_line: bool| {
            let key = term.to_lowercase();
            if !in_line.insert(key.clone()) {
                return;
            }
            let entry = found.entry(key).or_insert_with(|| Found {
                term,
                count: 0,
                example: String::new(),
            });
            entry.count += 1;
            if entry.example.is_empty() || (entry.example == entry.term && !whole_line) {
                entry.example = excerpt(line, position);
            }
        };
        for phrase in RE_PHRASE.find_iter(line) {
            let whole_line = phrase.as_str() == line;
            let at_start = starts_sentence(&line[..phrase.start()]);
            if let Some(term) =
                normalize(phrase.as_str(), whole_line, at_start, skip_single_words, &vocabulary)
            {
                record(term, phrase.start(), whole_line);
            }
        }
        for word in RE_ABBREVIATION.find_iter(line) {
            if is_abbreviation(word.as_str(), &vocabulary) {
                record(word.as_str().to_string(), word.start(), word.as_str() == line);
            }
        }
    }

    rank(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    fn terms(texts: &[&str], names: &[&str]) -> Vec<String> {
        without_names(find_candidates(&lines(texts)), &name_words(names.iter().copied()))
            .into_iter()
            .map(|candidate| candidate.text)
            .collect()
    }

    #[test]
    fn finds_names_inside_lines_and_ranks_them_by_use() {
        let found = find_candidates(
            &lines(&[
                "We went to the Ninja Academy today.",
                "Is the Ninja Academy far from Konoha?",
                "I love the village of Konoha and the {b}Ninja Academy{/b}.",
                "The academy is far.",
            ]));
        let top = &found[0];
        assert_eq!((top.text.as_str(), top.occurrences), ("Ninja Academy", 3));
        assert_eq!(top.example, "We went to the Ninja Academy today.");
        assert_eq!(found[1].text, "Konoha");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn capital_letter_of_a_sentence_means_nothing() {
        assert_eq!(
            terms(
                &[
                    "Oh God, not again.",
                    "Yes Master, right away.",
                    "At the Academy we study.",
                    "Tomorrow is fine. Maybe later.",
                    "oh yes, at last, my god, my master, tomorrow, maybe.",
                ],
                &[],
            ),
            ["Academy"]
        );
    }

    #[test]
    fn phrase_keeps_first_word_when_it_is_part_of_name() {
        assert_eq!(
            terms(
                &["Dark Book is here.", "Take the Dark Book home.", "It is dark here, take it."],
                &[],
            ),
            ["Dark Book"]
        );
    }

    #[test]
    fn menu_lines_are_taken_as_titles() {
        assert_eq!(
            terms(
                &["Potion of Health", "The Stun Charm", "Wild Oats", "Stay here for now", "Save", "save it"],
                &[],
            ),
            ["Potion of Health", "Stun Charm", "Wild Oats"]
        );
    }

    #[test]
    fn character_names_and_noise_are_dropped() {
        assert_eq!(
            terms(
                &["Hello there, Hima.", "Call Hima's mother, Mr Sato.", "Well, Hhhhh and Yamanaka Flower Shop.", "I will call."],
                &["Hima", "Mr Sato"],
            ),
            ["Yamanaka Flower Shop"]
        );
    }

    #[test]
    fn possessive_form_is_the_same_term() {
        assert_eq!(
            terms(&["I saw Miss Granger today.", "This is Miss Granger's book, and Bernie's Bowling Bar."], &[]),
            ["Miss Granger", "Bernie's Bowling Bar"]
        );
    }

    #[test]
    fn abbreviations_are_found_and_shouting_is_not() {
        assert_eq!(
            terms(
                &[
                    "The PPF sent me, and the PPF knows.",
                    "I told you to WAIT HERE, you hear me?",
                    "You should wait here. I will wait.",
                    "AAAH! Stop it.",
                    "Call AL now.",
                ],
                &["Al"],
            ),
            ["PPF"]
        );
    }

    #[test]
    fn cyrillic_names_are_found() {
        assert_eq!(terms(&["Мы пошли в Академию Ниндзя вчера."], &[]), ["Академию Ниндзя"]);
    }

    #[test]
    fn language_with_capitalized_nouns_keeps_only_phrases() {
        let mut texts: Vec<String> = (0..50)
            .map(|i| format!("Ich sehe den Wald und das Dorf {i}."))
            .collect();
        texts.push("Ich sehe den Wald mit Herr Lauert im Haus.".to_string());
        let found = find_candidates(&texts);
        let terms: Vec<_> = found.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(terms, ["Herr Lauert"]);
    }

    #[test]
    fn candidates_go_by_count_then_by_name() {
        let found = find_candidates(&lines(&["Zeta Gate", "BETA", "Beta Mine", "we saw Alpha Gate.", "they left Alpha Gate.", "and past Alpha Gate."]));
        let order: Vec<_> = found.iter().map(|c| (c.text.as_str(), c.occurrences)).collect();
        assert_eq!(order, [("Alpha Gate", 3), ("BETA", 1), ("Beta Mine", 1), ("Zeta Gate", 1)]);
    }
}
