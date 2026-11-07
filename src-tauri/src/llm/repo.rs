//! Таблицы LLM: `llm_providers`, `llm_profiles` в app.db и `llm_action_settings` в обеих базах.

use rusqlite::Connection;

use super::{Action, ActionSettings, Profile, ProfileId, Provider, ProviderId};
use crate::storage::DbError;

pub struct ProviderRepo<'c> {
    pub conn: &'c Connection,
}

impl ProviderRepo<'_> {
    pub fn get(&self, id: ProviderId) -> Result<Provider, DbError> {
        todo!()
    }

    /// По `position`.
    pub fn list(&self) -> Result<Vec<Provider>, DbError> {
        todo!()
    }

    pub fn save(&self, provider: &Provider) -> Result<(), DbError> {
        todo!()
    }

    /// Профили удаляются каскадом, у действий профиль обнуляется.
    pub fn delete(&self, id: ProviderId) -> Result<(), DbError> {
        todo!()
    }
}

pub struct ProfileRepo<'c> {
    pub conn: &'c Connection,
}

impl ProfileRepo<'_> {
    pub fn get(&self, id: ProfileId) -> Result<Profile, DbError> {
        todo!()
    }

    /// Общий список профилей по `position`.
    pub fn list(&self) -> Result<Vec<Profile>, DbError> {
        todo!()
    }

    /// Профиль действий, у которых свой не задан.
    pub fn first(&self) -> Result<Option<Profile>, DbError> {
        todo!()
    }

    pub fn save(&self, profile: &Profile) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, id: ProfileId) -> Result<(), DbError> {
        todo!()
    }
}

/// `llm_action_settings`. Таблица одинаковая в app.db и project.db, поэтому репозиторий один на обе базы.
/// В проекте внешних ключей нет: неизвестное действие пропускается, неизвестный профиль проверяет `LlmTargetService`.
pub struct ActionRepo<'c> {
    pub conn: &'c Connection,
}

impl ActionRepo<'_> {
    /// Нет строки: все поля пустые.
    pub fn settings(&self, action: Action) -> Result<ActionSettings, DbError> {
        todo!()
    }

    /// Все строки с известными действиями.
    pub fn all(&self) -> Result<Vec<(Action, ActionSettings)>, DbError> {
        todo!()
    }

    /// Все поля пустые: строка удаляется.
    pub fn save(&self, action: Action, settings: &ActionSettings) -> Result<(), DbError> {
        todo!()
    }
}
