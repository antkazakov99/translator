//! Проверки текста реплики: токены и теги, реплики без букв, упоминания названий.
//!
//! Скобки `[имя]` и `{тег}` это запись Ren'Py. Пока движок один, правила лежат здесь; второй движок со своей
//! записью получит их через `GameEngine`.

use std::sync::LazyLock;

use regex::Regex;

/// Токен: `[mc]`, `[friend.name]`, `[mc!c]`.
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([A-Za-z_][\w.]*)(?:![a-z]+)?\]").expect("регулярное выражение токена"));
/// Тег текста: `{i}`, `{/i}`, `{w=0.5}`.
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").expect("регулярное выражение тега"));

/// Токены и теги реплики по порядку сортировки: перевод обязан сохранить их все, иначе игра покажет другой текст.
pub fn markers(text: &str) -> Vec<&str> {
    let mut markers: Vec<&str> = TOKEN.find_iter(text).chain(TAG.find_iter(text)).map(|found| found.as_str()).collect();
    markers.sort_unstable();
    markers
}

/// Имена токенов без скобок.
pub fn token_names(text: &str) -> impl Iterator<Item = &str> {
    TOKEN.captures_iter(text).filter_map(|caps| caps.get(1)).map(|name| name.as_str())
}

/// Подставляет значения токенов: `[mom]` и `[mom!c]` становятся словом, токен без значения остаётся.
pub fn fill_tokens(text: &str, values: &std::collections::HashMap<String, String>) -> String {
    TOKEN
        .replace_all(text, |caps: &regex::Captures| match values.get(&caps[1]) {
            Some(value) => value.clone(),
            None => caps[0].to_string(),
        })
        .into_owned()
}

/// Текст без токенов и тегов.
pub fn outside_markers(text: &str) -> String {
    let text = TAG.replace_all(text, "");
    TOKEN.replace_all(&text, "").into_owned()
}

/// Буквы вне токенов и тегов.
fn letters_outside_markers(text: &str) -> usize {
    outside_markers(text).chars().filter(|c| c.is_alphabetic()).count()
}

/// Реплика, где кроме токенов и тегов нет букв, вроде «...» или «[mc]!»: её перевод совпадает с оригиналом.
pub fn is_wordless(text: &str) -> bool {
    letters_outside_markers(text) == 0
}

/// Можно ли взять перевод. Реплика без букв совпадает с оригиналом законно, как и смайлик вроде «=D»
/// с одной буквой: модель возвращала его без изменений.
pub fn usable_translation(original: &str, translated: &str) -> bool {
    let translated = translated.trim();
    !translated.is_empty()
        && markers(original) == markers(translated)
        && (translated != original.trim() || letters_outside_markers(original) <= 1)
}

/// Буквы текста в нижнем регистре: так сравниваются переводы без оглядки на знаки препинания.
pub fn letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphabetic()).flat_map(char::to_lowercase).collect()
}

/// Есть ли `name` в `text` отдельным словом. Оба уже в нижнем регистре.
pub fn mentions(text: &str, name: &str) -> bool {
    !name.is_empty()
        && text.match_indices(name).any(|(start, _)| {
            let before = text[..start].chars().next_back();
            let after = text[start + name.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_must_survive_translation() {
        assert_eq!(markers("{i}Hi{/i}, [mc]!"), ["[mc]", "{/i}", "{i}"]);
        assert!(usable_translation("Hi, [mc]!", "Привет, [mc]!"));
        assert!(!usable_translation("Hi, [mc]!", "Привет, Макс!"));
        assert!(!usable_translation("Hi, [mc]!", "Привет, [mc]! [mc]!"));
        assert!(!usable_translation("Hello.", "Hello."), "непереведённая реплика не перевод");
        assert!(usable_translation("=D", "=D"), "смайлик совпадает законно");
        assert!(!usable_translation("Hello.", "  "));
    }

    #[test]
    fn wordless_lines_and_mentions() {
        assert!(is_wordless("..."));
        assert!(is_wordless("[mc]!{w}"));
        assert!(!is_wordless("Oh, [mc]."));
        assert!(mentions("the silver key opens", "silver key"));
        assert!(!mentions("the silver keys", "silver key"));
        assert_eq!(letters("Да, да!"), "дада");
        assert_eq!(token_names("[mc] and [friend.name!c]").collect::<Vec<_>>(), ["mc", "friend.name"]);
    }
}
