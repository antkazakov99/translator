//! Модели: провайдеры, профили, настройки действий, цель запроса, HTTP и запуск.

pub mod action;
pub mod client;
pub mod model;
pub mod repo;
pub mod run;
pub mod service;
pub mod target;

pub use action::{Action, ActionSettings, BatchSize, ResolvedSettings, Temperature};
pub use client::{LlmClient, LlmError};
pub use model::{Params, Profile, ProfileId, Provider, ProviderId, ProviderKind};
pub use repo::{ActionRepo, ProfileRepo, ProviderRepo};
pub use run::{LlmRun, LlmRunRepo, RunId, RunProgress, RunSlot, RunStatus};
pub use service::LlmSettingsService;
pub use target::{ApiKey, ChatRequest, LlmTarget, LlmTargetService, ProfileChoice};
