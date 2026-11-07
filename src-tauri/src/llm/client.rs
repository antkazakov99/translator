//! HTTP к моделям: `ChatRequest` в запрос нужного протокола и ответ обратно в текст.

use thiserror::Error;

use super::{ChatRequest, LlmTarget, ProviderKind};

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("Нет ключа API для «{0}»")]
    NoKey(String),
    #[error("Сервер не ответил: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Сервер вернул {status}: {body}")]
    Status { status: u16, body: String },
    #[error("Пустой ответ модели")]
    Empty,
    #[error("Ответ модели обрезан лимитом")]
    Truncated,
}

/// Протокол запроса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmClient {
    /// Claude Messages API.
    Messages,
    /// OpenAI-совместимый Chat Completions: ChatGPT, Gemini, локальные и совместимые серверы.
    ChatCompletions,
}

impl LlmClient {
    pub fn for_kind(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Claude => Self::Messages,
            ProviderKind::Local | ProviderKind::OpenAi | ProviderKind::Gemini | ProviderKind::Compatible => Self::ChatCompletions,
        }
    }

    /// Лимит ответа включает запас профиля на размышления, температуру не получает ChatGPT,
    /// поля `params` профиля добавляются в тело, локальный сервер идёт без прокси.
    pub async fn send(self, target: &LlmTarget, request: &ChatRequest) -> Result<String, LlmError> {
        todo!()
    }
}
