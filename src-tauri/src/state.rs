//! Состояние приложения: app.db, ключи и открытые проекты.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use super::common::{AppError, Timestamp};
use super::files::FileStore;
use super::llm::{LlmRunRepo, ProviderRepo, RunSlot};
use super::projects::{Project, ProjectId, ProjectInfoRepo, ProjectRepo, ProjectRow, ProjectStatus};
use super::secrets::SecretStore;
use super::storage::{AppDb, CacheDb, DbError, ProjectDb};

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
    /// Открывает базы готового проекта. Запуски модели, оборванные закрытием приложения, становятся `failed`.
    pub fn open(id: ProjectId, dir: &Path) -> Result<Self, AppError> {
        let project = ProjectDb::open(dir)?;
        let files = FileStore::open(dir)?;
        let cache = CacheDb::open(dir)?;
        project.write(|tx| LlmRunRepo { conn: tx }.fail_stale(Timestamp::now()))?;
        Ok(Self { id, dir: dir.to_path_buf(), project, cache, files, snapshot_lock: Mutex::new(()) })
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
    const PROJECTS_DIR: &str = "projects";
    const APP_DB: &str = "app.db";

    /// Запуск приложения: app.db с миграциями, дочистка проектов `creating` и `deleting`,
    /// восстановление папок без строки, удаление ключей без провайдера.
    pub fn start(data_dir: PathBuf, secrets: SecretStore) -> Result<Self, AppError> {
        let app = AppDb::open(&data_dir.join(Self::APP_DB))?;
        let state = Self {
            data_dir,
            app,
            secrets,
            open_projects: Mutex::new(HashMap::new()),
            llm_run: Arc::new(RunSlot::default()),
        };
        state.finish_unfinished()?;
        state.restore_orphans()?;
        let providers = state.app.read(|tx| ProviderRepo { conn: tx }.list())?;
        let ids: Vec<_> = providers.iter().map(|provider| provider.id()).collect();
        state.secrets.delete_except(&ids);
        Ok(state)
    }

    /// `projects/<id>` в папке данных.
    pub fn project_dir(&self, id: ProjectId) -> PathBuf {
        self.projects_dir().join(id.to_string())
    }

    pub fn open_project(&self, id: ProjectId) -> Result<Arc<OpenProject>, AppError> {
        self.open_projects.lock().get(&id).cloned().ok_or(AppError::ProjectNotOpen)
    }

    fn projects_dir(&self) -> PathBuf {
        self.data_dir.join(Self::PROJECTS_DIR)
    }

    /// Создание и удаление, прерванные сбоем: сначала папка, потом строка.
    fn finish_unfinished(&self) -> Result<(), AppError> {
        let unfinished = self.app.read(|tx| ProjectRepo { conn: tx }.unfinished())?;
        for project in unfinished {
            remove_dir(&self.project_dir(project.id()))?;
            self.app.write(|tx| ProjectRepo { conn: tx }.delete(project.id()))?;
        }
        Ok(())
    }

    /// Папка без строки бывает только при потере app.db: проект возвращается в список без папки игры.
    /// Папку, где project.db не читается, не трогаем: это не наш проект или он повреждён.
    fn restore_orphans(&self) -> Result<(), AppError> {
        let entries = match std::fs::read_dir(self.projects_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(id) = ProjectId::parse(&name) else { continue };
            if !entry.path().is_dir() || self.app.read(|tx| ProjectRepo { conn: tx }.exists(id))? {
                continue;
            }
            match recover(&entry.path(), id) {
                Ok(project) => self.app.write(|tx| ProjectRepo { conn: tx }.save(&project))?,
                Err(error) => tracing::warn!("Папку проекта {name} не удалось вернуть в список: {error}"),
            }
        }
        Ok(())
    }
}

/// Проект по строке `project` его базы: готовый, без папки игры.
fn recover(dir: &Path, id: ProjectId) -> Result<Project, AppError> {
    let info = ProjectDb::open(dir)?.read(|tx| ProjectInfoRepo { conn: tx }.get())?;
    if info.id != id {
        return Err(DbError::Corrupt(format!("в базе проекта id {} вместо {id}", info.id)).into());
    }
    let row = ProjectRow {
        id: id.to_string(),
        status: ProjectStatus::Ready.as_str().to_string(),
        title: "Восстановленный проект".to_string(),
        engine: info.engine.as_str().to_string(),
        game_path: None,
        created_at: info.created_at.secs(),
        opened_at: None,
        archived: false,
    };
    Ok(Project::restore(row)?)
}

/// Удаляет папку целиком. Отсутствие папки не ошибка.
pub(crate) fn remove_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engines::EngineId;
    use crate::llm::{ApiKey, Provider, ProviderKind};
    use crate::projects::{GameRoot, Title};

    fn memory_secrets() -> SecretStore {
        SecretStore::Memory(Mutex::new(HashMap::new()))
    }

    /// Готовый проект с папкой и базами, как после создания.
    pub(crate) fn ready_project(state: &AppState, title: &str) -> Project {
        let root = GameRoot::new(&state.data_dir.join("games").join(title)).unwrap();
        let mut project = Project::create(Title::new(title).unwrap(), root, EngineId::Renpy, Timestamp::now());
        let dir = state.project_dir(project.id());
        FileStore::create(&dir).unwrap();
        ProjectDb::create(&dir, &project).unwrap();
        project.ready().unwrap();
        state.app.write(|tx| ProjectRepo { conn: tx }.save(&project)).unwrap();
        project
    }

    #[test]
    fn start_removes_unfinished_projects_with_their_folders() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let mut deleting = ready_project(&state, "deleting");
        deleting.mark_deleting();
        state.app.write(|tx| ProjectRepo { conn: tx }.save(&deleting)).unwrap();
        let kept = ready_project(&state, "kept");
        drop(state);

        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        assert!(!state.project_dir(deleting.id()).exists());
        assert!(matches!(state.app.read(|tx| ProjectRepo { conn: tx }.get(deleting.id())), Err(DbError::NotFound)));
        assert!(state.app.read(|tx| ProjectRepo { conn: tx }.get(kept.id())).is_ok());
    }

    #[test]
    fn start_restores_project_folders_without_rows() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let lost = ready_project(&state, "lost");
        std::fs::create_dir_all(state.projects_dir().join(ProjectId::generate().to_string())).unwrap();
        std::fs::create_dir_all(state.projects_dir().join("not-a-project")).unwrap();
        drop(state);
        std::fs::remove_file(dir.path().join(AppState::APP_DB)).unwrap();

        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let projects = state.app.read(|tx| ProjectRepo { conn: tx }.list()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id(), lost.id());
        assert_eq!(projects[0].engine(), EngineId::Renpy);
        assert!(!projects[0].is_bound());
    }

    #[test]
    fn start_forgets_keys_of_missing_providers() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let provider = Provider::new("Claude", ProviderKind::Claude, "https://api.anthropic.com", 0).unwrap();
        state.app.write(|tx| ProviderRepo { conn: tx }.save(&provider)).unwrap();
        let secrets = memory_secrets();
        let orphan = crate::llm::ProviderId::generate();
        secrets.set(orphan, ApiKey::new("ключ").unwrap());
        secrets.set(provider.id(), ApiKey::new("свой").unwrap());
        drop(state);

        let state = AppState::start(dir.path().to_path_buf(), secrets).unwrap();
        assert!(state.secrets.get(orphan).is_none());
        assert!(state.secrets.get(provider.id()).is_some());
    }

    #[test]
    fn opening_fails_stale_runs() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let project = ready_project(&state, "game");
        let path = state.project_dir(project.id());
        {
            let db = ProjectDb::open(&path).unwrap();
            db.write(|tx| LlmRunRepo { conn: tx }.start(crate::llm::Action::Lines, "local", "gemma", Timestamp::now())).unwrap();
        }
        let open = OpenProject::open(project.id(), &path).unwrap();
        let running: i64 = open
            .project
            .read(|tx| Ok(tx.query_row("SELECT count(*) FROM llm_runs WHERE status = 'running'", [], |row| row.get(0))?))
            .unwrap();
        assert_eq!(running, 0);
        assert!(path.join("cache.db").exists());
    }
}
