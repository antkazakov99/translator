//! Сцены по структуре скриптов: порядок по сюжету, переходы через экраны, сцена текста выбора.

use std::collections::{HashMap, HashSet};

use super::script::{SceneEdgeDto, ScreenLinks, ScriptStructure};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NaturalPart {
    Number(u64),
    Text(String),
}

/// Ключ сортировки, при котором `story_2` идёт раньше `story_10`.
pub(crate) fn natural_key(text: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut chars = text.to_lowercase().chars().collect::<Vec<_>>().into_iter().peekable();
    while let Some(&first) = chars.peek() {
        let digits = first.is_ascii_digit();
        let mut part = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() != digits {
                break;
            }
            part.push(c);
            chars.next();
        }
        parts.push(if digits {
            NaturalPart::Number(part.parse().unwrap_or(u64::MAX))
        } else {
            NaturalPart::Text(part)
        });
    }
    parts
}

/// Порядок сцен по сюжету: обход переходов от `start`. Сцены, до которых так не дойти,
/// например при свободном перемещении, идут после них в порядке файлов.
pub(crate) fn story_order(structures: &[ScriptStructure]) -> HashMap<String, usize> {
    let screens = screen_targets(structures.iter());
    let mut next: HashMap<String, Vec<String>> = HashMap::new();
    for structure in structures {
        for edge in structure.edges.iter().cloned().chain(screen_edges(structure, &screens)) {
            next.entry(edge.from).or_default().push(edge.to);
        }
        // Сцена без перехода продолжается следующей меткой файла: этот шаг идёт последним
        for pair in structure.labels.windows(2) {
            next.entry(pair[0].name.clone()).or_default().push(pair[1].name.clone());
        }
    }

    let mut order = HashMap::new();
    let roots = std::iter::once("start")
        .chain(structures.iter().flat_map(|s| s.labels.iter().map(|label| label.name.as_str())));
    for root in roots {
        let mut stack = vec![root.to_string()];
        while let Some(scene) = stack.pop() {
            if order.contains_key(&scene) {
                continue;
            }
            if let Some(targets) = next.get(&scene) {
                stack.extend(targets.iter().rev().cloned());
            }
            let rank = order.len();
            order.insert(scene, rank);
        }
    }
    order
}

/// Куда ведут кнопки каждого экрана проекта, включая вложенные и открытые ими экраны.
/// Строки вроде `Jump("slot_1")` могут не быть метками, поэтому остаются только настоящие метки.
pub(crate) fn screen_targets<'a>(
    structures: impl Iterator<Item = &'a ScriptStructure>,
) -> HashMap<String, Vec<String>> {
    let mut links: HashMap<&str, Vec<&ScreenLinks>> = HashMap::new();
    let mut labels = HashSet::new();
    for structure in structures {
        labels.extend(structure.labels.iter().map(|label| label.name.as_str()));
        for (name, screen) in &structure.screens {
            links.entry(name).or_default().push(screen);
        }
    }

    links
        .keys()
        .map(|name| {
            let mut targets = Vec::new();
            let mut seen = HashSet::from([*name]);
            let mut pending = vec![*name];
            while let Some(screen) = pending.pop() {
                for part in links.get(screen).into_iter().flatten() {
                    for target in &part.targets {
                        if labels.contains(target.as_str()) && !targets.contains(target) {
                            targets.push(target.clone());
                        }
                    }
                    for nested in &part.screens {
                        if seen.insert(nested) {
                            pending.push(nested);
                        }
                    }
                }
            }
            (name.to_string(), targets)
        })
        .collect()
}

/// Переходы сцен через кнопки показанных экранов.
pub(crate) fn screen_edges(structure: &ScriptStructure, screens: &HashMap<String, Vec<String>>) -> Vec<SceneEdgeDto> {
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for (scene, screen) in &structure.shown_screens {
        for target in screens.get(screen).into_iter().flatten() {
            if target != scene && seen.insert((scene, screen, target)) {
                edges.push(SceneEdgeDto {
                    from: scene.clone(),
                    to: target.clone(),
                    kind: "choice".to_string(),
                    condition: Some(format!("Экран: {}", screen)),
                });
            }
        }
    }
    edges
}

/// Какой сцене принадлежит текст выбора. Один текст может встречаться в разных сценах,
/// поэтому берётся первая сцена по порядку объявления: иначе ключ менялся бы между запусками.
pub(crate) fn choice_scenes(structure: &ScriptStructure) -> HashMap<String, String> {
    let mut scenes = HashMap::new();
    for label in &structure.labels {
        let Some(choices) = structure.scene_choices.get(&label.name) else {
            continue;
        };
        for option in choices.iter().flat_map(|choice| &choice.options) {
            scenes.entry(option.text.trim().to_string()).or_insert_with(|| label.name.clone());
        }
    }
    scenes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::renpy::script::RenpyParser;

    #[test]
    fn natural_order_counts_numbers() {
        let mut names = vec!["story_10.rpy", "story_2.rpy", "Story_1.rpy"];
        names.sort_by_cached_key(|name| natural_key(name));
        assert_eq!(names, ["Story_1.rpy", "story_2.rpy", "story_10.rpy"]);
    }

    #[test]
    fn story_follows_jumps_from_start() {
        let first = RenpyParser::parse_script_structure("label intro:\n    return\nlabel start:\n    jump ending\n");
        let second = RenpyParser::parse_script_structure("label ending:\n    return\n");
        let order = story_order(&[first, second]);
        assert!(order["start"] < order["ending"]);
        assert!(order["ending"] < order["intro"]);
    }
}
