//! Перевод и редактура реплик: пакеты, промпт, разбор ответа, черновики, история.

pub mod answer;
pub mod batcher;
pub mod commands;
pub mod hints;
pub mod model;
pub mod prompt;
pub mod repo;
pub mod search;
pub mod service;
pub mod text;

pub use answer::{AnswerTerm, LinesAnswer};
pub use batcher::{Batch, Batcher};
pub use model::{Draft, FileProgress, FileScenes, Hints, LineRun, LineState, PlacedLine, RevisionKind, Scene, SceneLine, SceneProgress, SceneRef, SentState, TranslationScope};
pub use prompt::{LinesMode, LinesPrompt};
pub use repo::TranslationRepo;
pub use search::{SceneHit, SearchText, scene_hits};
pub use service::{LinesRun, TranslationService};
