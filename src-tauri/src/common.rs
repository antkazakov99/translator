//! Общие типы ядра: время, ошибки и id записей.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::engines::{EngineError, EngineId};
use super::storage::DbError;

/// Unix-время в секундах.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn now() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self(secs)
    }

    pub fn from_secs(secs: i64) -> Self {
        Self(secs)
    }

    pub fn secs(self) -> i64 {
        self.0
    }
}

/// Нарушено правило модели или типа-значения. Неверные данные из базы тоже дают её, а не панику.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Пустое значение: {0}")]
    Empty(&'static str),
    #[error("Неверный id: {0}")]
    BadId(String),
    #[error("Неизвестное значение {field}: {value}")]
    UnknownValue { field: &'static str, value: String },
    #[error("Значение вне допустимого диапазона: {0}")]
    OutOfRange(&'static str),
    #[error("Путь должен быть абсолютным и без `..`: {0}")]
    BadPath(String),
    #[error("Неверный код языка: {0}")]
    BadLanguage(String),
    #[error("В папке игра другого движка")]
    OtherEngine,
    #[error("Действие недоступно в статусе проекта «{0}»")]
    WrongStatus(&'static str),
    #[error("Форма «{0}» совпадает с именем или другой формой")]
    DuplicateForm(String),
}

/// Ошибка сценария. Команда превращает её в текст для окна.
#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("Ошибка диска: {0}")]
    Io(#[from] std::io::Error),
    #[error("Ни один движок не узнал игру в этой папке")]
    UnknownGame,
    #[error("Движок «{0}» не поддерживается этой версией")]
    UnsupportedEngine(EngineId),
    #[error("Проект не найден")]
    ProjectNotFound,
    #[error("Проект не открыт")]
    ProjectNotOpen,
    #[error("Папка игры не привязана к проекту")]
    Unbound,
    #[error("Папки игры нет на диске")]
    GameMissing,
    #[error("Добавьте модель в настройках")]
    NoProfiles,
    #[error("Профиль модели не найден")]
    ProfileNotFound,
    #[error("Модель уже работает: дождитесь конца запуска или остановите его")]
    RunActive,
    #[error("Запуск остановлен")]
    Cancelled,
    #[error("Ошибка запроса к модели: {0}")]
    Llm(String),
}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}

/// Id записи app.db: UUID v7, чтобы id шли в порядке создания. Разные id не подставить друг вместо друга.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(::uuid::Uuid);

        impl $name {
            pub fn generate() -> Self {
                Self(::uuid::Uuid::now_v7())
            }

            /// Из текста базы или окна: принимается только UUID v7.
            pub fn parse(text: &str) -> Result<Self, $crate::common::ValidationError> {
                ::uuid::Uuid::parse_str(text)
                    .ok()
                    .filter(|id| id.get_version_num() == 7)
                    .map(Self)
                    .ok_or_else(|| $crate::common::ValidationError::BadId(text.to_string()))
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }
    };
}
pub(crate) use uuid_id;

/// Id строки project.db: `INTEGER PRIMARY KEY`.
macro_rules! row_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(i64);

        impl $name {
            pub fn from_row(id: i64) -> Self {
                Self(id)
            }

            pub fn get(self) -> i64 {
                self.0
            }
        }
    };
}
pub(crate) use row_id;

/// Ключ от движка: непрозрачная строка, уникальная во всей игре.
macro_rules! text_key {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(key: impl Into<String>) -> Result<Self, $crate::common::ValidationError> {
                let key = key.into();
                if key.is_empty() {
                    return Err($crate::common::ValidationError::Empty(stringify!($name)));
                }
                Ok(Self(key))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
pub(crate) use text_key;

/// Род: с ним согласуются глаголы в переводе. `None` в полях значит «неизвестен».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    Male,
    Female,
}

impl Gender {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "male" => Ok(Self::Male),
            "female" => Ok(Self::Female),
            _ => Err(ValidationError::UnknownValue { field: "gender", value: text.to_string() }),
        }
    }
}

/// Текст без пробелов по краям или `None`, если он пустой.
pub fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
