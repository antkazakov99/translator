//! Сценарии проектов: порядок шагов, диск и транзакции.

use std::collections::hash_map::Entry;
use std::sync::Arc;

use super::{GameRoot, Project, ProjectId, ProjectRepo, Title};
use crate::common::{AppError, Timestamp};
use crate::engines::{self, GameSource};
use crate::files::FileStore;
use crate::scan::ScanRepo;
use crate::state::{AppState, OpenProject};
use crate::storage::ProjectDb;

/// Итог смены папки.
#[derive(Debug)]
pub enum RebindCheck {
    /// Папка сменена, игра считана заново.
    Done(Project),
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
        let snapshot = engine.read(&GameSource::Folder(root), &info, &files)?;

        let db = ProjectDb::create(&dir, &project)?;
        db.write(|tx| ScanRepo { conn: tx }.save(&snapshot, None, engine.parser_version()).map(|_| ()))?;

        project.ready()?;
        self.state.app.write(|tx| ProjectRepo { conn: tx }.save(&project))?;
        Ok(project)
    }

    /// Сначала файлы, потом `opened_at`: с повреждённой project.db проект не поднимется наверх списка.
    pub fn open(&self, id: ProjectId) -> Result<Project, AppError> {
        let mut project = self.state.app.read(|tx| ProjectRepo { conn: tx }.get(id))?;
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

    pub fn archive(&self, id: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    pub fn unarchive(&self, id: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    pub fn rename(&self, id: ProjectId, title: Title) -> Result<(), AppError> {
        todo!()
    }

    /// Привязывает другую папку: движок узнаёт игру, модель сверяет движок, игра считывается заново
    /// с переносом перевода. Без `confirmed` чужая на вид игра только возвращает предупреждение.
    pub fn rebind(&self, id: ProjectId, root: GameRoot, confirmed: bool) -> Result<RebindCheck, AppError> {
        todo!()
    }

    pub fn unbind(&self, id: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    /// `mark_deleting` и сохранение, закрытие, удаление папки, потом строки. Сбой посередине дочищает запуск.
    pub fn delete(&self, id: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    pub fn list(&self) -> Result<Vec<Project>, AppError> {
        Ok(self.state.app.read(|tx| ProjectRepo { conn: tx }.list())?)
    }
}
