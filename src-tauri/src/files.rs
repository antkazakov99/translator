//! Папка `files/` проекта: содержимое файлов игры и загруженных шрифтов по хэшу.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::ValidationError;

/// SHA-256 содержимого: 64 строчные шестнадцатеричные цифры. Он же имя файла в `files/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        let valid = text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid {
            return Err(ValidationError::UnknownValue { field: "hash", value: text.to_string() });
        }
        Ok(Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Записанный файл: по хэшу его находят, размер идёт в `files.size`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub hash: ContentHash,
    pub size: u64,
}

/// Файлы по хэшу содержимого. Чьи это файлы, хранилище не знает.
///
/// - Файлы не меняются, только добавляются и удаляются.
/// - Запись идёт во временный файл и переименование: сбой оставит лишний файл, но не битый.
/// - Сначала файл, потом строка в базе. Уборка идёт под `snapshot_lock` после фиксации транзакции.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    /// Создаёт папку `files/` в папке проекта.
    pub fn create(project_dir: &Path) -> std::io::Result<Self> {
        todo!()
    }

    pub fn open(project_dir: &Path) -> std::io::Result<Self> {
        todo!()
    }

    /// Записывает поток, считая хэш на ходу: архивы игр весят гигабайты, целиком в память они не читаются.
    /// Одинаковое содержимое уже лежит, тогда временный файл просто удаляется.
    pub fn put(&self, content: impl Read) -> std::io::Result<StoredBlob> {
        todo!()
    }

    pub fn open_file(&self, hash: &ContentHash) -> std::io::Result<std::fs::File> {
        todo!()
    }

    pub fn read(&self, hash: &ContentHash) -> std::io::Result<Vec<u8>> {
        todo!()
    }

    pub fn path_of(&self, hash: &ContentHash) -> PathBuf {
        self.dir.join(hash.as_str())
    }

    /// Уборка: удаляет файлы, которых нет в `keep` (ссылки из `files` и `font_replacements`), и временные.
    /// Возвращает число удалённых.
    pub fn retain(&self, keep: &HashSet<ContentHash>) -> std::io::Result<usize> {
        todo!()
    }
}
