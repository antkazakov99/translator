//! Команды таблицы перевода.

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::{FileProgress, FileScenes, LineRun, LineState, LinesRun, Scene, SceneLine, SceneProgress, SceneRef, TranslationScope, TranslationService};
use crate::common::AppError;
use crate::engines::ContainerKey;
use crate::llm::{ProfileId, RunId, RunProgress, RunStatus};
use crate::projects::ProjectId;
use crate::scan::{FileId, SegmentId};
use crate::state::{AppState, blocking};

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
    pub description: Option<String>,
    pub lines: Vec<LineDto>,
}

impl From<Scene> for SceneDto {
    fn from(scene: Scene) -> Self {
        Self {
            key: scene.key.map(|key| key.to_string()),
            title: scene.title,
            description: scene.description,
            lines: scene.lines.into_iter().map(LineDto::from).collect(),
        }
    }
}

/// Сцена в списке таблицы. `key = None`: реплики файла вне сцен («Без сцены»).
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneItemDto {
    pub key: Option<String>,
    pub title: Option<String>,
    /// У сцены есть описание.
    pub described: bool,
    /// Игра изменила текст сцены после того, как модель его прочитала.
    pub stale: bool,
    pub total: u32,
    pub translated: u32,
    pub drafts: u32,
}

impl From<SceneProgress> for SceneItemDto {
    fn from(scene: SceneProgress) -> Self {
        Self {
            key: scene.key.map(|key| key.to_string()),
            title: scene.title,
            described: scene.described,
            stale: scene.stale,
            total: u32::try_from(scene.total).unwrap_or(u32::MAX),
            translated: u32::try_from(scene.translated).unwrap_or(u32::MAX),
            drafts: u32::try_from(scene.drafts).unwrap_or(u32::MAX),
        }
    }
}

/// Файл с прогрессом и его сцены по сюжету, «Без сцены» последней.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FileScenesDto {
    pub file: FileDto,
    pub scenes: Vec<SceneItemDto>,
}

impl From<FileScenes> for FileScenesDto {
    fn from(group: FileScenes) -> Self {
        Self { file: group.file.into(), scenes: group.scenes.into_iter().map(SceneItemDto::from).collect() }
    }
}

/// Какую сцену открыть в таблице.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SceneRefDto {
    Scene { key: String },
    /// Реплики файла вне сцен.
    Loose { file: RowRef },
}

impl TryFrom<SceneRefDto> for SceneRef {
    type Error = AppError;

    fn try_from(scene: SceneRefDto) -> Result<Self, AppError> {
        Ok(match scene {
            SceneRefDto::Scene { key } => Self::Scene(ContainerKey::new(key)?),
            SceneRefDto::Loose { file } => Self::Loose(FileId::from_row(file.0)),
        })
    }
}

/// Что переводить или вычитывать.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScopeDto {
    /// Сцены по ключам.
    Scenes { keys: Vec<String> },
    File { file: RowRef },
    Line { line: RowRef },
    /// Реплики файла вне сцен.
    Loose { file: RowRef },
    /// Все файлы проекта.
    Project,
}

impl TryFrom<ScopeDto> for TranslationScope {
    type Error = AppError;

    fn try_from(scope: ScopeDto) -> Result<Self, AppError> {
        Ok(match scope {
            ScopeDto::Scenes { keys } => {
                Self::Scenes(keys.into_iter().map(ContainerKey::new).collect::<Result<Vec<_>, _>>()?)
            }
            ScopeDto::File { file } => Self::File(FileId::from_row(file.0)),
            ScopeDto::Line { line } => Self::Line(SegmentId::from_row(line.0)),
            ScopeDto::Loose { file } => Self::Loose(FileId::from_row(file.0)),
            ScopeDto::Project => Self::Project,
        })
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

fn profile(id: Option<String>) -> Result<Option<ProfileId>, AppError> {
    Ok(id.as_deref().map(ProfileId::parse).transpose()?)
}

/// Итог запуска: по `run` его можно откатить.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LinesRunDto {
    pub run: RowRef,
    pub written: u32,
}

impl From<LinesRun> for LinesRunDto {
    fn from(result: LinesRun) -> Self {
        Self { run: RowRef(result.run.get()), written: u32::try_from(result.written).unwrap_or(u32::MAX) }
    }
}

/// Запуск перевода или редактуры для меню «Запуски».
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LineRunDto {
    pub id: RowRef,
    /// `lines` или `review`.
    pub action: String,
    pub model: String,
    pub status: RunStatus,
    pub error: Option<String>,
    /// Unix-время начала, секунды.
    #[specta(type = specta_typescript::Number)]
    pub started_at: i64,
    /// Реплики, в которые запуск что-то записал.
    pub lines: u32,
}

/// Итог отката.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RollbackDto {
    pub rolled: u32,
    /// Реплики, которые правили после запуска: их откат не трогал.
    pub left: u32,
}

/// Переводит пустые реплики моделью: черновики пишутся по пакетам, прогресс идёт в `progress`.
/// `profile` это разовый выбор модели. Возвращает запуск и число переведённых реплик.
#[tauri::command]
#[specta::specta]
pub async fn translate_lines(
    state: State<'_, AppState>,
    project: String,
    scope: ScopeDto,
    profile: Option<String>,
    progress: Channel<RunProgress>,
) -> Result<LinesRunDto, String> {
    let service = TranslationService { state: &state };
    Ok(service.translate(self::project(&project)?, scope.try_into()?, self::profile(profile)?, progress).await?.into())
}

/// Вычитывает готовые реплики моделью: исправления становятся черновиками. Возвращает запуск и число исправленных.
#[tauri::command]
#[specta::specta]
pub async fn review_lines(
    state: State<'_, AppState>,
    project: String,
    scope: ScopeDto,
    profile: Option<String>,
    progress: Channel<RunProgress>,
) -> Result<LinesRunDto, String> {
    let service = TranslationService { state: &state };
    Ok(service.review(self::project(&project)?, scope.try_into()?, self::profile(profile)?, progress).await?.into())
}

/// Последние запуски перевода и редактуры, новые первыми.
#[tauri::command]
#[specta::specta]
pub async fn line_runs(app: AppHandle, project: String) -> Result<Vec<LineRunDto>, String> {
    blocking(app, move |state| {
        let runs = TranslationService { state }.line_runs(self::project(&project)?)?;
        Ok(runs
            .into_iter()
            .map(|run: LineRun| LineRunDto {
                id: RowRef(run.id.get()),
                action: run.action.id().to_string(),
                model: run.model,
                status: run.status,
                error: run.error,
                started_at: run.started_at.secs(),
                lines: u32::try_from(run.lines).unwrap_or(u32::MAX),
            })
            .collect())
    })
    .await
}

/// Откатывает запуск модели: реплики, которые после него не правили, возвращаются к прежнему тексту.
#[tauri::command]
#[specta::specta]
pub async fn rollback_run(app: AppHandle, project: String, run: RowRef) -> Result<RollbackDto, String> {
    blocking(app, move |state| {
        let (rolled, left) = TranslationService { state }.rollback(self::project(&project)?, RunId::from_row(run.0))?;
        Ok(RollbackDto { rolled: u32::try_from(rolled).unwrap_or(u32::MAX), left: u32::try_from(left).unwrap_or(u32::MAX) })
    })
    .await
}

/// Очищает перевод области, как ручная очистка каждой реплики. Возвращает число очищенных.
#[tauri::command]
#[specta::specta]
pub async fn reset_lines(app: AppHandle, project: String, scope: ScopeDto) -> Result<u32, String> {
    blocking(app, move |state| {
        let cleared = TranslationService { state }.reset(self::project(&project)?, scope.try_into()?)?;
        Ok(u32::try_from(cleared).unwrap_or(u32::MAX))
    })
    .await
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

/// Файлы с прогрессом и их сцены для списка таблицы.
#[tauri::command]
#[specta::specta]
pub async fn translation_scenes(app: AppHandle, project: String) -> Result<Vec<FileScenesDto>, String> {
    blocking(app, move |state| {
        Ok(TranslationService { state }.scene_tree(self::project(&project)?)?.into_iter().map(FileScenesDto::from).collect())
    })
    .await
}

/// Реплики одной сцены или реплики файла вне сцен.
#[tauri::command]
#[specta::specta]
pub async fn scene_lines(app: AppHandle, project: String, scene: SceneRefDto) -> Result<Vec<SceneDto>, String> {
    blocking(app, move |state| {
        let scenes = TranslationService { state }.scene_lines(self::project(&project)?, scene.try_into()?)?;
        Ok(scenes.into_iter().map(SceneDto::from).collect())
    })
    .await
}

/// Реплики файла по сценам: «Пропавшие» выбирают из них реплику для привязки.
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
