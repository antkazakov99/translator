//! Проект и его типы-значения. Про базу и диск модель не знает.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::common::{Timestamp, ValidationError, uuid_id};
use crate::engines::EngineId;

uuid_id!(
    /// Id проекта, он же имя папки данных проекта.
    ProjectId
);

/// Статус это журнал операций с папкой проекта. `Creating` и `Deleting` видит только запуск приложения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Creating,
    Ready,
    Deleting,
}

impl ProjectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Deleting => "deleting",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "creating" => Ok(Self::Creating),
            "ready" => Ok(Self::Ready),
            "deleting" => Ok(Self::Deleting),
            _ => Err(ValidationError::UnknownValue { field: "projects.status", value: text.to_string() }),
        }
    }
}

/// Название проекта: не пустое, без пробелов по краям.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Title(String);

impl Title {
    pub fn new(text: &str) -> Result<Self, ValidationError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ValidationError::Empty("title"));
        }
        Ok(Self(text.to_string()))
    }

    /// Имя корневой папки игры: так проект называется при создании.
    pub fn from_root(root: &GameRoot) -> Self {
        root.as_path()
            .file_name()
            .and_then(|name| Self::new(&name.to_string_lossy()).ok())
            .unwrap_or_else(|| Self(root.as_path().to_string_lossy().into_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Корневая папка игры, а не вложенная. Абсолютный путь без `.` и `..`.
/// Регистр как на диске приводит сервис до создания: модель диск не читает.
/// Есть ли там игра, проверяет движок.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameRoot(PathBuf);

impl GameRoot {
    pub fn new(path: &Path) -> Result<Self, ValidationError> {
        let bad = || ValidationError::BadPath(path.to_string_lossy().into_owned());
        if !path.is_absolute() {
            return Err(bad());
        }
        let mut normal = PathBuf::new();
        for part in path.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => return Err(bad()),
                other => normal.push(other),
            }
        }
        Ok(Self(normal))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Строка `projects` как есть. Её заполняет и читает репозиторий.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub id: String,
    pub status: String,
    pub title: String,
    pub engine: String,
    pub game_path: Option<String>,
    pub created_at: i64,
    pub opened_at: Option<i64>,
    pub archived: bool,
}

/// Проект. Не зависит от папки игры: она только привязка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    id: ProjectId,
    status: ProjectStatus,
    title: Title,
    engine: EngineId,
    game_root: Option<GameRoot>,
    created_at: Timestamp,
    opened_at: Option<Timestamp>,
    archived: bool,
}

impl Project {
    /// Новый проект в статусе `Creating`: в списке его нет, пока не готова папка.
    pub fn create(title: Title, root: GameRoot, engine: EngineId, now: Timestamp) -> Self {
        Self {
            id: ProjectId::generate(),
            status: ProjectStatus::Creating,
            title,
            engine,
            game_root: Some(root),
            created_at: now,
            opened_at: None,
            archived: false,
        }
    }

    /// Из сохранённых полей. Неверные данные дают ошибку, а не панику.
    pub fn restore(row: ProjectRow) -> Result<Self, ValidationError> {
        Ok(Self {
            id: ProjectId::parse(&row.id)?,
            status: ProjectStatus::parse(&row.status)?,
            title: Title::new(&row.title)?,
            engine: EngineId::parse(&row.engine)?,
            game_root: row.game_path.as_deref().map(|path| GameRoot::new(Path::new(path))).transpose()?,
            created_at: Timestamp::from_secs(row.created_at),
            opened_at: row.opened_at.map(Timestamp::from_secs),
            archived: row.archived,
        })
    }

    /// Поля для сохранения.
    pub fn to_row(&self) -> ProjectRow {
        ProjectRow {
            id: self.id.to_string(),
            status: self.status.as_str().to_string(),
            title: self.title.as_str().to_string(),
            engine: self.engine.as_str().to_string(),
            game_path: self.game_root.as_ref().map(|root| root.as_path().to_string_lossy().into_owned()),
            created_at: self.created_at.secs(),
            opened_at: self.opened_at.map(Timestamp::secs),
            archived: self.archived,
        }
    }

    /// Папка и project.db готовы.
    pub fn ready(&mut self) -> Result<(), ValidationError> {
        if self.status != ProjectStatus::Creating {
            return Err(ValidationError::WrongStatus(self.status.as_str()));
        }
        self.status = ProjectStatus::Ready;
        Ok(())
    }

    /// Начало удаления: дальше проект не открыть.
    pub fn mark_deleting(&mut self) {
        self.status = ProjectStatus::Deleting;
    }

    /// Записывает факт открытия: время и выход из архива.
    pub fn open(&mut self, now: Timestamp) -> Result<(), ValidationError> {
        self.require_ready()?;
        self.opened_at = Some(now);
        self.archived = false;
        Ok(())
    }

    /// Убирает в архив: свёрнутый раздел внизу списка.
    pub fn archive(&mut self) -> Result<(), ValidationError> {
        self.require_ready()?;
        self.archived = true;
        Ok(())
    }

    /// Возвращает из архива без открытия.
    pub fn unarchive(&mut self) -> Result<(), ValidationError> {
        self.require_ready()?;
        self.archived = false;
        Ok(())
    }

    pub fn rename(&mut self, title: Title) {
        self.title = title;
    }

    /// Привязывает другую папку. Движок, который узнал в ней игру, должен совпасть с движком проекта.
    pub fn rebind(&mut self, root: GameRoot, engine: EngineId) -> Result<(), ValidationError> {
        self.require_ready()?;
        if engine != self.engine {
            return Err(ValidationError::OtherEngine);
        }
        self.game_root = Some(root);
        Ok(())
    }

    /// Отвязывает папку: проект работает дальше, недоступны установка мода и повторное считывание.
    pub fn unbind(&mut self) {
        self.game_root = None;
    }

    pub fn is_bound(&self) -> bool {
        self.game_root.is_some()
    }

    fn require_ready(&self) -> Result<(), ValidationError> {
        match self.status {
            ProjectStatus::Ready => Ok(()),
            other => Err(ValidationError::WrongStatus(other.as_str())),
        }
    }

    pub fn id(&self) -> ProjectId {
        self.id
    }

    pub fn status(&self) -> ProjectStatus {
        self.status
    }

    pub fn title(&self) -> &Title {
        &self.title
    }

    pub fn engine(&self) -> EngineId {
        self.engine
    }

    pub fn game_root(&self) -> Option<&GameRoot> {
        self.game_root.as_ref()
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn opened_at(&self) -> Option<Timestamp> {
        self.opened_at
    }

    pub fn is_archived(&self) -> bool {
        self.archived
    }
}
