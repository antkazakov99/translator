//! Кандидаты в термины глоссария. Модель принимает лишь малую часть текста игры, поэтому
//! названия сначала ищутся по всем репликам, а она проверяет и переводит найденное.

use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// С такой доли слов с заглавной в середине предложения заглавная перестаёт выделять
/// названия: так пишутся все существительные в немецком.
const CAPITALIZED_NOUNS_SHARE: f64 = 0.15;
/// Меньше слов не дают судить о языке.
const MIN_WORDS_FOR_LANGUAGE: usize = 200;
const MAX_EXAMPLE_CHARS: usize = 120;
const EXAMPLE_CONTEXT_CHARS: usize = 40;

static RE_MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}|\[[^\]]*\]").unwrap());
static RE_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}+(?:['’]\p{L}+)?").unwrap());
static RE_PHRASE: LazyLock<Regex> = LazyLock::new(|| {
    let word = r"\p{Lu}\p{Ll}+(?:['’]\p{Ll}+)?(?:-\p{Lu}?\p{Ll}+)?";
    Regex::new(&format!(r"\b{word}(?: (?:(?:of|the|de|la|del|von|der) )*{word})*\b")).unwrap()
});

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TermCandidate {
    pub term: String,
    /// В скольких разных строках игры встретился термин.
    pub count: usize,
    pub example: String,
}

#[derive(Debug, Default)]
pub struct CandidateSearch {
    pub candidates: Vec<TermCandidate>,
    /// Одиночные слова не рассматривались: в языке игры заглавная их не выделяет.
    pub single_words_skipped: bool,
}

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
    /// Встречается не только отдельной строкой, а внутри реплик.
    in_sentence: bool,
}

/// Начинается ли предложение после этого текста. Заглавная там ничего не значит.
fn starts_sentence(before: &str) -> bool {
    let before = before.trim_end();
    before.is_empty()
        || before.ends_with(['.', '!', '?', '…', ':', ';', '"', '“', '«', '(', '-', '—'])
}

/// Междометия вроде «Hhhhh» и «Grrrllk».
fn is_stuttering(term: &str) -> bool {
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
    names: &HashSet<String>,
) -> Option<String> {
    let at_start = at_start && !whole_line;
    let words = trim_phrase(phrase, at_start, vocabulary);
    let trimmed = words.len() < phrase.split(' ').count();
    if words.iter().all(|word| names.contains(&word.to_lowercase())) || is_stuttering(&words.join(" ")) {
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
    (!unclear && !vocabulary.is_common(word) && !names.contains(&word.to_lowercase()))
        .then(|| word.to_string())
}

/// Строка игры вокруг найденного термина.
fn excerpt(line: &str, position: usize) -> String {
    let skip = line[..position].chars().count().saturating_sub(EXAMPLE_CONTEXT_CHARS);
    line.chars().skip(skip).take(MAX_EXAMPLE_CHARS).collect()
}

/// Выбирает самые частые названия. Названия из реплик идут первыми, но треть мест остаётся
/// за отдельными строками: предметы и навыки часто встречаются всего раз, в меню.
fn select(found: HashMap<String, Found>, limit: usize) -> Vec<TermCandidate> {
    let by_count = |a: &Found, b: &Found| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term));
    let (mut in_sentences, mut titles): (Vec<Found>, Vec<Found>) =
        found.into_values().partition(|entry| entry.in_sentence);
    in_sentences.sort_by(by_count);
    titles.sort_by(by_count);

    let title_places = (limit / 3).min(titles.len());
    let sentence_places = (limit - title_places).min(in_sentences.len());
    let title_places = (limit - sentence_places).min(titles.len());
    let mut selected: Vec<Found> = in_sentences.into_iter().take(sentence_places).collect();
    selected.extend(titles.into_iter().take(title_places));
    selected.sort_by(by_count);
    selected
        .into_iter()
        .map(|entry| TermCandidate { term: entry.term, count: entry.count, example: entry.example })
        .collect()
}

/// Ищет названия: фразы с заглавной в репликах и строки, целиком похожие на название.
/// `names` содержит слова имён персонажей в нижнем регистре, `known` уже известные термины.
pub fn find_candidates(
    texts: &[String],
    names: &HashSet<String>,
    known: &HashSet<String>,
    limit: usize,
) -> CandidateSearch {
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
        for phrase in RE_PHRASE.find_iter(line) {
            let whole_line = phrase.as_str() == line;
            let at_start = starts_sentence(&line[..phrase.start()]);
            let Some(term) =
                normalize(phrase.as_str(), whole_line, at_start, skip_single_words, &vocabulary, names)
            else {
                continue;
            };
            let key = term.to_lowercase();
            if known.contains(&key) || !in_line.insert(key.clone()) {
                continue;
            }
            let entry = found.entry(key).or_insert_with(|| Found {
                term,
                count: 0,
                example: String::new(),
                in_sentence: false,
            });
            entry.count += 1;
            entry.in_sentence |= !whole_line;
            if entry.example.is_empty() || (entry.example == entry.term && !whole_line) {
                entry.example = excerpt(line, phrase.start());
            }
        }
    }

    CandidateSearch { candidates: select(found, limit), single_words_skipped: skip_single_words }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|text| text.to_string()).collect()
    }

    fn terms(texts: &[&str], names: &[&str]) -> Vec<String> {
        let names = names.iter().map(|name| name.to_string()).collect();
        find_candidates(&lines(texts), &names, &HashSet::new(), 60)
            .candidates
            .into_iter()
            .map(|candidate| candidate.term)
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
            ]),
            &HashSet::new(),
            &HashSet::new(),
            60,
        );
        let top = &found.candidates[0];
        assert_eq!((top.term.as_str(), top.count), ("Ninja Academy", 3));
        assert_eq!(top.example, "We went to the Ninja Academy today.");
        assert_eq!(found.candidates[1].term, "Konoha");
        assert_eq!(found.candidates.len(), 2);
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
                &["hima", "sato", "mr"],
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
    fn known_terms_are_not_offered_again() {
        let known = ["konoha".to_string()].into_iter().collect();
        let found = find_candidates(
            &lines(&["We left Konoha for Suna."]),
            &HashSet::new(),
            &known,
            60,
        );
        assert_eq!(found.candidates.len(), 1);
        assert_eq!(found.candidates[0].term, "Suna");
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
        let found = find_candidates(&texts, &HashSet::new(), &HashSet::new(), 60);
        assert!(found.single_words_skipped);
        let terms: Vec<_> = found.candidates.iter().map(|c| c.term.as_str()).collect();
        assert_eq!(terms, ["Herr Lauert"]);
    }

    #[test]
    fn menu_lines_keep_a_third_of_places() {
        let mut texts: Vec<String> = (0..9)
            .map(|i| format!("We saw Place{} near it.", char::from(b'a' + i)))
            .collect();
        texts.extend((0..9).map(|i| format!("Item{}", char::from(b'a' + i))));
        let found = find_candidates(&texts, &HashSet::new(), &HashSet::new(), 9);
        assert_eq!(found.candidates.len(), 9);
        let titles = found.candidates.iter().filter(|c| c.term.starts_with("Item")).count();
        assert_eq!(titles, 3);
    }
}
