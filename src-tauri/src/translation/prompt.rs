//! Промпт перевода и редактуры реплик.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::text::{is_wordless, mentions, token_names};
use super::{Batch, Hints, Scene, SceneLine};
use crate::llm::{ChatRequest, LlmTarget};
use crate::scan::SegmentId;

/// Лимит ответа без запаса на размышления: его добавляет `LlmClient` из профиля.
pub const LINE_ANSWER_TOKENS: u32 = 4096;
/// Предел подсказки терминов: в запрос идут термины, упомянутые в пакете.
const MAX_TERM_HINT_CHARS: usize = 1500;
/// Сколько языков имён с правилами идёт в один запрос: правила длинные.
const NAME_RULE_LANGUAGES: usize = 3;
/// Подпись реплики без говорящего: повествование и мысли героя.
const NARRATOR: &str = "Narrator";

const TRANSLATE_SYSTEM: &str = r#"You translate the dialogue of a visual novel from English into natural spoken Russian.
You receive a JSON array of lines in story order.

Rules:
1. Translate every item that has an "id", and return only those, with the same "id". Items without "id", including everything under "Earlier lines:", are context: never return or retranslate them. Their "translation" shows what is already written: stay consistent with its tone and with how characters address each other ("ты" or "вы"). Lines of "Narrator" are narration and the protagonist's thoughts: they are game text like any other and must be translated too.
2. Write as people really speak, not word for word, but keep everything the original says: do not drop, add or reinterpret anything. Keep the tone and the register, including rude language. "you" in narration usually means the protagonist. Every Russian sentence must be grammatical.
3. Idioms, set phrases and terms of endearment are translated by meaning, the way a Russian speaker would say it in that situation, never word by word: "Sleep well, honey." is "Спокойной ночи, милый." (or "милая", by the gender of the listener), not "Спи хорошо, медок."
4. Verbs and adjectives must agree with the gender of the speaker and of the person they speak to.
5. Text in [square brackets] is an in-game variable and text in {curly brackets} is a text tag. Copy each of them exactly as written, the same number of times, and never translate them. Never add a variable or tag the line does not have: a name written as plain text, like "MC", stays plain text. The game replaces a variable with its value, so build the sentence so that it reads correctly without declining the variable. Still translate the words around it: "My [roommate] Katy" is "Моя [roommate] Кэти", with the gender taken from the person the variable names.
6. Translate every line, including short ones and interjections. "..." stays "...". Nonsense words, scat singing, onomatopoeia and deliberately garbled speech have no hidden meaning: render them as similar-sounding Russian nonsense, never replace them with words you guessed.
7. A line starting with "Scene:" describes what happens. A line starting with "Speakers:" describes who talks. A line starting with "Tokens:" explains the variables. They are context: never translate or quote them.
8. A line starting with "Russian names:" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. A line starting with "Gender:" tells who is male or female. A line starting with "Name rules:" tells how to render names that are not listed.
9. A line starting with "Known terms:" gives established translations: use them.

Respond ONLY with a valid JSON array of objects:
[
  {"id": "12", "text": "Где ты была всю ночь, [mc]?"}
]"#;

const REVIEW_SYSTEM: &str = r#"You are the editor of a Russian translation of a visual novel. You receive a JSON array of lines in story order: each item with an "id" has the English "text" and its Russian "draft".

Check every draft and fix only real errors:
- the meaning is wrong, or something is dropped or added;
- verbs and adjectives do not agree with the gender of the speaker or of the person they speak to;
- the Russian is ungrammatical or garbled, or contains letters of other alphabets or untranslated English words;
- an idiom, set phrase or term of endearment is translated word by word instead of by meaning;
- names and terms do not follow "Russian names:" and "Known terms:";
- the tone or register differs from the original, including rude language, which must stay rude.
Do not rewrite a line that is correct only because you would phrase it differently. Lines of "Narrator" are narration and the protagonist's thoughts.

Text in [square brackets] is an in-game variable and text in {curly brackets} is a text tag: the corrected line must contain each of them exactly as in the original, the same number of times. Items without "id", including everything under "Earlier lines:", are context: never return them. Lines starting with "Scene:", "Speakers:", "Tokens:", "Russian names:", "Gender:", "Name rules:" and "Known terms:" are context too.

Respond ONLY with a valid JSON array of the lines you corrected, each with the full corrected line and a short reason in Russian. Return [] if every draft is fine:
[
  {"id": "12", "text": "Спокойной ночи, милый.", "issue": "Обращение переведено дословно и не в том роде"}
]"#;

/// Перевод заполняет пустые реплики, редактура получает реплики с полем `draft` и возвращает исправленные.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinesMode {
    Translate,
    Review,
}

impl LinesMode {
    /// Нужна ли реплика этому режиму. Реплика без букв совпадает с оригиналом: переводить и вычитывать в ней нечего.
    pub fn wants(self, line: &SceneLine) -> bool {
        let blank = line.is_blank();
        let wanted = match self {
            Self::Translate => blank,
            Self::Review => !blank,
        };
        wanted && !is_wordless(&line.source_text)
    }
}

/// Реплика в запросе. Поля опущены, а не пусты: модель копировала пустой перевод в ответ.
#[derive(Serialize)]
struct Item<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    speaker: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<&'a str>,
    /// Перевод, который проверяет редактура.
    #[serde(skip_serializing_if = "Option::is_none")]
    draft: Option<&'a str>,
}

pub struct LinesPrompt;

impl LinesPrompt {
    /// Подписи говорящих, описание сцены, подсказки глоссария. Реплики пакета получают номер с единицы по порядку
    /// `batch.lines`: по нему `LinesAnswer::accept` найдёт реплику. Реплики перед пакетом идут отдельным блоком:
    /// в одном списке с переводимыми слабая модель переводила их заново под выдуманными номерами.
    pub fn build<'a>(mode: LinesMode, scene: &'a Scene, batch: &Batch, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let numbers: HashMap<SegmentId, usize> = batch.lines.iter().enumerate().map(|(index, id)| (*id, index + 1)).collect();
        let shown_ids: HashSet<SegmentId> = batch.lines.iter().chain(&batch.context).copied().collect();
        let shown: Vec<&'a SceneLine> = scene.lines.iter().filter(|line| shown_ids.contains(&line.id)).collect();
        let first = shown.iter().position(|line| numbers.contains_key(&line.id)).unwrap_or(shown.len());

        let item = |line: &&'a SceneLine| -> Item<'a> {
            let number = numbers.get(&line.id);
            let written = Some(line.text.as_str()).filter(|text| !text.trim().is_empty());
            Item {
                id: number.map(usize::to_string),
                speaker: line.speaker.as_deref().unwrap_or(NARRATOR),
                text: &line.source_text,
                translation: written.filter(|_| number.is_none()),
                draft: written.filter(|_| number.is_some() && mode == LinesMode::Review),
            }
        };
        let earlier: Vec<Item> = shown[..first].iter().map(item).collect();
        let items: Vec<Item> = shown[first..].iter().map(item).collect();

        let mut context = Vec::new();
        if let Some(description) = scene.description.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            context.push(format!("Scene: {description}"));
        }
        context.extend(speakers_hint(&shown));
        let text: String = shown.iter().map(|line| format!("{}\n{}\n", line.speaker.as_deref().unwrap_or(NARRATOR), line.source_text)).collect();
        context.extend(glossary_hints(&text, hints));
        let lines: String = shown.iter().map(|line| format!("{}\n", line.source_text)).collect();
        context.extend(name_rules_hint(&lines, hints));
        if !earlier.is_empty() {
            context.push(format!("Earlier lines:\n{}", json(&earlier)));
        }
        let context: String = context.iter().map(|block| format!("{block}\n\n")).collect();

        ChatRequest {
            system: match mode {
                LinesMode::Translate => TRANSLATE_SYSTEM,
                LinesMode::Review => REVIEW_SYSTEM,
            }
            .to_string(),
            user: format!("{context}Lines:\n{}\n\nProvide JSON array now:", json(&items)),
            temperature: target.settings.temperature,
            max_tokens: LINE_ANSWER_TOKENS,
        }
    }
}

/// Список реплик в запрос. Сериализация строк и чисел не падает.
fn json(items: &[Item]) -> String {
    serde_json::to_string_pretty(items).unwrap_or_default()
}

/// Подсказки глоссария по тексту запроса: пояснения токенов, имена с родом, термины. Ими пользуются
/// и предложения модели в глоссарии.
pub(crate) fn glossary_hints(text: &str, hints: &Hints) -> Vec<String> {
    token_hint(text, hints).into_iter().chain(name_hint(text, hints)).chain(term_hint(text, hints)).collect()
}

/// Правила языков имён, упомянутых в тексте реплик: как имя пишется и склоняется. Не больше
/// `NAME_RULE_LANGUAGES` языков, первыми те, у кого больше упомянутых имён.
fn name_rules_hint(text: &str, hints: &Hints) -> Option<String> {
    let lower = text.to_lowercase();
    let mut languages: Vec<(&str, Vec<&str>)> = Vec::new();
    for (name, codes) in &hints.name_origins {
        if !mentions(&lower, &name.to_lowercase()) {
            continue;
        }
        for code in codes.iter().filter(|code| hints.name_rules.iter().any(|(known, _)| known == *code)) {
            match languages.iter_mut().find(|(known, _)| known == code) {
                Some((_, names)) if !names.contains(&name.as_str()) => names.push(name),
                Some(_) => {}
                None => languages.push((code, vec![name])),
            }
        }
    }
    languages.sort_by_key(|(_, names)| std::cmp::Reverse(names.len()));
    let blocks: Vec<String> = languages
        .iter()
        .take(NAME_RULE_LANGUAGES)
        .filter_map(|(code, names)| {
            let (_, rules) = hints.name_rules.iter().find(|(known, _)| known == code)?;
            Some(format!("[{code}] {}\n{}", names.join(", "), rules.trim()))
        })
        .collect();
    (!blocks.is_empty()).then(|| format!("Name rules (how names of each origin are spelled and declined in Russian):\n{}", blocks.join("\n\n")))
}

/// Род говорящих, если он известен.
fn speakers_hint(lines: &[&SceneLine]) -> Option<String> {
    let mut seen = HashSet::new();
    let known: Vec<String> = lines
        .iter()
        .filter_map(|line| Some((line.speaker.as_deref()?, line.speaker_gender?)))
        .filter(|(speaker, _)| seen.insert(*speaker))
        .map(|(speaker, gender)| format!("{speaker}: {}", gender.as_str()))
        .collect();
    (!known.is_empty()).then(|| format!("Speakers:\n{}", known.join("\n")))
}

/// Пояснения к токенам без значения, которые есть в тексте.
fn token_hint(text: &str, hints: &Hints) -> Option<String> {
    let names: HashSet<&str> = token_names(text).collect();
    let mut notes: Vec<String> = hints
        .tokens
        .iter()
        .filter(|(name, note)| names.contains(name.as_str()) && !note.trim().is_empty())
        .map(|(name, note)| format!("[{name}] = {}", note.trim()))
        .collect();
    notes.sort_unstable();
    (!notes.is_empty()).then(|| format!("Tokens: {}", notes.join("; ")))
}

/// Переводы имён тех, кто упомянут в тексте. Род идёт отдельной строкой: пометку рядом с именем модель переписывала в ответ.
fn name_hint(text: &str, hints: &Hints) -> Option<String> {
    let lower = text.to_lowercase();
    let mut seen = HashSet::new();
    let named: Vec<_> = hints
        .names
        .iter()
        .filter(|(original, translated, _)| {
            !translated.trim().is_empty() && seen.insert(original.as_str()) && mentions(&lower, &original.to_lowercase())
        })
        .collect();
    if named.is_empty() {
        return None;
    }
    let pairs: Vec<String> = named.iter().map(|(original, translated, _)| format!("{original} = {translated}")).collect();
    let genders: Vec<String> =
        named.iter().filter_map(|(original, _, gender)| Some(format!("{original} {}", gender.as_ref()?.as_str()))).collect();
    let mut hint = format!("Russian names: {}", pairs.join(", "));
    if !genders.is_empty() {
        hint.push_str(&format!("\nGender: {}", genders.join(", ")));
    }
    Some(hint)
}

/// Термины глоссария, которые есть в тексте, в пределах `MAX_TERM_HINT_CHARS`.
fn term_hint(text: &str, hints: &Hints) -> Option<String> {
    let lower = text.to_lowercase();
    let mut size = 0;
    let pairs: Vec<String> = hints
        .terms
        .iter()
        .filter(|(original, translated)| !translated.trim().is_empty() && mentions(&lower, &original.trim().to_lowercase()))
        .map(|(original, translated)| format!("{} = {}", original.trim(), translated.trim()))
        .take_while(|pair| {
            size += pair.chars().count() + 2;
            size <= MAX_TERM_HINT_CHARS
        })
        .collect();
    (!pairs.is_empty()).then(|| format!("Known terms: {}", pairs.join(", ")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::common::Gender;
    use crate::llm::{Action, ActionSettings, Profile, Provider, ProviderKind};
    use crate::translation::batcher::tests::{line, scene};

    pub fn target() -> LlmTarget {
        let provider = Provider::new("LM Studio", ProviderKind::Local, "http://127.0.0.1:1234/v1", 0).unwrap();
        let profile = Profile::new(provider.id(), "", "gemma", 0).unwrap();
        LlmTarget { provider, profile, key: None, settings: ActionSettings::default().resolve(Action::Lines) }
    }

    #[test]
    fn earlier_lines_are_separate_and_numbers_follow_the_batch() {
        let mut lines = vec![line(1, "Hi, Anna.", "Привет, Анна."), line(2, "Where were you?", ""), line(3, "...", ""), line(4, "Home.", "")];
        lines[1].speaker = Some("Max".to_string());
        lines[1].speaker_gender = Some(Gender::Male);
        let scene = Scene { description: Some("Утро на кухне.".to_string()), ..scene(lines) };
        let batch = Batch {
            lines: vec![SegmentId::from_row(2), SegmentId::from_row(4)],
            context: vec![SegmentId::from_row(1), SegmentId::from_row(3)],
        };
        let hints = Hints {
            names: vec![("Anna".to_string(), "Анна".to_string(), Some(Gender::Female))],
            terms: vec![("Silver Key".to_string(), "Серебряный ключ".to_string())],
            ..Default::default()
        };
        let request = LinesPrompt::build(LinesMode::Translate, &scene, &batch, &hints, &target());
        let user = &request.user;

        assert!(user.starts_with("Scene: Утро на кухне."));
        assert!(user.contains("Speakers:\nMax: male"));
        assert!(user.contains("Russian names: Anna = Анна\nGender: Anna female"));
        assert!(!user.contains("Known terms"), "термина нет в тексте");
        let (earlier, lines) = user.split_once("Lines:\n").unwrap();
        assert!(earlier.contains("\"translation\": \"Привет, Анна.\""));
        assert!(!earlier.contains("\"id\""));
        assert!(lines.contains("\"id\": \"1\",\n    \"speaker\": \"Max\""));
        assert!(lines.contains("\"id\": \"2\""));
        assert!(lines.contains("\"speaker\": \"Narrator\",\n    \"text\": \"...\"\n"), "готовая без перевода реплика идёт без полей");
        assert_eq!(request.max_tokens, LINE_ANSWER_TOKENS);
        assert_eq!(request.temperature, Action::Lines.default_temperature());
    }

    #[test]
    fn name_rules_come_for_names_mentioned_in_the_lines_only() {
        let mut lines = vec![line(1, "Mrs. Karanovic called Heike.", ""), line(2, "Tell Karanovic.", "")];
        lines[0].speaker = Some("Yuki".to_string());
        let scene = scene(lines);
        let batch = Batch { lines: vec![SegmentId::from_row(1), SegmentId::from_row(2)], context: Vec::new() };
        let codes = |codes: &[&str]| codes.iter().map(|code| code.to_string()).collect::<Vec<_>>();
        let hints = Hints {
            name_origins: vec![
                ("Karanovic".to_string(), codes(&["sr", "hr"])),
                ("Mrs. Karanovic".to_string(), codes(&["sr"])),
                ("Heike".to_string(), codes(&["de"])),
                ("Yuki".to_string(), codes(&["ja"])),
            ],
            name_rules: vec![
                ("sr".to_string(), "j это й".to_string()),
                ("de".to_string(), "ei это ай".to_string()),
                ("ja".to_string(), "shi это си".to_string()),
            ],
            ..Default::default()
        };
        let user = LinesPrompt::build(LinesMode::Translate, &scene, &batch, &hints, &target()).user;
        assert!(
            user.contains("Name rules (how names of each origin are spelled and declined in Russian):\n[sr] Karanovic, Mrs. Karanovic\nj это й\n\n[de] Heike\nei это ай\n\n"),
            "{user}"
        );
        assert!(!user.contains("shi это си"), "имя говорящего над репликой правил не просит");
        assert!(!user.contains("[hr]"), "у hr правил нет");
    }

    #[test]
    fn review_sends_the_draft_of_numbered_lines() {
        let scene = scene(vec![line(1, "Sleep well, honey.", "Спи хорошо, медок.")]);
        let batch = Batch { lines: vec![SegmentId::from_row(1)], context: Vec::new() };
        let request = LinesPrompt::build(LinesMode::Review, &scene, &batch, &Hints::default(), &target());
        assert!(request.system.starts_with("You are the editor"));
        assert!(request.user.contains("\"draft\": \"Спи хорошо, медок.\""));
        assert!(!request.user.contains("\"translation\""));
        assert!(!request.user.contains("Earlier lines"));
    }
}
