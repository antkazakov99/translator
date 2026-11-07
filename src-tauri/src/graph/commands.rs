//! Команда графа сцен.

use serde::Serialize;
use tauri::AppHandle;

use super::{GraphScene, GraphService};
use crate::engines::SnapshotEdge;
use crate::projects::ProjectId;
use crate::state::blocking;
use crate::translation::commands::RowRef;

/// Сцена графа.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GraphSceneDto {
    /// Ключ сцены от движка, как у сцен таблицы.
    pub key: String,
    pub title: Option<String>,
    /// Файл таблицы, где лежат реплики сцены. `None`: реплик нет.
    pub file: Option<RowRef>,
    pub total: u32,
    pub translated: u32,
    pub drafts: u32,
    pub first_line: Option<String>,
}

impl From<GraphScene> for GraphSceneDto {
    fn from(scene: GraphScene) -> Self {
        Self {
            key: scene.key.to_string(),
            title: scene.title,
            file: scene.file.map(|file| RowRef(file.get())),
            total: scene.total as u32,
            translated: scene.translated as u32,
            drafts: scene.drafts as u32,
            first_line: scene.first_line,
        }
    }
}

/// Переход между сценами.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeDto {
    pub from: String,
    pub to: String,
    /// Вид от движка: `jump`, `call`, `branch`, `choice`, `screen`.
    pub kind: String,
    /// Условие, текст пункта меню или имя экрана. Пусто: без условия.
    pub condition: String,
}

impl From<SnapshotEdge> for GraphEdgeDto {
    fn from(edge: SnapshotEdge) -> Self {
        Self { from: edge.from.to_string(), to: edge.to.to_string(), kind: edge.kind, condition: edge.condition }
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneGraphDto {
    /// В порядке сюжета.
    pub scenes: Vec<GraphSceneDto>,
    pub edges: Vec<GraphEdgeDto>,
}

/// Граф сцен проекта. Первый запрос после считывания может пересобрать переходы из файлов снимка.
#[tauri::command]
#[specta::specta]
pub async fn scene_graph(app: AppHandle, project: String) -> Result<SceneGraphDto, String> {
    blocking(app, move |state| {
        let graph = GraphService { state }.graph(ProjectId::parse(&project)?)?;
        Ok(SceneGraphDto {
            scenes: graph.scenes.into_iter().map(GraphSceneDto::from).collect(),
            edges: graph.edges.into_iter().map(GraphEdgeDto::from).collect(),
        })
    })
    .await
}
