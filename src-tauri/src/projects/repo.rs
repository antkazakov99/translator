//! Таблица `projects` в app.db и строка `project` в project.db.

use rusqlite::{Connection, params};

use super::{GameRoot, Project, ProjectId};
use crate::common::Timestamp;
use crate::name_rules::LanguageTag;
use crate::storage::DbError;

/// Строка `project` в project.db: копия id и движка для восстановления списка без app.db, язык перевода.
pub struct ProjectInfoRepo<'c> {
    pub conn: &'c Connection,
}

impl ProjectInfoRepo<'_> {
    /// Единственная строка, пишется при создании базы проекта.
    pub fn insert(&self, project: &Project, target_language: &LanguageTag, now: Timestamp) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO project (id, engine, target_language, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![project.id().to_string(), project.engine().as_str(), target_language.as_str(), now.secs()],
        )?;
        Ok(())
    }
}

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
