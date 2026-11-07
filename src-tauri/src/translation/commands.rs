//! Команды таблицы перевода.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::{FileProgress, LineState, Scene, SceneLine, TranslationService};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::scan::{FileId, SegmentId};
use crate::state::blocking;

/// Номер строки базы для окна. В JavaScript это `number`: точности до 2^53 номерам строк хватает.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
#[specta(transparent)]
pub struct RowRef(#[specta(type = specta_typescript::Number)] pub i64);

/// Файл с репликами и прогресс по нему.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FileDto {
    pub id: RowRef,
    pub path: String,
    pub total: u32,
    pub translated: u32,
    pub drafts: u32,
}

impl From<FileProgress> for FileDto {
    fn from(file: FileProgress) -> Self {
        Self {
            id: RowRef(file.id.get()),
            path: file.path,
            total: file.total as u32,
            translated: file.translated as u32,
            drafts: file.drafts as u32,
        }
    }
}

/// Реплика таблицы.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LineDto {
    pub id: RowRef,
    pub speaker: Option<String>,
    pub source_text: String,
    /// Действующий текст: перевод, черновик модели или перевод из игры.
    pub text: String,
    pub state: LineState,
    /// Перевод сделан для другого оригинала: игра изменила реплику.
    pub source_changed: bool,
    pub location: Option<String>,
}

impl From<SceneLine> for LineDto {
    fn from(line: SceneLine) -> Self {
        Self {
            id: RowRef(line.id.get()),
            speaker: line.speaker,
            source_text: line.source_text,
            text: line.text,
            state: line.state,
            source_changed: line.source_changed,
            location: line.location,
        }
    }
}

/// Подряд идущие реплики одной сцены файла.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneDto {
    /// Ключ сцены от движка. `None`: реплики вне сцен.
    pub key: Option<String>,
    pub title: Option<String>,
    pub lines: Vec<LineDto>,
}

impl From<Scene> for SceneDto {
    fn from(scene: Scene) -> Self {
        Self {
            key: scene.key.map(|key| key.to_string()),
            title: scene.title,
            lines: scene.lines.into_iter().map(LineDto::from).collect(),
        }
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

/// Файлы проекта с репликами и прогресс.
#[tauri::command]
#[specta::specta]
pub async fn translation_files(app: AppHandle, project: String) -> Result<Vec<FileDto>, String> {
    blocking(app, move |state| {
        Ok(TranslationService { state }.files(self::project(&project)?)?.into_iter().map(FileDto::from).collect())
    })
    .await
}

/// Реплики файла по сценам.
#[tauri::command]
#[specta::specta]
pub async fn file_lines(app: AppHandle, project: String, file: RowRef) -> Result<Vec<SceneDto>, String> {
    blocking(app, move |state| {
        let scenes = TranslationService { state }.file_scenes(self::project(&project)?, FileId::from_row(file.0))?;
        Ok(scenes.into_iter().map(SceneDto::from).collect())
    })
    .await
}

/// Правка пользователя. Пустой текст очищает реплику. Возвращает реплику после правки.
#[tauri::command]
#[specta::specta]
pub async fn save_line(app: AppHandle, project: String, line: RowRef, text: String) -> Result<LineDto, String> {
    blocking(app, move |state| {
        let line = TranslationService { state }.save_text(self::project(&project)?, SegmentId::from_row(line.0), &text)?;
        Ok(LineDto::from(line))
    })
    .await
}

/// Подтверждает черновики модели. Возвращает число подтверждённых.
#[tauri::command]
#[specta::specta]
pub async fn confirm_lines(app: AppHandle, project: String, lines: Vec<RowRef>) -> Result<u32, String> {
    blocking(app, move |state| {
        let lines: Vec<SegmentId> = lines.iter().map(|line| SegmentId::from_row(line.0)).collect();
        Ok(TranslationService { state }.confirm(self::project(&project)?, &lines)? as u32)
    })
    .await
}
