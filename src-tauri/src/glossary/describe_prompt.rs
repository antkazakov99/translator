//! Запросы описаний сцен: заметка по части, сжатие заметок, сборка описания, название. Ответы это обычный текст.

use std::sync::LazyLock;

use regex::Regex;

use super::parts::MAX_PART_TEXT;
use super::suggest::hints_block;
use crate::llm::{ChatRequest, LlmTarget};
use crate::translation::Hints;

const NOTE_ANSWER_TOKENS: u32 = 2048;
const TITLE_ANSWER_TOKENS: u32 = 768;
/// Название длиннее этого модель написала не как название.
const MAX_TITLE_CHARS: usize = 80;
/// Отказ длиннее этого уже содержит факты.
const MAX_REFUSAL_CHARS: usize = 120;

const PART_SYSTEM: &str = "You read a part of a visual novel scene and describe it for its translator into Russian. \
The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
Write in Russian, 2 to 5 short sentences, only what this text shows: what happens, who takes part, \
the mood, and how they address each other (\"ты\" or \"вы\"). \
Text in [square brackets] is an in-game variable: never infer anything from its name. \
A line starting with \"Tokens:\" explains such variables: rely on it, but never copy it word for word. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Write only the note text, without headings, labels or Markdown.";

const CONDENSE_SYSTEM: &str = "You merge notes about one scene of a visual novel. Write in Russian. \
Keep concrete facts: events, who takes part, relations, mood, how they address each other. Drop repetitions. \
Respond with plain text only.";

const SCENE_SYSTEM: &str = "You describe one scene of a visual novel for its translator into Russian. \
You receive notes about the parts of the scene. Write in Russian, 3 to 6 sentences, plain text only: \
what happens, who takes part, who they are to each other, the mood, and how they address each other (\"ты\" or \"вы\"). \
Use only the notes and do not invent facts. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Write only the description, without headings, labels or Markdown.";

const TITLE_SYSTEM: &str = "You name a scene of a visual novel for a list of scenes. You receive its description in Russian. \
First decide what the scene does for the story as a whole: introduces someone, a quarrel, a meeting, a confession, a decision, an everyday episode. \
Then answer in Russian with a neutral title of 2 to 6 words, a noun phrase and not a sentence, that names this: who and what the scene is about. \
Do not name a single action or detail from the middle of the scene, and do not add intrigue. \
Keep names as they are written in the description. One line of plain text, no quotes and no final period.";

/// Без этого вступление, которое знакомит с героем, получает название по самому заметному поступку.
const OPENING_NOTE: &str = "\n\nThis is the first scene of the game.";

/// Ответы без содержания и отказы: в описание они не идут.
const BLANK_NOTES: [&str; 3] = ["none", "empty string", "пустая строка"];
const REFUSALS: [&str; 5] = ["нет информации", "нет сведений", "не содержит информации", "ничего не сказано", "не упоминается"];

/// Заметка по одной части сцены.
pub struct PartPrompt;

impl PartPrompt {
    pub fn build(part: &str, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: PART_SYSTEM.to_string(),
            user: format!("{}Script:\n\n{part}\nNotes about this scene:", hints_block(part, hints)),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }
}

/// Заметки, которые вместе длиннее запроса, сжимаются группами.
pub struct CondensePrompt;

impl CondensePrompt {
    pub fn build(notes: &[String], target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: CONDENSE_SYSTEM.to_string(),
            user: format!("Notes:\n\n{}\n\nMerge them:", notes.join("\n\n")),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }

    /// Группы подряд идущих заметок, каждая в пределах запроса.
    pub fn groups(notes: Vec<String>) -> Vec<Vec<String>> {
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut size = 0;
        for note in notes {
            let length = note.chars().count();
            match groups.last_mut() {
                Some(group) if size + length <= MAX_PART_TEXT => {
                    group.push(note);
                    size += length;
                }
                _ => {
                    groups.push(vec![note]);
                    size = length;
                }
            }
        }
        groups
    }

    /// Заметки уже помещаются в запрос сборки.
    pub fn fits(notes: &[String]) -> bool {
        notes.iter().map(|note| note.chars().count()).sum::<usize>() <= MAX_PART_TEXT
    }
}

/// Описание сцены по заметкам её частей. Подсказки глоссария берутся по тексту всей сцены.
pub struct ScenePrompt;

impl ScenePrompt {
    pub fn build(notes: &[String], scene_text: &str, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: SCENE_SYSTEM.to_string(),
            user: format!("{}Notes about this scene:\n\n{}", hints_block(scene_text, hints), notes.join("\n\n")),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }
}

/// Название по готовому описанию.
pub struct TitlePrompt;

impl TitlePrompt {
    pub fn build(description: &str, opening: bool, target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: format!("{TITLE_SYSTEM}{}", if opening { OPENING_NOTE } else { "" }),
            user: description.to_string(),
            temperature: target.settings.temperature,
            max_tokens: TITLE_ANSWER_TOKENS,
        }
    }

    /// Первая непустая строка без кавычек и точки. `None`: модель не написала названия.
    pub fn accept(answer: &str) -> Option<String> {
        let line = answer.lines().map(str::trim).find(|line| !line.is_empty())?;
        let line = line.trim_matches(|c: char| c.is_whitespace() || "\"'«»“”.*#".contains(c));
        let title: String = line.chars().take(MAX_TITLE_CHARS).collect();
        (!title.trim().is_empty()).then(|| title.trim().to_string())
    }
}

const CHARACTER_PART_SYSTEM: &str = "You read a part of a visual novel script and take notes about the character {name}. \
The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
A line \"---\" separates scenes that may be far apart in the story. \
Write in Russian, 2 to 5 short sentences, only what this text shows: what the character does, \
occupation, relations with others, and how they speak (tone, slang, \"ты\" or \"вы\"). \
If the character is neither named nor speaks anywhere in the text, answer only with: Нет сведений о персонаже. \
Otherwise always write notes, even if the character appears only briefly. \
Text in [square brackets] is an in-game variable: never infer anything from its name. \
A line starting with \"Tokens:\" explains such variables: rely on it, but never copy it word for word. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Write only the note text, without headings, labels or Markdown.";

const TERM_PART_SYSTEM: &str = "You read a part of a visual novel script and take notes about the term {name}. \
It may be a place, an organization, an item, a skill, a title, an event or another concept of the game. \
The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
A line \"---\" separates scenes that may be far apart in the story. \
Write in Russian, 1 to 3 short sentences, only what this text shows: what it is, \
who or what it is connected with, and what happens with it. \
If the text says nothing about it, answer only with: Нет сведений о термине. \
Text in [square brackets] is an in-game variable: never infer anything from its name. \
A line starting with \"Tokens:\" explains such variables: rely on it, but never copy it word for word. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Write only the note text, without headings, labels or Markdown.";

const CHARACTER_SYSTEM: &str = "You write a glossary entry about one character of a visual novel for its translator into Russian. \
You receive notes about the character collected from the game. Answer with a JSON object with two string fields. \
\"description\": 2 to 5 sentences in Russian: who the character is, occupation, relations with others, place in the story. \
\"speech\": one short sentence in Russian on how the character speaks: tone, slang, \"ты\" or \"вы\" and to whom; \
an empty string if the notes say nothing about it. \
Use only the notes and do not invent facts. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Respond with the JSON object only.";

const TERM_SYSTEM: &str = "You write a glossary entry for a translator of a visual novel into Russian. \
You receive notes about one term collected from the game. Write in Russian, 2 to 4 sentences, plain text only: \
what it is, who or what it is connected with, and its place in the story. \
Use only the notes, do not invent facts, and do not translate the term itself in a new way. \
A line starting with \"Russian names:\" gives the exact spelling of names: always use it, declining the name as Russian grammar requires. \
A line starting with \"Gender:\" tells who is male or female. \
A line starting with \"Known terms:\" gives established translations: use them. \
Never copy these glossary hint lines into your answer. Write only the description, without headings, labels or Markdown.";

/// Чья запись описывается: от этого зависят запросы части и сборки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Character,
    Term,
}

/// Заметка о персонаже или термине по одной части его сцен.
pub struct EntryPartPrompt;

impl EntryPartPrompt {
    pub fn build(kind: EntryKind, name: &str, part: &str, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        let system = match kind {
            EntryKind::Character => CHARACTER_PART_SYSTEM,
            EntryKind::Term => TERM_PART_SYSTEM,
        };
        ChatRequest {
            system: system.replace("{name}", name),
            user: format!("{}Script:\n\n{part}\nNotes about {name}:", hints_block(&format!("{name}\n{part}"), hints)),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }
}

/// Описание и манера речи персонажа по заметкам.
pub struct CharacterPrompt;

/// Ответ сборки персонажа.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterAnswer {
    pub description: String,
    pub speech: Option<String>,
}

impl CharacterPrompt {
    pub fn build(name: &str, notes: &[String], text: &str, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: CHARACTER_SYSTEM.to_string(),
            user: format!("{}Notes about {name}:\n\n{}\n\nProvide JSON object now:", hints_block(&format!("{name}\n{text}"), hints), notes.join("\n\n")),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }

    /// Объект из ответа, даже обёрнутый в Markdown. `None`: описания нет.
    pub fn accept(answer: &str) -> Option<CharacterAnswer> {
        #[derive(serde::Deserialize)]
        struct Raw {
            description: Option<String>,
            speech: Option<String>,
        }
        let start = answer.find('{')?;
        let end = answer.rfind('}')?;
        let raw: Raw = serde_json::from_str(answer.get(start..=end)?).ok()?;
        let description = clean_note(raw.description.as_deref().unwrap_or_default());
        if description.is_empty() {
            return None;
        }
        let speech = raw.speech.map(|speech| clean_note(&speech)).filter(|speech| !speech.is_empty());
        Some(CharacterAnswer { description, speech })
    }
}

/// Статья термина по заметкам.
pub struct TermPrompt;

impl TermPrompt {
    pub fn build(name: &str, notes: &[String], text: &str, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        ChatRequest {
            system: TERM_SYSTEM.to_string(),
            user: format!("{}Notes about {name}:\n\n{}", hints_block(&format!("{name}\n{text}"), hints), notes.join("\n\n")),
            temperature: target.settings.temperature,
            max_tokens: NOTE_ANSWER_TOKENS,
        }
    }
}

/// Текст ответа без Markdown, заголовков и строк подсказок глоссария, которые модель иногда переписывает.
pub fn clean_note(text: &str) -> String {
    static EMPHASIS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\*+([^\s*\n](?:[^*\n]*[^\s*\n])?)\*+").expect("выражение выделения Markdown верное"));
    let plain = EMPHASIS.replace_all(text, "$1");
    plain
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') && line.trim_start_matches('#').starts_with(char::is_whitespace) {
                return None;
            }
            let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
            let label = line.to_lowercase();
            let hint = ["russian names:", "gender:", "known terms:", "tokens:"].iter().any(|prefix| label.starts_with(prefix));
            let heading = ["описание:", "описание сцены:", "заметки:", "description:", "notes:"].contains(&label.as_str());
            (!hint && !heading).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Заметка, которую стоит брать в описание: не пустая и не отказ модели.
pub fn is_useful(note: &str) -> bool {
    let text = note.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let blank = text.is_empty() || BLANK_NOTES.contains(&text.as_str());
    let refusal = text.chars().count() <= MAX_REFUSAL_CHARS && REFUSALS.iter().any(|refusal| text.contains(refusal));
    !blank && !refusal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Gender;
    use crate::translation::prompt::tests::target;

    #[test]
    fn part_request_has_hints_of_its_text_only() {
        let hints = Hints {
            names: vec![
                ("Anna".to_string(), "Анна".to_string(), Some(Gender::Female)),
                ("Bob".to_string(), "Боб".to_string(), None),
            ],
            ..Hints::default()
        };
        let request = PartPrompt::build("Anna: Hi.\n", &hints, &target());
        assert!(request.user.starts_with("Russian names: Anna = Анна\nGender: Anna female\n\nScript:\n\nAnna: Hi.\n"), "{}", request.user);
        assert!(!request.user.contains("Bob"));
    }

    #[test]
    fn notes_are_cleaned_and_refusals_dropped() {
        assert_eq!(clean_note("## Заметки\n**Анна** встречает Боба.\nRussian names: Anna = Анна\n- Они на «ты»."), "Анна встречает Боба.\nОни на «ты».");
        assert!(is_useful("Анна встречает Боба."));
        assert!(!is_useful("Нет сведений о сцене."));
        assert!(!is_useful("  none. "));
        assert!(!is_useful(""));
    }

    #[test]
    fn title_is_first_line_without_quotes_and_period() {
        assert_eq!(TitlePrompt::accept("\n«Знакомство с Анной».\nЛишнее").as_deref(), Some("Знакомство с Анной"));
        assert_eq!(TitlePrompt::accept("  \n"), None);
        assert!(TitlePrompt::build("Описание", true, &target()).system.ends_with("This is the first scene of the game."));
    }

    #[test]
    fn character_answer_is_read_from_json_even_in_markdown() {
        let answer = CharacterPrompt::accept("Вот:\n```json\n{\"description\": \"**Анна** учится.\", \"speech\": \" \"}\n```");
        assert_eq!(answer, Some(CharacterAnswer { description: "Анна учится.".to_string(), speech: None }));
        assert_eq!(CharacterPrompt::accept("{\"description\": \"\"}"), None);
        assert_eq!(CharacterPrompt::accept("Не JSON"), None);
        let request = EntryPartPrompt::build(EntryKind::Term, "Old Key", "Anna: The old key.\n", &Hints::default(), &target());
        assert!(request.system.contains("notes about the term Old Key"));
        assert!(request.user.ends_with("Notes about Old Key:"));
    }

    #[test]
    fn long_notes_are_grouped_within_request() {
        let note = "а".repeat(5_000);
        let notes = vec![note.clone(), note.clone(), note.clone()];
        assert!(!CondensePrompt::fits(&notes));
        let groups = CondensePrompt::groups(notes);
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
    }
}
