//! Ошибки работы с базой.

use thiserror::Error;

use crate::common::ValidationError;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Ошибка SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Ошибка диска: {0}")]
    Io(#[from] std::io::Error),
    /// Значение в колонке не прошло проверку типа или модели.
    #[error("Неверное значение в {table}.{column}: {source}")]
    BadValue {
        table: &'static str,
        column: &'static str,
        source: ValidationError,
    },
    #[error("Файл базы повреждён: {0}")]
    Corrupt(String),
    /// База записана более новой версией приложения: старый код не знает её схему.
    #[error("База от более новой версии приложения (шаг схемы {found}, известно {known})")]
    TooNew { found: u32, known: u32 },
    /// Шаг миграции не применился. Транзакция откатилась, база осталась на прежнем шаге.
    #[error("Миграция {version} «{name}» не применилась: {reason}")]
    Migration {
        version: u32,
        name: &'static str,
        reason: String,
    },
    #[error("Запись не найдена")]
    NotFound,
}

impl DbError {
    pub fn bad_value(table: &'static str, column: &'static str, source: ValidationError) -> Self {
        Self::BadValue { table, column, source }
    }
}
