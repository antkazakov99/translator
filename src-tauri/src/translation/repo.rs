//! `translations` и `revisions` в project.db. Каждая запись пишет историю и поднимает `project.search_rev`.

use rusqlite::Connection;

use super::{Draft, Scene};
use crate::common::Timestamp;
use crate::engines::ContainerKey;
use crate::llm::RunId;
use crate::scan::{FileId, SegmentId};
use crate::storage::DbError;

pub struct TranslationRepo<'c> {
    pub conn: &'c Connection,
}

impl TranslationRepo<'_> {
    /// Реплики сцены текущего снимка в порядке сюжета с подписями из глоссария.
    pub fn scene(&self, key: &ContainerKey) -> Result<Scene, DbError> {
        todo!()
    }

    /// Реплики файла, разбитые на сцены.
    pub fn file_scenes(&self, file: FileId) -> Result<Vec<Scene>, DbError> {
        todo!()
    }

    /// Сцена одной реплики: её соседи нужны как контекст.
    pub fn scene_of(&self, line: SegmentId) -> Result<Scene, DbError> {
        todo!()
    }

    /// Черновики перевода только в пустые реплики с тем же `source_hash`, что ушёл модели. Возвращает число записанных.
    pub fn save_drafts(&self, drafts: &[Draft], run: RunId, now: Timestamp) -> Result<usize, DbError> {
        todo!()
    }

    /// Исправления редактуры: заменяют и заполненные реплики, если текст и `source_hash` те же, что ушли модели.
    pub fn replace_with_drafts(&self, drafts: &[Draft], run: RunId, now: Timestamp) -> Result<usize, DbError> {
        todo!()
    }

    /// Правка пользователя: снимает `draft`. Пустой текст значит «очищено», запуск переведёт заново.
    pub fn save_user_text(&self, line: SegmentId, text: &str, now: Timestamp) -> Result<(), DbError> {
        todo!()
    }

    /// Подтверждение черновика.
    pub fn confirm(&self, lines: &[SegmentId], now: Timestamp) -> Result<usize, DbError> {
        todo!()
    }

    /// Откат запуска: только реплики, где последняя правка без переносов принадлежит запуску.
    /// Возвращает число откаченных и число оставленных.
    pub fn rollback_run(&self, run: RunId, now: Timestamp) -> Result<(usize, usize), DbError> {
        todo!()
    }
}
