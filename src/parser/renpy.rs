use std::collections::HashMap;
use std::path::Path;
use anyhow::Result;
use regex::Regex;

use crate::graph::{EdgeKind, ScenarioGraph, ScenarioNode, TranslationStatus};
use crate::parser::GameParser;

/// Парсер файлов сценариев и локализаций Ren'Py (.rpy)
#[derive(Debug, Default)]
pub struct RenpyParser {
    /// Таблица персонажей: псевдоним (например, "e") -> полное имя ("Эрика")
    pub speakers: HashMap<String, String>,
}

impl RenpyParser {
    pub fn new() -> Self {
        Self {
            speakers: HashMap::new(),
        }
    }

    /// Регистрирует известных персонажей (например, из скриптов игры)
    pub fn with_speakers(mut self, speakers: HashMap<String, String>) -> Self {
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
            _ => None, // Narration (авторский текст)
        }
    }
}

impl GameParser for RenpyParser {
    fn name(&self) -> &'static str {
        "renpy"
    }

    /// Парсит `.rpy` файл (как оригинальный, так и файл из `game/tl/<lang>/`)
    fn parse_file(&self, path: &Path, content: &str) -> Result<ScenarioGraph> {
        let mut graph = ScenarioGraph::new();
        let file_path = path.to_string_lossy().to_string();

        let re_strings_block = Regex::new(r#"^translate\s+\w+\s+strings\s*:"#).unwrap();
        let re_dialogue_block = Regex::new(r#"^translate\s+\w+\s+([\w\.]+)\s*:"#).unwrap();
        let re_old = Regex::new(r#"^\s*old\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();
        let re_new = Regex::new(r#"^\s*new\s+"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();
        let re_dialogue = Regex::new(r#"^\s*(?:(?P<speaker>[A-Za-z_][\w.]*)\s+)?"(?P<text>(?:[^"\\]|\\.)*)""#).unwrap();

        let mut in_strings = false;
        let mut in_block = false;
        let mut pending_old: Option<(String, usize)> = None;
        let mut pending_source: Option<(Option<String>, String, usize)> = None;
        let mut last_node_idx = None;

        for (idx, raw_line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let line = raw_line.trim_end();

            // Проверка на начало секции translate strings:
            if re_strings_block.is_match(line) {
                in_strings = true;
                in_block = false;
                pending_old = None;
                continue;
            }

            // Проверка на начало блока диалога: translate russian label_id:
            if let Some(caps) = re_dialogue_block.captures(line) {
                in_strings = false;
                in_block = true;
                pending_source = None;
                let label_name = caps.get(1).unwrap().as_str();
                let label_node = graph.add_node(ScenarioNode::label(label_name));
                last_node_idx = Some(label_node);
                continue;
            }

            // Если строка без отступа и не пустая — выход из текущего блока
            if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') {
                in_block = false;
                if !line.starts_with('#') {
                    in_strings = false;
                    pending_old = None;
                }
                continue;
            }

            // Режим обработки блока `translate strings:` (UI, меню, выборы)
            if in_strings {
                if let Some(caps) = re_old.captures(line) {
                    let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                    pending_old = Some((text, line_num));
                } else if let Some(caps) = re_new.captures(line) {
                    if let Some((old_text, old_line)) = pending_old.take() {
                        let new_text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                        let is_translated = Self::has_cyrillic(&new_text);
                        let mut dialogue = ScenarioNode::dialogue(Some("UI / Choice"), old_text, &file_path, old_line);
                        
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
                    }
                }
                continue;
            }

            // Режим обработки блока диалога `translate <lang> <label>:`
            if in_block {
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    // Строка-комментарий с оригинальным текстом
                    let comment = trimmed.trim_start_matches('#').trim();
                    if let Some(caps) = re_dialogue.captures(comment) {
                        let speaker = caps.name("speaker").map(|s| s.as_str().to_string());
                        let text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());
                        pending_source = Some((speaker, text, line_num));
                    }
                } else if !trimmed.is_empty() {
                    // Строка с переводом
                    if let Some(caps) = re_dialogue.captures(trimmed) {
                        let speaker_alias = caps.name("speaker").map(|s| s.as_str());
                        let target_text = Self::unescape_renpy_string(caps.name("text").unwrap().as_str());

                        if let Some((src_speaker, src_text, src_line)) = pending_source.take() {
                            let resolved_speaker = self.resolve_speaker(src_speaker.as_deref().or(speaker_alias));
                            let is_translated = Self::has_cyrillic(&target_text);

                            let mut dialogue = ScenarioNode::dialogue(resolved_speaker, src_text, &file_path, src_line);
                            if let Some(d) = dialogue.as_dialogue_mut() {
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
    fn test_parse_translation_dialogue() {
        let rpy = r#"
# game/script.rpy:40
translate russian intro_scene_1234abcd:

    # mc "Where am I?"
    mc "Где я?"

    # e "You are safe now."
    e "Теперь ты в безопасности."
"#;
        let mut parser = RenpyParser::new();
        parser.speakers.insert("mc".into(), "Протагонист".into());
        parser.speakers.insert("e".into(), "Эрика".into());

        let graph = parser.parse_file(Path::new("intro.rpy"), rpy).unwrap();
        let (total, translated) = graph.translation_stats();

        assert_eq!(total, 2);
        assert_eq!(translated, 2);
    }

    #[test]
    fn test_parse_translation_strings() {
        let rpy = r#"
translate russian strings:

    # game/screens.rpy:10
    old "Start"
    new "Начать"

    # game/screens.rpy:12
    old "Load"
    new "Загрузить"

    old "Quit"
    new "Quit"
"#;
        let parser = RenpyParser::new();
        let graph = parser.parse_file(Path::new("screens.rpy"), rpy).unwrap();
        let (total, translated) = graph.translation_stats();

        assert_eq!(total, 3);
        assert_eq!(translated, 2); // "Quit" -> "Quit" не содержит кириллицы, считается непереведенным
    }
}
