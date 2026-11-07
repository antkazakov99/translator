//! Шрифты: шрифты игры из снимка, замены для языка перевода, проверка символов.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::common::{AppError, Timestamp};
use crate::files::ContentHash;
use crate::projects::ProjectId;
use crate::state::AppState;
use crate::storage::DbError;

/// Замена шрифта игры (`font_replacements`). Сам шрифт лежит в `files/` и переезжает с папкой проекта.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FontReplacement {
    /// Путь шрифта в игре, который заменяется.
    pub target_path: String,
    /// Имя загруженного файла, для показа.
    pub file_name: String,
    pub hash: ContentHash,
    pub added_at: Timestamp,
}

/// Шрифт игры из снимка.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameFont {
    pub path: String,
    pub hash: ContentHash,
    /// Символы перевода, которых в шрифте нет.
    pub missing_chars: Vec<char>,
    pub replacement: Option<FontReplacement>,
}

/// Таблица `cmap` шрифта: какие символы в нём есть.
#[derive(Debug, Clone, Default)]
pub struct Cmap {
    chars: BTreeSet<char>,
}

impl Cmap {
    /// TrueType и OpenType. Шрифты игры недоверенные: битый файл даёт ошибку, а не панику.
    pub fn parse(font: &[u8]) -> Result<Self, String> {
        todo!()
    }

    /// Символы текста без пробелов, которых нет в шрифте.
    pub fn missing(&self, text: &str) -> Vec<char> {
        let mut missing: Vec<char> = text
            .chars()
            .filter(|ch| !ch.is_whitespace() && !self.chars.contains(ch))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        missing.sort_unstable();
        missing
    }
}

pub struct FontRepo<'c> {
    pub conn: &'c Connection,
}

impl FontRepo<'_> {
    pub fn replacements(&self) -> Result<Vec<FontReplacement>, DbError> {
        todo!()
    }

    pub fn save(&self, replacement: &FontReplacement) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, target_path: &str) -> Result<(), DbError> {
        todo!()
    }
}

pub struct FontService<'a> {
    pub state: &'a AppState,
}

impl FontService<'_> {
    /// Шрифты игры с недостающими символами перевода и заменами. Без папки игры.
    pub fn list(&self, project: ProjectId) -> Result<Vec<GameFont>, AppError> {
        todo!()
    }

    /// Под `snapshot_lock`: файл в `files/`, потом строка в базе.
    pub fn replace(&self, project: ProjectId, target_path: &str, font: &Path) -> Result<FontReplacement, AppError> {
        todo!()
    }

    /// Убирает замену. Файл удалит следующая уборка `files/`.
    pub fn remove(&self, project: ProjectId, target_path: &str) -> Result<(), AppError> {
        todo!()
    }
}
