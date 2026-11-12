//! Текстовые переменные игры: чем скрипты задают переменную, которая стоит в реплике токеном `[mc]`.
//!
//! Разбор построчный и приблизительный, Python он не исполняет. Считаются `define`, `default`, `$ x = ...`,
//! строки блоков `python:` вне функций и классов и действия экранов (`SetVariable`, `VariableInputValue`).
//! Чего разбор не видит, остаётся `Unknown`: шаг токенов тогда судит по репликам.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::script::unescape_python_string;
use super::snapshot::script_texts;
use crate::engines::{EngineError, SnapshotFile, TextVariable, VariableSource};
use crate::files::FileStore;

/// Сколько разных строк переменной запоминается: дальше и так ясно, что значение меняется.
const MAX_VALUES: usize = 12;
/// Сколько раз итог идёт по ссылке на другую переменную: `define mc = Character("[name]")`.
const MAX_ALIAS_DEPTH: usize = 3;

/// `define x = ...`, `default x = ...`, в том числе с приоритетом (`define -1 x = ...`).
static RE_DEFINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:define|default)\s+(?:-?\d+\s+)?(?P<name>[A-Za-z_][\w.]*)\s*(?P<op>[-+|]?=)\s*(?P<code>[^=].*)$").expect("регулярное выражение define")
});

/// Присваивание в строке Python: `x = ...`, `store.x = ...`, `x += ...`. Сравнение `x == y` не подходит.
static RE_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<name>[A-Za-z_][\w.]*)\s*(?P<op>(?:\*\*|//|>>|<<|[-+*/%|&^@])?=)\s*(?P<code>[^=].*)$").expect("регулярное выражение присваивания")
});

/// Присваивание нескольким переменным сразу: `a, b = ...`.
static RE_TUPLE_ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<names>[A-Za-z_][\w.]*(?:\s*,\s*[A-Za-z_][\w.]*)+)\s*=\s*[^=]").expect("регулярное выражение присваивания нескольким")
});

/// Блок Python: `python:`, `init python:`, `init -1 python in gui:`, `python hide:`.
static RE_PYTHON_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:init\s+(?:-?\d+\s+)?)?python\b(?P<mods>[^:]*):\s*(?:#.*)?$").expect("регулярное выражение блока python")
});

/// Хранилище блока: `python in gui:`.
static RE_BLOCK_STORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bin\s+([A-Za-z_][\w.]*)").expect("регулярное выражение хранилища"));

/// Начало функции или класса: присваивания внутри локальные.
static RE_SCOPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:async\s+)?(?:def|class)\b").expect("регулярное выражение функции"));

/// Персонаж: в реплике `[mc]` движок подставляет его имя.
static RE_CHARACTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:renpy\.)?(?P<kind>Character|DynamicCharacter|ADVCharacter|NVLCharacter|adv_character)\s*\(\s*(?P<rest>.*)$")
        .expect("регулярное выражение персонажа")
});

/// Поле ввода экрана, которое пишет в переменную: `input value VariableInputValue("mc")`.
static RE_INPUT_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\bVariableInputValue\(\s*(?:variable\s*=\s*)?[uU]?["'](?P<name>[A-Za-z_][\w.]*)["']"#).expect("регулярное выражение поля ввода")
});

/// Поле ввода экрана, которое пишет в поле объекта: `FieldInputValue(persistent, "name")`.
static RE_FIELD_INPUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\bFieldInputValue\(\s*(?P<object>[A-Za-z_][\w.]*)\s*,\s*[uU]?["'](?P<field>[A-Za-z_]\w*)["']"#).expect("регулярное выражение поля ввода")
});

/// Действие кнопки, которое пишет в переменную: `SetVariable("title", "Mr.")`, `ToggleVariable("flag")`.
static RE_SET_VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(?P<action>SetVariable|ToggleVariable)\(\s*[uU]?["'](?P<name>[A-Za-z_][\w.]*)["']"#).expect("регулярное выражение действия")
});

/// Строка из одной переменной: `"[name]"`, `"[name!c]"`.
static RE_ONLY_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([A-Za-z_][\w.]*)(?:![a-z]+)?\]$").expect("регулярное выражение токена"));

/// Тег текста в вопросе игроку: `{i}`, `{/i}`, `{color=#fff}`.
static RE_TEXT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").expect("регулярное выражение тега"));

/// Что скрипты снимка говорят о переменных `names`. Нечитаемый `.rpyc` пропускается, как при считывании.
pub(crate) fn text_variables(names: &[String], files: &[SnapshotFile], store: &FileStore) -> Result<Vec<TextVariable>, EngineError> {
    let mut tally = Tally::default();
    if !names.is_empty() {
        for script in script_texts(files, store, None)? {
            tally.read(&script.text);
        }
    }
    Ok(names.iter().map(|name| TextVariable { name: name.clone(), source: tally.verdict(name) }).collect())
}

/// Чем присваивание задаёт переменную.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Assigned {
    /// Строка-литерал, в том числе пустая.
    Text(String),
    /// Ввод игрока и вопрос, который ему задан.
    Input(Option<String>),
    /// Значение другой переменной: `"[name]"`, `Character("[name]")`.
    Alias(String),
    /// Выражение, число, изменение на месте.
    Computed,
}

/// Сколько раз и чем скрипты задают одну переменную.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Uses {
    /// Разные непустые строки по порядку скриптов, не больше `MAX_VALUES`.
    texts: Vec<String>,
    /// Присваиваний пустой строки: начальное значение, которое что-то потом заменит.
    blank: usize,
    inputs: usize,
    /// Первый вопрос игроку.
    prompt: Option<String>,
    /// Переменные, чьё значение берёт эта.
    aliases: Vec<String>,
    computed: usize,
}

impl Uses {
    fn add(&mut self, assigned: Assigned) {
        match assigned {
            Assigned::Text(text) if text.trim().is_empty() => self.blank += 1,
            Assigned::Text(text) => {
                if !self.texts.contains(&text) && self.texts.len() < MAX_VALUES {
                    self.texts.push(text);
                }
            }
            Assigned::Input(prompt) => {
                self.inputs += 1;
                self.prompt = self.prompt.take().or(prompt);
            }
            Assigned::Alias(name) => {
                if !self.aliases.contains(&name) {
                    self.aliases.push(name);
                }
            }
            Assigned::Computed => self.computed += 1,
        }
    }
}

/// Открытая функция или класс в коде Python: строки глубже её отступа задают локальные переменные.
#[derive(Debug, Default)]
struct Scope {
    inside: Option<usize>,
}

impl Scope {
    /// Стоит ли строка вне функций и классов. Вызывается для каждой строки кода по порядку.
    fn top_level(&mut self, indent: usize, code: &str) -> bool {
        if self.inside.is_some_and(|open| indent > open) {
            return false;
        }
        self.inside = None;
        if RE_SCOPE.is_match(code) {
            self.inside = Some(indent);
            return false;
        }
        true
    }
}

/// Открытый блок `python:`.
#[derive(Debug)]
struct PythonBlock {
    indent: usize,
    /// Хранилище блока `python in gui:`.
    store: Option<String>,
    /// `python hide:`: переменные блока локальные.
    hidden: bool,
    scope: Scope,
}

/// Присваивания всех переменных игры. Имя с хранилищем: `mc`, `persistent.name`, `gui.about`.
#[derive(Debug, Default)]
struct Tally {
    uses: HashMap<String, Uses>,
}

impl Tally {
    fn record(&mut self, name: &str, assigned: Assigned) {
        self.uses.entry(plain_name(name).to_string()).or_default().add(assigned);
    }

    /// Читает один скрипт: `.rpy` как есть или декомпилированный `.rpyc`, где код Python идёт строками `$`.
    fn read(&mut self, text: &str) {
        let mut block: Option<PythonBlock> = None;
        let mut dollar = Scope::default();
        for raw in text.lines() {
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indent = line.len() - trimmed.len();
            self.screen_actions(trimmed);
            if let Some(open) = &mut block {
                if indent > open.indent {
                    if open.scope.top_level(indent, trimmed) && !open.hidden {
                        let store = open.store.clone();
                        self.python(trimmed, store.as_deref());
                    }
                    continue;
                }
                block = None;
            }
            if let Some(code) = trimmed.strip_prefix('$') {
                let inner = code.trim_start();
                if dollar.top_level(code.len() - inner.len(), inner) {
                    self.python(inner, None);
                }
                continue;
            }
            dollar = Scope::default();
            if let Some(caps) = RE_PYTHON_BLOCK.captures(trimmed) {
                let mods = &caps["mods"];
                block = Some(PythonBlock {
                    indent,
                    store: RE_BLOCK_STORE.captures(mods).map(|caps| caps[1].to_string()),
                    hidden: mods.split_whitespace().any(|word| word == "hide"),
                    scope: Scope::default(),
                });
            } else if let Some(caps) = RE_DEFINE.captures(trimmed) {
                self.record(&caps["name"], assigned(&caps["op"], &caps["code"]));
            }
        }
    }

    /// Строка кода Python вне функций. `store`: хранилище блока, в нём лежат простые имена.
    fn python(&mut self, code: &str, store: Option<&str>) {
        let stored = |name: &str| match store {
            Some(store) if !name.contains('.') => format!("{store}.{name}"),
            _ => name.to_string(),
        };
        if let Some(caps) = RE_ASSIGN.captures(code) {
            self.record(&stored(&caps["name"]), assigned(&caps["op"], &caps["code"]));
        } else if let Some(caps) = RE_TUPLE_ASSIGN.captures(code) {
            for name in caps["names"].split(',') {
                self.record(&stored(name.trim()), Assigned::Computed);
            }
        }
    }

    /// Поля ввода и действия кнопок в строке: они пишут в переменную без присваивания.
    fn screen_actions(&mut self, line: &str) {
        if !line.contains("InputValue(") && !line.contains("Variable(") {
            return;
        }
        for caps in RE_INPUT_VALUE.captures_iter(line) {
            self.record(&caps["name"], Assigned::Input(None));
        }
        for caps in RE_FIELD_INPUT.captures_iter(line) {
            self.record(&format!("{}.{}", &caps["object"], &caps["field"]), Assigned::Input(None));
        }
        for caps in RE_SET_VARIABLE.captures_iter(line) {
            let rest = caps.get(0).map_or("", |found| &line[found.end()..]);
            let value = (&caps["action"] == "SetVariable").then(|| rest.trim_start().strip_prefix(',')).flatten().and_then(leading_value);
            let assigned = match value {
                Some((value, after)) if after.trim_start().starts_with(')') => worded(value),
                _ => Assigned::Computed,
            };
            self.record(&caps["name"], assigned);
        }
    }

    fn verdict(&self, name: &str) -> VariableSource {
        self.verdict_at(plain_name(name), 0)
    }

    fn verdict_at(&self, name: &str, depth: usize) -> VariableSource {
        let Some(uses) = self.uses.get(name) else { return VariableSource::Unknown };
        if uses.inputs > 0 {
            return VariableSource::PlayerInput { prompt: uses.prompt.clone() };
        }
        match (uses.computed, uses.texts.as_slice(), uses.aliases.as_slice()) {
            (0, [], [alias]) if depth < MAX_ALIAS_DEPTH => self.verdict_at(alias, depth + 1),
            (0, [value], []) => VariableSource::Constant(value.clone()),
            _ => VariableSource::Varies { values: uses.texts.clone() },
        }
    }
}

/// Имя без общего хранилища: `store.mc` и `renpy.store.mc` это `mc`.
fn plain_name(name: &str) -> &str {
    name.strip_prefix("renpy.store.").or_else(|| name.strip_prefix("store.")).unwrap_or(name)
}

/// Чем выражение справа от `op` задаёт переменную.
fn assigned(op: &str, code: &str) -> Assigned {
    let code = code.trim();
    if let Some(at) = code.find("renpy.input(") {
        let arguments = code[at + "renpy.input(".len()..].trim_start();
        let arguments = arguments.strip_prefix("prompt").map_or(arguments, |rest| rest.trim_start().trim_start_matches('=').trim_start());
        return Assigned::Input(leading_value(arguments).map(|(prompt, _)| plain_prompt(&prompt)).filter(|prompt| !prompt.is_empty()));
    }
    if op != "=" {
        return Assigned::Computed;
    }
    if let Some((value, after)) = leading_value(code) {
        let after = after.trim_start();
        return if after.is_empty() || after.starts_with('#') { worded(value) } else { Assigned::Computed };
    }
    let Some(caps) = RE_CHARACTER.captures(code) else { return Assigned::Computed };
    let rest = &caps["rest"];
    let Some((name, _)) = leading_value(rest) else { return Assigned::Computed };
    // Имя динамического персонажа это выражение: обычно имя переменной.
    if &caps["kind"] == "DynamicCharacter" || rest.replace(' ', "").contains("dynamic=True") {
        let simple = name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') && !name.is_empty();
        return if simple { Assigned::Alias(plain_name(&name).to_string()) } else { Assigned::Computed };
    }
    worded(name)
}

/// Вопрос игроку одной строкой и без тегов текста: общий код разметки движка не знает.
fn plain_prompt(prompt: &str) -> String {
    RE_TEXT_TAG.replace_all(prompt, "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Строка как значение: текст, ссылка на переменную (`"[name]"`) или текст с подстановкой, который считает игра.
fn worded(value: String) -> Assigned {
    if let Some(caps) = RE_ONLY_TOKEN.captures(value.trim()) {
        return Assigned::Alias(plain_name(&caps[1]).to_string());
    }
    if value.contains('[') { Assigned::Computed } else { Assigned::Text(value) }
}

/// Строка в начале выражения, сама или в `_()`: значение и остаток кода после неё.
fn leading_value(code: &str) -> Option<(String, &str)> {
    let code = code.trim_start();
    let wrapped = ["__(", "_p(", "_("].iter().find_map(|call| code.strip_prefix(call));
    let (value, after) = leading_string(wrapped.unwrap_or(code).trim_start())?;
    match wrapped {
        Some(_) => Some((value, after.trim_start().strip_prefix(')')?)),
        None => Some((value, after)),
    }
}

/// Строка-литерал Python в начале `code`: значение и остаток после неё. Строки в тройных кавычках, f-строки
/// и сырые строки не читаются.
fn leading_string(code: &str) -> Option<(String, &str)> {
    let code = code.strip_prefix(['u', 'U']).filter(|rest| rest.starts_with(['"', '\''])).unwrap_or(code);
    let quote = code.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let body = &code[1..];
    if body.starts_with(quote) && body[1..].starts_with(quote) {
        return None;
    }
    let mut escaped = false;
    for (index, c) in body.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == quote => return Some((unescape_python_string(&body[..index]), &body[index + 1..])),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::FileKind;

    fn verdicts(script: &str, names: &[&str]) -> Vec<VariableSource> {
        let mut tally = Tally::default();
        tally.read(script);
        names.iter().map(|name| tally.verdict(name)).collect()
    }

    fn constant(value: &str) -> VariableSource {
        VariableSource::Constant(value.to_string())
    }

    fn varies(values: &[&str]) -> VariableSource {
        VariableSource::Varies { values: values.iter().map(|value| value.to_string()).collect() }
    }

    fn input(prompt: Option<&str>) -> VariableSource {
        VariableSource::PlayerInput { prompt: prompt.map(str::to_string) }
    }

    #[test]
    fn one_string_is_a_constant_and_several_vary() {
        let script = r#"
define mom = _("Mom")  # как герой зовёт Селию
default town = 'Old "Harbor"'
default title = "Mr."
default score = 0
default nick = ""
default full = "[first] Smith"

label start:
    $ town = "Old \"Harbor\""
    if female:
        $ title = "Ms."
    $ score += 1
    $ greeting = "Hello, %s" % name
    $ a, b = 1, "two"
    $ ready == True
"#;
        assert_eq!(
            verdicts(script, &["mom", "town", "title", "score", "nick", "full", "greeting", "a", "b", "ready", "nobody"]),
            [
                constant("Mom"),
                constant("Old \"Harbor\""),
                varies(&["Mr.", "Ms."]),
                varies(&[]),
                varies(&[]),
                varies(&[]),
                varies(&[]),
                varies(&[]),
                varies(&[]),
                VariableSource::Unknown,
                VariableSource::Unknown,
            ],
            "пустая строка и число не значение, сравнение не присваивание, незаданная переменная неизвестна"
        );
    }

    #[test]
    fn input_wins_over_defaults_and_keeps_the_question() {
        let script = r#"
default mc = "Max"
default pet = ""
default persistent.nick = "Stranger"

label start:
    $ mc = renpy.input(_("{i}What is\n your name?{/i}"), length=12).strip() or "Max"
    python:
        pet = renpy.input(prompt='Name your dog')
        pet = pet.strip()
        if not pet:
            pet = "Rex"

screen profile():
    input value VariableInputValue("town")
    input value FieldInputValue(persistent, "nick")
"#;
        assert_eq!(
            verdicts(script, &["mc", "pet", "town", "persistent.nick"]),
            [input(Some("What is your name?")), input(Some("Name your dog")), input(None), input(None)]
        );
    }

    #[test]
    fn character_gives_its_name_and_follows_the_variable_in_it() {
        let script = r##"
define e = Character("Eileen", color="#fff")
define mc = Character("[name]")
define n = Character(None, kind=nvl)
define d = DynamicCharacter("boss_name")
define both = Character("[name] & [e]")
default boss_name = "Vera"
default shown = "[name!c]"

label start:
    $ name = renpy.input("Name?")
"##;
        assert_eq!(
            verdicts(script, &["e", "mc", "n", "d", "both", "shown", "store.mc"]),
            [constant("Eileen"), input(Some("Name?")), varies(&[]), constant("Vera"), varies(&[]), input(Some("Name?")), input(Some("Name?"))]
        );
        assert_eq!(verdicts("define a = Character(\"[b]\")\ndefine b = Character(\"[a]\")\n", &["a"]), [varies(&[])], "круг ссылок не константа");
        assert_eq!(verdicts("define mc = Character(\"[hero]\")\n", &["mc"]), [VariableSource::Unknown], "ссылка на незаданную переменную");
    }

    #[test]
    fn python_blocks_count_only_code_outside_functions() {
        let script = r#"
init python:
    title = "Captain"
    def rename(name):
        name = name.strip()
        title = "Nobody"
        return name
    class Hero:
        rank = "Private"
    rank = "Major"

init -1 python in gui:
    about = "A game"

python hide:
    secret = "x"

define config.name = "My Game"

label start:
    $ place = "Bar"
"#;
        assert_eq!(
            verdicts(script, &["title", "name", "rank", "gui.about", "about", "secret", "config.name", "place"]),
            [
                constant("Captain"),
                VariableSource::Unknown,
                constant("Major"),
                constant("A game"),
                VariableSource::Unknown,
                VariableSource::Unknown,
                constant("My Game"),
                constant("Bar"),
            ],
            "переменные функций, классов и скрытых блоков локальные, имя хранилища входит в имя переменной"
        );
    }

    #[test]
    fn decompiled_python_keeps_function_bodies_by_indent() {
        // Так декомпилятор пишет блок python из .rpyc: строки `$` с отступом тела.
        let script = "$ def rename(name):\n$     name = name.strip()\n$     return name\n$ title = \"Captain\"\n";
        assert_eq!(verdicts(script, &["name", "title"]), [VariableSource::Unknown, constant("Captain")]);
    }

    #[test]
    fn screen_actions_assign_too() {
        let script = r#"
default gender = "male"
default mode = "easy"

screen pick():
    textbutton "She" action [SetVariable("gender", "female"), Jump("start")]
    textbutton "Hard" action SetVariable("mode", difficulty)
    textbutton "Flag" action ToggleVariable("flag")
"#;
        assert_eq!(verdicts(script, &["gender", "mode", "flag"]), [varies(&["male", "female"]), varies(&["easy"]), varies(&[])]);
    }

    #[test]
    fn reads_scripts_of_the_snapshot_and_skips_translations() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let file = |path: &str, kind: FileKind, content: &str| {
            let blob = store.put(content.as_bytes()).unwrap();
            SnapshotFile { path: path.into(), kind, hash: blob.hash, size: blob.size }
        };
        let files = vec![
            file("game/script.rpy", FileKind::Script, "default mom = \"Mom\"\n"),
            file("game/tl/russian/script.rpy", FileKind::Translation, "default mom = \"Мама\"\n"),
        ];
        let names = ["mom".to_string(), "friend.name".to_string()];
        let found = text_variables(&names, &files, &store).unwrap();
        assert_eq!(
            found,
            [
                TextVariable { name: "mom".to_string(), source: constant("Mom") },
                TextVariable { name: "friend.name".to_string(), source: VariableSource::Unknown },
            ]
        );
        assert!(text_variables(&[], &files, &store).unwrap().is_empty());
    }

    /// Считывает настоящую игру из `TRANSLATOR_TEST_VARIABLES_GAME` (папка `game`) и печатает итог по каждому
    /// токену её реплик.
    #[test]
    #[ignore = "Читает настоящую игру: TRANSLATOR_TEST_VARIABLES_GAME"]
    fn classifies_tokens_of_a_real_game() {
        use crate::engines::{GameEngine, GameSource, renpy::RenpyEngine};
        use crate::projects::GameRoot;

        let Some(game) = crate::engines::renpy::local_test_game("TRANSLATOR_TEST_VARIABLES_GAME") else { return };
        let root = GameRoot::new(std::path::Path::new(&game).parent().unwrap()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let info = RenpyEngine.detect(&root).expect("игра Ren'Py");
        let snapshot = RenpyEngine.read(&GameSource::Folder(root), &info, &store).unwrap();
        let mut lines: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for segment in &snapshot.segments {
            for name in crate::translation::text::token_names(&segment.source_text) {
                *lines.entry(name.to_string()).or_default() += 1;
            }
        }
        let names: Vec<String> = lines.keys().cloned().collect();
        let started = std::time::Instant::now();
        let found = RenpyEngine.text_variables(&names, &snapshot.files, &store).unwrap();
        println!("{game}: токенов {}, разбор {:?}", names.len(), started.elapsed());
        assert_eq!(found.len(), names.len());
        for variable in &found {
            println!("{:>6}  [{}]  {:?}", lines[&variable.name], variable.name, variable.source);
        }
        let known = found.iter().filter(|variable| variable.source != VariableSource::Unknown).count();
        println!("известно {known} из {}", found.len());
    }
}
