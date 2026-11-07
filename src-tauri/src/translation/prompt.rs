//! Промпт перевода и редактуры реплик.

use super::{Batch, Hints, Scene};
use crate::llm::{ChatRequest, LlmTarget};

/// Перевод заполняет пустые реплики, редактура получает реплики с полем `draft` и возвращает исправленные.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinesMode {
    Translate,
    Review,
}

pub struct LinesPrompt;

impl LinesPrompt {
    /// Подписи говорящих, описание сцены, подсказки глоссария. Токены со значением уже заменены,
    /// реплики без букв вне токенов и тегов модели не отправляются. Лимит ответа включает запас на размышления.
    pub fn build(mode: LinesMode, scene: &Scene, batch: &Batch, hints: &Hints, target: &LlmTarget) -> ChatRequest {
        todo!()
    }
}
