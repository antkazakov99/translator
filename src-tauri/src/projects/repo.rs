//! Таблица `projects` в app.db.

use rusqlite::Connection;

use super::{GameRoot, Project, ProjectId};
use crate::storage::DbError;

/// Живёт одну операцию и хранит только ссылку на соединение или транзакцию сервиса.
pub struct ProjectRepo<'c> {
    pub conn: &'c Connection,
}

impl ProjectRepo<'_> {
    /// Проект в любом статусе: сервис сам решает, что делать с `creating` и `deleting`.
    pub fn get(&self, id: ProjectId) -> Result<Project, DbError> {
        todo!()
    }

    /// Готовые проекты по `opened_at`, архивные в конце.
    pub fn list(&self) -> Result<Vec<Project>, DbError> {
        todo!()
    }

    /// Готовые проекты, привязанные к папке: при выборе папки окно предлагает открыть их.
    pub fn find_by_root(&self, root: &GameRoot) -> Result<Vec<Project>, DbError> {
        todo!()
    }

    /// Строки `creating` и `deleting` для дочистки при запуске приложения.
    pub fn unfinished(&self) -> Result<Vec<Project>, DbError> {
        todo!()
    }

    /// Вставляет или заменяет строку целиком.
    pub fn save(&self, project: &Project) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, id: ProjectId) -> Result<(), DbError> {
        todo!()
    }
}
