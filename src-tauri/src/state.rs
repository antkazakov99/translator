//! Состояние приложения: app.db, ключи и открытые проекты.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use super::common::AppError;
use super::files::FileStore;
use super::llm::RunSlot;
use super::projects::ProjectId;
use super::secrets::SecretStore;
use super::storage::{AppDb, CacheDb, ProjectDb};

/// Базы и файлы одного открытого проекта. Один на проект: два соединения на запись к одному файлу ждали бы друг друга.
/// Базы закрываются в `Drop` последнего `Arc`, поэтому запуск модели, начатый раньше закрытия, дописывает своё.
pub struct OpenProject {
    pub id: ProjectId,
    pub dir: PathBuf,
    pub project: ProjectDb,
    pub cache: CacheDb,
    pub files: FileStore,
    /// Считывание, замена шрифтов и уборка `files/` не идут одновременно.
    pub snapshot_lock: Mutex<()>,
}

impl OpenProject {
    /// Открывает базы готового проекта. При открытии `running` в `llm_runs` становится `failed`.
    pub fn open(id: ProjectId, dir: &Path) -> Result<Self, AppError> {
        todo!()
    }
}

/// Состояние Tauri.
pub struct AppState {
    /// Папка данных из `app_local_data_dir`: `identifier` из `tauri.conf.json`, не `TranslatorApp` 1.0.
    pub data_dir: PathBuf,
    pub app: AppDb,
    pub secrets: SecretStore,
    pub open_projects: Mutex<HashMap<ProjectId, Arc<OpenProject>>>,
    /// Текущий запуск модели: один за раз во всём приложении, его занимает `LlmRun`.
    pub llm_run: Arc<RunSlot>,
}

impl AppState {
    /// Запуск приложения: app.db с миграциями, дочистка проектов `creating` и `deleting`,
    /// восстановление папок без строки, удаление ключей без провайдера.
    pub fn start(data_dir: PathBuf) -> Result<Self, AppError> {
        todo!()
    }

    /// `projects/<id>` в папке данных.
    pub fn project_dir(&self, id: ProjectId) -> PathBuf {
        self.data_dir.join("projects").join(id.to_string())
    }

    pub fn open_project(&self, id: ProjectId) -> Result<Arc<OpenProject>, AppError> {
        self.open_projects.lock().get(&id).cloned().ok_or(AppError::ProjectNotOpen)
    }
}
