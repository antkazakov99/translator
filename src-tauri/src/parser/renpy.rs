use std::collections::HashMap;
use std::path::Path;
use anyhow::{Context, Result};
use regex::Regex;

use crate::graph::{EdgeKind, ScenarioGraph, ScenarioNode, TranslationStatus};
use crate::parser::GameParser;

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
        self.speaker_to_alias = speakers.iter().map(|(k, v)| (v.clone(), k.clone())).collect();
        self.speakers = speakers;
        self
    }

    /// Извлекает определения персонажей вида `define e = Character("Erika")`
    pub fn extract_speakers(content: &str) -> HashMap<String, String> {
        let re = Regex::new(r#"^\s*define\s+(\w+)\s*=\s*Character\(\s*[_(\s]*['"]([^'"]+)['"]"#).unwrap();
        let mut map = HashMap::new();
        for line in content.lines() {
            if let Some(caps) = re.captures(line) {
                let alias = caps.get(1).unwrap().as_str().to_string();
                let name = caps.get(2).unwrap().as_str().to_string();
                map.insert(alias, name);
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
                        'n' => { result.push('\n'); chars.next(); }
                        't' => { result.push('\t'); chars.next(); }
                        'r' => { result.push('\r'); chars.next(); }
                        '"' => { result.push('"'); chars.next(); }
                        '\\' => { result.push('\\'); chars.next(); }
                        '\'' => { result.push('\''); chars.next(); }
                        _ => { result.push(c); }
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
        text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c) || c == 'Ё' || c == 'ё')
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
                                    output.push_str(&format!("\ntranslate {} {}:\n\n", target_lang, block_id));
                                    current_block_id = Some(block_id.clone());
                                }
                            }

                            let alias_prefix = self.resolve_alias(d.speaker.as_deref());
                            let original_escaped = Self::escape_renpy_string(&d.original_text);
                            let target_text = d.translated_text.as_deref().unwrap_or(&d.original_text);
                            let target_escaped = Self::escape_renpy_string(target_text);

                            output.push_str(&format!("    # {}\"{}\"\n", alias_prefix, original_escaped));
                            output.push_str(&format!("    {}\"{}\"\n\n", alias_prefix, target_escaped));
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
    pub fn save_translation_file(&self, graph: &ScenarioGraph, output_path: &Path, target_lang: &str) -> Result<()> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Не удалось создать папку мода: {:?}", parent))?;
        }
        let content = self.generate_translation_file(graph, target_lang);
        std::fs::write(output_path, content)
            .with_context(|| format!("Не удалось записать файл перевода: {:?}", output_path))?;
        Ok(())
    }
}

impl GameParser for RenpyParser {
    fn name(&self) -> &'static str {
        "renpy"
    }

    /// Парсит `.rpy` файл и группирует реплики по реальным смысловым сценам
    fn parse_file(&self, path: &Path, content: &str) -> Result<ScenarioGraph> {
        let mut graph = ScenarioGraph::new();
        let file_path = path.to_string_lossy().to_string();

        let re_strings_block = Regex::new(r#"^translate\s+\w+\s+strings\s*:"#).unwrap();
        let re_dialogue_block = Regex::new(r#"^translate\s+\w+\s+([\w\.]+)\s*:"#).unwrap();
        let re_old = Regex::new(r#"^\s*old\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();
        let re_new = Regex::new(r#"^\s*new\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();
        let re_dialogue = Regex::new(r#"^\s*(?:(?P<speaker>[A-Za-z_][\w.]*)\s+)?"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();
        // Регулярка для отрезания хэшей Ren'Py вида _1234abcd и повторов _1234abcd_1
        let re_label_hash = Regex::new(r#"_[0-9a-fA-F]{6,10}(?:_\d+)*$"#).unwrap();

        let mut seen_ui_strings: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
        let mut in_strings = false;
        let mut in_block = false;
        let mut current_block_id: Option<String> = None;
        let mut current_scene_name: Option<String> = None;
        let mut pending_old: Option<(String, usize)> = None;
        let mut pending_source: Option<(Option<String>, String, usize)> = None;
        let mut last_node_idx = None;

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let line = raw_line.trim_end();

            // Блок интерфейсных строк (strings) НЕ должен разрывать текущую сюжетную сцену!
            if re_strings_block.is_match(line) {
                in_strings = true;
                in_block = false;
                pending_old = None;
                continue;
            }

            if let Some(caps) = re_dialogue_block.captures(line) {
                in_strings = false;
                in_block = true;
                pending_source = None;

                let raw_block_id = caps.get(1).unwrap().as_str().to_string();
                current_block_id = Some(raw_block_id.clone());

                // Отрезаем хэш: например "end_scene_eri_52d5be01" -> "end_scene_eri"
                let scene_name = re_label_hash.replace(&raw_block_id, "").to_string();

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
                if let Some(caps) = re_old.captures(line) {
                    let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                    pending_old = Some((text, line_num));
                } else if let Some(caps) = re_new.captures(line) {
                    if let Some((old_text, old_line)) = pending_old.take() {
                        let new_text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
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
                            let mut dialogue = ScenarioNode::dialogue(Some("UI / Choice"), old_text.clone(), &file_path, old_line);
                            if let Some(d) = dialogue.as_dialogue_mut() {
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
                    if let Some(caps) = re_dialogue.captures(comment) {
                        let speaker = caps.name("speaker").map(|s| s.as_str().to_string());
                        let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                        pending_source = Some((speaker, text, line_num));
                    }
                } else if !trimmed.is_empty() {
                    if let Some(caps) = re_dialogue.captures(trimmed) {
                        let speaker_alias = caps.name("speaker").map(|s| s.as_str());
                        let target_text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());

                        if let Some((src_speaker, src_text, src_line)) = pending_source.take() {
                            let resolved_speaker = self.resolve_speaker(src_speaker.as_deref().or(speaker_alias));
                            let is_translated = Self::has_cyrillic(&target_text);

                            let mut dialogue = ScenarioNode::dialogue(resolved_speaker, src_text, &file_path, src_line);
                            if let Some(d) = dialogue.as_dialogue_mut() {
                                d.block_id = current_block_id.clone();
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
        assert_eq!(total, 1, "Дублирующиеся строки UI должны быть схлопнуты в 1 запись");
        assert_eq!(translated, 1);
    }
}
