//! Повторное считывание и пересборка снимка.

use super::{EdgeRepo, MissingSegment, ScanId, ScanRepo, SegmentId};
use crate::engines;
use crate::projects::ProjectInfoRepo;
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

    /// Пересобирает отставшие производные таблицы. Пока это `edges`: `term_candidates` соберёт шаг глоссария,
    /// cache.db шаг поиска.
    pub fn refresh_derived(&self, project: ProjectId) -> Result<(), AppError> {
        self.refresh_edges(project)?;
        Ok(())
    }

    /// Пересобирает `edges`, если они собраны по прошлому считыванию или старой версией движка.
    /// Нужны только файлы снимка, папка игры не нужна. Возвращает, была ли пересборка.
    pub fn refresh_edges(&self, project: ProjectId) -> Result<bool, AppError> {
        let open = self.state.open_project(project)?;
        // Уборка files/ не удалит файлы, которые сейчас читаются.
        let _lock = open.snapshot_lock.lock();
        let (info, scan, state, files) = open.project.read(|tx| {
            let scans = ScanRepo { conn: tx };
            Ok((ProjectInfoRepo { conn: tx }.get()?, scans.latest()?, EdgeRepo { conn: tx }.state()?, scans.current_files()?))
        })?;
        let Some(scan) = scan else { return Ok(false) };
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        if state == Some((engine.edges_version(), scan.id)) {
            return Ok(false);
        }
        let edges = engine.read_edges(&files, &open.files)?;
        open.project.write(|tx| EdgeRepo { conn: tx }.replace(&edges, engine.edges_version(), scan.id))?;
        Ok(true)
    }

    pub fn missing(&self, project: ProjectId) -> Result<Vec<MissingSegment>, AppError> {
        todo!()
    }

    /// Переносит перевод пропавшей реплики на текущую руками. Пишет строку `transfer`.
    pub fn attach_missing(&self, project: ProjectId, missing: SegmentId, target: SegmentId) -> Result<(), AppError> {
        todo!()
    }
}
