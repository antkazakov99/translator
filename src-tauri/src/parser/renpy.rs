use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::graph::{EdgeKind, ScenarioGraph, ScenarioNode, TranslationStatus};
use crate::parser::GameParser;

// Глобально скомпилированные регулярные выражения для максимальной производительности (LazyLock)
static RE_CHAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:define\s+|\$\s*)?([A-Za-z_]\w*)\s*=\s*(?:renpy\.)?(?:Character|DynamicCharacter|adv_character)\s*\(\s*[_(\s]*['"]([^'"]+)['"]"#
    ).unwrap()
});

static RE_CUSTOM_CHARS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:define\s+|\$\s*)?([A-Za-z_]\w*)\s*=\s*Chars\s*\(\s*['"]([^'"]+)['"]"#).unwrap()
});

// Имя метки может содержать точку, после него идут параметры и `hide`
static RE_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*label\s+([\w.]+)\s*(?:\(.*\))?\s*(?:hide\s*)?:"#).unwrap()
});

static RE_STRUCT_MENU: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*menu(?:\s+[A-Za-z_]\w*)?\s*(?:\(.*\))?\s*:"#).unwrap()
});

static RE_STRUCT_CHOICE_OPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"^\s*(?:"(?P<text>(?:[^"\\]|\\.)*)"|'(?P<text_sq>(?:[^'\\]|\\.)*)')"#,
        r#"\s*(?:\(.*\)\s*)?(?:if\s+.+?)?\s*:\s*(?:#.*)?$"#,
    ))
    .unwrap()
});

static RE_STRUCT_JUMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:if\s+(?P<cond>[^:]+):\s*)?(?P<cmd>jump|call)\s+(?P<target>[\w.]+)"#)
        .unwrap()
});

/// Слова после `jump` и `call`, которые не называют метку: `call screen inventory`.
const NON_LABEL_TARGETS: &[&str] = &["screen", "expression"];

/// Переход из кода Python и из действия кнопки: `Jump("map")`, `renpy.call("intro")`.
static RE_ACTION_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"\b(?P<cmd>Jump|Call|renpy\.jump|renpy\.call|renpy\.jump_out_of_context|renpy\.call_in_new_context)"#,
        r#"\(\s*["'](?P<target>[\w.]+)["']"#,
    ))
    .unwrap()
});

static RE_SCREEN_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*screen\s+(\w+)\s*(?:\(.*\))?\s*:"#).unwrap());

/// Показ экрана из сцены.
static RE_SHOWN_SCREEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:call|show)\s+screen\s+(\w+)|\brenpy\.(?:call|show)_screen\(\s*["'](\w+)["']"#)
        .unwrap()
});

/// Экран, открытый из другого экрана.
static RE_SCREEN_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*use\s+(\w+)|\b(?:Show|ToggleScreen)\(\s*["'](\w+)["']"#).unwrap()
});

/// Код этих блоков не выполняется внутри сцены, хотя стоит после её метки.
static RE_INIT_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?:\s*init\b|python\b).*:\s*(?:#.*)?$"#).unwrap()
});

static RE_STRUCT_IF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*(?:elif|if)\s+(.+?)\s*:"#).unwrap());

static RE_STRUCT_ELSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*else\s*:"#).unwrap());

static RE_STRINGS_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^translate\s+\w+\s+strings\s*:"#).unwrap());

static RE_DIALOGUE_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^translate\s+\w+\s+([\w\.]+)\s*:"#).unwrap());

static RE_OLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*old\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap());

static RE_NEW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*new\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap());

// Говорящий: имя в кавычках или псевдоним с атрибутами спрайта (`e happy "..."`, `e @ sad "..."`)
static RE_DIALOGUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"^\s*(?:(?:"(?P<name>(?:[^"\\]|\\.)*)""#,
        r#"|(?P<speaker>[A-Za-z_][\w.]*)(?:\s+(?:@|-?[A-Za-z_]\w*))*)\s+)?"#,
        r#"(?:"(?P<text>(?:[^"\\]|\\.)*)"|'(?P<text_sq>(?:[^'\\]|\\.)*)')"#,
    ))
    .unwrap()
});

/// Операторы Ren'Py со строкой в аргументах: по виду они не отличаются от реплик.
const NON_DIALOGUE_STATEMENTS: &[&str] = &[
    "voice", "play", "queue", "stop", "show", "hide", "scene", "with", "image", "define",
    "default", "transform", "style", "screen", "init", "python", "label", "menu", "jump", "call",
    "return", "pause", "window", "if", "elif", "while", "for",
];

// Блоки определений: их свойства со строками выглядят как реплики (`font "a.ttf"`).
// Блоки python не входят: списки строк в них игрок видит на экранах
static RE_DEFINITION_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?:screen|style|transform|image|layeredimage)\b.*:\s*(?:#.*)?$"#).unwrap()
});

/// Операторы экрана с текстом, который видит игрок.
const SCREEN_TEXT_STATEMENTS: &[&str] = &["text", "textbutton", "tooltip"];

static RE_REPEATED_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(" {2,}").unwrap());

static RE_SOURCE_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*#\s*(?:game[/\\])?(?P<file>.+?\.rpy):(?P<line>\d+)"#).unwrap()
});

static RE_LABEL_HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"_[0-9a-fA-F]{6,10}(?:_\d+)*$"#).unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceOptionDto {
    pub text: String,
    pub target_jump: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceBlockDto {
    pub prompt: Option<String>,
    pub options: Vec<ChoiceOptionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEdgeDto {
    pub from: String,              // имя исходной сцены
    pub to: String,                // имя целевой сцены
    pub kind: String,              // "jump" | "branch" | "choice" | "linear" | "call"
    pub condition: Option<String>, // например "luka_corr > 0" или "Выбор: Yes"
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LabelInfoDto {
    pub name: String,
    pub line_number: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ScriptStructure {
    pub labels: Vec<LabelInfoDto>,
    pub scene_choices: HashMap<String, Vec<ChoiceBlockDto>>,
    pub scene_jumps: HashMap<String, Vec<(String, Option<String>)>>,
    pub edges: Vec<SceneEdgeDto>,
    /// Переходы кнопок по именам экранов файла.
    pub screens: HashMap<String, ScreenLinks>,
    /// Какие экраны показывают сцены файла: сцена и имя экрана.
    pub shown_screens: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct ScreenLinks {
    pub targets: Vec<String>,
    /// Экраны, вложенные через `use` и открытые кнопками.
    pub screens: Vec<String>,
}

/// Блок `menu:`, в котором находится разбор. Меню могут быть вложенными.
struct OpenMenu {
    indent: usize,
    /// Отступ вариантов: глубже идут только их тела.
    option_indent: Option<usize>,
    options: Vec<ChoiceOptionDto>,
}

impl ScriptStructure {
    /// Закрывает меню, в тело которых строка с таким отступом уже не входит.
    fn close_menus(&mut self, scene: Option<&str>, menus: &mut Vec<OpenMenu>, indent: usize) {
        while menus.last().is_some_and(|menu| menu.indent >= indent) {
            let options = menus.pop().map(|menu| menu.options).unwrap_or_default();
            if let Some(scene) = scene.filter(|_| !options.is_empty()) {
                self.scene_choices
                    .entry(scene.to_string())
                    .or_default()
                    .push(ChoiceBlockDto { prompt: None, options });
            }
        }
    }

    /// Записывает переход сцены с учётом открытых меню и условий.
    fn add_transition(
        &mut self,
        scene: &str,
        target: &str,
        is_call: bool,
        inline_condition: Option<String>,
        menus: &mut [OpenMenu],
        conditions: &[(usize, String)],
    ) {
        let cond = inline_condition.or_else(|| conditions.last().map(|(_, cond)| cond.clone()));
        let kind = if !menus.is_empty() {
            "choice"
        } else if cond.is_some() {
            "branch"
        } else if is_call {
            "call"
        } else {
            "jump"
        };

        let option = menus.last_mut().and_then(|menu| menu.options.last_mut());
        let condition_label = match option {
            Some(opt) => {
                opt.target_jump = Some(target.to_string());
                Some(format!("Выбор: \"{}\"", opt.text))
            }
            None => cond.clone(),
        };

        self.edges.push(SceneEdgeDto {
            from: scene.to_string(),
            to: target.to_string(),
            kind: kind.to_string(),
            condition: condition_label,
        });
        self.scene_jumps
            .entry(scene.to_string())
            .or_default()
            .push((target.to_string(), cond));
    }
}

/// Отслеживает блоки определений в оригинальном скрипте.
#[derive(Default)]
struct DefinitionBlocks {
    /// Отступ открытого блока и признак экрана.
    current: Option<(usize, bool)>,
}

impl DefinitionBlocks {
    /// Скрыта ли строка от списка реплик. Вызывается по порядку для всех строк с кодом.
    fn hides(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        if let Some((block_indent, is_screen)) = self.current {
            if indent > block_indent {
                let speaker = parse_say_line(trimmed).and_then(|say| say.speaker);
                return !(is_screen && speaker.is_some_and(|name| SCREEN_TEXT_STATEMENTS.contains(&name)));
            }
            self.current = None;
        }
        if RE_DEFINITION_BLOCK.is_match(trimmed) {
            self.current = Some((indent, trimmed.starts_with("screen")));
            return true;
        }
        false
    }
}

/// Реплика, найденная в строке скрипта.
struct SayLine<'a> {
    speaker: Option<&'a str>,
    /// Текст без кавычек, экранирование не снято.
    text: &'a str,
    /// Границы текста вместе с кавычками.
    quoted: std::ops::Range<usize>,
}

fn parse_say_line(line: &str) -> Option<SayLine<'_>> {
    let caps = RE_DIALOGUE.captures(line)?;
    if let Some(keyword) = caps.name("speaker")
        && NON_DIALOGUE_STATEMENTS.contains(&keyword.as_str())
    {
        return None;
    }
    let text = caps.name("text").or_else(|| caps.name("text_sq"))?;
    Some(SayLine {
        speaker: caps
            .name("speaker")
            .or_else(|| caps.name("name"))
            .map(|speaker| speaker.as_str()),
        text: text.as_str(),
        quoted: text.start() - 1..text.end() + 1,
    })
}

/// Реплика со всеми частями, нужными для её компиляции.
pub struct SayParts {
    pub speaker: Option<String>,
    /// Говорящий задан строкой, а не псевдонимом персонажа.
    pub speaker_is_string: bool,
    /// Атрибуты спрайта между говорящим и текстом, включая `@`.
    pub attributes: Vec<String>,
    /// Текст без кавычек, экранирование не снято.
    pub text: String,
    /// Всё, что стоит после текста.
    pub rest: String,
}

/// Файл перевода объявляет блоки `translate` в начале строки. Слово внутри
/// реплики или комментария оригинального скрипта признаком не считается.
fn has_translate_blocks(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim_start_matches('\u{feff}').trim_end();
        line.starts_with("translate ") && line.ends_with(':')
    })
}

/// Парсер и генератор файлов сценариев и локализаций Ren'Py (.rpy)
#[derive(Debug, Default)]
pub struct RenpyParser {
    /// Таблица персонажей: псевдоним (например, "e") -> полное имя ("Эрика")
    pub speakers: HashMap<String, String>,
    /// Обратная таблица: полное имя ("Эрика") -> псевдоним ("e")
    pub speaker_to_alias: HashMap<String, String>,
}

impl RenpyParser {
    pub fn new() -> Self {
        Self {
            speakers: HashMap::new(),
            speaker_to_alias: HashMap::new(),
        }
    }

    /// Регистрирует известных персонажей (например, из скриптов игры)
    pub fn with_speakers(mut self, speakers: HashMap<String, String>) -> Self {
        self.speaker_to_alias = speakers
            .iter()
            .map(|(k, v)| (v.clone(), k.clone()))
            .collect();
        self.speakers = speakers;
        self
    }

    /// Извлекает определения персонажей вида `define e = Character("Erika")`,
    /// `e = Character('Erika')`, `player = Chars('Jack')`, и т.д.
    pub fn extract_speakers(content: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        let mut custom_names: HashMap<String, String> = HashMap::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if let Some(caps) = RE_CHAR.captures(trimmed) {
                let alias = caps.get(1).unwrap().as_str().to_string();
                let name = caps.get(2).unwrap().as_str().to_string();
                if name != "none" && !name.is_empty() {
                    map.insert(alias, name);
                }
            } else if let Some(caps) = RE_CUSTOM_CHARS.captures(trimmed) {
                let alias = caps.get(1).unwrap().as_str().to_string();
                let name = caps.get(2).unwrap().as_str().to_string();
                if name != "none" && !name.is_empty() {
                    custom_names.insert(alias.clone(), name.clone());
                    map.insert(alias, name);
                }
            }
        }

        // Подставляем реальные имена переменных (например [player.name] -> Jack)
        if let Some(player_name) = custom_names.get("player") {
            for v in map.values_mut() {
                if v == "[player.name]" || v == "[player]" {
                    *v = player_name.clone();
                }
            }
        }

        map
    }

    /// Схлопывает повторы пробелов, как лексер Ren'Py в обычных строках.
    pub fn collapse_spaces(s: &str) -> String {
        RE_REPEATED_SPACES.replace_all(s, " ").into_owned()
    }

    /// Разбирает строку как `Lexer.string` в Ren'Py. Текст обязан совпадать с движком:
    /// по нему игра ищет строковые переводы `old`.
    pub fn unescape_renpy_string(s: &str) -> String {
        let collapsed = Self::collapse_spaces(s);
        let mut result = String::with_capacity(collapsed.len());
        let mut chars = collapsed.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                result.push(c);
                continue;
            }
            match chars.next() {
                Some('{') => result.push_str("{{"),
                Some('[') => result.push_str("[["),
                Some('%') => result.push_str("%%"),
                Some('n') => result.push('\n'),
                Some('u') => {
                    let mut code = 0;
                    for _ in 0..4 {
                        let Some(digit) = chars.peek().and_then(|next| next.to_digit(16)) else {
                            break;
                        };
                        code = code * 16 + digit;
                        chars.next();
                    }
                    result.extend(char::from_u32(code).filter(|c| *c != '\0'));
                }
                Some(other) => result.push(other),
                None => result.push(c),
            }
        }
        result
    }

    /// Экранирует текст для вставки в строку Ren'Py в двойных кавычках.
    /// Табуляция остаётся как есть: `\t` движок прочитал бы как букву `t`.
    pub fn escape_renpy_string(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '\n' => result.push_str("\\n"),
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                _ => result.push(c),
            }
        }
        result
    }

    /// Проверяет наличие кириллических символов в тексте
    pub fn has_cyrillic(text: &str) -> bool {
        text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
    }

    /// Разбирает реплику на части. Текст файла остаётся без снятия экранирования.
    pub fn say_statement(line: &str) -> Option<SayParts> {
        let say = parse_say_line(line)?;
        let caps = RE_DIALOGUE.captures(line)?;
        let attributes = caps
            .name("speaker")
            .map(|speaker| &line[speaker.end()..say.quoted.start])
            .unwrap_or_default();
        Some(SayParts {
            speaker: say.speaker.map(str::to_string),
            speaker_is_string: caps.name("name").is_some(),
            attributes: attributes.split_whitespace().map(str::to_string).collect(),
            text: say.text.to_string(),
            rest: line[say.quoted.end..].to_string(),
        })
    }

    /// Текст строки `old` со снятым экранированием.
    pub fn old_text(line: &str) -> Option<String> {
        RE_OLD
            .captures(line)
            .map(|caps| Self::unescape_renpy_string(caps.name("text").unwrap().as_str()))
    }

    /// Текст строки `new` со снятым экранированием.
    pub fn new_text(line: &str) -> Option<String> {
        RE_NEW
            .captures(line)
            .map(|caps| Self::unescape_renpy_string(caps.name("text").unwrap().as_str()))
    }

    /// Заменяет текст реплики или строки `new` в строке файла перевода.
    pub fn replace_dialogue_text(line: &str, text: &str) -> Option<String> {
        let say = parse_say_line(line)?;
        Some(format!(
            "{}\"{}\"{}",
            &line[..say.quoted.start],
            Self::escape_renpy_string(text),
            &line[say.quoted.end..]
        ))
    }

    /// Исходные тексты строк `old` из файла перевода.
    pub fn string_translation_sources(content: &str) -> std::collections::HashSet<String> {
        content
            .lines()
            .filter_map(|line| RE_OLD.captures(line))
            .map(|caps| Self::unescape_renpy_string(caps.name("text").unwrap().as_str()))
            .collect()
    }

    /// Быстрый подсчет статистики файла перевода (всего строк, переведено)
    /// без построения графа petgraph и без генерации UUID. Реплики определяются
    /// так же, как в полном разборе, поэтому счётчики не расходятся.
    pub fn quick_translation_stats(content: &str) -> (usize, usize) {
        if !has_translate_blocks(content) {
            let mut blocks = DefinitionBlocks::default();
            let total = content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.is_empty()
                        && !trimmed.starts_with('#')
                        && !blocks.hides(line)
                        && parse_say_line(trimmed).is_some()
                })
                .count();
            return (total, 0);
        }

        let mut total = 0;
        let mut translated = 0;
        let mut seen_ui_strings = std::collections::HashSet::new();

        let mut in_strings = false;
        let mut in_block = false;
        let mut pending_dialogue = false;
        let mut pending_old: Option<&str> = None;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("translate ") {
                if trimmed.contains(" strings:") {
                    in_strings = true;
                    in_block = false;
                    pending_dialogue = false;
                    pending_old = None;
                } else if trimmed.ends_with(':') {
                    in_strings = false;
                    in_block = true;
                    pending_dialogue = false;
                }
                continue;
            }

            if in_strings {
                if trimmed.starts_with("old ") {
                    if let Some(s) = trimmed.find('"')
                        && let Some(e) = trimmed.rfind('"')
                        && e > s
                    {
                        pending_old = Some(&trimmed[s + 1..e]);
                    }
                } else if trimmed.starts_with("new ")
                    && let Some(old_str) = pending_old.take()
                    && seen_ui_strings.insert(old_str)
                {
                    total += 1;
                    if let Some(s) = trimmed.find('"')
                        && let Some(e) = trimmed.rfind('"')
                        && e > s
                    {
                        let new_str = &trimmed[s + 1..e];
                        if Self::has_cyrillic(new_str) {
                            translated += 1;
                        }
                    }
                }
                continue;
            }

            if in_block {
                if trimmed.starts_with('#') {
                    // Игнорируем комментарии путей вроде `# game/story.rpy:10`
                    if parse_say_line(trimmed.trim_start_matches('#').trim()).is_some() {
                        pending_dialogue = true;
                    }
                } else if pending_dialogue && let Some(say) = parse_say_line(trimmed) {
                    total += 1;
                    if Self::has_cyrillic(say.text) {
                        translated += 1;
                    }
                    pending_dialogue = false;
                }
            }
        }

        (total, translated)
    }

    /// Определяет полное имя персонажа по его псевдониму
    fn resolve_speaker(&self, alias: Option<&str>) -> Option<String> {
        match alias {
            Some(key) if !key.is_empty() => {
                if let Some(full) = self.speakers.get(key) {
                    Some(full.clone())
                } else {
                    Some(key.to_string())
                }
            }
            _ => None, // Narration
        }
    }

    /// Получает псевдоним (код) персонажа для записи в файл
    fn resolve_alias(&self, speaker_name: Option<&str>) -> String {
        match speaker_name {
            Some("UI / Choice") | None => "".to_string(),
            Some(name) => {
                if let Some(alias) = self.speaker_to_alias.get(name) {
                    format!("{} ", alias)
                } else {
                    format!("{} ", name)
                }
            }
        }
    }

    /// Генерирует готовое содержимое файла .rpy на основе графа диалогов
    pub fn generate_translation_file(&self, graph: &ScenarioGraph, target_lang: &str) -> String {
        let mut output = String::new();
        let mut string_entries = Vec::new();
        let mut current_block_id: Option<String> = None;

        for idx in graph.node_indices() {
            if let Some(ScenarioNode::Dialogue(d)) = graph.get_node(idx) {
                if d.speaker.as_deref() == Some("UI / Choice") {
                    string_entries.push(d);
                } else {
                    if let Some(ref block_id) = d.block_id
                        && current_block_id.as_deref() != Some(block_id)
                    {
                        output.push_str(&format!("\ntranslate {} {}:\n\n", target_lang, block_id));
                        current_block_id = Some(block_id.clone());
                    }

                    let alias_prefix = self.resolve_alias(d.speaker.as_deref());
                    let original_escaped = Self::escape_renpy_string(&d.original_text);
                    let target_text = d.translated_text.as_deref().unwrap_or(&d.original_text);
                    let target_escaped = Self::escape_renpy_string(target_text);

                    output.push_str(&format!("    # {}\"{}\"\n", alias_prefix, original_escaped));
                    output.push_str(&format!("    {}\"{}\"\n\n", alias_prefix, target_escaped));
                }
            }
        }

        if !string_entries.is_empty() {
            output.push_str(&format!("\ntranslate {} strings:\n\n", target_lang));
            for d in string_entries {
                let old_escaped = Self::escape_renpy_string(&d.original_text);
                let new_text = d.translated_text.as_deref().unwrap_or(&d.original_text);
                let new_escaped = Self::escape_renpy_string(new_text);

                output.push_str(&format!("    old \"{}\"\n", old_escaped));
                output.push_str(&format!("    new \"{}\"\n\n", new_escaped));
            }
        }

        output
    }

    /// Сохраняет сгенерированный мод .rpy на диск
    pub fn save_translation_file(
        &self,
        graph: &ScenarioGraph,
        output_path: &Path,
        target_lang: &str,
    ) -> Result<()> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Не удалось создать папку мода: {:?}", parent))?;
        }
        let content = self.generate_translation_file(graph, target_lang);
        std::fs::write(output_path, content)
            .with_context(|| format!("Не удалось записать файл перевода: {:?}", output_path))?;
        Ok(())
    }

    /// Парсит оригинальный файл скрипта .rpy для извлечения графа: выборов (menu:), переходов (jump) и условий (if)
    pub fn parse_script_structure(content: &str) -> ScriptStructure {
        let mut structure = ScriptStructure::default();

        let mut current_label: Option<String> = None;
        let mut menus: Vec<OpenMenu> = Vec::new();
        // Условия открытых блоков if/elif/else с их отступами
        let mut conditions: Vec<(usize, String)> = Vec::new();
        let mut open_screen: Option<(usize, String)> = None;
        let mut init_indent: Option<usize> = None;

        for (idx, line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let indent = line.len() - line.trim_start().len();

            // Экран объявляют и внутри метки, поэтому он определяется по своему отступу
            if let Some((screen_indent, screen)) = &open_screen {
                if indent > *screen_indent {
                    let links = structure.screens.entry(screen.clone()).or_default();
                    links.targets.extend(
                        RE_ACTION_TARGET
                            .captures_iter(line)
                            .map(|caps| caps.name("target").unwrap().as_str().to_string()),
                    );
                    links.screens.extend(RE_SCREEN_LINK.captures_iter(line).filter_map(|caps| {
                        Some(caps.get(1).or_else(|| caps.get(2))?.as_str().to_string())
                    }));
                    continue;
                }
                open_screen = None;
            }
            if let Some(caps) = RE_SCREEN_BLOCK.captures(line) {
                let screen = caps.get(1).unwrap().as_str().to_string();
                structure.screens.entry(screen.clone()).or_default();
                open_screen = Some((indent, screen));
                continue;
            }

            // 1. Проверяем объявление сцены (label)
            if let Some(caps) = RE_LABEL.captures(line) {
                structure.close_menus(current_label.as_deref(), &mut menus, 0);
                conditions.clear();
                init_indent = None;

                let label_name = caps.get(1).unwrap().as_str().to_string();
                structure.labels.push(LabelInfoDto {
                    name: label_name.clone(),
                    line_number: line_num,
                });
                current_label = Some(label_name);
                continue;
            }

            let lbl = match &current_label {
                Some(l) => l.clone(),
                None => continue,
            };

            // Отступ показывает, какие блоки уже закончились
            structure.close_menus(Some(&lbl), &mut menus, indent);
            conditions.retain(|(block_indent, _)| *block_indent < indent);
            if init_indent.is_some_and(|block_indent| indent <= block_indent) {
                init_indent = None;
            }
            if RE_INIT_BLOCK.is_match(line) {
                init_indent.get_or_insert(indent);
            }

            // 2. Проверяем начало блока menu:
            if RE_STRUCT_MENU.is_match(line) {
                menus.push(OpenMenu {
                    indent,
                    option_indent: None,
                    options: Vec::new(),
                });
                continue;
            }

            // 3. Варианты выбора внутри menu
            if let Some(menu) = menus.last_mut()
                && menu.option_indent.is_none_or(|expected| expected == indent)
                && let Some(caps) = RE_STRUCT_CHOICE_OPT.captures(line)
            {
                let text = caps.name("text").or_else(|| caps.name("text_sq")).unwrap();
                menu.option_indent = Some(indent);
                menu.options.push(ChoiceOptionDto {
                    text: Self::unescape_renpy_string(text.as_str()),
                    target_jump: None,
                });
                continue;
            }

            // 4. Ветвления if/elif/else
            if let Some(caps) = RE_STRUCT_IF.captures(line) {
                conditions.push((indent, caps.get(1).unwrap().as_str().trim().to_string()));
            } else if RE_STRUCT_ELSE.is_match(line) {
                conditions.push((indent, "иначе".to_string()));
            }

            // 5. Переходы jump / call
            if let Some(caps) = RE_STRUCT_JUMP.captures(line)
                && !NON_LABEL_TARGETS.contains(&caps.name("target").unwrap().as_str())
            {
                let inline_cond = caps.name("cond").map(|c| c.as_str().trim().to_string());
                structure.add_transition(
                    &lbl,
                    caps.name("target").unwrap().as_str(),
                    caps.name("cmd").unwrap().as_str() == "call",
                    inline_cond,
                    &mut menus,
                    &conditions,
                );
            }

            // 6. Экраны, показанные сценой
            if let Some(caps) = RE_SHOWN_SCREEN.captures(line)
                && let Some(screen) = caps.get(1).or_else(|| caps.get(2))
            {
                structure.shown_screens.push((lbl.clone(), screen.as_str().to_string()));
            }

            // 7. Переходы из кода Python. Объявления выполняются вне сцены
            if init_indent.is_none() && !trimmed.starts_with("define ") && !trimmed.starts_with("default ") {
                for caps in RE_ACTION_TARGET.captures_iter(line) {
                    let cmd = caps.name("cmd").unwrap().as_str();
                    structure.add_transition(
                        &lbl,
                        caps.name("target").unwrap().as_str(),
                        cmd.to_lowercase().contains("call"),
                        None,
                        &mut menus,
                        &conditions,
                    );
                }
            }
        }

        structure.close_menus(current_label.as_deref(), &mut menus, 0);
        structure
    }

    /// Парсит оригинальный или декомпилированный скрипт .rpy (где нет блоков translate)
    pub fn parse_original_script(&self, path: &Path, content: &str) -> Result<ScenarioGraph> {
        let mut graph = ScenarioGraph::new();
        let file_path = path.to_string_lossy().to_string();

        let mut current_scene_name = "Общие диалоги".to_string();
        let mut last_node_idx = None;
        let mut blocks = DefinitionBlocks::default();

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let trimmed = raw_line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let hidden = blocks.hides(raw_line);
            if let Some(caps) = RE_LABEL.captures(trimmed) {
                let label_name = caps.get(1).unwrap().as_str().to_string();
                current_scene_name = label_name.clone();
                let label_node = graph.add_node(ScenarioNode::label(label_name));
                last_node_idx = Some(label_node);
                continue;
            }
            if hidden {
                continue;
            }

            if let Some(say) = parse_say_line(trimmed) {
                let speaker_alias = say.speaker;
                let text = Self::unescape_renpy_string(say.text);
                let resolved_speaker = self.resolve_speaker(speaker_alias);

                let mut dialogue =
                    ScenarioNode::dialogue(resolved_speaker, text, &file_path, line_num);
                if let Some(d) = dialogue.as_dialogue_mut() {
                    d.block_id = Some(current_scene_name.clone());
                    d.speaker_alias = speaker_alias.map(|s| s.to_string());
                }

                let node_idx = graph.add_node(dialogue);
                if let Some(prev) = last_node_idx {
                    graph.add_edge(prev, node_idx, EdgeKind::Linear);
                }
                last_node_idx = Some(node_idx);
            }
        }

        Ok(graph)
    }
}

impl GameParser for RenpyParser {
    fn name(&self) -> &'static str {
        "renpy"
    }

    /// Парсит `.rpy` файл и группирует реплики по реальным смысловым сценам
    fn parse_file(&self, path: &Path, content: &str) -> Result<ScenarioGraph> {
        if !has_translate_blocks(content) {
            return self.parse_original_script(path, content);
        }

        let mut graph = ScenarioGraph::new();
        let file_path = path.to_string_lossy().to_string();

        let mut seen_ui_strings: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
        let mut in_strings = false;
        let mut in_block = false;
        let mut current_block_id: Option<String> = None;
        let mut current_scene_name: Option<String> = None;
        let mut pending_comment_line: Option<usize> = None;
        let mut pending_old: Option<(String, usize)> = None;
        let mut pending_source: Option<(Option<String>, String, usize)> = None;
        let mut last_node_idx = None;

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let line = raw_line.trim_end();

            // Блок интерфейсных строк (strings) НЕ должен разрывать текущую сюжетную сцену!
            if RE_STRINGS_BLOCK.is_match(line) {
                in_strings = true;
                in_block = false;
                pending_old = None;
                continue;
            }

            if let Some(caps) = RE_DIALOGUE_BLOCK.captures(line) {
                in_strings = false;
                in_block = true;
                pending_source = None;

                let raw_block_id = caps.get(1).unwrap().as_str().to_string();
                current_block_id = Some(raw_block_id.clone());

                // Отрезаем хэш: например "end_scene_eri_52d5be01" -> "end_scene_eri"
                let scene_name = RE_LABEL_HASH.replace(&raw_block_id, "").to_string();

                if current_scene_name.as_deref() != Some(&scene_name) {
                    current_scene_name = Some(scene_name.clone());
                    let label_node = graph.add_node(ScenarioNode::label(scene_name));
                    last_node_idx = Some(label_node);
                }
                continue;
            }

            if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
                in_block = false;
                if !line.starts_with('#') {
                    in_strings = false;
                    pending_old = None;
                }
                continue;
            }

            if in_strings {
                if let Some(caps) = RE_SOURCE_COMMENT.captures(line) {
                    if let Ok(line_val) = caps.name("line").unwrap().as_str().parse::<usize>() {
                        pending_comment_line = Some(line_val);
                    }
                    continue;
                }

                if let Some(caps) = RE_OLD.captures(line) {
                    let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                    let src_line = pending_comment_line.take().unwrap_or(line_num);
                    pending_old = Some((text, src_line));
                } else if let Some(caps) = RE_NEW.captures(line)
                    && let Some((old_text, old_line)) = pending_old.take()
                {
                    let new_text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                    let is_translated = Self::has_cyrillic(&new_text);

                    // Дедупликация строк UI: если точно такой же old_text уже встречался
                    if let Some(&existing_idx) = seen_ui_strings.get(&old_text) {
                        if let Some(existing_node) = graph.get_node_mut(existing_idx)
                            && let Some(d) = existing_node.as_dialogue_mut()
                        {
                            d.target_lines.push(line_num);
                            if is_translated && d.translated_text.is_none() {
                                d.translated_text = Some(new_text);
                                d.status = TranslationStatus::Translated;
                            }
                        }
                    } else {
                        let mut dialogue = ScenarioNode::dialogue(
                            Some("UI / Choice"),
                            old_text.clone(),
                            &file_path,
                            old_line,
                        );
                        if let Some(d) = dialogue.as_dialogue_mut() {
                            d.speaker_alias = Some("UI".to_string());
                            d.target_lines.push(line_num);
                            if is_translated {
                                d.translated_text = Some(new_text);
                                d.status = TranslationStatus::Translated;
                            }
                        }

                        let node_idx = graph.add_node(dialogue);
                        if let Some(prev) = last_node_idx {
                            graph.add_edge(prev, node_idx, EdgeKind::Linear);
                        }
                        last_node_idx = Some(node_idx);
                        seen_ui_strings.insert(old_text, node_idx);
                    }
                }
                continue;
            }

            if in_block {
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    let comment = trimmed.trim_start_matches('#').trim();
                    if let Some(say) = parse_say_line(comment) {
                        let speaker = say.speaker.map(str::to_string);
                        let text = Self::unescape_renpy_string(say.text);
                        pending_source = Some((speaker, text, line_num));
                    }
                } else if let Some(say) = parse_say_line(trimmed) {
                    let speaker_alias = say.speaker;
                    let target_text = Self::unescape_renpy_string(say.text);

                    if let Some((src_speaker, src_text, src_line)) = pending_source.take() {
                        let raw_alias = src_speaker
                            .clone()
                            .or_else(|| speaker_alias.map(|s| s.to_string()));
                        let resolved_speaker =
                            self.resolve_speaker(src_speaker.as_deref().or(speaker_alias));
                        let is_translated = Self::has_cyrillic(&target_text);

                        let mut dialogue = ScenarioNode::dialogue(
                            resolved_speaker,
                            src_text,
                            &file_path,
                            src_line,
                        );
                        if let Some(d) = dialogue.as_dialogue_mut() {
                            d.block_id = current_block_id.clone();
                            d.speaker_alias = raw_alias;
                            d.target_lines.push(line_num);
                            if is_translated {
                                d.translated_text = Some(target_text);
                                d.status = TranslationStatus::Translated;
                            }
                        }

                        let node_idx = graph.add_node(dialogue);
                        if let Some(prev) = last_node_idx {
                            graph.add_edge(prev, node_idx, EdgeKind::Linear);
                        }
                        last_node_idx = Some(node_idx);
                    }
                }
            }
        }

        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_speakers() {
        let script = r##"
            define mc = Character("Главный Герой", color="#c8ffc8")
            define e = Character('Erika')
            define narrator = Character(None)
        "##;
        let speakers = RenpyParser::extract_speakers(script);
        assert_eq!(speakers.get("mc").unwrap(), "Главный Герой");
        assert_eq!(speakers.get("e").unwrap(), "Erika");
    }

    #[test]
    fn test_strings_are_read_like_the_engine_lexer() {
        let unescape = RenpyParser::unescape_renpy_string;
        assert_eq!(unescape("Hello.  World   again"), "Hello. World again");
        assert_eq!(unescape(r"100\% \{b} \[name]"), "100%% {{b} [[name]");
        assert_eq!(unescape(r#"\"a\" \\ \'b\'"#), "\"a\" \\ 'b'");
        assert_eq!(unescape(r"one\ntwo \t \u0410"), "one\ntwo t А");

        let text = "tab\there\nsecond \"line\" C:\\new %%";
        assert_eq!(unescape(&RenpyParser::escape_renpy_string(text)), text);
    }

    fn dialogue_texts(rpy: &str) -> Vec<(Option<String>, String, Option<String>)> {
        let graph = RenpyParser::new().parse_file(Path::new("t.rpy"), rpy).unwrap();
        graph
            .node_indices()
            .filter_map(|idx| graph.get_node(idx)?.as_dialogue().cloned())
            .map(|d| (d.speaker, d.original_text, d.translated_text))
            .collect()
    }

    #[test]
    fn test_word_translate_in_dialogue_keeps_original_script_mode() {
        let rpy = "label start:\n    e \"I can translate this.\"\n    # translate later\n";
        assert_eq!(RenpyParser::quick_translation_stats(rpy), (1, 0));
        assert_eq!(dialogue_texts(rpy).len(), 1);

        let with_bom = "\u{feff}translate russian start_abcd1234:\n    # e \"Hi\"\n    e \"Привет\"\n";
        assert!(has_translate_blocks(with_bom));
    }

    #[test]
    fn test_dialogue_forms() {
        let rpy = r#"label start:
    e happy "With attributes"
    e @ sad -angry "Say with"
    "Stranger" "Named by string"
    e 'Single quotes'
    "Narration" with dissolve
    voice "audio/line.ogg"
    play music "theme.ogg"
    show text "Title"
    define x = "value"
"#;
        let texts: Vec<_> = dialogue_texts(rpy).into_iter().map(|d| (d.0, d.1)).collect();
        assert_eq!(
            texts,
            vec![
                (Some("e".to_string()), "With attributes".to_string()),
                (Some("e".to_string()), "Say with".to_string()),
                (Some("Stranger".to_string()), "Named by string".to_string()),
                (Some("e".to_string()), "Single quotes".to_string()),
                (None, "Narration".to_string()),
            ]
        );
        assert_eq!(RenpyParser::quick_translation_stats(rpy), (5, 0));
    }

    #[test]
    fn test_voice_line_keeps_dialogue_translation() {
        let rpy = r#"translate russian start_abcd1234:

    # voice "audio/line.ogg"
    # e happy "Hello"
    voice "audio/line.ogg"
    e happy "Привет"
"#;
        assert_eq!(
            dialogue_texts(rpy),
            vec![(Some("e".to_string()), "Hello".to_string(), Some("Привет".to_string()))]
        );
        assert_eq!(RenpyParser::quick_translation_stats(rpy), (1, 1));
    }

    #[test]
    fn test_replace_dialogue_text_keeps_line_shape() {
        assert_eq!(
            RenpyParser::replace_dialogue_text("    e happy \"Старый\" with dissolve\r", "Новый \"текст\""),
            Some("    e happy \"Новый \\\"текст\\\"\" with dissolve\r".to_string())
        );
        assert_eq!(
            RenpyParser::replace_dialogue_text("    new 'old'", "new"),
            Some("    new \"new\"".to_string())
        );
        assert_eq!(RenpyParser::replace_dialogue_text("    voice \"a.ogg\"", "x"), None);
    }

    #[test]
    fn test_parse_translation_dialogue_scene_grouping() {
        // Проверяем, что хэши _1234abcd и повторы _1234abcd_1 схлопываются в одну сцену!
        let rpy = r#"
translate russian intro_scene_1234abcd:
    # mc "Line 1"
    mc "Строка 1"

translate russian intro_scene_5678ef01:
    # mc "Line 2"
    mc "Строка 2"

translate russian intro_scene_a20cefa7_1:
    # mc "Line 3 (duplicate prompt)"
    mc "Строка 3"

translate russian other_scene_aabbccdd:
    # mc "Line 4"
    mc "Строка 4"

translate russian other_scene_97ddad46_2:
    # mc "Line 5"
    mc "Строка 5"
"#;
        let parser = RenpyParser::new();
        let graph = parser.parse_file(Path::new("intro.rpy"), rpy).unwrap();

        let mut scene_labels = Vec::new();
        for idx in graph.node_indices() {
            if let Some(ScenarioNode::Label(name)) = graph.get_node(idx) {
                scene_labels.push(name.clone());
            }
        }

        // Должно быть только ДВЕ сцены: intro_scene и other_scene!
        assert_eq!(scene_labels, vec!["intro_scene", "other_scene"]);
    }

    #[test]
    fn test_ui_strings_deduplication() {
        let rpy = r#"
translate russian strings:
    old "What is your name? "
    new "Как тебя зовут? "

translate russian strings:
    old "What is your name? "
    new "Как тебя зовут? "

translate russian strings:
    old "What is your name? "
    new "Как тебя зовут? "
"#;
        let parser = RenpyParser::new();
        let graph = parser.parse_file(Path::new("story.rpy"), rpy).unwrap();
        let (total, translated) = graph.translation_stats();
        assert_eq!(
            total, 1,
            "Дублирующиеся строки UI должны быть схлопнуты в 1 запись"
        );
        assert_eq!(translated, 1);
    }

    #[test]
    fn test_definition_blocks_hide_their_properties() {
        let script = concat!(
            "style big:\n",
            "    font \"a.ttf\"\n",
            "image logo:\n",
            "    \"logo.png\"\n",
            "screen hud(x):\n",
            "    # Комментарий без отступа не закрывает блок\n",
            "# Первая колонка\n",
            "    add \"bg.png\" zoom 0.5\n",
            "    text \"Score\" size 20\n",
            "    textbutton \"Start\" action Start()\n",
            "    hover_color \"#fff\"\n",
            "label start:\n",
            "    e \"Hello\"\n",
            "    e \"After\"\n",
        );
        let graph = RenpyParser::new()
            .parse_original_script(Path::new("story.rpy"), script)
            .unwrap();
        let lines: Vec<_> = graph.dialogues().map(|line| line.original_text.as_str()).collect();
        assert_eq!(lines, ["Score", "Start", "Hello", "After"]);
        assert_eq!(RenpyParser::quick_translation_stats(script), (4, 0));
    }

    #[test]
    fn test_nested_menu_keeps_outer_options() {
        let script = r#"
label start:
    menu ask(screen="choice"):
        "Pick one"
        "Say \"hi\"":
            menu:
                "Inner":
                    jump inner
            jump after_inner
        'Quoted' if gold > 1:
            jump quoted
        "Last" (price=1):
            jump last
    jump outside
"#;
        let structure = RenpyParser::parse_script_structure(script);
        let blocks = &structure.scene_choices["start"];
        let texts = |block: &ChoiceBlockDto| {
            block.options.iter().map(|option| option.text.clone()).collect::<Vec<_>>()
        };
        assert_eq!(texts(&blocks[0]), ["Inner"]);
        assert_eq!(texts(&blocks[1]), ["Say \"hi\"", "Quoted", "Last"]);
        assert_eq!(blocks[0].options[0].target_jump.as_deref(), Some("inner"));
        assert_eq!(blocks[1].options[0].target_jump.as_deref(), Some("after_inner"));

        let edge = |to: &str| structure.edges.iter().find(|edge| edge.to == to).unwrap();
        assert_eq!(edge("after_inner").condition.as_deref(), Some("Выбор: \"Say \"hi\"\""));
        assert_eq!(edge("last").kind, "choice");
        assert_eq!(edge("outside").kind, "jump");
    }

    #[test]
    fn test_condition_ends_with_its_block() {
        let script = concat!(
            "label start:\n",
            "    if flag:\n",
            "        $ x = 1\n",
            "    jump next\n",
            "    if a:\n",
            "        jump one\n",
            "    elif b:\n",
            "        if c:\n",
            "            jump two\n",
            "        jump three\n",
            "    else:\n",
            "        jump four\n",
            "    call five\n",
        );
        let structure = RenpyParser::parse_script_structure(script);
        let found: Vec<_> = structure
            .edges
            .iter()
            .map(|edge| (edge.to.as_str(), edge.kind.as_str(), edge.condition.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                ("next", "jump", None),
                ("one", "branch", Some("a")),
                ("two", "branch", Some("c")),
                ("three", "branch", Some("b")),
                ("four", "branch", Some("иначе")),
                ("five", "call", None),
            ]
        );
    }

    #[test]
    fn test_screens_and_python_transitions() {
        let script = concat!(
            "label start:\n",
            "    show screen hud\n",
            "    $ renpy.show_screen(\"phone\")\n",
            "    $ renpy.jump(\"second\")\n",
            "    $ obj.action = Jump(\"third\")\n",
            "    if flag:\n",
            "        $ renpy.call(\"fourth\")\n",
            "    # $ renpy.jump(\"commented\")\n",
            "    define trap = Jump(\"defined\")\n",
            "    screen inner():\n",
            "        textbutton \"Go\" action Jump(\"from_inner\")\n",
            "    jump after\n",
            "screen hud:\n",
            "    if flag:\n",
            "        imagebutton auto \"a_%s\" action [Play(\"sound\", \"a\"), Show(\"phone\")]\n",
            "    hotspot (0, 0, 1, 1) action Jump(\"from_hud\") alt \"x\"\n",
            "init python:\n",
            "    def helper():\n",
            "        renpy.jump(\"from_init\")\n",
        );

        let structure = RenpyParser::parse_script_structure(script);

        let edges: Vec<_> = structure
            .edges
            .iter()
            .map(|edge| (edge.to.as_str(), edge.kind.as_str()))
            .collect();
        assert_eq!(
            edges,
            [("second", "jump"), ("third", "jump"), ("fourth", "branch"), ("after", "jump")]
        );
        assert_eq!(structure.screens["inner"].targets, ["from_inner"]);
        assert_eq!(structure.screens["hud"].targets, ["from_hud"]);
        assert_eq!(structure.screens["hud"].screens, ["phone"]);
        let shown: Vec<_> = structure.shown_screens.iter().map(|(_, screen)| screen.as_str()).collect();
        assert_eq!(shown, ["hud", "phone"]);
    }

    #[test]
    fn test_jump_targets_and_labels() {
        let script = concat!(
            "label greet(name, title=(1, 2)):\n",
            "    e \"Hi\"\n",
            "    call screen inventory\n",
            "    jump expression target\n",
            "    call expression \"x\" pass (1)\n",
            "    jump chapter.part\n",
            "label chapter.part hide:\n",
            "    e \"Bye\"\n",
        );
        let structure = RenpyParser::parse_script_structure(script);
        let names: Vec<_> = structure.labels.iter().map(|label| label.name.as_str()).collect();
        assert_eq!(names, ["greet", "chapter.part"]);
        assert_eq!(structure.edges.len(), 1);
        assert_eq!(structure.edges[0].to, "chapter.part");

        let graph = RenpyParser::new()
            .parse_original_script(Path::new("story.rpy"), script)
            .unwrap();
        let lines: Vec<_> = graph.dialogues().map(|line| line.block_id.clone().unwrap()).collect();
        assert_eq!(lines, ["greet", "chapter.part"]);
    }

    #[test]
    fn test_parse_script_structure_graph() {
        let script = r#"
label start:
    mc "Hello"
    menu:
        "Go left":
            jump left_path
        "Go right":
            jump right_path

label left_path:
    mc "On the left"
    if gold > 10:
        jump castle
    else:
        jump dungeon

label right_path:
    mc "On the right"
    jump castle

label castle:
    mc "At the castle"

label dungeon:
    mc "In the dungeon"
"#;
        let structure = RenpyParser::parse_script_structure(script);
        assert_eq!(structure.edges.len(), 5);
        assert!(
            structure
                .edges
                .iter()
                .any(|e| e.from == "start" && e.to == "left_path" && e.kind == "choice")
        );
        assert!(
            structure
                .edges
                .iter()
                .any(|e| e.from == "start" && e.to == "right_path" && e.kind == "choice")
        );
        assert!(
            structure
                .edges
                .iter()
                .any(|e| e.from == "left_path" && e.to == "castle" && e.kind == "branch")
        );
        assert!(
            structure
                .edges
                .iter()
                .any(|e| e.from == "left_path" && e.to == "dungeon" && e.kind == "branch")
        );
        assert!(
            structure
                .edges
                .iter()
                .any(|e| e.from == "right_path" && e.to == "castle" && e.kind == "jump")
        );
    }
}
