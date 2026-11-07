//! Проекты: список, создание, открытие, смена папки игры, удаление.

pub mod model;
pub mod repo;
pub mod service;

pub use model::{GameRoot, Project, ProjectId, ProjectRow, ProjectStatus, Title};
pub use repo::{ProjectInfo, ProjectInfoRepo, ProjectRepo};
pub use service::{ProjectService, RebindCheck};
