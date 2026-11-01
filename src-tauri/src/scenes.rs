//! Где персонаж встречается в игре: сцены, в которых он говорит или о нём говорят.

use crate::parser::renpy::RenpyParser;
use crate::terms::RE_WORD;
use rayon::prelude::*;
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Сцена опознаётся по файлу и метке: вне меток реплики в разных файлах попали бы в одну.
pub fn scene_key(file: &str, scene: &str) -> String {
    format!("{file}::{scene}")
}

const MIN_NAME_WORD_CHARS: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneLine {
    pub line_number: usize,
    pub speaker: Option<String>,
    pub text: String,
    /// Реплика самого персонажа, а не упоминание о нём.
    pub spoken: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CharacterScene {
    pub key: String,
    pub scene: String,
    pub file: String,
    /// Полный путь: файл из архива лежит в кэше, а `file` этого не показывает.
    pub path: String,
    pub spoken: usize,
    pub mentioned: usize,
    pub lines: Vec<SceneLine>,
}

/// Реплика оригинального скрипта и её место в игре.
pub struct ScriptLine {
    pub file: String,
    pub path: String,
    pub scene: String,
    pub line_number: usize,
    pub alias: Option<String>,
    pub speaker: Option<String>,
    pub text: String,
}

impl ScriptLine {
    pub fn key(&self) -> String {
        scene_key(&self.file, &self.scene)
    }
}

/// Читает реплики всех скриптов. `file_name` даёт имя файла для показа и ключей сцен.
pub fn load_lines(scripts: &[PathBuf], file_name: impl Fn(&Path) -> String + Sync) -> Vec<ScriptLine> {
    let per_file: Vec<Vec<ScriptLine>> = scripts
        .par_iter()
        .map(|path| {
            let Ok(content) = fs::read_to_string(path) else {
                return Vec::new();
            };
            let Ok(graph) = RenpyParser::new().parse_original_script(path, &content) else {
                return Vec::new();
            };
            let file = file_name(path);
            graph
                .dialogues()
                .map(|line| ScriptLine {
                    file: file.clone(),
                    path: path.to_string_lossy().into_owned(),
                    scene: line.block_id.clone().unwrap_or_default(),
                    line_number: line.line_number,
                    alias: line.speaker_alias.clone(),
                    speaker: line.speaker.clone(),
                    text: line.original_text.clone(),
                })
                .collect()
        })
        .collect();
    per_file.into_iter().flatten().collect()
}

/// Реплики игры с тем, что считается один раз на все запросы.
pub struct GameLines {
    pub lines: Vec<ScriptLine>,
    /// Значения строковых переменных из скриптов.
    pub variables: HashMap<String, Vec<String>>,
    /// Слова, которые встречаются со строчной буквы.
    lowercase: HashSet<String>,
}

impl GameLines {
    pub fn new(lines: Vec<ScriptLine>, variables: HashMap<String, Vec<String>>) -> Self {
        let lowercase = lines
            .iter()
            .flat_map(|line| RE_WORD.find_iter(&line.text))
            .map(|word| word.as_str())
            .filter(|word| word.starts_with(char::is_lowercase))
            .map(str::to_string)
            .collect();
        Self { lines, variables, lowercase }
    }
}

/// Выражение для упоминаний: полные имена и слова имён, которые не встречаются со строчной
/// буквы. Так «Agent» и «Girl» не считаются упоминанием, а «Lisa» считается.
fn mention_pattern(names: &[&str], lowercase: &HashSet<String>) -> Option<Regex> {
    let mut patterns = Vec::new();
    for name in names.iter().filter(|name| !name.contains('[')) {
        patterns.push(name.trim().to_string());
        patterns.extend(
            name.split_whitespace()
                .filter(|word| word.chars().count() >= MIN_NAME_WORD_CHARS)
                .filter(|word| !lowercase.contains(word.to_lowercase().as_str()))
                .map(str::to_string),
        );
    }
    patterns.retain(|name| !name.is_empty());
    if patterns.is_empty() {
        return None;
    }
    let patterns: Vec<String> = patterns.iter().map(|name| regex::escape(name)).collect();
    Regex::new(&format!(r"\b(?:{})\b", patterns.join("|"))).ok()
}

/// Сцены персонажа в порядке файлов и строк.
pub fn character_scenes(game: &GameLines, aliases: &[&str], names: &[&str]) -> Vec<CharacterScene> {
    let mention = mention_pattern(names, &game.lowercase);
    scenes_with(game, |line| {
        let spoken = line
            .alias
            .as_deref()
            .is_some_and(|own| aliases.iter().any(|alias| own.eq_ignore_ascii_case(alias)));
        (spoken || mention.as_ref().is_some_and(|pattern| pattern.is_match(&line.text))).then_some(spoken)
    })
}

/// Сцены, в репликах которых есть токен `[name]`, в том числе с флагом вроде `[name!c]`.
pub fn token_scenes(game: &GameLines, name: &str) -> Vec<CharacterScene> {
    matching_scenes(game, &format!(r"\[{}(?:![a-z]+)?\]", regex::escape(name)))
}

/// Сцены, в репликах которых встречается термин. Регистр не важен, термин ищется целым словом.
pub fn term_scenes(game: &GameLines, term: &str) -> Vec<CharacterScene> {
    let term = term.trim();
    if term.is_empty() {
        return Vec::new();
    }
    matching_scenes(game, &format!(r"(?i)(?:^|\W){}(?:$|\W)", regex::escape(term)))
}

fn matching_scenes(game: &GameLines, pattern: &str) -> Vec<CharacterScene> {
    let Ok(pattern) = Regex::new(pattern) else {
        return Vec::new();
    };
    scenes_with(game, |line| pattern.is_match(&line.text).then_some(false))
}

/// Собирает сцены с подходящими строками. `classify` возвращает `None` для лишней строки,
/// а для подходящей говорит, это реплика самого персонажа или упоминание.
fn scenes_with(game: &GameLines, classify: impl Fn(&ScriptLine) -> Option<bool>) -> Vec<CharacterScene> {
    let mut positions: HashMap<(&str, &str), usize> = HashMap::new();
    let mut scenes: Vec<CharacterScene> = Vec::new();
    for line in &game.lines {
        let Some(spoken) = classify(line) else {
            continue;
        };
        let position = *positions.entry((&line.file, &line.scene)).or_insert_with(|| {
            scenes.push(CharacterScene {
                key: line.key(),
                scene: line.scene.clone(),
                file: line.file.clone(),
                path: line.path.clone(),
                spoken: 0,
                mentioned: 0,
                lines: Vec::new(),
            });
            scenes.len() - 1
        });
        let scene = &mut scenes[position];
        if spoken {
            scene.spoken += 1;
        } else {
            scene.mentioned += 1;
        }
        scene.lines.push(SceneLine {
            line_number: line.line_number,
            speaker: line.speaker.clone().or_else(|| line.alias.clone()),
            text: line.text.clone(),
            spoken,
        });
    }
    scenes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenes(content: &str, alias: &str, name: &str) -> Vec<CharacterScene> {
        let dir = std::env::temp_dir().join(format!("translator_scenes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.rpy");
        fs::write(&path, content).unwrap();
        let game = GameLines::new(load_lines(&[path], |_| "story.rpy".to_string()), HashMap::new());
        let found = character_scenes(&game, &[alias], &[name]);
        let _ = fs::remove_dir_all(dir);
        found
    }

    fn game_of(content: &str) -> GameLines {
        let dir = std::env::temp_dir().join(format!("translator_scenes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.rpy");
        fs::write(&path, content).unwrap();
        let game = GameLines::new(load_lines(&[path], |_| "story.rpy".to_string()), HashMap::new());
        let _ = fs::remove_dir_all(dir);
        game
    }

    #[test]
    fn tokens_are_found_with_their_flags() {
        let game = game_of(concat!(
            "label home:\n",
            "    m \"Ask [landlady].\"\n",
            "    m \"[landlady!c] is out.\"\n",
            "    m \"The landlady left and [landlady_2] stayed.\"\n",
            "label bar:\n",
            "    m \"Nothing.\"\n",
        ));
        let found = token_scenes(&game, "landlady");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].scene.as_str(), found[0].spoken, found[0].mentioned), ("home", 0, 2));
        assert!(token_scenes(&game, "nick").is_empty());
    }

    #[test]
    fn terms_are_found_as_whole_words_in_any_case() {
        let game = game_of(concat!(
            "label home:\n",
            "    m \"Back to the ppf hq now.\"\n",
            "label bar:\n",
            "    m \"The PPF HQs are closed.\"\n",
            "label end:\n",
            "    m \"PPF HQ\"\n",
        ));
        let found = term_scenes(&game, " PPF HQ ");
        let names: Vec<&str> = found.iter().map(|scene| scene.scene.as_str()).collect();
        assert_eq!(names, ["home", "end"]);
        assert!(term_scenes(&game, " ").is_empty());
    }

    #[test]
    fn groups_spoken_lines_and_mentions_by_scene() {
        let found = scenes(
            concat!(
                "label home:\n",
                "    k \"I'm going out.\"\n",
                "    m \"Have fun, Kate.\"\n",
                "    m \"See you later.\"\n",
                "label office:\n",
                "    m \"Nothing here.\"\n",
                "label bar:\n",
                "    m \"Where is Kate Smith?\"\n",
            ),
            "k",
            "Kate Smith",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].key, "story.rpy::home");
        assert_eq!((found[0].scene.as_str(), found[0].spoken, found[0].mentioned), ("home", 1, 1));
        assert!(found[0].lines[0].spoken && !found[0].lines[1].spoken);
        assert_eq!(found[0].lines[1].text, "Have fun, Kate.");
        assert_eq!((found[1].scene.as_str(), found[1].spoken, found[1].mentioned), ("bar", 0, 1));
    }

    #[test]
    fn common_word_of_name_is_not_a_mention() {
        let found = scenes(
            concat!(
                "label one:\n",
                "    m \"Agent Lisa is here.\"\n",
                "label two:\n",
                "    m \"The Agent called, he is an agent of the state.\"\n",
                "label three:\n",
                "    m \"Lisa left.\"\n",
            ),
            "agl",
            "Agent Lisa",
        );
        let names: Vec<_> = found.iter().map(|scene| scene.scene.as_str()).collect();
        assert_eq!(names, ["one", "three"]);
    }

    #[test]
    fn merged_codes_of_character_are_counted_together() {
        let dir = std::env::temp_dir().join(format!("translator_scenes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.rpy");
        fs::write(
            &path,
            "label one:\n    hi \"First.\"\nlabel two:\n    hil \"Second.\"\n    m \"Bye, Hilde.\"\n",
        )
        .unwrap();
        let game = GameLines::new(load_lines(&[path], |_| "story.rpy".to_string()), HashMap::new());
        let found = character_scenes(&game, &["hil", "hi"], &["Hilde", "Hilde"]);
        let _ = fs::remove_dir_all(dir);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].spoken, found[1].spoken, found[1].mentioned), (1, 1, 1));
    }

    #[test]
    fn name_with_variable_gives_only_spoken_lines() {
        let found = scenes(
            "label one:\n    p \"Hello.\"\n    m \"Hi, player.\"\n",
            "p",
            "[player_name]",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lines.len(), 1);
    }
}
