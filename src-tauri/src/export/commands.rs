//! Команды мода: сборка, установка в игру и снятие.

use serde::Serialize;
use tauri::AppHandle;

use super::{ExportService, ModResult};
use crate::common::AppError;
use crate::engines::ModArchive;
use crate::projects::ProjectId;
use crate::state::blocking;

/// Итог сборки или установки мода.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModResultDto {
    /// Собранный архив в папке проекта.
    pub path: String,
    pub files: u32,
    /// Переводов реплик в моде.
    pub written: u32,
    /// Переводов, которые в мод не вошли.
    pub skipped: u32,
    /// В папке игры на диске есть свой перевод: он важнее архива.
    pub tl_on_disk: bool,
    /// Файлы перевода игры, которые не удалось включить в мод.
    pub left_out: Vec<String>,
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

impl ModResultDto {
    fn new(archive: ModArchive, tl_on_disk: bool) -> Self {
        Self {
            path: archive.path.to_string_lossy().into_owned(),
            files: count(archive.files),
            written: count(archive.written),
            skipped: count(archive.skipped),
            tl_on_disk,
            left_out: archive.left_out,
        }
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

/// Собирает `ru_lang.rpa` в папке проекта. Папка игры не нужна.
#[tauri::command]
#[specta::specta]
pub async fn build_mod(app: AppHandle, project: String) -> Result<ModResultDto, String> {
    blocking(app, move |state| {
        let archive = ExportService { state }.build(self::project(&project)?)?;
        Ok(ModResultDto::new(archive, false))
    })
    .await
}

/// Собирает мод заново и ставит в привязанную папку игры.
#[tauri::command]
#[specta::specta]
pub async fn install_mod(app: AppHandle, project: String) -> Result<ModResultDto, String> {
    blocking(app, move |state| {
        let ModResult { archive, tl_on_disk } = ExportService { state }.install(self::project(&project)?)?;
        Ok(ModResultDto::new(archive, tl_on_disk))
    })
    .await
}

/// Убирает мод из папки игры.
#[tauri::command]
#[specta::specta]
pub async fn uninstall_mod(app: AppHandle, project: String) -> Result<(), String> {
    blocking(app, move |state| ExportService { state }.uninstall(self::project(&project)?)).await
}
