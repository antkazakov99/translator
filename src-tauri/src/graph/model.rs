//! Сцена графа и граф целиком.

use crate::engines::{ContainerKey, SnapshotEdge};
use crate::scan::FileId;

/// Сцена текущего снимка для графа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphScene {
    pub key: ContainerKey,
    /// Название из описаний сцен.
    pub title: Option<String>,
    /// Файл, где лежат реплики сцены: по нему окно открывает её в таблице. `None`: реплик нет.
    pub file: Option<FileId>,
    pub total: usize,
    pub translated: usize,
    pub drafts: usize,
    /// Первая реплика по сюжету: по ней сцену узнают на графе.
    pub first_line: Option<String>,
}

/// Сцены в порядке сюжета и переходы между ними.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneGraph {
    pub scenes: Vec<GraphScene>,
    pub edges: Vec<SnapshotEdge>,
}
