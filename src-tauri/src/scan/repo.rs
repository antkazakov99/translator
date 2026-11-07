//! Снимок игры в project.db: `scans`, `files`, `speakers`, `containers`, `segments`.

use rusqlite::Connection;

use super::{MatchPlan, MatchSegment, MissingSegment, Scan, ScanId, SegmentId};
use crate::engines::{GameSnapshot, SnapshotFile};
use crate::projects::GameRoot;
use crate::storage::DbError;

pub struct ScanRepo<'c> {
    pub conn: &'c Connection,
}

impl ScanRepo<'_> {
    /// Пишет первый снимок: строка `scans` и все записи с её `last_seen_scan`.
    pub fn save(&self, snapshot: &GameSnapshot, root: Option<&GameRoot>, parser_version: u32) -> Result<ScanId, DbError> {
        todo!()
    }

    /// Последнее считывание.
    pub fn latest(&self) -> Result<Option<Scan>, DbError> {
        todo!()
    }

    /// Файлы текущего снимка: для пересборки из `files/` и сборки мода.
    pub fn current_files(&self) -> Result<Vec<SnapshotFile>, DbError> {
        todo!()
    }

    /// Текущие реплики для сопоставления.
    pub fn current_segments(&self) -> Result<Vec<MatchSegment<SegmentId>>, DbError> {
        todo!()
    }

    /// Повторное считывание: новые строки, перенос `translations` и `revisions` по плану, строки `transfer`,
    /// `missing = 1` у старых, перенос ключей описаний сцен, удаление лишнего. Поднимает `project.search_rev`.
    pub fn apply(&self, snapshot: &GameSnapshot, plan: &MatchPlan, root: Option<&GameRoot>, parser_version: u32) -> Result<ScanId, DbError> {
        todo!()
    }

    /// «Пропавшие».
    pub fn missing(&self) -> Result<Vec<MissingSegment>, DbError> {
        todo!()
    }

    /// Ключи реплик текущего снимка: для проверки «та ли это игра» при смене папки.
    pub fn segment_keys(&self) -> Result<Vec<String>, DbError> {
        todo!()
    }
}
