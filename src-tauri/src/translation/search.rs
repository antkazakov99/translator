//! Поиск сцен по репликам: подстрока в оригинале или действующем переводе без учёта регистра.

use std::collections::HashMap;

use crate::engines::ContainerKey;
use crate::scan::FileId;

/// Короче запрос реплики не ищет: одна буква нашлась бы почти везде.
pub const MIN_QUERY: usize = 2;

/// Реплика текущего снимка для поиска.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchText {
    /// Сцена реплики. `None`: реплика вне сцен, она ищется в «Без сцены» своего файла.
    pub scene: Option<ContainerKey>,
    pub file: FileId,
    pub source_text: String,
    /// Действующий текст: перевод, черновик модели или перевод из игры.
    pub text: String,
}

/// Сцена, где нашлись реплики, и сколько их. У реплик вне сцен `scene = None` и их файл.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneHit {
    pub scene: Option<ContainerKey>,
    pub file: FileId,
    pub lines: usize,
}

/// Сцены с совпадениями в порядке первой найденной реплики. Пустой или короткий запрос ничего не находит.
pub fn scene_hits(texts: &[SearchText], query: &str) -> Vec<SceneHit> {
    let needle = query.trim().to_lowercase();
    if needle.chars().count() < MIN_QUERY {
        return Vec::new();
    }
    let mut hits: Vec<SceneHit> = Vec::new();
    let mut index: HashMap<(Option<&ContainerKey>, Option<FileId>), usize> = HashMap::new();
    for line in texts {
        if !line.source_text.to_lowercase().contains(&needle) && !line.text.to_lowercase().contains(&needle) {
            continue;
        }
        // Сцена одна на всю игру, а «Без сцены» у каждого файла своя.
        let slot = (line.scene.as_ref(), if line.scene.is_some() { None } else { Some(line.file) });
        match index.get(&slot) {
            Some(&at) => hits[at].lines += 1,
            None => {
                index.insert(slot, hits.len());
                hits.push(SceneHit { scene: line.scene.clone(), file: line.file, lines: 1 });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(scene: Option<&str>, file: i64, source: &str, text: &str) -> SearchText {
        SearchText {
            scene: scene.map(|key| ContainerKey::new(key).unwrap()),
            file: FileId::from_row(file),
            source_text: source.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn counts_matching_lines_per_scene_in_source_and_translation_ignoring_case() {
        let texts = [
            line(Some("label:start"), 1, "Hello, Eileen.", "Привет, Эйлин."),
            line(Some("label:start"), 1, "Nothing here.", ""),
            line(Some("label:left"), 2, "Bye.", "Пока, ЭЙЛИН."),
            line(Some("label:start"), 1, "EILEEN!", ""),
        ];
        let hits = scene_hits(&texts, " эйлин ");
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].scene.as_ref().map(ContainerKey::as_str), hits[0].lines), (Some("label:start"), 1));
        assert_eq!((hits[1].scene.as_ref().map(ContainerKey::as_str), hits[1].lines), (Some("label:left"), 1));
        let hits = scene_hits(&texts, "eileen");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].lines, 2, "регистр оригинала не важен");
    }

    #[test]
    fn loose_lines_are_grouped_by_file() {
        let texts = [line(None, 1, "Start game", ""), line(None, 2, "Start over", ""), line(None, 1, "Restart", "")];
        let hits = scene_hits(&texts, "start");
        let found: Vec<_> = hits.iter().map(|hit| (hit.scene.is_none(), hit.file.get(), hit.lines)).collect();
        assert_eq!(found, [(true, 1, 2), (true, 2, 1)]);
    }

    #[test]
    fn short_or_empty_query_finds_nothing() {
        let texts = [line(Some("label:start"), 1, "a", "б")];
        assert!(scene_hits(&texts, "").is_empty());
        assert!(scene_hits(&texts, " а ").is_empty());
    }
}
