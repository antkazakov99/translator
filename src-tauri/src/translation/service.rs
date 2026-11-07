//! Порядок шагов перевода и редактуры. Ответ модели ждётся вне транзакции, каждый пакет пишется короткой.

use tauri::ipc::Channel;

use super::{FileProgress, Scene, SceneLine, TranslationRepo, TranslationScope};
use crate::common::{AppError, Timestamp};
use crate::llm::{ProfileId, RunId, RunProgress};
use crate::projects::ProjectId;
use crate::scan::{FileId, SegmentId};
use crate::state::AppState;

pub struct TranslationService<'a> {
    pub state: &'a AppState,
}

impl TranslationService<'_> {
    /// Заполняет пустые реплики. Сцена без описания сначала читается целиком. Возвращает число переведённых.
    pub async fn translate(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        todo!()
    }

    /// Вычитывает готовые реплики, исправления становятся черновиками. Возвращает число исправленных.
    pub async fn review(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        todo!()
    }

    /// Сброс перевода: пустые правки, как ручная очистка строки.
    pub fn reset(&self, project: ProjectId, scope: TranslationScope) -> Result<usize, AppError> {
        todo!()
    }

    /// Файлы с репликами и прогресс по ним.
    pub fn files(&self, project: ProjectId) -> Result<Vec<FileProgress>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| TranslationRepo { conn: tx }.files())?)
    }

    /// Реплики файла по сценам, в порядке файла.
    pub fn file_scenes(&self, project: ProjectId, file: FileId) -> Result<Vec<Scene>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| TranslationRepo { conn: tx }.file_scenes(file))?)
    }

    /// Правка пользователя одной транзакцией с историей. Возвращает реплику после правки.
    pub fn save_text(&self, project: ProjectId, line: SegmentId, text: &str) -> Result<SceneLine, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| {
            let repo = TranslationRepo { conn: tx };
            repo.save_user_text(line, text, Timestamp::now())?;
            repo.line(line)
        })?)
    }

    /// Подтверждает черновики модели. Возвращает число подтверждённых.
    pub fn confirm(&self, project: ProjectId, lines: &[SegmentId]) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| TranslationRepo { conn: tx }.confirm(lines, Timestamp::now()))?)
    }

    /// Откат запуска модели. Возвращает число откаченных и оставленных реплик.
    pub fn rollback(&self, project: ProjectId, run: RunId) -> Result<(usize, usize), AppError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;
    use crate::translation::LineState;

    /// Открытый проект по синтетической игре.
    fn project(dir: &std::path::Path) -> (AppState, ProjectId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        (state, id)
    }

    fn revisions(state: &AppState, project: ProjectId) -> i64 {
        let open = state.open_project(project).unwrap();
        open.project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))?)).unwrap()
    }

    #[test]
    fn file_lines_come_by_scenes_in_file_order() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };

        let files = service.files(id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "game/script.rpy");
        assert_eq!(files[0].translated, 0);

        let scenes = service.file_scenes(id, files[0].id).unwrap();
        let keys: Vec<_> = scenes.iter().map(|scene| scene.key.as_ref().map(|key| key.as_str())).collect();
        assert_eq!(keys, [Some("label:start"), Some("label:left")]);
        let first = &scenes[0].lines[0];
        assert_eq!(first.source_text, "Hello.");
        assert_eq!(first.speaker.as_deref(), Some("Eileen"));
        assert_eq!(first.state, LineState::Empty);
        assert!(first.location.is_some());
    }

    #[test]
    fn user_edit_writes_history_once_and_clearing_keeps_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let line = service.file_scenes(id, file).unwrap()[0].lines[0].id;

        let saved = service.save_text(id, line, "Привет.").unwrap();
        assert_eq!((saved.text.as_str(), saved.state), ("Привет.", LineState::Done));
        service.save_text(id, line, "Привет.").unwrap();
        assert_eq!(revisions(&state, id), 1, "тот же текст историю не пишет");
        assert_eq!(service.files(id).unwrap()[0].translated, 1);

        let cleared = service.save_text(id, line, "   ").unwrap();
        assert_eq!((cleared.text.as_str(), cleared.state), ("", LineState::Empty));
        assert_eq!(revisions(&state, id), 2);
        assert_eq!(service.files(id).unwrap()[0].translated, 0);
    }

    #[test]
    fn confirming_a_draft_makes_it_done_and_changed_source_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let line = service.file_scenes(id, file).unwrap()[0].lines[0].id;
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                tx.execute(
                    "INSERT INTO translations (segment_id, text, draft, source_hash, updated_at)
                     VALUES (?1, 'Здравствуй.', 1, 'старый', 0)",
                    [line.get()],
                )?;
                Ok(())
            })
            .unwrap();

        let draft = &service.file_scenes(id, file).unwrap()[0].lines[0];
        assert_eq!(draft.state, LineState::Draft);
        assert!(draft.source_changed);
        assert_eq!(service.files(id).unwrap()[0].drafts, 1);

        assert_eq!(service.confirm(id, &[line, line]).unwrap(), 1);
        let done = &service.file_scenes(id, file).unwrap()[0].lines[0];
        assert_eq!((done.text.as_str(), done.state), ("Здравствуй.", LineState::Done));
        assert_eq!(revisions(&state, id), 1);
    }

    #[test]
    fn closed_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        ProjectService { state: &state }.close(id);
        assert!(matches!(TranslationService { state: &state }.files(id), Err(AppError::ProjectNotOpen)));
    }
}
