//! Порт движков: договор с игрой без реализации. Сервисы знают только `GameEngine`.

pub mod renpy;

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::common::{ValidationError, text_key};
use super::files::{ContentHash, FileStore};
use super::projects::GameRoot;

/// Код движка. Записывается в проект при создании: по нему мод собирается без папки игры.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    Renpy,
}

impl EngineId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Renpy => "renpy",
        }
    }

    /// Неизвестный код из базы даёт ошибку, а не панику: базу могла записать более новая версия.
    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "renpy" => Ok(Self::Renpy),
            _ => Err(ValidationError::UnknownValue { field: "engine", value: text.to_string() }),
        }
    }
}

impl fmt::Display for EngineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Ошибка чтения игры: {0}")]
    Io(#[from] std::io::Error),
    /// Файл игры не разобрался. Файлы игр недоверенные: разбор возвращает ошибку, а не панику.
    #[error("Не удалось разобрать {path}: {reason}")]
    Format { path: String, reason: String },
    /// Мод не собрать: например, правка файла с оператором, который нельзя восстановить.
    #[error("Не удалось собрать мод: {0}")]
    Build(String),
}

/// Что движок узнал при распознавании и что нужно для сборки мода без папки игры.
/// У Ren'Py в `details`: формат `.rpyc` и раскладка `AstLayout`, предлагает ли игра сама переключить язык.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameInfo {
    pub engine: EngineId,
    pub version: Option<String>,
    /// Читает только сам движок.
    pub details: serde_json::Value,
}

/// Вид файла снимка.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    /// Скрипт оригинала как есть, в том числе `.rpyc` из архива.
    Script,
    /// Перевод, который уже есть в игре.
    Translation,
    /// Картинки перевода.
    Asset,
    /// Шрифты игры.
    Font,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Translation => "translation",
            Self::Asset => "asset",
            Self::Font => "font",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "script" => Ok(Self::Script),
            "translation" => Ok(Self::Translation),
            "asset" => Ok(Self::Asset),
            "font" => Ok(Self::Font),
            _ => Err(ValidationError::UnknownValue { field: "files.kind", value: text.to_string() }),
        }
    }
}

text_key!(
    /// Ключ контейнера от движка: у Ren'Py `label:day1`, `screen:inventory`.
    ContainerKey
);
text_key!(
    /// Ключ реплики от движка. Уникален среди текущих реплик, у повторов может перейти к другой реплике.
    SegmentKey
);
text_key!(
    /// Ключ говорящего от движка: у Ren'Py `code:lis`, `label:Radio Chatter`.
    SpeakerKey
);

/// Файл снимка: содержимое уже лежит в `FileStore`, здесь только сведения.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFile {
    /// От корня игры, через `/`.
    pub path: String,
    pub kind: FileKind,
    pub hash: ContentHash,
    pub size: u64,
}

/// Сцена или экран.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotContainer {
    pub key: ContainerKey,
    pub parent: Option<ContainerKey>,
    /// Значения задаёт движок: `scene`, `screen`.
    pub kind: String,
    /// Путь файла снимка.
    pub file: Option<String>,
    pub story_order: Option<i64>,
}

/// Реплика.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSegment {
    pub key: SegmentKey,
    pub container: Option<ContainerKey>,
    pub file: String,
    pub speaker: Option<SpeakerKey>,
    pub source_text: String,
    pub source_hash: String,
    /// Перевод, который уже есть в игре.
    pub game_text: Option<String>,
    /// Строка в файле, только для показа.
    pub location: Option<String>,
    pub story_order: Option<i64>,
    /// JSON, который читает только движок.
    pub engine_meta: Option<serde_json::Value>,
}

/// Говорящий: код персонажа или подпись-строка. Это не персонаж глоссария.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSpeaker {
    pub key: SpeakerKey,
    /// Значения задаёт движок: `code`, `label`.
    pub kind: String,
    /// Подпись, которую показывает игра.
    pub label: Option<String>,
}

/// Считанная игра. Содержимое файлов движок пишет в `FileStore` по мере чтения, в памяти его нет.
#[derive(Debug, Clone)]
pub struct GameSnapshot {
    pub info: GameInfo,
    pub files: Vec<SnapshotFile>,
    pub containers: Vec<SnapshotContainer>,
    pub segments: Vec<SnapshotSegment>,
    pub speakers: Vec<SnapshotSpeaker>,
}

/// Откуда читать игру.
#[derive(Debug, Clone)]
pub enum GameSource {
    /// Папка игры.
    Folder(GameRoot),
    /// Файлы прошлого снимка в `files/`: выросла `parser_version`, снимок пересобирается без папки игры.
    /// `GameInfo` при этом берётся из прошлого считывания.
    Stored(Vec<SnapshotFile>),
}

/// Готовая пара для мода: ключ реплики и перевод.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentTranslation {
    pub key: SegmentKey,
    pub text: String,
}

/// Замена шрифта игры загруженным.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSwap {
    pub target_path: String,
    pub hash: ContentHash,
}

/// Всё для сборки мода из проекта. Содержимое файлов движок читает из `FileStore`.
#[derive(Debug, Clone)]
pub struct ModInput {
    /// Текущий снимок.
    pub snapshot: GameSnapshot,
    /// Действующие переводы реплик текущего снимка.
    pub translations: Vec<SegmentTranslation>,
    /// Подпись говорящего и её перевод. Выбирает сервис экспорта, движок только кладёт в мод.
    pub speaker_labels: Vec<(String, String)>,
    pub fonts: Vec<FontSwap>,
    /// Код языка перевода (`ru`). Как назвать язык внутри игры, решает движок.
    pub target_language: String,
}

/// Собранный мод до установки: файл в `exports/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModArchive {
    pub path: PathBuf,
}

/// Движок игры. Без состояния: все движки лежат в статическом `ENGINES`, поэтому трейт `Sync`.
pub trait GameEngine: Sync {
    fn id(&self) -> EngineId;

    /// Версия разбора. Выросла: разбор в cache.db и снимок пересобираются из `files/`.
    fn parser_version(&self) -> u32;

    /// Узнаёт игру в корневой папке.
    fn detect(&self, root: &GameRoot) -> Option<GameInfo>;

    /// Считывает игру. Содержимое файлов пишет в `files` по мере чтения.
    fn read(&self, source: &GameSource, info: &GameInfo, files: &FileStore) -> Result<GameSnapshot, EngineError>;

    /// Файлы, от которых зависит разбор `file`. По их хэшам cache.db видит, что разбор устарел.
    fn parse_dependencies(&self, file: &SnapshotFile, all: &[SnapshotFile]) -> Vec<String>;

    /// Собирает мод без папки игры: всё нужное лежит в `input` и `files`.
    fn build_mod(&self, input: &ModInput, files: &FileStore, out_dir: &std::path::Path) -> Result<ModArchive, EngineError>;

    /// Ставит мод в игру, не трогая её файлы.
    fn install(&self, root: &GameRoot, archive: &ModArchive) -> Result<(), EngineError>;

    /// Убирает установленный мод.
    fn uninstall(&self, root: &GameRoot) -> Result<(), EngineError>;
}

/// Все движки.
pub static ENGINES: &[&dyn GameEngine] = &[&renpy::RenpyEngine];

/// Первый движок, который узнал игру в папке.
pub fn detect(root: &GameRoot) -> Option<(&'static dyn GameEngine, GameInfo)> {
    ENGINES.iter().find_map(|engine| engine.detect(root).map(|info| (*engine, info)))
}

/// Движок проекта по сохранённому коду, без папки игры.
pub fn by_id(id: EngineId) -> Option<&'static dyn GameEngine> {
    ENGINES.iter().copied().find(|engine| engine.id() == id)
}
