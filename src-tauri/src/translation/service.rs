//! Порядок шагов перевода и редактуры. Ответ модели ждётся вне транзакции, каждый пакет пишется короткой.

use tauri::ipc::Channel;

use super::TranslationScope;
use crate::common::AppError;
use crate::llm::{ProfileId, RunId, RunProgress};
use crate::projects::ProjectId;
use crate::scan::SegmentId;
use crate::state::AppState;

pub struct TranslationService<'a> {
    pub state: &'a AppState,
}

impl TranslationService<'_> {
    /// Заполняет пустые реплики. Сцена без описания сначала читается целиком. Возвращает число переведённых.
    pub async fn translate(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        todo!()
    }

    /// Вычитывает готовые реплики, исправления становятся черновиками. Возвращает число исправленных.
    pub async fn review(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        todo!()
    }

    /// Сброс перевода: пустые правки, как ручная очистка строки.
    pub fn reset(&self, project: ProjectId, scope: TranslationScope) -> Result<usize, AppError> {
        todo!()
    }

    pub fn save_text(&self, project: ProjectId, line: SegmentId, text: &str) -> Result<(), AppError> {
        todo!()
    }

    pub fn confirm(&self, project: ProjectId, lines: &[SegmentId]) -> Result<usize, AppError> {
        todo!()
    }

    /// Откат запуска модели. Возвращает число откаченных и оставленных реплик.
    pub fn rollback(&self, project: ProjectId, run: RunId) -> Result<(usize, usize), AppError> {
        todo!()
    }
}
