//! Запуск модели: один за раз во всём приложении, остановка, прогресс, строка `llm_runs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use rusqlite::Connection;
use serde::Serialize;
use tauri::ipc::Channel;

use super::{Action, LlmTarget};
use crate::common::{AppError, Timestamp, row_id};
use crate::state::{AppState, OpenProject};
use crate::storage::DbError;

row_id!(
    /// Строка `llm_runs`.
    RunId
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Done,
    Cancelled,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// Прогресс запуска в окно. Итог пакета может прийти позже итога команды.
#[derive(Debug, Clone, Serialize)]
pub struct RunProgress {
    pub done: usize,
    pub total: usize,
    pub message: Option<String>,
}

/// Место для одного запуска во всём приложении. Флаг остановки общий на все запросы запуска.
#[derive(Default)]
pub struct RunSlot(Mutex<Option<Arc<AtomicBool>>>);

impl RunSlot {
    /// Занимает место. Второй запуск получает `RunActive`.
    fn take(&self) -> Result<Arc<AtomicBool>, AppError> {
        let mut slot = self.0.lock();
        if slot.is_some() {
            return Err(AppError::RunActive);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(cancel.clone());
        Ok(cancel)
    }

    /// Останавливает текущий запуск между пакетами.
    pub fn cancel(&self) {
        if let Some(flag) = self.0.lock().as_ref() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn release(&self) {
        *self.0.lock() = None;
    }

    pub fn is_active(&self) -> bool {
        self.0.lock().is_some()
    }
}

/// Идущий запуск. Ведёт его сервис действия: `LlmRun` не знает шагов операции и не держит транзакцию во время ответа.
pub struct LlmRun {
    slot: Arc<RunSlot>,
    cancel: Arc<AtomicBool>,
    progress: Channel<RunProgress>,
    project: Option<Arc<OpenProject>>,
    id: Option<RunId>,
    finished: bool,
}

impl LlmRun {
    /// Занимает место и пишет строку `running` в `llm_runs` проекта.
    /// Действия без проекта (настройки приложения) строки не пишут.
    pub fn begin(
        state: &AppState,
        project: Option<Arc<OpenProject>>,
        action: Action,
        target: &LlmTarget,
        progress: Channel<RunProgress>,
    ) -> Result<LlmRun, AppError> {
        todo!()
    }

    pub fn id(&self) -> Option<RunId> {
        self.id
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Ошибку канала не считает ошибкой запуска: окно могли перезагрузить.
    pub fn report(&self, progress: RunProgress) {
        let _ = self.progress.send(progress);
    }

    /// Пишет итог и освобождает место.
    pub fn finish(mut self, status: RunStatus, error: Option<&str>) -> Result<(), AppError> {
        self.finished = true;
        self.record(status, error)
    }

    fn record(&self, status: RunStatus, error: Option<&str>) -> Result<(), AppError> {
        todo!()
    }
}

impl Drop for LlmRun {
    /// Запуск без `finish` оборвался: строка получает `failed`, место освобождается.
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.record(RunStatus::Failed, None);
        }
        self.slot.release();
    }
}

/// Таблица `llm_runs` в project.db.
pub struct LlmRunRepo<'c> {
    pub conn: &'c Connection,
}

impl LlmRunRepo<'_> {
    /// Имена провайдера и модели строкой: профиль могут удалить, а запись останется понятной.
    pub fn start(&self, action: Action, provider: &str, model: &str, now: Timestamp) -> Result<RunId, DbError> {
        todo!()
    }

    pub fn finish(&self, id: RunId, status: RunStatus, error: Option<&str>, now: Timestamp) -> Result<(), DbError> {
        todo!()
    }

    /// При открытии проекта: `running` после аварийного закрытия становится `failed`.
    pub fn fail_stale(&self) -> Result<usize, DbError> {
        todo!()
    }
}
