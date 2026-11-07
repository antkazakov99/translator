//! Команды Tauri для списка проектов. Правил здесь нет: только вызов сервиса вне потока окна и текст ошибки.

use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

use super::{GameRoot, Project, ProjectId, ProjectService, Title};
use crate::common::AppError;
use crate::state::blocking;

/// Проект для окна.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub title: String,
    pub engine: String,
    /// Корневая папка игры. `None`: проект отвязан от игры.
    pub game_root: Option<String>,
    /// Папка привязана, но её нет на диске: установка мода недоступна.
    pub game_missing: bool,
    #[specta(type = specta_typescript::Number)]
    pub created_at: i64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub opened_at: Option<i64>,
    pub archived: bool,
}

impl ProjectDto {
    /// Читает диск: есть ли папка игры. Поэтому собирается в потоке команды, а не в модели.
    fn of(project: &Project) -> Self {
        let root = project.game_root().map(GameRoot::as_path);
        Self {
            id: project.id().to_string(),
            title: project.title().as_str().to_string(),
            engine: project.engine().as_str().to_string(),
            game_root: root.map(|root| root.to_string_lossy().into_owned()),
            game_missing: root.is_some_and(|root| !root.is_dir()),
            created_at: project.created_at().secs(),
            opened_at: project.opened_at().map(|at| at.secs()),
            archived: project.is_archived(),
        }
    }
}

fn id(text: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(text)?)
}

/// Готовые проекты: основной список, архивные в конце.
#[tauri::command]
#[specta::specta]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectDto>, String> {
    blocking(app, |state| Ok(ProjectService { state }.list()?.iter().map(ProjectDto::of).collect())).await
}

/// Проекты, уже привязанные к папке: окно предлагает открыть их вместо нового.
#[tauri::command]
#[specta::specta]
pub async fn find_projects(app: AppHandle, root: String) -> Result<Vec<ProjectDto>, String> {
    blocking(app, move |state| {
        let root = ProjectService::resolve_root(Path::new(&root))?;
        Ok(ProjectService { state }.find_by_root(&root)?.iter().map(ProjectDto::of).collect())
    })
    .await
}

/// Создаёт проект по корневой папке игры: считывает её целиком.
#[tauri::command]
#[specta::specta]
pub async fn create_project(app: AppHandle, root: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| {
        let root = ProjectService::resolve_root(Path::new(&root))?;
        Ok(ProjectDto::of(&ProjectService { state }.create(root)?))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn open_project(app: AppHandle, id: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| Ok(ProjectDto::of(&ProjectService { state }.open(self::id(&id)?)?))).await
}

#[tauri::command]
#[specta::specta]
pub async fn close_project(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| {
        ProjectService { state }.close(self::id(&id)?);
        Ok(())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn archive_project(app: AppHandle, id: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| Ok(ProjectDto::of(&ProjectService { state }.archive(self::id(&id)?)?))).await
}

#[tauri::command]
#[specta::specta]
pub async fn unarchive_project(app: AppHandle, id: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| Ok(ProjectDto::of(&ProjectService { state }.unarchive(self::id(&id)?)?))).await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_project(app: AppHandle, id: String, title: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| {
        let title = Title::new(&title)?;
        Ok(ProjectDto::of(&ProjectService { state }.rename(self::id(&id)?, title)?))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn unbind_project(app: AppHandle, id: String) -> Result<ProjectDto, String> {
    blocking(app, move |state| Ok(ProjectDto::of(&ProjectService { state }.unbind(self::id(&id)?)?))).await
}

/// Удаляет проект и его папку данных. Папку игры не трогает.
#[tauri::command]
#[specta::specta]
pub async fn delete_project(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| ProjectService { state }.delete(self::id(&id)?)).await
}
