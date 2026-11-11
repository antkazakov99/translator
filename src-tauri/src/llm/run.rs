//! Запуск модели: один за раз во всём приложении, остановка, прогресс, строка `llm_runs`.
//! Цепочка запусков («Вся игра») занимает место один раз, её шаги идут обычными запусками внутри неё.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
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

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Running, Self::Done, Self::Cancelled, Self::Failed].into_iter().find(|status| status.as_str() == text)
    }
}

/// Прогресс запуска в окно: сколько пакетов готово из скольких и что сейчас идёт.
/// Сообщение канала может прийти позже итога команды.
#[derive(Debug, Clone, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunProgress {
    pub done: u32,
    pub total: u32,
    /// Реплик записано за запуск.
    pub written: u32,
    pub message: Option<String>,
    /// Шаг цепочки «Вся игра»: есть только у запуска внутри цепочки.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub stage: Option<RunStage>,
}

/// Шаг цепочки в ходе запуска: «Шаг 2 из 4 · Перевод терминов».
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunStage {
    pub index: u32,
    pub count: u32,
    pub title: String,
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

/// Цепочка запусков: место занято на всю цепочку, флаг остановки и канал хода общие для её шагов.
/// Шаги идут обычными `LlmRun` со своими строками `llm_runs`. Место освобождается с последней копией цепочки.
#[derive(Clone)]
pub struct RunChain(Arc<ChainInner>);

struct ChainInner {
    slot: Arc<RunSlot>,
    cancel: Arc<AtomicBool>,
    progress: Channel<RunProgress>,
    stage: Mutex<Option<RunStage>>,
}

impl Drop for ChainInner {
    fn drop(&mut self) {
        self.slot.release();
    }
}

impl RunChain {
    /// Занимает место. Второй запуск получает `RunActive`.
    pub fn begin(state: &AppState, progress: Channel<RunProgress>) -> Result<Self, AppError> {
        let cancel = state.llm_run.take()?;
        Ok(Self(Arc::new(ChainInner { slot: state.llm_run.clone(), cancel, progress, stage: Mutex::new(None) })))
    }

    /// Шаг `index` из `count`: его несут все отчёты запусков дальше. Окно сразу узнаёт о новом шаге.
    pub fn stage(&self, index: usize, count: usize, title: &str) {
        let stage = RunStage { index: index as u32, count: count as u32, title: title.to_string() };
        *self.0.stage.lock() = Some(stage.clone());
        let _ = self.0.progress.send(RunProgress { stage: Some(stage), ..RunProgress::default() });
    }

    pub fn cancelled(&self) -> bool {
        self.0.cancel.load(Ordering::Relaxed)
    }
}

/// Где идёт запуск: сам по себе со своим местом или шагом цепочки, которая место уже заняла.
pub enum RunHost {
    Own(Channel<RunProgress>),
    Chain(RunChain),
}

impl From<Channel<RunProgress>> for RunHost {
    fn from(progress: Channel<RunProgress>) -> Self {
        Self::Own(progress)
    }
}

impl From<&RunChain> for RunHost {
    fn from(chain: &RunChain) -> Self {
        Self::Chain(chain.clone())
    }
}

/// Идущий запуск. Ведёт его сервис действия: `LlmRun` не знает шагов операции и не держит транзакцию во время ответа.
pub struct LlmRun {
    /// Своё место запуска. У шага цепочки `None`: место держит цепочка.
    slot: Option<Arc<RunSlot>>,
    chain: Option<RunChain>,
    cancel: Arc<AtomicBool>,
    progress: Channel<RunProgress>,
    project: Option<Arc<OpenProject>>,
    id: Option<RunId>,
    finished: bool,
}

impl LlmRun {
    /// Занимает место (у шага цепочки оно уже занято) и пишет строку `running` в `llm_runs` проекта.
    /// Действия без проекта (настройки приложения) строки не пишут.
    pub fn begin(
        state: &AppState,
        project: Option<Arc<OpenProject>>,
        action: Action,
        target: &LlmTarget,
        host: impl Into<RunHost>,
    ) -> Result<LlmRun, AppError> {
        let (slot, chain, cancel, progress) = match host.into() {
            RunHost::Own(progress) => {
                let cancel = state.llm_run.take()?;
                (Some(state.llm_run.clone()), None, cancel, progress)
            }
            RunHost::Chain(chain) => {
                let (cancel, progress) = (chain.0.cancel.clone(), chain.0.progress.clone());
                (None, Some(chain), cancel, progress)
            }
        };
        let id = match &project {
            Some(open) => {
                let started = open.project.write(|tx| {
                    LlmRunRepo { conn: tx }.start(action, target.provider.name(), target.profile.model(), Timestamp::now())
                });
                match started {
                    Ok(id) => Some(id),
                    Err(error) => {
                        if let Some(slot) = &slot {
                            slot.release();
                        }
                        return Err(error.into());
                    }
                }
            }
            None => None,
        };
        Ok(LlmRun { slot, chain, cancel, progress, project, id, finished: false })
    }

    pub fn id(&self) -> Option<RunId> {
        self.id
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Ошибку канала не считает ошибкой запуска: окно могли перезагрузить. Шаг цепочки добавляет к ходу шаг.
    pub fn report(&self, mut progress: RunProgress) {
        if let Some(chain) = &self.chain {
            progress.stage = chain.0.stage.lock().clone();
        }
        let _ = self.progress.send(progress);
    }

    /// Пишет итог и освобождает своё место.
    pub fn finish(mut self, status: RunStatus, error: Option<&str>) -> Result<(), AppError> {
        self.finished = true;
        self.record(status, error)
    }

    fn record(&self, status: RunStatus, error: Option<&str>) -> Result<(), AppError> {
        if let (Some(open), Some(id)) = (&self.project, self.id) {
            open.project.write(|tx| LlmRunRepo { conn: tx }.finish(id, status, error, Timestamp::now()))?;
        }
        Ok(())
    }
}

impl Drop for LlmRun {
    /// Запуск без `finish` оборвался: строка получает `failed`, своё место освобождается.
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.record(RunStatus::Failed, None);
        }
        if let Some(slot) = &self.slot {
            slot.release();
        }
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
