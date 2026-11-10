//! Действия модели и их настройки: проект, потом приложение, потом значения из кода.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::ProfileId;
use crate::common::ValidationError;

/// Действие модели. Совпадает со справочником `llm_actions`: новое действие это новый вариант и новая миграция.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Analysis,
    Descriptions,
    Names,
    Terms,
    Tokens,
    Speakers,
    Lines,
    Review,
}

impl Action {
    /// Порядок в меню.
    pub const ALL: [Action; 8] = [
        Self::Lines,
        Self::Review,
        Self::Analysis,
        Self::Descriptions,
        Self::Names,
        Self::Terms,
        Self::Speakers,
        Self::Tokens,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Analysis => "analysis",
            Self::Descriptions => "descriptions",
            Self::Names => "names",
            Self::Terms => "terms",
            Self::Tokens => "tokens",
            Self::Speakers => "speakers",
            Self::Lines => "lines",
            Self::Review => "review",
        }
    }

    /// `None` для неизвестного действия: строку проекта с ним пропускают, а не падают.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == text)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Analysis => "Разбор сцен",
            Self::Descriptions => "Обновление описаний",
            Self::Names => "Перевод имён",
            Self::Terms => "Перевод терминов",
            Self::Tokens => "Обновление пояснений",
            Self::Speakers => "Перевод подписей",
            Self::Lines => "Перевод реплик",
            Self::Review => "Редактура реплик",
        }
    }

    pub fn default_temperature(self) -> Temperature {
        let value = match self {
            Self::Names | Self::Analysis => 0.0,
            Self::Descriptions | Self::Terms | Self::Tokens | Self::Speakers => 0.2,
            Self::Lines | Self::Review => 0.3,
        };
        Temperature(value)
    }

    pub fn default_batch_size(self) -> BatchSize {
        let value = match self {
            Self::Lines | Self::Review => 30,
            Self::Descriptions => 3,
            Self::Names | Self::Analysis => 1,
            Self::Tokens => 10,
            Self::Speakers => 15,
            Self::Terms => 20,
        };
        BatchSize(NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN))
    }
}

/// Температура от 0 до 2.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Temperature(f32);

impl Temperature {
    /// Служебные запросы, например починка сломанного JSON ответа.
    pub const ZERO: Temperature = Temperature(0.0);

    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !(0.0..=2.0).contains(&value) {
            return Err(ValidationError::OutOfRange("temperature"));
        }
        Ok(Self(value as f32))
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// Кратчайшая запись `f32` в `f64`: 0.3 остаётся 0.3, а не 0.30000001192.
    pub fn as_f64(self) -> f64 {
        self.0.to_string().parse().unwrap_or(f64::from(self.0))
    }
}

/// Сколько записей в одном запросе: больше нуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BatchSize(NonZeroU32);

impl BatchSize {
    pub fn new(value: i64) -> Result<Self, ValidationError> {
        u32::try_from(value)
            .ok()
            .and_then(NonZeroU32::new)
            .map(Self)
            .ok_or(ValidationError::OutOfRange("batch_size"))
    }

    pub fn get(self) -> usize {
        self.0.get() as usize
    }
}

/// Настройки действия на одном уровне: приложение или проект. `None` значит «как уровнем ниже».
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionSettings {
    pub profile: Option<ProfileId>,
    pub temperature: Option<Temperature>,
    pub batch_size: Option<BatchSize>,
}

impl ActionSettings {
    /// Склеивает уровни по полю: `project.or(app)`.
    pub fn or(self, other: ActionSettings) -> ActionSettings {
        ActionSettings {
            profile: self.profile.or(other.profile),
            temperature: self.temperature.or(other.temperature),
            batch_size: self.batch_size.or(other.batch_size),
        }
    }

    /// Добивает пустое значениями из кода. Профиль остаётся необязательным: его выбирает `ProfileChoice`.
    pub fn resolve(self, action: Action) -> ResolvedSettings {
        ResolvedSettings {
            action,
            profile: self.profile,
            temperature: self.temperature.unwrap_or(action.default_temperature()),
            batch_size: self.batch_size.unwrap_or(action.default_batch_size()),
        }
    }
}

/// Итог настроек действия.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ResolvedSettings {
    pub action: Action,
    pub profile: Option<ProfileId>,
    pub temperature: Temperature,
    pub batch_size: BatchSize,
}
