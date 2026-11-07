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
        self.conn.execute(
            "INSERT INTO llm_runs (action_id, provider, model, status, started_at) VALUES (?1, ?2, ?3, 'running', ?4)",
            rusqlite::params![action.id(), provider, model, now.secs()],
        )?;
        Ok(RunId::from_row(self.conn.last_insert_rowid()))
    }

    pub fn finish(&self, id: RunId, status: RunStatus, error: Option<&str>, now: Timestamp) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE llm_runs SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1",
            rusqlite::params![id.get(), status.as_str(), error, now.secs()],
        )?;
        Ok(())
    }

    /// При открытии проекта: `running` после аварийного закрытия становится `failed`.
    pub fn fail_stale(&self, now: Timestamp) -> Result<usize, DbError> {
        Ok(self.conn.execute(
            "UPDATE llm_runs SET status = 'failed', error = 'Приложение закрылось во время запуска', finished_at = ?1
             WHERE status = 'running'",
            [now.secs()],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    #[test]
    fn stale_runs_become_failed_and_new_can_start() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("runs.db")).unwrap();
        db.write(|tx| Ok(tx.execute_batch(include_str!("../storage/schema/project/0001_init.sql"))?)).unwrap();

        let first = db.write(|tx| LlmRunRepo { conn: tx }.start(Action::Lines, "LM Studio", "gemma", Timestamp::from_secs(1))).unwrap();
        let second = db.write(|tx| LlmRunRepo { conn: tx }.start(Action::Lines, "LM Studio", "gemma", Timestamp::from_secs(2)));
        assert!(second.is_err(), "два running в одной базе не пишутся");

        assert_eq!(db.write(|tx| LlmRunRepo { conn: tx }.fail_stale(Timestamp::from_secs(3))).unwrap(), 1);
        let status: String = db
            .read(|tx| Ok(tx.query_row("SELECT status FROM llm_runs WHERE id = ?1", [first.get()], |row| row.get(0))?))
            .unwrap();
        assert_eq!(status, "failed");

        let next = db.write(|tx| LlmRunRepo { conn: tx }.start(Action::Review, "Claude", "sonnet", Timestamp::from_secs(4))).unwrap();
        db.write(|tx| LlmRunRepo { conn: tx }.finish(next, RunStatus::Done, None, Timestamp::from_secs(5))).unwrap();
        assert_eq!(db.write(|tx| LlmRunRepo { conn: tx }.fail_stale(Timestamp::from_secs(6))).unwrap(), 0);
    }
}
