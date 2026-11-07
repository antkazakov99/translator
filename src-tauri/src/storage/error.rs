//! Ошибки работы с базой.

use thiserror::Error;

use crate::common::ValidationError;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Ошибка SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
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
    #[error("База от более новой версии приложения (схема {found}, известно {known})")]
    TooNew { found: u32, known: u32 },
    #[error("Запись не найдена")]
    NotFound,
}

impl DbError {
    pub fn bad_value(table: &'static str, column: &'static str, source: ValidationError) -> Self {
        Self::BadValue { table, column, source }
    }
}
