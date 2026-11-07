//! Граф сцен проекта.

use super::{GraphRepo, SceneGraph};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::scan::{EdgeRepo, ScanService};
use crate::state::AppState;

pub struct GraphService<'a> {
    pub state: &'a AppState,
}

impl GraphService<'_> {
    /// Сцены и переходы. Отставшие переходы сначала пересобираются из `files/`: у проекта, созданного
    /// до таблицы `edges`, или после роста версии сборки переходов первый запрос дольше.
    pub fn graph(&self, project: ProjectId) -> Result<SceneGraph, AppError> {
        ScanService { state: self.state }.refresh_edges(project)?;
        let open = self.state.open_project(project)?;
        let graph = open.project.read(|tx| {
            Ok(SceneGraph { scenes: GraphRepo { conn: tx }.scenes()?, edges: EdgeRepo { conn: tx }.list()? })
        })?;
        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    #[test]
    fn graph_has_scenes_with_progress_and_rebuilds_lost_edges() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.path().join("Sample Game"));
        let project = ProjectService { state: &state }.create(root).unwrap();
        ProjectService { state: &state }.open(project.id()).unwrap();
        let service = GraphService { state: &state };

        let graph = service.graph(project.id()).unwrap();
        let keys: Vec<&str> = graph.scenes.iter().map(|scene| scene.key.as_str()).collect();
        assert_eq!(keys, ["label:start", "label:left"]);
        let start = &graph.scenes[0];
        assert!(start.total > 0);
        assert_eq!(start.translated, 0);
        assert_eq!(start.first_line.as_deref(), Some("Hello."));
        assert!(start.file.is_some());
        let to_left: Vec<&str> = graph
            .edges
            .iter()
            .filter(|edge| edge.from.as_str() == "label:start" && edge.to.as_str() == "label:left")
            .map(|edge| edge.kind.as_str())
            .collect();
        assert!(to_left.contains(&"choice") && to_left.contains(&"jump"), "{:?}", graph.edges);

        // Проект без собранных переходов (создан до таблицы edges): граф собирает их из files/.
        let open = state.open_project(project.id()).unwrap();
        open.project
            .write(|tx| Ok(tx.execute_batch("DELETE FROM edges; DELETE FROM derived WHERE name = 'edges';")?))
            .unwrap();
        assert_eq!(service.graph(project.id()).unwrap().edges, graph.edges);
        assert!(!ScanService { state: &state }.refresh_edges(project.id()).unwrap());
    }
}
