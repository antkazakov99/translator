//! Команды панели «Автоперевод».

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::{GameRun, GameRunService, GameStep, NamesMode, StepWork};
use crate::common::AppError;
use crate::glossary::commands::NameSuggestionsDto;
use crate::glossary::{CharacterId, NameField, NamePick};
use crate::llm::RunProgress;
use crate::projects::ProjectId;
use crate::state::{AppState, blocking};
use crate::translation::commands::RowRef;

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

/// Что осталось: «сцен без описания: 140».
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkCountDto {
    pub label: String,
    pub count: u32,
}

/// Сводка шага для карточки.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StepWorkDto {
    pub step: GameStep,
    pub title: String,
    pub left: Vec<WorkCountDto>,
    /// Примерное число запросов к модели.
    pub requests: u32,
    /// Делать нечего: шаг недоступен.
    pub empty: bool,
}

impl From<StepWork> for StepWorkDto {
    fn from(work: StepWork) -> Self {
        Self {
            empty: work.is_empty(),
            step: work.step,
            title: work.step.title().to_string(),
            left: work.left.into_iter().map(|count| WorkCountDto { label: count.label.to_string(), count: count.count as u32 }).collect(),
            requests: work.requests as u32,
        }
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StepResultDto {
    pub step: GameStep,
    pub written: u32,
}

/// Итог цепочки. `names`: ручной выбор имён, цепочка ждёт отбора и продолжится шагами `rest`.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GameRunDto {
    pub finished: Vec<StepResultDto>,
    pub stopped: bool,
    pub error: Option<String>,
    pub names: Option<NameSuggestionsDto>,
    pub rest: Vec<GameStep>,
}

impl From<GameRun> for GameRunDto {
    fn from(run: GameRun) -> Self {
        Self {
            finished: run.finished.into_iter().map(|done| StepResultDto { step: done.step, written: done.written as u32 }).collect(),
            stopped: run.stopped,
            error: run.error,
            names: run.names.map(Into::into),
            rest: run.rest,
        }
    }
}

/// Написание поля персонажа из отбора.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NamePickDto {
    pub character: RowRef,
    pub field: NameField,
    pub source: String,
    pub translation: String,
}

/// Сводка шагов автоперевода: сколько осталось и примерное число запросов.
#[tauri::command]
#[specta::specta]
pub async fn game_overview(app: AppHandle, project: String) -> Result<Vec<StepWorkDto>, String> {
    blocking(app, move |state| Ok(GameRunService { state }.overview(self::project(&project)?)?.into_iter().map(Into::into).collect())).await
}

/// Отмеченные шаги по порядку одной цепочкой. Повторный запуск продолжает с того же места.
#[tauri::command]
#[specta::specta]
pub async fn run_game(
    state: State<'_, AppState>,
    project: String,
    steps: Vec<GameStep>,
    names: NamesMode,
    progress: Channel<RunProgress>,
) -> Result<GameRunDto, String> {
    let run = GameRunService { state: &state }.run(self::project(&project)?, &steps, names, progress).await?;
    Ok(run.into())
}

/// Сброс всего прогресса: переводы, описания, заметки, отметки, весь глоссарий и законченные запуски.
#[tauri::command]
#[specta::specta]
pub async fn reset_game(app: AppHandle, project: String) -> Result<(), String> {
    blocking(app, move |state| GameRunService { state }.reset(self::project(&project)?)).await
}

/// Написания из отбора имён в пустые поля персонажей. Возвращает число изменённых персонажей.
#[tauri::command]
#[specta::specta]
pub async fn write_names(app: AppHandle, project: String, names: Vec<NamePickDto>) -> Result<u32, String> {
    blocking(app, move |state| {
        let picks: Vec<NamePick> = names
            .into_iter()
            .map(|pick| NamePick { character: CharacterId::from_row(pick.character.0), field: pick.field, source: pick.source, translation: pick.translation })
            .collect();
        Ok(GameRunService { state }.write_names(self::project(&project)?, &picks)? as u32)
    })
    .await
}
