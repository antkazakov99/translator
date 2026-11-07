//! Один файл базы: соединение на запись под `Mutex` и пул соединений на чтение.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::{Connection, Transaction};

use super::DbError;
use crate::projects::Project;

/// Один файл SQLite.
///
/// - Запись идёт через одно соединение под `Mutex`: записи встают в очередь вместо `SQLITE_BUSY`.
/// - Чтение в режиме WAL идёт параллельно с записью, каждое в своей транзакции.
/// - Каждое соединение открывается с `journal_mode = WAL`, `foreign_keys = ON`, `busy_timeout = 5000`.
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
    /// Свободные соединения на чтение. Пустой пул открывает новое.
    readers: Mutex<Vec<Connection>>,
}

impl Database {
    /// Открывает файл и настраивает соединение на запись. Миграции делают обёртки.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        todo!()
    }

    /// Транзакция только для чтения: `query_only = ON`, все запросы видят одно состояние базы.
    pub fn read<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, DbError>) -> Result<T, DbError> {
        todo!()
    }

    /// Транзакция `BEGIN IMMEDIATE`. `Ok` фиксирует её, `Err` откатывает.
    pub fn write<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, DbError>) -> Result<T, DbError> {
        todo!()
    }

    /// Номер схемы из `PRAGMA user_version`.
    pub fn user_version(&self) -> Result<u32, DbError> {
        todo!()
    }

    /// Применяет недостающие миграции одной транзакцией. База новее кода не открывается.
    /// Пересборка таблицы выключает `foreign_keys` до транзакции и проверяет `foreign_key_check` перед фиксацией.
    fn migrate(&self, migrations: &[&str]) -> Result<(), DbError> {
        todo!()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Database {
    /// `wal_checkpoint(TRUNCATE)`: после закрытия папку можно копировать без `-wal` и `-shm`.
    fn drop(&mut self) {
        // TODO: checkpoint без паники, ошибку только в лог.
    }
}

/// Миграции app.db по порядку. Номер последней применённой хранится в `user_version`.
const APP_MIGRATIONS: &[&str] = &[];

/// Миграции project.db.
const PROJECT_MIGRATIONS: &[&str] = &[];

/// Версия формата cache.db. Не совпала с `user_version`: базу удаляют и создают заново.
const CACHE_VERSION: u32 = 1;

/// Схема cache.db: миграций нет, только создание.
const CACHE_SCHEMA: &str = "";

/// app.db: проекты, провайдеры и профили LLM, настройки действий, правила имён.
pub struct AppDb(Database);

impl AppDb {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        todo!()
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
    /// Новая база проекта со строкой `project`.
    pub fn create(dir: &Path, project: &Project) -> Result<Self, DbError> {
        todo!()
    }

    pub fn open(dir: &Path) -> Result<Self, DbError> {
        todo!()
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
    /// Открывает кэш, а при чужой версии формата удаляет файл и создаёт заново.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        todo!()
    }
}

impl Deref for CacheDb {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0
    }
}
