use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslationStatus {
    Untranslated,
    Translated,
    Reviewed,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueData {
    pub id: String,
    /// Исходный ID блока в движке (например, "end_scene_eri_52d5be01" в Ren'Py)
    pub block_id: Option<String>,
    pub speaker: Option<String>,
    pub speaker_alias: Option<String>,
    pub original_text: String,
    pub translated_text: Option<String>,
    pub file_path: String,
    pub line_number: usize,
    pub instruction: Option<String>,
    pub status: TranslationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub text: String,
    pub translation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceData {
    pub id: String,
    pub prompt: Option<String>,
    pub options: Vec<ChoiceOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ScenarioNode {
    Label(String),
    Dialogue(DialogueData),
    Choice(ChoiceData),
    Event(EventData),
}

impl ScenarioNode {
    pub fn dialogue(
        speaker: Option<impl Into<String>>,
        original: impl Into<String>,
        file: impl Into<String>,
        line: usize,
    ) -> Self {
        Self::Dialogue(DialogueData {
            id: Uuid::new_v4().to_string(),
            block_id: None,
            speaker: speaker.map(|s| s.into()),
            speaker_alias: None,
            original_text: original.into(),
            translated_text: None,
            file_path: file.into(),
            line_number: line,
            instruction: None,
            status: TranslationStatus::Untranslated,
        })
    }

    pub fn choice(options: Vec<impl Into<String>>) -> Self {
        Self::Choice(ChoiceData {
            id: Uuid::new_v4().to_string(),
            prompt: None,
            options: options
                .into_iter()
                .map(|opt| ChoiceOption {
                    text: opt.into(),
                    translation: None,
                })
                .collect(),
        })
    }

    pub fn label(name: impl Into<String>) -> Self {
        Self::Label(name.into())
    }

    pub fn as_dialogue(&self) -> Option<&DialogueData> {
        match self {
            Self::Dialogue(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_dialogue_mut(&mut self) -> Option<&mut DialogueData> {
        match self {
            Self::Dialogue(data) => Some(data),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EdgeKind {
    /// Обычный последовательный переход
    Linear,
    /// Ветвление по выбору игрока (индекс варианта и текст)
    Branch { option_index: usize, text: String },
    /// Условный переход (if / elif / else)
    Condition(String),
    /// Переход на другую метку (jump / call)
    Jump(String),
}

/// Граф сценария визуальной новеллы / игры (DAG)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioGraph {
    graph: DiGraph<ScenarioNode, EdgeKind>,
    #[serde(skip)]
    node_map: HashMap<String, NodeIndex>,
}

impl Default for ScenarioGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ScenarioGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// Добавить узел в граф
    pub fn add_node(&mut self, node: ScenarioNode) -> NodeIndex {
        let id = match &node {
            ScenarioNode::Dialogue(d) => d.id.clone(),
            ScenarioNode::Choice(c) => c.id.clone(),
            ScenarioNode::Event(e) => e.id.clone(),
            ScenarioNode::Label(name) => format!("label:{}", name),
        };
        let idx = self.graph.add_node(node);
        self.node_map.insert(id, idx);
        idx
    }

    /// Соединить два узла направленным ребром
    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex, kind: EdgeKind) {
        self.graph.add_edge(from, to, kind);
    }

    /// Получить узел по индексу
    pub fn get_node(&self, idx: NodeIndex) -> Option<&ScenarioNode> {
        self.graph.node_weight(idx)
    }

    /// Получить мутабельный узел по индексу
    pub fn get_node_mut(&mut self, idx: NodeIndex) -> Option<&mut ScenarioNode> {
        self.graph.node_weight_mut(idx)
    }

    /// Итератор по всем индексам узлов в графе
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> {
        self.graph.node_indices()
    }

    /// Собирает скользящий контекст (N предшествующих диалогов) строго вдоль текущего пути в графе.
    pub fn get_parent_context(&self, start: NodeIndex, max_lines: usize) -> Vec<DialogueData> {
        let mut context = Vec::new();
        let mut current = start;

        while context.len() < max_lines {
            let mut incoming = self.graph.edges_directed(current, Direction::Incoming);
            if let Some(edge) = incoming.next() {
                let parent_idx = edge.source();
                if let Some(node) = self.get_node(parent_idx) {
                    if let ScenarioNode::Dialogue(d) = node {
                        context.push(d.clone());
                    }
                }
                current = parent_idx;
            } else {
                break;
            }
        }

        context.reverse();
        context
    }

    /// Статистика перевода: (всего диалоговых строк, переведено)
    pub fn translation_stats(&self) -> (usize, usize) {
        let mut total = 0;
        let mut translated = 0;
        for node in self.graph.node_weights() {
            if let ScenarioNode::Dialogue(d) = node {
                total += 1;
                if d.translated_text.is_some()
                    || d.status == TranslationStatus::Translated
                    || d.status == TranslationStatus::Reviewed
                {
                    translated += 1;
                }
            }
        }
        (total, translated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_branching_and_context_isolation() {
        let mut sg = ScenarioGraph::new();

        let n1 = sg.add_node(ScenarioNode::dialogue(
            Some("MC"),
            "Hey bartender, what's good here?",
            "script.rpy",
            10,
        ));
        let n2 = sg.add_node(ScenarioNode::dialogue(
            Some("Bartender"),
            "Depends. You want a fight or a drink?",
            "script.rpy",
            11,
        ));
        sg.add_edge(n1, n2, EdgeKind::Linear);

        let choice = sg.add_node(ScenarioNode::choice(vec!["Fight", "Drink"]));
        sg.add_edge(n2, choice, EdgeKind::Linear);

        let fight_1 = sg.add_node(ScenarioNode::dialogue(
            Some("MC"),
            "Let's fight!",
            "script.rpy",
            20,
        ));
        let fight_2 = sg.add_node(ScenarioNode::dialogue(
            Some("Bartender"),
            "Eat this!",
            "script.rpy",
            21,
        ));
        sg.add_edge(
            choice,
            fight_1,
            EdgeKind::Branch {
                option_index: 0,
                text: "Fight".to_string(),
            },
        );
        sg.add_edge(fight_1, fight_2, EdgeKind::Linear);

        let drink_1 = sg.add_node(ScenarioNode::dialogue(
            Some("MC"),
            "Just beer, please.",
            "script.rpy",
            30,
        ));
        let drink_2 = sg.add_node(ScenarioNode::dialogue(
            Some("Bartender"),
            "Here's your cold ale.",
            "script.rpy",
            31,
        ));
        sg.add_edge(
            choice,
            drink_1,
            EdgeKind::Branch {
                option_index: 1,
                text: "Drink".to_string(),
            },
        );
        sg.add_edge(drink_1, drink_2, EdgeKind::Linear);

        let drink_context = sg.get_parent_context(drink_2, 5);
        let texts: Vec<&str> = drink_context
            .iter()
            .map(|d| d.original_text.as_str())
            .collect();

        assert_eq!(
            texts,
            vec![
                "Hey bartender, what's good here?",
                "Depends. You want a fight or a drink?",
                "Just beer, please."
            ]
        );

        assert!(!texts.contains(&"Let's fight!"));
        assert!(!texts.contains(&"Eat this!"));
    }
}
