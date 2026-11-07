//! Повторное считывание и пересборка снимка.

use super::{MissingSegment, ScanId, SegmentId};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::AppState;

pub struct ScanService<'a> {
    pub state: &'a AppState,
}

impl ScanService<'_> {
    /// Считывает игру из привязанной папки и переносит перевод на новую версию.
    /// Под `snapshot_lock`: чтение вне транзакции, запись снимка и переносов одной транзакцией, потом уборка `files/`.
    pub fn rescan(&self, project: ProjectId) -> Result<ScanId, AppError> {
        todo!()
    }

    /// Выросла `parser_version`: снимок пересобирается из `files/` последнего считывания, без папки игры.
    pub fn rebuild_from_files(&self, project: ProjectId) -> Result<ScanId, AppError> {
        todo!()
    }

    /// Пересобирает отставшие `edges` и `term_candidates` (`derived`) и cache.db. Идёт в фоне после открытия.
    pub fn refresh_derived(&self, project: ProjectId) -> Result<(), AppError> {
        todo!()
    }

    pub fn missing(&self, project: ProjectId) -> Result<Vec<MissingSegment>, AppError> {
        todo!()
    }

    /// Переносит перевод пропавшей реплики на текущую руками. Пишет строку `transfer`.
    pub fn attach_missing(&self, project: ProjectId, missing: SegmentId, target: SegmentId) -> Result<(), AppError> {
        todo!()
    }
}
