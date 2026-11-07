//! Один файл базы: соединение на запись под `Mutex` и пул соединений на чтение.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::{DbError, migrations};
use crate::common::Timestamp;
use crate::glossary::NameKey;
use crate::name_rules::LanguageTag;
use crate::projects::{Project, ProjectInfoRepo};

/// Сколько ждать чужую блокировку файла, прежде чем вернуть `SQLITE_BUSY`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Сколько свободных соединений на чтение держать открытыми. Лишние закрываются после чтения.
const MAX_IDLE_READERS: usize = 4;

/// Один файл SQLite.
///
/// - Запись идёт через одно соединение под `Mutex`: записи встают в очередь вместо `SQLITE_BUSY`.
/// - Чтение в режиме WAL идёт параллельно с записью, каждое в своей транзакции.
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
    /// Свободные соединения на чтение. Пустой пул открывает новое.
    readers: Mutex<Vec<Connection>>,
}

impl Database {
    /// Открывает или создаёт файл и настраивает соединение на запись. Миграции делают обёртки.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let writer = connect(path, false)?;
        Ok(Self { path: path.to_path_buf(), writer: Mutex::new(writer), readers: Mutex::new(Vec::new()) })
    }

    /// Транзакция только для чтения: `query_only = ON`, все запросы видят одно состояние базы.
    pub fn read<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, DbError>) -> Result<T, DbError> {
        let pooled = self.readers.lock().pop();
        let mut conn = match pooled {
            Some(conn) => conn,
            None => connect(&self.path, true)?,
        };
        let result = (|| {
            let tx = conn.transaction()?;
            let value = f(&tx)?;
            tx.commit()?;
            Ok(value)
        })();
        let mut pool = self.readers.lock();
        if pool.len() < MAX_IDLE_READERS {
            pool.push(conn);
        }
        result
    }

    /// Транзакция `BEGIN IMMEDIATE`. `Ok` фиксирует её, `Err` откатывает.
    pub fn write<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, DbError>) -> Result<T, DbError> {
        let mut conn = self.writer.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Соединение на запись целиком: миграциям нужны прагмы вне транзакции.
    pub(super) fn with_writer<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, DbError>) -> Result<T, DbError> {
        f(&mut self.writer.lock())
    }

    /// `PRAGMA user_version`: версия формата у баз без миграций.
    pub fn user_version(&self) -> Result<u32, DbError> {
        Ok(self.writer.lock().pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Database {
    /// `wal_checkpoint(TRUNCATE)`: после закрытия папку можно копировать без `-wal` и `-shm`.
    fn drop(&mut self) {
        self.readers.get_mut().clear();
        let result = self.writer.get_mut().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        if let Err(error) = result {
            tracing::warn!("Не удалось перенести журнал WAL в {}: {error}", self.path.display());
        }
    }
}

/// Соединение с настройками, которые действуют только на него самого.
fn connect(path: &Path, read_only: bool) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::Corrupt(format!("режим журнала {mode} вместо WAL: {}", path.display())));
    }
    conn.pragma_update(None, "foreign_keys", true)?;
    if read_only {
        conn.pragma_update(None, "query_only", true)?;
    }
    register_functions(&conn)?;
    Ok(conn)
}

/// Функции приложения в SQL: миграции пересчитывают через них ключи, правило остаётся в Rust.
fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "name_key",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let text: Option<String> = ctx.get(0)?;
            Ok(text.as_deref().and_then(NameKey::new).map(|key| key.as_str().to_string()))
        },
    )
}

/// Версия формата cache.db. Не совпала с `user_version`: базу удаляют и создают заново.
const CACHE_VERSION: u32 = 1;

/// Схема cache.db: миграций нет, только создание.
const CACHE_SCHEMA: &str = include_str!("schema/cache.sql");

/// app.db: проекты, провайдеры и профили LLM, настройки действий, правила имён.
pub struct AppDb(Database);

impl AppDb {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Database::open(path)?;
        db.with_writer(|conn| migrations::migrate(conn, path, migrations::APP))?;
        Ok(Self(db))
    }
}

impl Deref for AppDb {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0
    }
}

/// project.db одного проекта: снимок игры, переводы, глоссарий, заметки, запуски.
pub struct ProjectDb(Database);

impl ProjectDb {
    const FILE: &str = "project.db";

    /// Новая база проекта со строкой `project`. Существующую базу не трогает.
    pub fn create(dir: &Path, project: &Project) -> Result<Self, DbError> {
        let path = dir.join(Self::FILE);
        if path.exists() {
            return Err(DbError::Corrupt(format!("база проекта уже есть: {}", path.display())));
        }
        std::fs::create_dir_all(dir)?;
        let db = Self::open_path(&path)?;
        let language = LanguageTag::parse("ru").map_err(|error| DbError::bad_value("project", "target_language", error))?;
        db.write(|tx| ProjectInfoRepo { conn: tx }.insert(project, &language, Timestamp::now()))?;
        Ok(db)
    }

    /// Открывает базу готового проекта и применяет недостающие шаги миграций.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        let path = dir.join(Self::FILE);
        if !path.is_file() {
            return Err(DbError::NotFound);
        }
        Self::open_path(&path)
    }

    fn open_path(path: &Path) -> Result<Self, DbError> {
        let db = Database::open(path)?;
        db.with_writer(|conn| migrations::migrate(conn, path, migrations::PROJECT))?;
        Ok(Self(db))
    }
}

impl Deref for ProjectDb {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0
    }
}

/// cache.db одного проекта: разбор файлов и поисковый индекс. Можно удалить.
pub struct CacheDb(Database);

impl CacheDb {
    const FILE: &str = "cache.db";

    /// Открывает кэш, а при чужой версии формата удаляет файл и создаёт заново.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(Self::FILE);
        let db = Database::open(&path)?;
        if db.user_version()? == CACHE_VERSION {
            return Ok(Self(db));
        }
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let file = dir.join(format!("{}{suffix}", Self::FILE));
            match std::fs::remove_file(&file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
        let db = Database::open(&path)?;
        db.write(|tx| {
            tx.execute_batch(CACHE_SCHEMA)?;
            tx.pragma_update(None, "user_version", CACHE_VERSION)?;
            Ok(())
        })?;
        Ok(Self(db))
    }
}

impl Deref for CacheDb {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::EngineId;
    use crate::projects::{GameRoot, Title};

    fn temp_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        db.write(|tx| Ok(tx.execute_batch("CREATE TABLE items (name TEXT NOT NULL) STRICT;")?)).unwrap();
        (dir, db)
    }

    fn count(db: &Database) -> i64 {
        db.read(|tx| Ok(tx.query_row("SELECT count(*) FROM items", [], |row| row.get(0))?)).unwrap()
    }

    #[test]
    fn write_commits_on_ok_and_rolls_back_on_err() {
        let (_dir, db) = temp_db();
        db.write(|tx| Ok(tx.execute("INSERT INTO items VALUES ('a')", [])?)).unwrap();
        let failed: Result<(), DbError> = db.write(|tx| {
            tx.execute("INSERT INTO items VALUES ('b')", [])?;
            Err(DbError::NotFound)
        });
        assert!(failed.is_err());
        assert_eq!(count(&db), 1);
    }

    #[test]
    fn read_cannot_write() {
        let (_dir, db) = temp_db();
        let result = db.read(|tx| Ok(tx.execute("INSERT INTO items VALUES ('a')", [])?));
        assert!(result.is_err());
        assert_eq!(count(&db), 0);
    }

    #[test]
    fn readers_return_to_pool() {
        let (_dir, db) = temp_db();
        for _ in 0..10 {
            count(&db);
        }
        assert_eq!(db.readers.lock().len(), 1);
    }

    #[test]
    fn connections_use_wal_and_foreign_keys() {
        let (_dir, db) = temp_db();
        let (mode, keys): (String, bool) = db
            .read(|tx| {
                let mode = tx.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
                let keys = tx.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
                Ok((mode, keys))
            })
            .unwrap();
        assert_eq!(mode, "wal");
        assert!(keys);
    }

    #[test]
    fn name_key_works_in_sql() {
        let (_dir, db) = temp_db();
        let (key, empty): (String, Option<String>) = db
            .read(|tx| Ok(tx.query_row("SELECT name_key(' Город '), name_key('  ')", [], |row| Ok((row.get(0)?, row.get(1)?)))?))
            .unwrap();
        assert_eq!(key, "город");
        assert_eq!(empty, None);
    }

    #[test]
    fn closing_truncates_wal() {
        let (dir, db) = temp_db();
        db.write(|tx| Ok(tx.execute("INSERT INTO items VALUES ('a')", [])?)).unwrap();
        drop(db);
        let wal = dir.path().join("test.db-wal");
        assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0);
    }

    #[test]
    fn app_db_gets_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDb::open(&dir.path().join("data").join("app.db")).unwrap();
        let actions: i64 = db.read(|tx| Ok(tx.query_row("SELECT count(*) FROM llm_actions", [], |row| row.get(0))?)).unwrap();
        assert_eq!(actions, 7);
    }

    #[test]
    fn project_db_is_created_once_with_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let root = GameRoot::new(dir.path()).unwrap();
        let project = Project::create(Title::new("Игра").unwrap(), root, EngineId::Renpy, Timestamp::now());
        let project_dir = dir.path().join("projects").join(project.id().to_string());

        let db = ProjectDb::create(&project_dir, &project).unwrap();
        let (id, language): (String, String) = db
            .read(|tx| Ok(tx.query_row("SELECT id, target_language FROM project", [], |row| Ok((row.get(0)?, row.get(1)?)))?))
            .unwrap();
        assert_eq!(id, project.id().to_string());
        assert_eq!(language, "ru");
        drop(db);

        assert!(ProjectDb::create(&project_dir, &project).is_err());
        assert!(ProjectDb::open(&project_dir).is_ok());
        assert!(matches!(ProjectDb::open(&dir.path().join("missing")), Err(DbError::NotFound)));
    }

    #[test]
    fn cache_is_recreated_on_other_version() {
        let dir = tempfile::tempdir().unwrap();
        let db = CacheDb::open(dir.path()).unwrap();
        db.write(|tx| {
            tx.execute("INSERT INTO meta VALUES ('scan_id', '1')", [])?;
            tx.pragma_update(None, "user_version", 99)?;
            Ok(())
        })
        .unwrap();
        drop(db);

        let db = CacheDb::open(dir.path()).unwrap();
        assert_eq!(db.user_version().unwrap(), CACHE_VERSION);
        let (meta, fts): (i64, i64) = db
            .read(|tx| {
                let meta = tx.query_row("SELECT count(*) FROM meta", [], |row| row.get(0))?;
                let fts = tx.query_row("SELECT count(*) FROM segments_fts WHERE segments_fts MATCH 'город'", [], |row| row.get(0))?;
                Ok((meta, fts))
            })
            .unwrap();
        assert_eq!((meta, fts), (0, 0));
    }
}
