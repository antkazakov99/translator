//! Автоперевод: шаги модели по всей игре одной очередью внутри одного места запуска.

pub mod commands;
pub mod model;
pub mod service;

pub use model::{GameRun, GameStep, NamesMode, StepResult, StepWork, WorkCount, ordered};
pub use service::GameRunService;
