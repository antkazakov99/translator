//! Сценарии проектов: порядок шагов, диск и транзакции.

use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{GameRoot, Project, ProjectId, ProjectRepo, Title};
use crate::common::{AppError, Timestamp};
use crate::engines::{self, GameSource};
use crate::files::FileStore;
use crate::scan::{ScanReport, ScanRepo, ScanService};
use crate::state::{AppState, OpenProject, remove_dir};
use crate::storage::{DbError, ProjectDb};

/// Итог смены папки.
#[derive(Debug)]
pub enum RebindCheck {
    /// Папка сменена, игра считана заново.
    Done { project: Project, report: ScanReport },
    /// Общих ключей реплик со старым снимком меньше половины: окно спрашивает, та ли это игра.
    LooksLikeOtherGame { shared: usize, total: usize },
}

pub struct ProjectService<'a> {
    pub state: &'a AppState,
}

impl ProjectService<'_> {
    /// Создаёт проект по корневой папке игры. Сбой до конца оставляет строку `creating`,
    /// следующий запуск приложения удалит её вместе с папкой.
    pub fn create(&self, root: GameRoot) -> Result<Project, AppError> {
        let (engine, info) = engines::detect(&root).ok_or(AppError::UnknownGame)?;
        let mut project = Project::create(Title::from_root(&root), root.clone(), engine.id(), Timestamp::now());
        self.state.app.write(|tx| ProjectRepo { conn: tx }.save(&project))?;

        let dir = self.state.project_dir(project.id());
        let files = FileStore::create(&dir)?;
        // Долго и вне транзакции: файлы пишутся в files/ по мере чтения.
        let snapshot = engine.read(&GameSource::Folder(root.clone()), &info, &files)?;

        let db = ProjectDb::create(&dir, &project)?;
        db.write(|tx| ScanRepo { conn: tx }.save(&snapshot, Some(&root), engine.parser_version(), engine.edges_version()).map(|_| ()))?;

        project.ready()?;
        self.state.app.write(|tx| ProjectRepo { conn: tx }.save(&project))?;
        Ok(project)
    }

    /// Сначала файлы, потом `opened_at`: с повреждённой project.db проект не поднимется наверх списка.
    pub fn open(&self, id: ProjectId) -> Result<Project, AppError> {
        let mut project = self.get(id)?;
        project.open(Timestamp::now())?;

        // Уже открытый проект второй раз не открывается.
        if let Entry::Vacant(slot) = self.state.open_projects.lock().entry(id) {
            slot.insert(Arc::new(OpenProject::open(id, &self.state.project_dir(id))?));
        }

        self.state.app.write(|tx| ProjectRepo { conn: tx }.save(&project))?;
        Ok(project)
    }

    /// Убирает проект из открытых. Базы закроются в `Drop` последнего `Arc`.
    pub fn close(&self, id: ProjectId) {
        self.state.open_projects.lock().remove(&id);
    }

    pub fn archive(&self, id: ProjectId) -> Result<Project, AppError> {
        self.update(id, |project| Ok(project.archive()?))
    }

    pub fn unarchive(&self, id: ProjectId) -> Result<Project, AppError> {
        self.update(id, |project| Ok(project.unarchive()?))
    }

    pub fn rename(&self, id: ProjectId, title: Title) -> Result<Project, AppError> {
        self.update(id, |project| {
            project.rename(title);
            Ok(())
        })
    }

    /// Привязывает другую папку: движок узнаёт игру, модель сверяет движок, игра считывается заново
    /// с переносом перевода. Без `confirmed` чужая на вид игра только возвращает предупреждение.
    pub fn rebind(&self, id: ProjectId, root: GameRoot, confirmed: bool) -> Result<RebindCheck, AppError> {
        let (engine, info) = engines::detect(&root).ok_or(AppError::UnknownGame)?;
        // Правило модели до долгого чтения: другой движок или неготовый проект сразу дают ошибку.
        self.get(id)?.rebind(root.clone(), engine.id())?;

        self.open(id)?;
        let open = self.state.open_project(id)?;
        let _lock = open.snapshot_lock.lock();
        let snapshot = engine.read(&GameSource::Folder(root.clone()), &info, &open.files)?;
        if !confirmed {
            let old: HashSet<String> = open.project.read(|tx| ScanRepo { conn: tx }.segment_keys())?.into_iter().collect();
            let shared = snapshot.segments.iter().filter(|segment| old.contains(segment.key.as_str())).count();
            if !old.is_empty() && shared * 2 < old.len() {
                // Считанные файлы этой папки не нужны, пока пользователь не подтвердит её.
                ScanService::clean_files(&open);
                return Ok(RebindCheck::LooksLikeOtherGame { shared, total: old.len() });
            }
        }
        let report = ScanService::apply(&open, engine, &snapshot, Some(&root))?;
        let project = self.update(id, |project| Ok(project.rebind(root, engine.id())?))?;
        Ok(RebindCheck::Done { project, report })
    }

    /// Отвязывает папку игры: проект работает дальше, недоступны установка мода и повторное считывание.
    pub fn unbind(&self, id: ProjectId) -> Result<Project, AppError> {
        self.update(id, |project| {
            project.unbind();
            Ok(())
        })
    }

    /// `mark_deleting` и сохранение, закрытие, удаление папки, потом строки. Сбой посередине дочищает запуск.
    pub fn delete(&self, id: ProjectId) -> Result<(), AppError> {
        self.update(id, |project| {
            project.mark_deleting();
            Ok(())
        })?;
        self.close(id);
        remove_dir(&self.state.project_dir(id))?;
        self.state.app.write(|tx| ProjectRepo { conn: tx }.delete(id))?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Project>, AppError> {
        Ok(self.state.app.read(|tx| ProjectRepo { conn: tx }.list())?)
    }

    /// Готовые проекты, привязанные к папке: при выборе папки окно предлагает открыть их.
    pub fn find_by_root(&self, root: &GameRoot) -> Result<Vec<Project>, AppError> {
        Ok(self.state.app.read(|tx| ProjectRepo { conn: tx }.find_by_root(root))?)
    }

    /// Путь из окна в корень игры: абсолютный, с регистром как на диске. Вложенную папку на родителя
    /// не исправляет: что в ней игра, решает движок.
    pub fn resolve_root(path: &Path) -> Result<GameRoot, AppError> {
        let canonical = std::fs::canonicalize(path)?;
        Ok(GameRoot::new(&without_verbatim_prefix(canonical))?)
    }

    fn get(&self, id: ProjectId) -> Result<Project, AppError> {
        self.state.app.read(|tx| ProjectRepo { conn: tx }.get(id)).map_err(not_found)
    }

    /// Чтение, правило модели и запись одной транзакцией.
    fn update(&self, id: ProjectId, change: impl FnOnce(&mut Project) -> Result<(), AppError>) -> Result<Project, AppError> {
        let mut result = Ok(());
        let project = self
            .state
            .app
            .write(|tx| {
                let repo = ProjectRepo { conn: tx };
                let mut project = repo.get(id)?;
                result = change(&mut project);
                if result.is_ok() {
                    repo.save(&project)?;
                }
                Ok(project)
            })
            .map_err(not_found)?;
        result.map(|()| project)
    }
}

/// `canonicalize` в Windows отдаёт путь с `\\?\`: такой путь пользователь не узнает, а сравнение с его вводом не совпадёт.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn not_found(error: DbError) -> AppError {
    match error {
        DbError::NotFound => AppError::ProjectNotFound,
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use parking_lot::Mutex;

    use super::*;
    use crate::common::ValidationError;
    use crate::secrets::SecretStore;
    use crate::state::tests::ready_project;

    fn state(dir: &std::path::Path) -> AppState {
        AppState::start(dir.to_path_buf(), SecretStore::Memory(Mutex::new(HashMap::new()))).unwrap()
    }

    #[test]
    fn open_once_and_close() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let service = ProjectService { state: &state };
        let project = ready_project(&state, "game");

        let opened = service.open(project.id()).unwrap();
        assert!(opened.opened_at().is_some());
        let first = state.open_project(project.id()).unwrap();
        service.open(project.id()).unwrap();
        assert!(Arc::ptr_eq(&first, &state.open_project(project.id()).unwrap()));

        service.close(project.id());
        assert!(matches!(state.open_project(project.id()), Err(AppError::ProjectNotOpen)));
    }

    #[test]
    fn opening_returns_project_from_archive() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let service = ProjectService { state: &state };
        let project = ready_project(&state, "game");

        assert!(service.archive(project.id()).unwrap().is_archived());
        assert!(!service.open(project.id()).unwrap().is_archived());
        assert!(!service.list().unwrap()[0].is_archived());
    }

    #[test]
    fn rename_and_unbind_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let service = ProjectService { state: &state };
        let project = ready_project(&state, "game");
        let root = project.game_root().unwrap().clone();
        assert_eq!(service.find_by_root(&root).unwrap().len(), 1);

        service.rename(project.id(), Title::new("Новое имя").unwrap()).unwrap();
        service.unbind(project.id()).unwrap();
        let saved = &service.list().unwrap()[0];
        assert_eq!(saved.title().as_str(), "Новое имя");
        assert!(!saved.is_bound());
        assert!(service.find_by_root(&root).unwrap().is_empty());
    }

    #[test]
    fn delete_closes_and_removes_everything() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let service = ProjectService { state: &state };
        let project = ready_project(&state, "game");
        service.open(project.id()).unwrap();

        service.delete(project.id()).unwrap();
        assert!(!state.project_dir(project.id()).exists());
        assert!(service.list().unwrap().is_empty());
        assert!(matches!(service.open(project.id()), Err(AppError::ProjectNotFound)));
    }

    #[test]
    fn create_reads_the_game_into_a_ready_project() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let service = ProjectService { state: &state };
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.path().join("Sample Game"));

        let project = service.create(root.clone()).unwrap();
        assert_eq!(project.title().as_str(), "Sample Game");
        assert_eq!(project.engine(), crate::engines::EngineId::Renpy);
        assert_eq!(service.find_by_root(&root).unwrap().len(), 1);

        service.open(project.id()).unwrap();
        let open = state.open_project(project.id()).unwrap();
        let (segments, speakers, scenes, search_rev): (i64, i64, i64, i64) = open
            .project
            .read(|tx| {
                Ok(tx.query_row(
                    "SELECT (SELECT count(*) FROM segments), (SELECT count(*) FROM speakers),
                            (SELECT count(*) FROM containers WHERE kind = 'scene'), (SELECT search_rev FROM project)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?)
            })
            .unwrap();
        assert!(segments >= 6);
        assert!(speakers >= 2);
        assert_eq!(scenes, 2);
        assert_eq!(search_rev, 1);

        let (files, scan) = open
            .project
            .read(|tx| {
                let repo = ScanRepo { conn: tx };
                Ok((repo.current_files()?, repo.latest()?))
            })
            .unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| open.files.contains(&file.hash)));
        let scan = scan.unwrap();
        assert_eq!(scan.game_root.as_ref(), Some(&root));
        assert_eq!(scan.game_info.version.as_deref(), Some("8.1.3.23091805"));
    }

    #[test]
    fn folder_without_a_game_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let service = ProjectService { state: &state };
        let root = GameRoot::new(dir.path()).unwrap();
        assert!(matches!(service.create(root), Err(AppError::UnknownGame)));
        assert!(service.list().unwrap().is_empty());
    }

    #[test]
    fn unfinished_project_cannot_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let service = ProjectService { state: &state };
        let mut project = ready_project(&state, "game");
        project.mark_deleting();
        state.app.write(|tx| ProjectRepo { conn: tx }.save(&project)).unwrap();

        let result = service.open(project.id());
        assert!(matches!(result, Err(AppError::Validation(ValidationError::WrongStatus("deleting")))));
        assert!(state.open_project(project.id()).is_err());
    }
}
