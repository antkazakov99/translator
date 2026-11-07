//! Перевод и редактура реплик: пакеты, промпт, разбор ответа, черновики, история.

pub mod answer;
pub mod batcher;
pub mod model;
pub mod prompt;
pub mod repo;
pub mod service;

pub use answer::LinesAnswer;
pub use batcher::{Batch, Batcher};
pub use model::{Draft, Hints, LineState, RevisionKind, Scene, SceneLine, SentState, TranslationScope};
pub use prompt::{LinesMode, LinesPrompt};
pub use repo::TranslationRepo;
pub use service::TranslationService;
