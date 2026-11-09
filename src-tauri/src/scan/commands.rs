//! Команды считывания: повторное считывание игры и «Пропавшие».

use serde::Serialize;
use tauri::AppHandle;

use super::{MissingSegment, ScanReport, ScanService, SegmentId};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::blocking;
use crate::translation::commands::RowRef;

/// Итог считывания.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanReportDto {
    pub lines: u32,
    /// Новые реплики без пары в прошлой версии.
    pub new_lines: u32,
    /// Переводы, перенесённые на реплики с другим ключом.
    pub transferred: u32,
    /// Переводы, ставшие черновиками: оригинал изменился или пара сомнительная.
    pub drafts: u32,
    /// Реплик в «Пропавших».
    pub missing: u32,
}

impl From<ScanReport> for ScanReportDto {
    fn from(report: ScanReport) -> Self {
        Self {
            lines: report.lines as u32,
            new_lines: report.new_lines as u32,
            transferred: report.transferred as u32,
            drafts: report.drafts as u32,
            missing: report.missing as u32,
        }
    }
}

/// Реплика прошлой версии игры с переводом или историей.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MissingLineDto {
    pub id: RowRef,
    pub scene: Option<String>,
    pub source_text: String,
    pub translation: Option<String>,
    pub file: String,
    pub location: Option<String>,
}

impl From<MissingSegment> for MissingLineDto {
    fn from(line: MissingSegment) -> Self {
        Self {
            id: RowRef(line.id.get()),
            scene: line.container.map(|key| key.to_string()),
            source_text: line.source_text,
            translation: line.translation,
            file: line.file,
            location: line.location,
        }
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

/// Считывает привязанную папку игры заново и переносит перевод на новую версию.
#[tauri::command]
#[specta::specta]
pub async fn rescan_game(app: AppHandle, project: String) -> Result<ScanReportDto, String> {
    blocking(app, move |state| Ok(ScanService { state }.rescan(self::project(&project)?)?.into())).await
}

#[tauri::command]
#[specta::specta]
pub async fn missing_lines(app: AppHandle, project: String) -> Result<Vec<MissingLineDto>, String> {
    blocking(app, move |state| {
        Ok(ScanService { state }.missing(self::project(&project)?)?.into_iter().map(MissingLineDto::from).collect())
    })
    .await
}

/// Переносит перевод пропавшей реплики на реплику игры.
#[tauri::command]
#[specta::specta]
pub async fn attach_missing_line(app: AppHandle, project: String, missing: RowRef, target: RowRef) -> Result<(), String> {
    blocking(app, move |state| {
        ScanService { state }.attach_missing(self::project(&project)?, SegmentId::from_row(missing.0), SegmentId::from_row(target.0))
    })
    .await
}
