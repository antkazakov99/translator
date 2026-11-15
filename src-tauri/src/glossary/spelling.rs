//! Строковые правила написаний: чем одно написание приходится другому (аббревиатура, короткая форма,
//! множественное число, опечатка) и что написание значит само (должность перед именем, обрывок, обычное слово).
//! Правила ничего не решают о записях глоссария: по ним `relations.rs` называет отношения, а `resolve.rs`
//! раскладывает находку на входе.

use crate::translation::text::mentions;

/// Обращения перед именем: не должность, термином не становятся.
const HONORIFICS: [&str; 10] = ["mr", "mrs", "ms", "miss", "mx", "dr", "sir", "lady", "lord", "madam"];

/// Служебные слова, которые аббревиатура пропускает: «People's Police Force» это PPF, «Order of the Dawn» это OD.
const MINOR_WORDS: [&str; 10] = ["of", "the", "and", "for", "a", "an", "in", "on", "at", "to"];

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

/// `long` это полная форма имени `short`: одно слово, длиннее и начинается с него («Stuart» у «Stu»).
pub fn is_fuller(long: &str, short: &str) -> bool {
    let (long, short) = (long.trim().to_lowercase(), short.trim().to_lowercase());
    !short.is_empty() && !long.contains(char::is_whitespace) && !short.contains(char::is_whitespace) && long.len() > short.len() && long.starts_with(&short)
}

/// Два написания это варианты одного имени или одной фамилии: одно начинает другое («Alexa» и «Alexandra»,
/// «Jovanka» и «Jovankam») или отличается опечаткой («Camila» и «Camilla»). Какое из них главное, правило
/// не говорит: это решает частота в игре.
pub fn name_variant(one: &str, other: &str) -> bool {
    is_fuller(one, other) || is_fuller(other, one) || typo(one, other)
}

/// Слова названия без знаков: по ним сравниваются написания.
fn bare_words(name: &str) -> Vec<String> {
    name.split(|c: char| c.is_whitespace() || c == '-')
        .map(|word| word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Сокращение слова не короче этого.
const MIN_CUT_WORD: usize = 2;
/// Слово длиннее своего сокращения хотя бы на столько букв: «points» не сокращение «point».
const MIN_CUT_GAP: usize = 2;

/// `short` сокращает слово `long`: та же первая буква, а остальные идут в слове в том же порядке
/// («dept» у «department», «mk» у «mark», «lab» у «laboratory»). Оба без знаков и строчными.
fn cuts_word(short: &str, long: &str) -> bool {
    let (short_chars, long_chars) = (short.chars().count(), long.chars().count());
    if short_chars < MIN_CUT_WORD || long_chars < short_chars + MIN_CUT_GAP || short.chars().next() != long.chars().next() {
        return false;
    }
    let mut rest = long.chars();
    short.chars().all(|letter| rest.any(|other| other == letter))
}

/// Два названия похожи на одно, записанное по-разному: те же слова, слитно или раздельно («Night life»
/// и «Nightlife», «Mk2» и «Mk 2»), или одно из слов сокращено («Dept. of Knowledge» и «Department of
/// Knowledge»). Одно слово сокращением другого считается только с точкой («Dept.» и «Department»): без неё
/// похожими стали бы «Mine» и «Machine». Правило грубое: оно даёт пару для вопроса модели, а не слияние.
pub fn respelled(one: &str, other: &str) -> bool {
    let (a, b) = (bare_words(one), bare_words(other));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.concat() == b.concat() {
        return true;
    }
    if a.len() != b.len() {
        return false;
    }
    let dotted = |name: &str| name.trim().ends_with('.');
    if a.len() == 1 {
        return (dotted(one) && cuts_word(&a[0], &b[0])) || (dotted(other) && cuts_word(&b[0], &a[0]));
    }
    let cut = |x: &str, y: &str| cuts_word(x, y) || cuts_word(y, x);
    let same = a.iter().zip(&b).filter(|(x, y)| x == y).count();
    same > 0 && same < a.len() && a.iter().zip(&b).all(|(x, y)| x == y || cut(x, y))
}

/// Должности и звания перед именем: по ним «Detective-Colonel Karanovic» делится на должность и имя.
const RANKS: [&str; 34] = [
    "detective", "colonel", "general", "major", "captain", "lieutenant", "sergeant", "corporal", "private", "officer", "agent", "inspector",
    "commander", "admiral", "chief", "director", "president", "doctor", "professor", "nurse", "coach", "principal", "judge", "senator",
    "king", "queen", "prince", "princess", "duke", "count", "father", "sister", "brother", "deputy",
];

/// Должность или обращение перед именем: «Detective-Colonel» и «Karanovic» у «Detective-Colonel Karanovic»,
/// «Mr.» и «Lee» у «Mr. Lee». Должность со служебными словами берёт после каждого из них одно слово:
/// «Chief of Engineering» и «Ristovski» у «Chief of Engineering Ristovski». `None`: впереди нет должности
/// или после неё не осталось имени («Director of Engineering» это должность целиком).
pub fn title_and_name(name: &str) -> Option<(String, String)> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let titled = |word: &str| {
        word.split('-').all(|piece| {
            let piece = piece.trim_matches('.').to_lowercase();
            !piece.is_empty() && (RANKS.contains(&piece.as_str()) || HONORIFICS.contains(&piece.as_str()))
        })
    };
    let mut count = words.iter().take_while(|word| titled(word)).count();
    if count == 0 {
        return None;
    }
    while words.get(count).is_some_and(|word| MINOR_WORDS.contains(word)) {
        count += words[count..].iter().take_while(|word| MINOR_WORDS.contains(*word)).count() + 1;
    }
    if count >= words.len() {
        return None;
    }
    Some((words[..count].join(" "), words[count..].join(" ")))
}

/// Обрывок названия, а не имя: начинается со служебного слова строчными («of Engineering», «the Guard»).
/// «The Fairy Queen» с заглавным артиклем обрывком не считается.
pub fn is_fragment(name: &str) -> bool {
    name.split_whitespace().next().is_some_and(|word| MINOR_WORDS.contains(&word))
}

/// Имя без артикля впереди: «Fairy Queen» у «The Fairy Queen».
pub fn without_article(name: &str) -> &str {
    let name = name.trim();
    for article in ["the ", "a ", "an "] {
        if name.len() > article.len() && name.is_char_boundary(article.len()) && name[..article.len()].eq_ignore_ascii_case(article) {
            let rest = name[article.len()..].trim_start();
            if !rest.is_empty() {
                return rest;
            }
        }
    }
    name
}

/// Титул как имя одного лица: несколько слов, последнее из них звание («Fairy Queen», «The Fairy King»).
/// «Order of the King» с предлогом это название, а не лицо.
pub fn is_person_title(name: &str) -> bool {
    let words: Vec<String> = without_article(name).split_whitespace().map(str::to_lowercase).collect();
    words.len() >= 2 && words.last().is_some_and(|last| RANKS.contains(&last.as_str())) && !words.iter().any(|word| MINOR_WORDS.contains(&word.as_str()))
}

/// Одно название множественное число другого: «pixies» у «pixie», «truthsayers» у «truthsayer».
pub fn plural_of(plural: &str, single: &str) -> bool {
    let (plural, single) = (plural.trim().to_lowercase(), single.trim().to_lowercase());
    single.chars().count() >= 3 && (plural == format!("{single}s") || plural == format!("{single}es"))
}

/// Имя из одного слова это обычное слово, а не имя: в репликах игры оно есть со строчной буквы и ни разу
/// с заглавной посреди фразы («Boobjob» строкой выбора в меню и «a boobjob» в репликах). Имя, которое модель
/// сама написала строчными, тоже обычное слово.
pub fn is_common_word(name: &str, lines: &[String]) -> bool {
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) || !name.chars().any(char::is_alphabetic) {
        return false;
    }
    let lower = name.to_lowercase();
    if lower == name {
        return true;
    }
    let mut lowercase = false;
    for line in lines {
        for (start, _) in line.match_indices(name) {
            let after = line[start + name.len()..].chars().next();
            let before = line[..start].trim_end().chars().next_back();
            let whole = !line[..start].chars().next_back().is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric);
            if whole && before.is_some_and(|c| c.is_alphanumeric() || c == ',') {
                return false;
            }
        }
        lowercase |= mentions(line, &lower);
    }
    lowercase
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

/// Одна аббревиатура, записанная с точками и без: «P.P.F.» и «PPF».
pub fn same_acronym(one: &str, other: &str) -> bool {
    matches!((acronym(one), acronym(other)), (Some(one), Some(other)) if one == other)
}

/// Опечатка не короче этого: в коротких именах одна буква это другое имя («Maria» и «Marta»).
const MIN_TYPO_CHARS: usize = 6;

/// Два написания отличаются одной опечаткой: одна буква заменена, пропущена, лишняя или две соседние
/// переставлены («Karanovic» и «Karanovich», «Dimitria» и «Dimitira»). Регистр не учитывается, оба не короче
/// `MIN_TYPO_CHARS`.
pub fn typo(one: &str, other: &str) -> bool {
    let a: Vec<char> = one.trim().to_lowercase().chars().collect();
    let b: Vec<char> = other.trim().to_lowercase().chars().collect();
    if a.len().min(b.len()) < MIN_TYPO_CHARS || a == b {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    match long.len() - short.len() {
        0 => {
            let differ: Vec<usize> = (0..short.len()).filter(|&index| short[index] != long[index]).collect();
            match differ.as_slice() {
                [_] => true,
                [first, second] => second - first == 1 && short[*first] == long[*second] && short[*second] == long[*first],
                _ => false,
            }
        }
        1 => {
            let same = short.iter().zip(long.iter()).take_while(|(a, b)| a == b).count();
            short[same..] == long[same + 1..]
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(same_acronym("P.P.F.", "PPF") && !same_acronym("PPF", "PPA") && !same_acronym("Force", "Force"));
    }

    #[test]
    fn titles_split_off_the_name() {
        assert_eq!(title_and_name("Detective-Colonel Karanovic"), Some(("Detective-Colonel".to_string(), "Karanovic".to_string())));
        assert_eq!(title_and_name("Mr. Lee"), Some(("Mr.".to_string(), "Lee".to_string())));
        assert_eq!(title_and_name("Captain Jack Morgan"), Some(("Captain".to_string(), "Jack Morgan".to_string())));
        assert_eq!(title_and_name("Vera Karanovic"), None);
        assert_eq!(title_and_name("Captain"), None, "без имени после должности делить нечего");
        assert_eq!(title_and_name("Director of Engineering"), None, "должность со служебным словом это одна должность");
        assert_eq!(title_and_name("Chief of the Guard"), None);
        assert_eq!(title_and_name("Chief of Engineering Ristovski"), Some(("Chief of Engineering".to_string(), "Ristovski".to_string())));
        assert_eq!(title_and_name("Queen of the Fairies Titania"), Some(("Queen of the Fairies".to_string(), "Titania".to_string())));
        assert_eq!(title_and_name("Captain of"), None);
        assert!(is_fragment("of Engineering") && is_fragment("the Guard"));
        assert!(!is_fragment("The Fairy Queen") && !is_fragment("Karanovic"));
        assert!(is_common_title("Mr.") && is_common_title("Princess") && is_common_title(" sister "));
        assert!(is_token_label("[mc]") && is_token_label(" [player_name] "));
        assert!(!is_token_label("[mc]'s mom") && !is_token_label("[]") && !is_token_label("[a] [b]") && !is_token_label("Eileen"));
        assert!(!is_common_title("Detective-Colonel") && !is_common_title("Grand Duke") && !is_common_title("Selena"));
    }

    #[test]
    fn common_words_articles_titles_and_plurals() {
        let lines = |texts: &[&str]| texts.iter().map(|text| text.to_string()).collect::<Vec<_>>();
        let game = lines(&["Boobjob", "Give him a... boobjob.", "Are you ready for your b-boobjob?", "Merek", "I saw Merek today.", "Crone! Ask the crone, Crone knows."]);
        assert!(is_common_word("Boobjob", &game), "только строчными и строкой меню");
        assert!(!is_common_word("Merek", &game) && !is_common_word("Crone", &game), "с заглавной посреди фразы это имя");
        assert!(!is_common_word("Kastia", &game), "строчными не встречается");
        assert!(is_common_word("boobjob", &game) && !is_common_word("Fairy Queen", &game));

        assert_eq!(without_article("The Fairy Queen"), "Fairy Queen");
        assert_eq!((without_article("the Order"), without_article("Theodore"), without_article("The")), ("Order", "Theodore", "The"));
        assert!(is_person_title("Fairy Queen") && is_person_title("The Fairy King"));
        assert!(!is_person_title("Queen") && !is_person_title("Order of the King") && !is_person_title("Ancient Forest"));
        assert!(plural_of("pixies", "pixie") && plural_of("Witches", "witch") && !plural_of("pixie", "pixies") && !plural_of("as", "a"));
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
    fn respelled_names_share_words_or_cut_one() {
        assert!(respelled("Dept. of Knowledge", "Department of Knowledge"));
        assert!(respelled("Department of Measure", "dept. of measure"), "регистр не важен");
        assert!(respelled("Night life", "Nightlife") && respelled("CREED Mk2", "CREED Mk 2") && respelled("Counter-intelligence", "Counter intelligence"));
        assert!(respelled("Dept.", "Department") && !respelled("Dept", "Department"), "одно слово только с точкой");
        assert!(!respelled("Mine", "Machine") && !respelled("Voyeur", "Voyeurism"));
        assert!(!respelled("Fey Court", "Fairy Court"), "синоним не сокращение");
        assert!(!respelled("Department of Measure", "Department of Knowledge") && !respelled("Research point", "Research points"));
        assert!(!respelled("Security tasks", "Insurgence tasks") && !respelled("Dept. of Knowledge", "Department of the Interior"));
        assert!(!respelled("Mk II", "Mark Two"), "хотя бы одно слово общее");
    }

    #[test]
    fn name_variants_start_alike_or_differ_by_a_typo() {
        assert!(name_variant("Alexa", "Alexandra") && name_variant("Jovankam", "Jovanka") && name_variant("Veraaaa", "Vera"));
        assert!(name_variant("Camila", "Camilla"));
        assert!(!name_variant("Rita", "Margaret") && !name_variant("Maria", "Marta") && !name_variant("Vera", "Vera"));
    }

    #[test]
    fn typos_differ_by_one_letter() {
        assert!(typo("Karanovic", "Karanovich"), "лишняя буква");
        assert!(typo("Dimitria", "dimitira"), "соседние буквы переставлены, регистр не важен");
        assert!(typo("Vellnikov", "Velnikov") && typo("Tominskova", "Tominskowa"));
        assert!(!typo("Maria", "Marta"), "короткие имена разные");
        assert!(!typo("Karanovic", "Karanovic") && !typo("Karanovic", "Karanovski") && !typo("Dimitria", "Dimitrios"));
    }
}
