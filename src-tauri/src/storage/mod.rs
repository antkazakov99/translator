//! Базы SQLite: соединения, транзакции и миграции. SQL над таблицами лежит в репозиториях фич.

pub mod database;
pub mod error;

pub use database::{AppDb, CacheDb, Database, ProjectDb};
pub use error::DbError;
