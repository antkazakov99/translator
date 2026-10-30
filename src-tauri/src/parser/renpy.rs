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

static RE_STRUCT_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*label\s+([A-Za-z_]\w*)\s*:"#).unwrap());

static RE_STRUCT_MENU: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*menu(?:\s*([A-Za-z_]\w*))?\s*:"#).unwrap());

static RE_STRUCT_CHOICE_OPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*"([^"]+)"\s*(?:if\s+[^:]+)?\s*:"#).unwrap());

static RE_STRUCT_JUMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:if\s+(?P<cond>[^:]+):\s*)?(?P<cmd>jump|call)\s+(?P<target>[A-Za-z_]\w*)"#)
        .unwrap()
});

static RE_STRUCT_IF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*(?:elif|if)\s+(.+?)\s*:"#).unwrap());

static RE_STRUCT_ELSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*else\s*:"#).unwrap());

static RE_ORIG_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*label\s+([A-Za-z0-9_.]+)\s*:"#).unwrap());

static RE_ORIG_DIALOGUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:(?P<speaker>[A-Za-z_][\w.]*)\s+)?"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap()
});

static RE_STRINGS_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^translate\s+\w+\s+strings\s*:"#).unwrap());

static RE_DIALOGUE_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^translate\s+\w+\s+([\w\.]+)\s*:"#).unwrap());

static RE_OLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*old\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap());

static RE_NEW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*new\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap());

static RE_DIALOGUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:(?P<speaker>[A-Za-z_][\w.]*)\s+)?"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap()
});

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
            for (_, v) in map.iter_mut() {
                if v == "[player.name]" || v == "[player]" {
                    *v = player_name.clone();
                }
            }
        }

        map
    }

    /// Разбирает кавычки и экранирование в стиле строк Ren'Py
    pub fn unescape_renpy_string(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    match next {
                        'n' => {
                            result.push('\n');
                            chars.next();
                        }
                        't' => {
                            result.push('\t');
                            chars.next();
                        }
                        'r' => {
                            result.push('\r');
                            chars.next();
                        }
                        '"' => {
                            result.push('"');
                            chars.next();
                        }
                        '\\' => {
                            result.push('\\');
                            chars.next();
                        }
                        '\'' => {
                            result.push('\'');
                            chars.next();
                        }
                        _ => {
                            result.push(c);
                        }
                    }
                } else {
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Экранирует текст для вставки в строку Ren'Py в двойных кавычках
    pub fn escape_renpy_string(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '\n' => result.push_str("\\n"),
                '\t' => result.push_str("\\t"),
                '\r' => result.push_str("\\r"),
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                _ => result.push(c),
            }
        }
        result
    }

    /// Проверяет наличие кириллических символов в тексте
    pub fn has_cyrillic(text: &str) -> bool {
        text.chars()
            .any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) || c == 'Ё' || c == 'ё')
    }

    /// Сверхбыстрый подсчет статистики файла перевода (всего строк, переведено)
    /// без построения графа petgraph, без регулярных выражений и без генерации UUID.
    /// Работает в 100+ раз быстрее полного парсинга.
    pub fn quick_translation_stats(content: &str) -> (usize, usize) {
        if !content.contains("translate ") {
            let mut total = 0;
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with('#') && trimmed.contains('"') {
                    if !trimmed.starts_with("define ")
                        && !trimmed.starts_with("image ")
                        && !trimmed.starts_with("default ")
                        && !trimmed.starts_with('$')
                    {
                        total += 1;
                    }
                }
            }
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
                    if let Some(s) = trimmed.find('"') {
                        if let Some(e) = trimmed.rfind('"') {
                            if e > s {
                                pending_old = Some(&trimmed[s + 1..e]);
                            }
                        }
                    }
                } else if trimmed.starts_with("new ") {
                    if let Some(old_str) = pending_old.take() {
                        if seen_ui_strings.insert(old_str) {
                            total += 1;
                            if let Some(s) = trimmed.find('"') {
                                if let Some(e) = trimmed.rfind('"') {
                                    if e > s {
                                        let new_str = &trimmed[s + 1..e];
                                        if Self::has_cyrillic(new_str) {
                                            translated += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }

            if in_block {
                if trimmed.starts_with('#') {
                    // Игнорируем комментарии путей вроде `# game/story.rpy:10`
                    if trimmed.contains('"') {
                        pending_dialogue = true;
                    }
                } else if pending_dialogue && trimmed.contains('"') {
                    total += 1;
                    if Self::has_cyrillic(trimmed) {
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
            if let Some(node) = graph.get_node(idx) {
                match node {
                    ScenarioNode::Dialogue(d) => {
                        if d.speaker.as_deref() == Some("UI / Choice") {
                            string_entries.push(d);
                        } else {
                            if let Some(ref block_id) = d.block_id {
                                if current_block_id.as_deref() != Some(block_id) {
                                    output.push_str(&format!(
                                        "\ntranslate {} {}:\n\n",
                                        target_lang, block_id
                                    ));
                                    current_block_id = Some(block_id.clone());
                                }
                            }

                            let alias_prefix = self.resolve_alias(d.speaker.as_deref());
                            let original_escaped = Self::escape_renpy_string(&d.original_text);
                            let target_text =
                                d.translated_text.as_deref().unwrap_or(&d.original_text);
                            let target_escaped = Self::escape_renpy_string(target_text);

                            output.push_str(&format!(
                                "    # {}\"{}\"\n",
                                alias_prefix, original_escaped
                            ));
                            output.push_str(&format!(
                                "    {}\"{}\"\n\n",
                                alias_prefix, target_escaped
                            ));
                        }
                    }
                    _ => {}
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
        let mut in_menu = false;
        let mut menu_indent = 0;
        let mut current_menu_options: Vec<ChoiceOptionDto> = Vec::new();
        let mut current_condition: Option<String> = None;
        let mut last_choice_idx: Option<usize> = None;

        for (idx, line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let indent = line.len() - line.trim_start().len();

            // 1. Проверяем объявление сцены (label)
            if let Some(caps) = RE_STRUCT_LABEL.captures(line) {
                if in_menu && !current_menu_options.is_empty() {
                    if let Some(ref lbl) = current_label {
                        structure
                            .scene_choices
                            .entry(lbl.clone())
                            .or_default()
                            .push(ChoiceBlockDto {
                                prompt: None,
                                options: std::mem::take(&mut current_menu_options),
                            });
                    }
                }
                in_menu = false;
                current_condition = None;
                last_choice_idx = None;

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

            // 2. Проверяем начало блока menu:
            if RE_STRUCT_MENU.is_match(line) {
                if in_menu && !current_menu_options.is_empty() {
                    structure
                        .scene_choices
                        .entry(lbl.clone())
                        .or_default()
                        .push(ChoiceBlockDto {
                            prompt: None,
                            options: std::mem::take(&mut current_menu_options),
                        });
                }
                in_menu = true;
                menu_indent = indent;
                last_choice_idx = None;
                continue;
            }

            // Если были в menu, но отступ уменьшился
            if in_menu && indent <= menu_indent && !RE_STRUCT_CHOICE_OPT.is_match(line) {
                if !current_menu_options.is_empty() {
                    structure
                        .scene_choices
                        .entry(lbl.clone())
                        .or_default()
                        .push(ChoiceBlockDto {
                            prompt: None,
                            options: std::mem::take(&mut current_menu_options),
                        });
                }
                in_menu = false;
                last_choice_idx = None;
            }

            // 3. Варианты выбора внутри menu
            if in_menu {
                if let Some(caps) = RE_STRUCT_CHOICE_OPT.captures(line) {
                    let opt_text = caps.get(1).unwrap().as_str().to_string();
                    current_menu_options.push(ChoiceOptionDto {
                        text: opt_text,
                        target_jump: None,
                    });
                    last_choice_idx = Some(current_menu_options.len() - 1);
                    continue;
                }
            }

            // 4. Ветвления if/elif/else
            if let Some(caps) = RE_STRUCT_IF.captures(line) {
                current_condition = Some(caps.get(1).unwrap().as_str().trim().to_string());
            } else if RE_STRUCT_ELSE.is_match(line) {
                current_condition = Some("иначе".to_string());
            }

            // 5. Переходы jump / call
            if let Some(caps) = RE_STRUCT_JUMP.captures(line) {
                let cmd = caps.name("cmd").unwrap().as_str();
                let target = caps.name("target").unwrap().as_str().to_string();
                let inline_cond = caps.name("cond").map(|c| c.as_str().trim().to_string());
                let cond = inline_cond.or_else(|| current_condition.clone());

                let kind = if in_menu {
                    "choice".to_string()
                } else if cond.is_some() {
                    "branch".to_string()
                } else if cmd == "call" {
                    "call".to_string()
                } else {
                    "jump".to_string()
                };

                let condition_label = if in_menu {
                    if let Some(idx) = last_choice_idx {
                        if let Some(opt) = current_menu_options.get_mut(idx) {
                            opt.target_jump = Some(target.clone());
                            Some(format!("Выбор: \"{}\"", opt.text))
                        } else {
                            cond.clone()
                        }
                    } else {
                        cond.clone()
                    }
                } else {
                    cond.clone()
                };

                structure.edges.push(SceneEdgeDto {
                    from: lbl.clone(),
                    to: target.clone(),
                    kind,
                    condition: condition_label,
                });

                structure
                    .scene_jumps
                    .entry(lbl.clone())
                    .or_default()
                    .push((target, cond));
            }
        }

        if in_menu && !current_menu_options.is_empty() {
            if let Some(lbl) = current_label {
                structure
                    .scene_choices
                    .entry(lbl)
                    .or_default()
                    .push(ChoiceBlockDto {
                        prompt: None,
                        options: current_menu_options,
                    });
            }
        }

        structure
    }

    /// Парсит оригинальный или декомпилированный скрипт .rpy (где нет блоков translate)
    pub fn parse_original_script(&self, path: &Path, content: &str) -> Result<ScenarioGraph> {
        let mut graph = ScenarioGraph::new();
        let file_path = path.to_string_lossy().to_string();

        let mut current_scene_name = "Общие диалоги".to_string();
        let mut last_node_idx = None;

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let trimmed = raw_line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if let Some(caps) = RE_ORIG_LABEL.captures(trimmed) {
                let label_name = caps.get(1).unwrap().as_str().to_string();
                current_scene_name = label_name.clone();
                let label_node = graph.add_node(ScenarioNode::label(label_name));
                last_node_idx = Some(label_node);
                continue;
            }

            // Исключаем ключевые команды движка
            if trimmed.starts_with('$')
                || trimmed.starts_with("jump ")
                || trimmed.starts_with("call ")
                || trimmed.starts_with("scene ")
                || trimmed.starts_with("show ")
                || trimmed.starts_with("hide ")
                || trimmed.starts_with("with ")
                || trimmed.starts_with("play ")
                || trimmed.starts_with("stop ")
                || trimmed.starts_with("define ")
                || trimmed.starts_with("default ")
                || trimmed.starts_with("python")
                || trimmed.starts_with("init ")
                || trimmed.starts_with("return")
            {
                continue;
            }

            if let Some(caps) = RE_ORIG_DIALOGUE.captures(trimmed) {
                let speaker_alias = caps.name("speaker").map(|s| s.as_str());
                let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
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
        if !content.contains("translate ") {
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
                } else if let Some(caps) = RE_NEW.captures(line) {
                    if let Some((old_text, old_line)) = pending_old.take() {
                        let new_text =
                            Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                        let is_translated = Self::has_cyrillic(&new_text);

                        // Дедупликация строк UI: если точно такой же old_text уже встречался
                        if let Some(&existing_idx) = seen_ui_strings.get(&old_text) {
                            if is_translated {
                                if let Some(existing_node) = graph.get_node_mut(existing_idx) {
                                    if let Some(d) = existing_node.as_dialogue_mut() {
                                        if d.translated_text.is_none() {
                                            d.translated_text = Some(new_text);
                                            d.status = TranslationStatus::Translated;
                                        }
                                    }
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
                }
                continue;
            }

            if in_block {
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    let comment = trimmed.trim_start_matches('#').trim();
                    if let Some(caps) = RE_DIALOGUE.captures(comment) {
                        let speaker = caps.name("speaker").map(|s| s.as_str().to_string());
                        let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                        pending_source = Some((speaker, text, line_num));
                    }
                } else if !trimmed.is_empty() {
                    if let Some(caps) = RE_DIALOGUE.captures(trimmed) {
                        let speaker_alias = caps.name("speaker").map(|s| s.as_str());
                        let target_text =
                            Self::unescape_renpy_string(caps.name("text").unwrap().as_str());

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
