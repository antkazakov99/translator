//! Ход запуска модели по пакетам: место запуска, прогресс, остановка и ошибка, которая обрывает запуск.
//! Им пользуются описания (`describe.rs`), предложения по записям глоссария (`suggest_service.rs`) и чтение сцены
//! внутри запуска перевода (`RunAsk`).

use std::sync::Arc;

use crate::common::AppError;
use crate::llm::{Action, ChatRequest, LlmClient, LlmError, LlmRun, LlmTarget, RunHost, RunProgress, RunStatus};
use crate::state::{AppState, OpenProject};

/// Сколько раз спрашивается пакет при сбое сети.
const ATTEMPTS: usize = 3;

/// Предложения запуска. Остановка и ошибка не теряют готового: `error` говорит, что запуск оборвался.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestions<T> {
    pub items: Vec<T>,
    pub error: Option<String>,
    /// Сколько записей запуск записал сам: цепочка автоперевода пишет после каждого пакета, окно не пишет.
    pub written: usize,
}

impl<T> Default for Suggestions<T> {
    fn default() -> Self {
        Self { items: Vec::new(), error: None, written: 0 }
    }
}

/// Ответ на запрос пакета.
pub(crate) enum Sent {
    Text(String),
    /// Модель отказалась отвечать: пакет пропускается.
    Skipped,
    /// Запуск остановлен или оборвался.
    Stop,
}

/// Ход запуска предложений по пакетам: место запуска, прогресс, ошибка, которая обрывает запуск.
pub(crate) struct BatchRun<'a> {
    run: LlmRun,
    target: &'a LlmTarget,
    report: RunProgress,
    error: Option<String>,
}

impl<'a> BatchRun<'a> {
    pub(crate) fn begin(
        state: &AppState,
        open: &Arc<OpenProject>,
        action: Action,
        target: &'a LlmTarget,
        host: impl Into<RunHost>,
        total: usize,
    ) -> Result<Self, AppError> {
        let run = LlmRun::begin(state, Some(open.clone()), action, target, host)?;
        let report = RunProgress { total: total as u32, ..RunProgress::default() };
        run.report(report.clone());
        Ok(Self { run, target, report, error: None })
    }

    /// Пакет: запрос, разбор в `items`, отчёт. `false`: запуск остановлен или оборвался, дальше не спрашивать.
    pub(crate) async fn ask<T>(&mut self, request: &ChatRequest, items: &mut Vec<T>, accept: impl FnOnce(&str) -> Vec<T>) -> bool {
        match self.send(request).await {
            Sent::Text(text) => {
                let accepted = accept(&text);
                if accepted.is_empty() {
                    tracing::warn!("Из ответа модели ничего не взято:\n{text}");
                }
                items.extend(accepted);
            }
            Sent::Skipped => {}
            Sent::Stop => return false,
        }
        self.step(items.len())
    }

    /// Запрос без отчёта. Ошибка запоминается и обрывает запуск.
    pub(crate) async fn send(&mut self, request: &ChatRequest) -> Sent {
        let target = self.target;
        self.send_to(target, request).await
    }

    /// Запрос к модели другого действия внутри этого запуска, например разбор сцены в запуске описаний.
    pub(crate) async fn send_to(&mut self, target: &LlmTarget, request: &ChatRequest) -> Sent {
        if self.run.cancelled() || self.error.is_some() {
            return Sent::Stop;
        }
        match ask(LlmClient::for_kind(target.provider.kind()), target, request, &self.run).await {
            Ok(Some(text)) => Sent::Text(text),
            Ok(None) => {
                tracing::warn!("Модель отказалась отвечать на пакет, он пропущен");
                Sent::Skipped
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Sent::Stop
            }
        }
    }

    /// Ошибка вне запроса к модели, например при записи ответа: запоминается и обрывает запуск.
    pub(crate) fn fail(&mut self, error: impl ToString) {
        self.error = Some(error.to_string());
    }

    /// Запуск узнал о запросах, которых не было в плане, например о сжатии заметок.
    pub(crate) fn grow(&mut self, more: usize) {
        self.report.total += more as u32;
        self.run.report(self.report.clone());
    }

    /// Что модель делает сейчас: строка хода запуска для окна.
    pub(crate) fn note(&mut self, message: String) {
        self.report.message = Some(message);
        self.run.report(self.report.clone());
    }

    /// Пакет готов: отчёт с числом предложений. `false`: запуск остановлен.
    pub(crate) fn step(&mut self, written: usize) -> bool {
        self.report.done += 1;
        self.report.written = written as u32;
        self.run.report(self.report.clone());
        !self.run.cancelled()
    }

    /// Итог в `llm_runs`. Готовое не теряется и при ошибке.
    pub(crate) fn finish<T>(self, items: Vec<T>) -> Result<Suggestions<T>, AppError> {
        let status = match &self.error {
            Some(_) => RunStatus::Failed,
            None if self.run.cancelled() => RunStatus::Cancelled,
            None => RunStatus::Done,
        };
        self.run.finish(status, self.error.as_deref())?;
        Ok(Suggestions { items, error: self.error, written: 0 })
    }
}

/// Запросы запуска, в котором читаются сцены: свой запуск описаний или чужой запуск, например перевода.
pub(crate) trait Ask {
    /// Запрос без отчёта. Ошибка запоминается и обрывает запуск.
    async fn send(&mut self, request: &ChatRequest) -> Sent;
    /// Запрос готов. `false`: запуск остановлен.
    fn step(&mut self, written: usize) -> bool;
    /// Запросов больше, чем было в плане.
    fn grow(&mut self, more: usize);
    /// Что модель делает сейчас: строка хода запуска для окна.
    fn note(&mut self, message: String);
}

impl Ask for BatchRun<'_> {
    async fn send(&mut self, request: &ChatRequest) -> Sent {
        BatchRun::send(self, request).await
    }

    fn step(&mut self, written: usize) -> bool {
        BatchRun::step(self, written)
    }

    fn grow(&mut self, more: usize) {
        BatchRun::grow(self, more);
    }

    fn note(&mut self, message: String) {
        BatchRun::note(self, message);
    }
}

/// Запросы чтения сцены внутри запуска перевода: прогресс общий, ошибка запоминается для запуска.
pub(crate) struct RunAsk<'a> {
    pub run: &'a LlmRun,
    pub target: &'a LlmTarget,
    pub progress: &'a mut RunProgress,
    pub error: Option<LlmError>,
}

impl Ask for RunAsk<'_> {
    async fn send(&mut self, request: &ChatRequest) -> Sent {
        if self.run.cancelled() || self.error.is_some() {
            return Sent::Stop;
        }
        match ask(LlmClient::for_kind(self.target.provider.kind()), self.target, request, self.run).await {
            Ok(Some(text)) => Sent::Text(text),
            Ok(None) => Sent::Skipped,
            Err(error) => {
                self.error = Some(error);
                Sent::Stop
            }
        }
    }

    fn step(&mut self, _written: usize) -> bool {
        self.progress.done += 1;
        self.run.report(self.progress.clone());
        !self.run.cancelled()
    }

    fn grow(&mut self, more: usize) {
        self.progress.total += more as u32;
        self.run.report(self.progress.clone());
    }

    fn note(&mut self, message: String) {
        self.progress.message = Some(message);
        self.run.report(self.progress.clone());
    }
}

/// Один запрос с повтором при сбое сети. `None`: модель отказалась отвечать.
async fn ask(client: LlmClient, target: &LlmTarget, request: &ChatRequest, run: &LlmRun) -> Result<Option<String>, LlmError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match client.send(target, request).await {
            Ok(text) => return Ok(Some(text)),
            Err(LlmError::Refused) => return Ok(None),
            Err(error) if error.is_transient() && attempt < ATTEMPTS && !run.cancelled() => {
                tracing::warn!("Запрос к модели не удался, повтор: {error}");
            }
            Err(error) => return Err(error),
        }
    }
}

