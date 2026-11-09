//! Сцена для перевода, черновик модели и подсказки глоссария.

use serde::Serialize;

use crate::common::{Gender, Timestamp, ValidationError};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::llm::{Action, RunId, RunStatus};
use crate::scan::{FileId, SegmentId};

/// Что переводить или вычитывать.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationScope {
    Scenes(Vec<ContainerKey>),
    File(FileId),
    Line(SegmentId),
    /// Все файлы с пустыми репликами по очереди.
    Project,
}

/// Состояние реплики в таблице.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LineState {
    /// Нет перевода.
    Empty,
    /// Перевод модели, который пользователь не проверил («Модель»).
    Draft,
    /// Перевод пользователя или подтверждённый.
    Done,
    /// Перевод, который уже есть в игре.
    Game,
}

/// Реплика сцены в порядке сюжета.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneLine {
    pub id: SegmentId,
    /// Подпись говорящего для модели: имя из глоссария, иначе подпись из игры.
    pub speaker: Option<String>,
    pub speaker_gender: Option<Gender>,
    /// Ключ говорящего: по нему глоссарий даёт подпись и род.
    pub speaker_key: Option<SpeakerKey>,
    pub source_text: String,
    pub source_hash: String,
    /// Действующий текст: `translations.text`, без строки `game_text`.
    pub text: String,
    pub state: LineState,
    /// Строка в файле, только для показа.
    pub location: Option<String>,
    /// Перевод сделан для другого оригинала: игра изменила реплику после перевода.
    pub source_changed: bool,
}

impl LineState {
    /// Состояние по строке `translations` (текст и `draft`) и переводу из игры.
    pub fn of(row: Option<(&str, bool)>, game_text: Option<&str>) -> Self {
        match row {
            Some((text, _)) if text.trim().is_empty() => Self::Empty,
            Some((_, true)) => Self::Draft,
            Some((_, false)) => Self::Done,
            None if game_text.is_some_and(|text| !text.trim().is_empty()) => Self::Game,
            None => Self::Empty,
        }
    }
}

/// Реплика текущего снимка с местом в игре: сцена и файл. Глоссарий показывает по ним реплики записи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedLine {
    pub scene: Option<ContainerKey>,
    pub scene_title: Option<String>,
    pub file: FileId,
    pub line: SceneLine,
}

/// Файл с репликами текущего снимка и прогресс перевода в нём.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProgress {
    pub id: FileId,
    pub path: String,
    pub total: usize,
    /// Реплики с действующим текстом: перевод, черновик или перевод из игры.
    pub translated: usize,
    pub drafts: usize,
}

/// Запуск перевода или редактуры реплик для меню «Запуски».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRun {
    pub id: RunId,
    pub action: Action,
    pub model: String,
    pub status: RunStatus,
    pub error: Option<String>,
    pub started_at: Timestamp,
    /// Реплики, в которые запуск что-то записал.
    pub lines: usize,
}

impl SceneLine {
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Что ушло модели: по этому репозиторий проверит, что пользователь не успел поправить реплику.
    pub fn sent(&self) -> SentState {
        SentState { text: self.text.clone(), source_hash: self.source_hash.clone() }
    }
}

/// Сцена для перевода.
#[derive(Debug, Clone)]
pub struct Scene {
    pub key: Option<ContainerKey>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub lines: Vec<SceneLine>,
}

/// Действующий текст и `source_hash` реплики в момент отправки модели.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentState {
    pub text: String,
    pub source_hash: String,
}

/// Перевод или исправление модели. Пишется, только если реплика осталась такой же, какой ушла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub line: SegmentId,
    pub text: String,
    pub sent: SentState,
}

/// Подсказки глоссария в запрос: имена с родом, термины, токены, правила имён.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Оригинал, перевод и род.
    pub names: Vec<(String, String, Option<Gender>)>,
    /// Оригинал, перевод и признак предположения (`unconfirmed`).
    pub terms: Vec<(String, String, bool)>,
    /// Токен без значения и его пояснение: модель видит токен в тексте.
    pub tokens: Vec<(String, String)>,
    /// Имя, фамилия, полное имя или форма персонажа и коды языков их происхождения, у которых есть правила.
    pub name_origins: Vec<(String, Vec<String>)>,
    /// Правила имён: код языка и текст.
    pub name_rules: Vec<(String, String)>,
}

/// Вид строки истории.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RevisionKind {
    User,
    Llm,
    Rollback,
    /// Перенос при считывании: откат запуска его не считает.
    Transfer,
}

impl RevisionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Llm => "llm",
            Self::Rollback => "rollback",
            Self::Transfer => "transfer",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "user" => Ok(Self::User),
            "llm" => Ok(Self::Llm),
            "rollback" => Ok(Self::Rollback),
            "transfer" => Ok(Self::Transfer),
            _ => Err(ValidationError::UnknownValue { field: "revisions.kind", value: text.to_string() }),
        }
    }
}
