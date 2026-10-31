use flate2::Compression;
use flate2::write::ZlibEncoder;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;

use crate::parser::renpy::RenpyParser;
use crate::pickle::{PickleEncoder, PickleValue};
use crate::stable_hash_str;

static RE_SCRIPT_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^script_version\b[^=\n]*=\s*(\d+)").unwrap());

static RE_TRANSLATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^translate\s+(\S+)\s+([\w.]+)\s*:\s*$").unwrap());

/// Как узлы дерева скрипта хранятся в `.rpyc` разных версий Ren'Py.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstLayout {
    /// Ren'Py 7 и старше
    Python2,
    /// Ren'Py 8 с именем узла-кортежем `name`
    Python3,
    /// Ren'Py 8.4 и новее: `name_version` и `name_serial`, без значений по умолчанию
    CompactNames,
}

/// Формат скомпилированных скриптов конкретной игры.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptFormat {
    pub layout: AstLayout,
    pub script_version: i64,
}

impl ScriptFormat {
    /// Определяет формат по файлам движка рядом с папкой `game`, ничего не запуская.
    pub fn detect(game_dir: &Path) -> Result<Self, String> {
        let root = game_dir
            .parent()
            .ok_or("Не удалось определить корневую папку игры")?;
        let engine = root.join("renpy");
        let init = fs::read_to_string(engine.join("__init__.py"))
            .map_err(|e| format!("Не удалось прочитать движок игры {:?}: {}", engine, e))?;
        let script_version = RE_SCRIPT_VERSION
            .captures(&init)
            .and_then(|caps| caps[1].parse().ok())
            .ok_or_else(|| format!("Не найдена версия скриптов в {:?}", engine))?;

        let has_compact_names = ["ast.py", "ast.pyc", "ast.pyo"].iter().any(|name| {
            fs::read(engine.join(name))
                .is_ok_and(|data| data.windows(12).any(|part| part == b"name_version"))
        });
        let has_python3 = fs::read_dir(root.join("lib")).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.starts_with("py3-") || name.starts_with("python3")
            })
        });
        let layout = match (has_compact_names, has_python3) {
            (true, _) => AstLayout::CompactNames,
            (false, true) => AstLayout::Python3,
            (false, false) => AstLayout::Python2,
        };
        Ok(Self {
            layout,
            script_version,
        })
    }
}

fn text(value: impl Into<String>) -> PickleValue {
    PickleValue::String(value.into())
}

fn dict(fields: Vec<(&str, PickleValue)>) -> PickleValue {
    PickleValue::Dict(fields.into_iter().map(|(key, value)| (text(key), value)).collect())
}

/// Собирает узлы дерева скрипта одного файла.
struct NodeBuilder {
    format: ScriptFormat,
    filename: String,
    stamp: i64,
    serial: i64,
}

impl NodeBuilder {
    fn node(&mut self, class: &str, line: usize, mut fields: Vec<(&str, PickleValue)>) -> PickleValue {
        self.serial += 1;
        fields.push(("filename", text(self.filename.as_str())));
        fields.push(("linenumber", PickleValue::Int(line as i64)));
        if self.format.layout == AstLayout::CompactNames {
            fields.push(("col_offset", PickleValue::Int(0)));
            fields.push(("name_version", PickleValue::Int(self.stamp)));
            fields.push(("name_serial", PickleValue::Int(self.serial)));
        } else {
            fields.push(("next", PickleValue::None));
            fields.push((
                "name",
                PickleValue::Tuple(vec![
                    text(self.filename.as_str()),
                    PickleValue::Int(self.stamp),
                    PickleValue::Int(self.serial),
                ]),
            ));
        }
        object(class, Some(PickleValue::Tuple(vec![PickleValue::None, dict(fields)])))
    }

    fn init(&mut self, line: usize, block: Vec<PickleValue>) -> PickleValue {
        self.node(
            "Init",
            line,
            vec![
                ("block", PickleValue::List(block)),
                ("priority", PickleValue::Int(0)),
            ],
        )
    }

    fn translate_string(
        &mut self,
        language: &str,
        (old, new): (String, String),
        (line, new_line): (usize, usize),
    ) -> PickleValue {
        let location = PickleValue::Tuple(vec![
            text(self.filename.as_str()),
            PickleValue::Int(new_line as i64),
        ]);
        self.node(
            "TranslateString",
            line,
            vec![
                ("language", text(language)),
                ("old", text(old)),
                ("new", text(new)),
                ("newloc", location),
            ],
        )
    }

    fn say(&mut self, line: usize, say: SayStatement) -> PickleValue {
        let mut fields = vec![
            ("who", say.who.map_or(PickleValue::None, text)),
            ("who_fast", PickleValue::Bool(say.who_fast)),
            ("what", text(say.what)),
            ("with_", say.with_.map_or(PickleValue::None, text)),
        ];
        let attributes = attribute_names(say.attributes);
        let temporary_attributes = attribute_names(say.temporary_attributes);
        let arguments = say.arguments.map_or(PickleValue::None, argument_info);
        if self.format.layout == AstLayout::CompactNames {
            if !say.interact {
                fields.push(("interact", PickleValue::Bool(false)));
            }
            for (name, value) in [
                ("attributes", attributes),
                ("arguments", arguments),
                ("temporary_attributes", temporary_attributes),
            ] {
                if value != PickleValue::None {
                    fields.push((name, value));
                }
            }
        } else {
            fields.extend([
                ("interact", PickleValue::Bool(say.interact)),
                ("attributes", attributes),
                ("arguments", arguments),
                ("temporary_attributes", temporary_attributes),
                ("rollback", text("normal")),
            ]);
        }
        self.node("Say", line, fields)
    }

    fn translate(
        &mut self,
        line: usize,
        language: &str,
        identifier: &str,
        block: Vec<PickleValue>,
    ) -> PickleValue {
        let mut fields = vec![
            ("identifier", text(identifier)),
            ("language", text(language)),
            ("block", PickleValue::List(block)),
        ];
        if self.format.layout != AstLayout::CompactNames {
            fields.push(("alternate", PickleValue::None));
        }
        self.node("Translate", line, fields)
    }

    fn python(&mut self, line: usize, source: String) -> PickleValue {
        let filename = text(self.filename.as_str());
        let line_number = PickleValue::Int(line as i64);
        let mut state = vec![PickleValue::Int(1), text(source)];
        match self.format.layout {
            AstLayout::Python2 => state.extend([
                PickleValue::Tuple(vec![filename, line_number, PickleValue::Int(self.stamp)]),
                text("exec"),
            ]),
            AstLayout::Python3 => state.extend([
                PickleValue::Tuple(vec![filename, line_number, PickleValue::Int(self.stamp)]),
                text("exec"),
                PickleValue::Int(3),
            ]),
            AstLayout::CompactNames => state.extend([
                PickleValue::Tuple(vec![filename, line_number]),
                text("exec"),
                PickleValue::Int(3),
            ]),
        }

        let code = object("PyCode", Some(PickleValue::Tuple(state)));
        let mut fields = vec![("code", code)];
        if self.format.layout != AstLayout::CompactNames {
            fields.push(("hide", PickleValue::Bool(false)));
            fields.push(("store", text("store")));
        }
        self.node("Python", line, fields)
    }

    fn end_of_file(&mut self) -> PickleValue {
        let fields = if self.format.layout == AstLayout::CompactNames {
            Vec::new()
        } else {
            vec![("expression", PickleValue::None)]
        };
        self.node("Return", 1, fields)
    }
}

fn object(class: &str, state: Option<PickleValue>) -> PickleValue {
    PickleValue::Object {
        class_module: "renpy.ast".to_string(),
        class_name: class.to_string(),
        state: state.map(Box::new),
        args: Vec::new(),
    }
}

fn attribute_names(attributes: Vec<String>) -> PickleValue {
    if attributes.is_empty() {
        PickleValue::None
    } else {
        PickleValue::Tuple(attributes.into_iter().map(text).collect())
    }
}

/// `ArgumentInfo`: класс виден из `renpy.ast` во всех версиях. Поля `extrapos` и
/// `extrakw` нужны старым движкам, новые их не используют.
fn argument_info(arguments: Vec<SayArgument>) -> PickleValue {
    let arguments = arguments
        .into_iter()
        .map(|(name, expression)| {
            PickleValue::Tuple(vec![name.map_or(PickleValue::None, text), text(expression)])
        })
        .collect();
    object(
        "ArgumentInfo",
        Some(dict(vec![
            ("arguments", PickleValue::List(arguments)),
            ("extrapos", PickleValue::None),
            ("extrakw", PickleValue::None),
            ("__version__", PickleValue::Int(1)),
        ])),
    )
}

/// Аргумент реплики: необязательное имя и выражение.
type SayArgument = (Option<String>, String);

struct SayStatement {
    who: Option<String>,
    who_fast: bool,
    attributes: Vec<String>,
    temporary_attributes: Vec<String>,
    what: String,
    with_: Option<String>,
    interact: bool,
    arguments: Option<Vec<SayArgument>>,
}

fn unsupported(name: &str, line: usize, text: &str) -> String {
    format!(
        "{}, строка {}: приложение не компилирует конструкцию «{}»",
        name,
        line,
        text.trim()
    )
}

/// Отделяет ключевое слово от начала строки: за ним не может идти продолжение имени.
fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(keyword)?;
    let is_name_char = |c: char| c.is_alphanumeric() || c == '_';
    (!rest.starts_with(is_name_char)).then_some(rest)
}

fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Ищет конец выражения: первый подходящий символ вне строк и скобок либо конец текста.
/// `None` значит, что строка или скобка в тексте не закрыта.
fn expression_end(text: &str, is_end: impl Fn(char) -> bool) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, c) in text.char_indices() {
        if let Some(open) = quote {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                _ if c == open => quote = None,
                _ => {}
            }
            continue;
        }
        match c {
            _ if depth == 0 && is_end(c) => return Some(index),
            '"' | '\'' | '`' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    (depth == 0 && quote.is_none()).then_some(text.len())
}

/// Отделяет простое выражение после `with`: оно кончается на пробеле или комментарии.
fn split_simple_expression(text: &str) -> Option<(&str, &str)> {
    let end = expression_end(text, |c| c == '#' || c.is_whitespace())?;
    Some(text.split_at(end))
}

/// Разбирает аргументы реплики после `(` с теми же запретами, что и `parse_arguments`
/// в Ren'Py. Возвращает их и текст после `)`. Распаковка `*` и `**` не поддерживается:
/// разные версии движка хранят её по-разному.
fn parse_arguments(text: &str) -> Option<(Vec<SayArgument>, &str)> {
    let mut arguments: Vec<SayArgument> = Vec::new();
    let mut rest = text.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix(')') {
            return Some((arguments, after));
        }
        if rest.starts_with('*') {
            return None;
        }

        let (argument, after) = rest.split_at(expression_end(rest, |c| c == ',' || c == ')')?);
        let keyword = argument
            .split_once('=')
            .map(|(name, value)| (name.trim(), value))
            .filter(|(name, value)| is_name(name) && !value.starts_with('='));
        let (name, expression) = keyword.map_or((None, argument), |(name, value)| (Some(name), value));
        let expression = expression.trim();
        let repeated = arguments.iter().any(|(other, _)| other.as_deref() == name);
        let after_keyword = name.is_none() && arguments.iter().any(|(other, _)| other.is_some());
        if expression.is_empty() || (name.is_some() && repeated) || after_keyword {
            return None;
        }
        arguments.push((name.map(str::to_string), expression.to_string()));

        rest = match after.strip_prefix(',') {
            Some(next) => next.trim_start(),
            None if after.starts_with(')') => after,
            None => return None,
        };
    }
}

/// Разбирает реплику. Атрибуты после `@` действуют только на время этой реплики.
fn parse_say(name: &str, line_number: usize, line: &str) -> Result<SayStatement, String> {
    let fail = || unsupported(name, line_number, line);
    let say = RenpyParser::say_statement(line).ok_or_else(fail)?;
    let mut attribute_groups = say.attributes.split(|attribute| attribute == "@");
    let attributes = attribute_groups.next().unwrap_or_default().to_vec();
    let temporary_attributes = attribute_groups.next().unwrap_or_default().to_vec();
    if attribute_groups.next().is_some() {
        return Err(fail());
    }

    let mut with_ = None;
    let mut interact = true;
    let mut arguments = None;
    let mut tail = say.rest.trim();
    while !tail.is_empty() && !tail.starts_with('#') {
        if let Some(rest) = strip_keyword(tail, "nointeract") {
            interact = false;
            tail = rest.trim_start();
        } else if let Some(rest) = strip_keyword(tail, "with")
            && with_.is_none()
        {
            let (expression, rest) = split_simple_expression(rest.trim_start()).ok_or_else(fail)?;
            if expression.is_empty() {
                return Err(fail());
            }
            with_ = Some(expression.to_string());
            tail = rest.trim_start();
        } else if let Some(rest) = tail.strip_prefix('(')
            && arguments.is_none()
        {
            let (parsed, rest) = parse_arguments(rest).ok_or_else(fail)?;
            arguments = Some(parsed);
            tail = rest.trim_start();
        } else {
            return Err(fail());
        }
    }

    let (who, who_fast) = match (say.speaker, say.speaker_is_string) {
        (Some(speaker), true) => (Some(format!("\"{}\"", speaker)), false),
        (Some(speaker), false) => {
            let fast = !speaker.contains('.');
            (Some(speaker), fast)
        }
        (None, _) => (None, false),
    };
    Ok(SayStatement {
        who,
        who_fast,
        attributes,
        temporary_attributes,
        what: RenpyParser::unescape_renpy_string(&say.text),
        with_,
        interact,
        arguments,
    })
}

/// Вложенные строки блока, начиная с `start`: `(номер, текст)` без пустых и комментариев.
fn block_lines<'a>(lines: &[&'a str], start: usize) -> (Vec<(usize, &'a str)>, usize) {
    let mut body = Vec::new();
    let mut index = start;
    while let Some(line) = lines.get(index) {
        let trimmed = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !trimmed.is_empty() && !indented && !trimmed.starts_with('#') {
            break;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            body.push((index + 1, *line));
        }
        index += 1;
    }
    (body, index)
}

fn compile_statements(
    builder: &mut NodeBuilder,
    name: &str,
    source: &str,
) -> Result<Vec<PickleValue>, String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let lines: Vec<&str> = source.lines().collect();
    let mut statements = Vec::new();
    let mut seen_strings = HashSet::new();
    let mut index = 0;

    while let Some(line) = lines.get(index) {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }

        let (body, next) = block_lines(&lines, index + 1);
        if trimmed == "init python:" {
            let code: Vec<&str> = body.iter().map(|(_, text)| *text).collect();
            let indent = code.iter().map(|text| text.len() - text.trim_start().len()).min();
            let indent = indent.unwrap_or(0);
            let body_source = code
                .iter()
                .map(|text| &text[indent..])
                .collect::<Vec<_>>()
                .join("\n");
            let python = builder.python(line_number, format!("\n{}\n", body_source));
            statements.push(builder.init(line_number, vec![python]));
        } else if let Some(caps) = RE_TRANSLATE.captures(trimmed) {
            let (language, identifier) = (&caps[1], &caps[2]);
            match identifier {
                "strings" => {
                    let mut block = Vec::new();
                    let mut pairs = body.iter();
                    while let Some(&(old_line, old_text)) = pairs.next() {
                        let old = RenpyParser::old_text(old_text)
                            .ok_or_else(|| unsupported(name, old_line, old_text))?;
                        let &(new_line, new_text) = pairs
                            .next()
                            .ok_or_else(|| unsupported(name, old_line, old_text))?;
                        let new = RenpyParser::new_text(new_text)
                            .ok_or_else(|| unsupported(name, new_line, new_text))?;
                        // Повтор `old` в одном языке Ren'Py считает ошибкой загрузки
                        if seen_strings.insert((language.to_string(), old.clone())) {
                            block.push(builder.translate_string(
                                language,
                                (old, new),
                                (old_line, new_line),
                            ));
                        }
                    }
                    statements.push(builder.init(line_number, block));
                }
                "python" | "style" => return Err(unsupported(name, line_number, line)),
                _ => {
                    let mut block = Vec::new();
                    for &(body_line, body_text) in &body {
                        block.push(if body_text.trim() == "pass" {
                            builder.node("Pass", body_line, Vec::new())
                        } else {
                            let say = parse_say(name, body_line, body_text)?;
                            builder.say(body_line, say)
                        });
                    }
                    statements.push(builder.translate(line_number, language, identifier, block));
                    statements.push(builder.node("EndTranslate", line_number, Vec::new()));
                }
            }
        } else {
            return Err(unsupported(name, line_number, line));
        }
        index = next;
    }
    Ok(statements)
}

fn compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(3));
    encoder
        .write_all(data)
        .map_err(|e| format!("Ошибка сжатия скрипта: {}", e))?;
    encoder
        .finish()
        .map_err(|e| format!("Ошибка сжатия скрипта: {}", e))
}

/// Компилирует файл перевода `name` (относительно `tl/<язык>`) в байты `.rpyc`.
/// Поддерживаются блоки `translate` с репликами, `translate ... strings` и `init python`.
pub fn compile_script(
    format: ScriptFormat,
    language_dir: &str,
    name: &str,
    source: &str,
) -> Result<Vec<u8>, String> {
    let identity = stable_hash_str(&format!("{}\n{}", name, source));
    let mut builder = NodeBuilder {
        format,
        filename: format!("game/tl/{}/{}", language_dir, name),
        stamp: (identity & 0x7fff_ffff) as i64,
        serial: 0,
    };
    let mut statements = compile_statements(&mut builder, name, source)?;
    statements.push(builder.end_of_file());

    let mut data = vec![
        ("version", PickleValue::Int(format.script_version)),
        ("key", text("unlocked")),
    ];
    if format.layout != AstLayout::Python2 {
        let builtins = if format.layout == AstLayout::Python3 {
            "__builtin__"
        } else {
            "builtins"
        };
        data.push((
            "deferred_parse_errors",
            PickleValue::Object {
                class_module: "collections".to_string(),
                class_name: "defaultdict".to_string(),
                state: None,
                args: vec![PickleValue::Global {
                    module: builtins.to_string(),
                    name: "list".to_string(),
                }],
            },
        ));
    }
    let root = PickleValue::Tuple(vec![dict(data), PickleValue::List(statements)]);
    let slot = compress(&PickleEncoder::new().encode_value(&root))?;

    // Заголовок: сигнатура и таблица из трёх слотов; второй слот повторяет первый
    let header_size = 10 + 12 * 3;
    let mut output = Vec::with_capacity(header_size + slot.len() * 2 + 16);
    output.extend_from_slice(b"RENPY RPC2");
    for (id, offset) in [(1u32, header_size), (2, header_size + slot.len())] {
        output.extend_from_slice(&id.to_le_bytes());
        output.extend_from_slice(&(offset as u32).to_le_bytes());
        output.extend_from_slice(&(slot.len() as u32).to_le_bytes());
    }
    output.extend_from_slice(&[0; 12]);
    output.extend_from_slice(&slot);
    output.extend_from_slice(&slot);

    // В конце файла движок ждёт 16 байт отпечатка исходника. Из архива он только
    // учитывает их в общем отпечатке скриптов, поэтому подходит любое стабильное значение
    output.extend_from_slice(&identity.to_le_bytes());
    output.extend_from_slice(&stable_hash_str(source).to_le_bytes());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickle::PickleDecoder;
    use crate::rpyc::RpycParser;
    use std::collections::HashMap;

    const FORMATS: [AstLayout; 3] = [
        AstLayout::Python2,
        AstLayout::Python3,
        AstLayout::CompactNames,
    ];

    fn format(layout: AstLayout) -> ScriptFormat {
        ScriptFormat {
            layout,
            script_version: 5003000,
        }
    }

    fn compile(layout: AstLayout, source: &str) -> Result<Vec<u8>, String> {
        compile_script(format(layout), "russian", "story.rpy", source)
    }

    fn decoded(bytes: &[u8]) -> String {
        let slots = RpycParser::read_slots(bytes).unwrap();
        assert_eq!(slots.len(), 2);
        let first = RpycParser::decompress_slot(bytes, &slots[0]).unwrap();
        assert_eq!(first, RpycParser::decompress_slot(bytes, &slots[1]).unwrap());
        format!("{:?}", PickleDecoder::new(&first).decode().unwrap())
    }

    #[test]
    fn writes_container_like_the_engine() {
        let bytes = compile(AstLayout::Python2, "init python:\n    x = 1\n").unwrap();
        assert_eq!(&bytes[..10], b"RENPY RPC2");
        assert_eq!(u32::from_le_bytes(bytes[14..18].try_into().unwrap()), 46);
        assert_eq!(&bytes[34..46], &[0; 12]);
        let slots = RpycParser::read_slots(&bytes).unwrap();
        let end = (slots[1].offset + slots[1].length) as usize;
        assert_eq!(bytes.len(), end + 16);
    }

    #[test]
    fn keeps_translations_in_every_layout() {
        let source = concat!(
            "\u{feff}# Комментарий\n",
            "translate russian strings:\n\n",
            "    # game/screens.rpy:10\n",
            "    old \"New \\\"Game\\\"\"\n",
            "    new \"Новая игра\"\n\n",
            "translate russian start_abcd1234:\n\n",
            "    # e happy \"Hello\"\n",
            "    e happy \"Привет\"\n",
            "    \"Stranger\" \"Кто там?\"\n",
            "    \"Автор\"\n",
            "    pass\n\n",
            "init python:\n",
            "    if True:\n",
            "        config.language = \"russian\"\n",
        );
        for layout in FORMATS {
            let text = decoded(&compile(layout, source).unwrap());
            for expected in [
                "New \\\"Game\\\"",
                "Новая игра",
                "start_abcd1234",
                "Привет",
                "\\\"Stranger\\\"",
                "happy",
                "EndTranslate",
                "Pass",
                "\\nif True:\\n    config.language = \\\"russian\\\"\\n",
            ] {
                assert!(text.contains(expected), "{layout:?}: {expected} in {text}");
            }
        }
    }

    #[test]
    fn node_names_follow_the_layout() {
        let source = "translate russian strings:\n    old \"a\"\n    new \"b\"\n";
        let old = decoded(&compile(AstLayout::Python3, source).unwrap());
        assert!(old.contains("String(\"name\")") && old.contains("defaultdict"));
        assert!(old.contains("__builtin__"));
        let new = decoded(&compile(AstLayout::CompactNames, source).unwrap());
        assert!(new.contains("name_version") && !new.contains("String(\"name\")"));
        assert!(new.contains("\"builtins\""));
        let python2 = decoded(&compile(AstLayout::Python2, source).unwrap());
        assert!(!python2.contains("defaultdict"));
    }

    #[test]
    fn keeps_nointeract_and_with_on_dialogue() {
        let source = concat!(
            "translate russian a_1:\n",
            "    e \"Вопрос\" nointeract\n",
            "    e \"Удар\" with vpunch # комментарий\n",
            "    e \"Всё\" nointeract with Fade(0.1, 0, color=\"#fff (\")\n",
            "    e \"Обычная\"\n",
        );
        for layout in FORMATS {
            let text = decoded(&compile(layout, source).unwrap());
            for expected in ["\"vpunch\"", "Fade(0.1, 0, color=\\\"#fff (\\\")"] {
                assert!(text.contains(expected), "{layout:?}: {expected} in {text}");
            }
            assert!(text.matches("Bool(false)").count() >= 2, "{layout:?}: {text}");
            assert!(!text.contains("комментарий"), "{layout:?}: {text}");
        }

        // В новых версиях поле `with_` не имеет значения по умолчанию
        let compact = decoded(&compile(AstLayout::CompactNames, "translate russian a_1:\n    e \"Hi\"\n").unwrap());
        assert!(compact.contains("with_"), "{compact}");
        assert!(!compact.contains("arguments"), "{compact}");
    }

    #[test]
    fn keeps_arguments_and_temporary_attributes_on_dialogue() {
        let source = concat!(
            "translate russian a_1:\n",
            "    gen \"(Да, *а*?)\" (\"base\", xpos=\"far (left\", ypos=\"head\") # комментарий\n",
            "    her happy @ cheeks -blush \"Привет\" (size == 2, f(1, 2)) with fade\n",
            "    \"Автор\" () nointeract\n",
        );
        for layout in FORMATS {
            let text = decoded(&compile(layout, source).unwrap());
            for expected in [
                "ArgumentInfo",
                "\"\\\"base\\\"\"",
                "\"xpos\"",
                "\"\\\"far (left\\\"\"",
                "\"size == 2\"",
                "\"f(1, 2)\"",
                "\"temporary_attributes\"",
                "\"cheeks\"",
                "\"-blush\"",
                "\"happy\"",
                "\"fade\"",
                "(Да, *а*?)",
            ] {
                assert!(text.contains(expected), "{layout:?}: {expected} in {text}");
            }
            assert!(!text.contains("комментарий"), "{layout:?}: {text}");
            assert!(!text.contains("\"@\""), "{layout:?}: {text}");
        }
    }

    #[test]
    fn rejects_statements_it_cannot_compile() {
        for (source, line) in [
            ("translate russian a_1:\n    voice \"a.ogg\"\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (*args)\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (**kwargs)\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (\"a\"\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (a=1, \"b\")\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (a=1, a=2)\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (a) (b)\n", 2),
            ("translate russian a_1:\n    e \"Hi\" (,)\n", 2),
            ("translate russian a_1:\n    e @ a @ b \"Hi\"\n", 2),
            ("translate russian a_1:\n    e \"Hi\" id custom\n", 2),
            ("translate russian a_1:\n    e \"Hi\" with\n", 2),
            ("translate russian a_1:\n    e \"Hi\" with fade with dissolve\n", 2),
            ("translate russian a_1:\n    e \"Hi\" with Fade(1\n", 2),
            ("translate russian a_1:\n    e \"Hi\" nointeractive\n", 2),
            ("translate russian python:\n    x = 1\n", 1),
            ("translate russian strings:\n    old \"a\"\n", 2),
            ("# ok\nlabel start:\n", 2),
        ] {
            let error = compile(AstLayout::Python2, source).unwrap_err();
            assert!(error.contains(&format!("строка {line}:")), "{source}: {error}");
        }
    }

    #[test]
    fn restores_translation_blocks_from_compiled_scripts() {
        let source = concat!(
            "translate russian a_1:\n",
            "    gen happy @ cheeks \"Да,  \\\"так\\\"\" (\"base\", xpos=\"far_left\") nointeract with fade\n",
            "translate russian a_2:\n",
            "    \"Автор\"\n",
            "translate russian a_3:\n",
            "    pass\n",
            "translate russian strings:\n",
            "    old \"Start\"\n",
            "    new \"Начать\"\n",
        );
        let originals = HashMap::from([("a_1".to_string(), "gen \"Yes\"".to_string())]);
        for layout in FORMATS {
            let compiled = compile(layout, source).unwrap();
            let text = RpycParser::decompile_translation(&compiled, &originals).unwrap();
            for expected in [
                "translate russian a_1:\n    # gen \"Yes\"\n",
                "    gen happy @ cheeks \"Да, \\\"так\\\"\" nointeract (\"base\", xpos=\"far_left\") with fade\n",
                "translate russian a_2:\n    # \"Автор\"\n    \"Автор\"\n",
                "translate russian a_3:\n    pass\n",
                "translate russian strings:\n\n    old \"Start\"\n    new \"Начать\"\n",
            ] {
                assert!(text.contains(expected), "{layout:?}: {expected} in {text}");
            }

            let again = compile(layout, &text).unwrap();
            let restored = RpycParser::decompile_translation(&again, &originals).unwrap();
            assert_eq!(restored, text, "{layout:?}");
        }
    }

    #[test]
    fn marks_statements_it_cannot_restore() {
        let compiled = compile(AstLayout::Python3, "init python:\n    x = 1\n").unwrap();
        let text = RpycParser::decompile_translation(&compiled, &HashMap::new()).unwrap();
        assert!(text.contains("<Python>"), "{text}");
        assert!(compile(AstLayout::Python3, &text).is_err());
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn compiles_real_translation_folder() {
        let Some(dir) = crate::local_test_game(crate::TEST_TL_DIR) else {
            return;
        };
        let mut files = Vec::new();
        crate::collect_rpy_files(Path::new(&dir), &mut files);
        let failures: Vec<String> = files
            .iter()
            .filter_map(|file| compile(AstLayout::Python3, &fs::read_to_string(file).ok()?).err())
            .collect();
        for failure in &failures {
            println!("{failure}");
        }
        assert!(failures.is_empty(), "не собрано {} из {}", failures.len(), files.len());
    }

    /// Восстанавливает запакованный перевод игры и собирает его заново.
    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn recompiles_packed_translation() {
        let Some(game) = crate::local_test_game(crate::TEST_PACKED_TL_GAME) else {
            return;
        };
        let game = Path::new(&game);
        let cache = std::env::temp_dir().join(format!("translator_packed_{}", uuid::Uuid::new_v4()));
        crate::rpa::RpaArchive::extract_scripts_from_game_archives(game, &cache).unwrap();

        let mut files = Vec::new();
        crate::collect_rpy_files(&cache.join("tl").join(crate::TL_LANG), &mut files);
        let format = ScriptFormat::detect(game).unwrap();
        let mut failures = std::collections::BTreeMap::new();
        let (mut blocks, mut with_original) = (0, 0);
        let mut samples = Vec::new();
        for file in &files {
            let text = fs::read_to_string(file).unwrap();
            blocks += text.matches("    # ").count();
            with_original += text
                .lines()
                .filter(|line| line.trim_start().starts_with("# "))
                .filter(|line| !crate::parser::renpy::RenpyParser::has_cyrillic(line))
                .count();
            for (index, line) in text.lines().enumerate() {
                let is_russian_comment = line.trim_start().starts_with("# ")
                    && crate::parser::renpy::RenpyParser::has_cyrillic(line);
                if is_russian_comment && samples.len() < 3 {
                    samples.push((index, line.to_string()));
                }
            }
            if let Err(error) = compile_script(format, "russian", "file.rpy", &text) {
                let kind = error.split('«').nth(1).unwrap_or(&error).split(' ').next().unwrap_or("").to_string();
                *failures.entry(kind).or_insert(0) += 1;
            }
        }
        let packed = crate::rpa::RpaArchive::packed_translation_files(game).unwrap();
        println!(
            "файлов {}, реплик {}, с оригиналом {}, файлов архива {}, не собрано {:?}\nбез оригинала: {:?}",
            files.len(),
            blocks,
            with_original,
            packed.len(),
            failures,
            samples
        );
        let _ = fs::remove_dir_all(&cache);
        assert!(!files.is_empty());
    }

    /// Пишет архивы для проверки настоящим движком: `TRANSLATOR_RPYC_PROBE_DIR=<папка>`
    /// и `cargo test -- --ignored`. Без переменной тест ничего не проверяет.
    #[test]
    #[ignore = "Создаёт файлы для проверки настоящим движком Ren'Py"]
    fn write_engine_probe_archives() {
        let Ok(dir) = std::env::var("TRANSLATOR_RPYC_PROBE_DIR") else {
            return;
        };
        let sources = [
            (
                "translator_saved_strings.rpy",
                "translate russian strings:\n\n    old \"New Game\"\n    new \"Новая игра\"\n",
            ),
            (
                "translator_language.rpy",
                "init python:\n    config.language = \"russian\"\n",
            ),
            (
                "story.rpy",
                concat!(
                    "translate russian start_abcd1234:\n",
                    "    # e \"Hello\"\n",
                    "    e happy \"Привет\"\n",
                    "    \"Автор\"\n",
                    "    \"Stranger\" \"Кто там?\"\n",
                    "translate russian start_abcd1235:\n",
                    "    pass\n",
                    "translate russian line_a:\n",
                    "    e happy @ cheeks \"Блок мода\" (\"base\", xpos=\"far_left\", size=1 + 1)\n",
                ),
            ),
        ];
        for layout in FORMATS {
            let files: Vec<_> = sources
                .iter()
                .map(|(name, source)| {
                    (
                        format!("tl/russian/{}c", name),
                        compile_script(format(layout), "russian", name, source).unwrap(),
                    )
                })
                .collect();
            let path = Path::new(&dir).join(format!("{layout:?}.rpa"));
            crate::rpa::RpaArchive::create_archive(&path, &files).unwrap();
        }
    }

    #[test]
    fn detects_format_from_engine_files() {
        let root = std::env::temp_dir().join(format!("translator_format_{}", uuid::Uuid::new_v4()));
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        assert!(ScriptFormat::detect(&game).is_err());

        let engine = root.join("renpy");
        fs::create_dir_all(&engine).unwrap();
        fs::write(engine.join("__init__.py"), "x = 1\nscript_version = 5003000\n").unwrap();
        fs::write(engine.join("ast.py"), "class Node: pass\n").unwrap();
        fs::create_dir_all(root.join("lib").join("windows-x86_64")).unwrap();
        assert_eq!(ScriptFormat::detect(&game).unwrap(), format(AstLayout::Python2));

        fs::create_dir_all(root.join("lib").join("py3-windows-x86_64")).unwrap();
        assert_eq!(ScriptFormat::detect(&game).unwrap(), format(AstLayout::Python3));

        fs::write(engine.join("__init__.py"), "script_version: int = 5003000\n").unwrap();
        fs::write(engine.join("ast.py"), "name_version: int\n").unwrap();
        assert_eq!(ScriptFormat::detect(&game).unwrap(), format(AstLayout::CompactNames));
        let _ = fs::remove_dir_all(&root);
    }
}
