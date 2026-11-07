//! Таблица `projects` в app.db и строка `project` в project.db.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{GameRoot, Project, ProjectId, ProjectRow};
use crate::common::Timestamp;
use crate::engines::EngineId;
use crate::name_rules::LanguageTag;
use crate::storage::DbError;

/// Что project.db знает о своём проекте: по этому список восстанавливается без app.db.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    pub id: ProjectId,
    pub engine: EngineId,
    pub created_at: Timestamp,
}

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

    pub fn get(&self) -> Result<ProjectInfo, DbError> {
        let (id, engine, created_at): (String, String, i64) = self
            .conn
            .query_row("SELECT id, engine, created_at FROM project", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?
            .ok_or(DbError::NotFound)?;
        Ok(ProjectInfo {
            id: ProjectId::parse(&id).map_err(|error| DbError::bad_value("project", "id", error))?,
            engine: EngineId::parse(&engine).map_err(|error| DbError::bad_value("project", "engine", error))?,
            created_at: Timestamp::from_secs(created_at),
        })
    }
}

const COLUMNS: &str = "id, status, title, engine, game_path, created_at, opened_at, archived";

/// Живёт одну операцию и хранит только ссылку на соединение или транзакцию сервиса.
pub struct ProjectRepo<'c> {
    pub conn: &'c Connection,
}

impl ProjectRepo<'_> {
    /// Проект в любом статусе: сервис сам решает, что делать с `creating` и `deleting`.
    pub fn get(&self, id: ProjectId) -> Result<Project, DbError> {
        let sql = format!("SELECT {COLUMNS} FROM projects WHERE id = ?1");
        let row = self.conn.query_row(&sql, [id.to_string()], read_row).optional()?;
        restore(row.ok_or(DbError::NotFound)?)
    }

    /// Готовые проекты: сначала основной список по последнему открытию, архивные в конце.
    pub fn list(&self) -> Result<Vec<Project>, DbError> {
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM projects WHERE status = 'ready'
                 ORDER BY archived, opened_at DESC NULLS LAST, created_at DESC"
            ),
            [],
        )
    }

    /// Готовые проекты, привязанные к папке: при выборе папки окно предлагает открыть их.
    pub fn find_by_root(&self, root: &GameRoot) -> Result<Vec<Project>, DbError> {
        let path = root.as_path().to_string_lossy().into_owned();
        self.query(
            &format!(
                "SELECT {COLUMNS} FROM projects WHERE status = 'ready' AND game_path = ?1
                 ORDER BY opened_at DESC NULLS LAST, created_at DESC"
            ),
            [path],
        )
    }

    /// Строки `creating` и `deleting` для дочистки при запуске приложения.
    pub fn unfinished(&self) -> Result<Vec<Project>, DbError> {
        self.query(&format!("SELECT {COLUMNS} FROM projects WHERE status <> 'ready'"), [])
    }

    /// Есть ли строка в любом статусе: папка без строки восстанавливается в список.
    pub fn exists(&self, id: ProjectId) -> Result<bool, DbError> {
        Ok(self.conn.prepare("SELECT 1 FROM projects WHERE id = ?1")?.exists([id.to_string()])?)
    }

    /// Вставляет или заменяет строку целиком.
    pub fn save(&self, project: &Project) -> Result<(), DbError> {
        let row = project.to_row();
        self.conn.execute(
            &format!(
                "INSERT INTO projects ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                   status = excluded.status, title = excluded.title, engine = excluded.engine,
                   game_path = excluded.game_path, created_at = excluded.created_at,
                   opened_at = excluded.opened_at, archived = excluded.archived"
            ),
            params![
                row.id,
                row.status,
                row.title,
                row.engine,
                row.game_path,
                row.created_at,
                row.opened_at,
                row.archived
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: ProjectId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }

    fn query(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<Project>, DbError> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map(params, read_row)?.collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(restore).collect()
    }
}

fn read_row(row: &Row) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: row.get(0)?,
        status: row.get(1)?,
        title: row.get(2)?,
        engine: row.get(3)?,
        game_path: row.get(4)?,
        created_at: row.get(5)?,
        opened_at: row.get(6)?,
        archived: row.get(7)?,
    })
}

fn restore(row: ProjectRow) -> Result<Project, DbError> {
    Project::restore(row).map_err(|error| DbError::bad_value("projects", "row", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::Title;
    use crate::storage::AppDb;

    fn project(dir: &std::path::Path, title: &str) -> Project {
        let root = GameRoot::new(&dir.join(title)).unwrap();
        Project::create(Title::new(title).unwrap(), root, EngineId::Renpy, Timestamp::from_secs(100))
    }

    #[test]
    fn saved_project_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDb::open(&dir.path().join("app.db")).unwrap();
        let mut saved = project(dir.path(), "Игра");
        saved.ready().unwrap();
        saved.open(Timestamp::from_secs(200)).unwrap();
        db.write(|tx| ProjectRepo { conn: tx }.save(&saved)).unwrap();
        saved.archive().unwrap();
        db.write(|tx| ProjectRepo { conn: tx }.save(&saved)).unwrap();

        let read = db.read(|tx| ProjectRepo { conn: tx }.get(saved.id())).unwrap();
        assert_eq!(read, saved);
    }

    #[test]
    fn list_hides_unfinished_and_puts_archive_last() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDb::open(&dir.path().join("app.db")).unwrap();
        let creating = project(dir.path(), "creating");
        let mut old = project(dir.path(), "old");
        let mut recent = project(dir.path(), "recent");
        let mut archived = project(dir.path(), "archived");
        for (project, opened) in [(&mut old, 10), (&mut recent, 20), (&mut archived, 30)] {
            project.ready().unwrap();
            project.open(Timestamp::from_secs(opened)).unwrap();
        }
        archived.archive().unwrap();
        db.write(|tx| {
            let repo = ProjectRepo { conn: tx };
            [&creating, &old, &recent, &archived].into_iter().try_for_each(|project| repo.save(project))
        })
        .unwrap();

        let titles: Vec<String> = db
            .read(|tx| ProjectRepo { conn: tx }.list())
            .unwrap()
            .iter()
            .map(|project| project.title().as_str().to_string())
            .collect();
        assert_eq!(titles, ["recent", "old", "archived"]);

        let unfinished = db.read(|tx| ProjectRepo { conn: tx }.unfinished()).unwrap();
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].id(), creating.id());

        let root = recent.game_root().unwrap().clone();
        let found = db.read(|tx| ProjectRepo { conn: tx }.find_by_root(&root)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id(), recent.id());
    }

    #[test]
    fn broken_row_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDb::open(&dir.path().join("app.db")).unwrap();
        let id = ProjectId::generate();
        db.write(|tx| {
            tx.execute(
                "INSERT INTO projects (id, title, engine, created_at) VALUES (?1, 'x', 'unity', 0)",
                [id.to_string()],
            )?;
            Ok(())
        })
        .unwrap();
        let result = db.read(|tx| ProjectRepo { conn: tx }.get(id));
        assert!(matches!(result, Err(DbError::BadValue { .. })));
    }
}
