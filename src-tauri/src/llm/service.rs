//! Настройки LLM приложения: провайдеры, ключи, профили, настройки действий.

use super::{Action, ActionSettings, ApiKey, Profile, ProfileId, Provider, ProviderId, ProviderRepo};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::AppState;

pub struct LlmSettingsService<'a> {
    pub state: &'a AppState,
}

impl LlmSettingsService<'_> {
    /// Сначала база, потом ключ. `None` в `key` оставляет прежний ключ.
    pub fn save_provider(&self, provider: &Provider, key: Option<ApiKey>) -> Result<(), AppError> {
        todo!()
    }

    /// Сначала база, потом ключ: ключ без провайдера убирает запуск приложения.
    pub fn delete_provider(&self, id: ProviderId) -> Result<(), AppError> {
        self.state.app.write(|tx| ProviderRepo { conn: tx }.delete(id))?;
        self.state.secrets.delete(id);
        Ok(())
    }

    pub fn save_profile(&self, profile: &Profile) -> Result<(), AppError> {
        todo!()
    }

    pub fn delete_profile(&self, id: ProfileId) -> Result<(), AppError> {
        todo!()
    }

    /// Меняет порядок профилей: первый берут действия без своего профиля.
    pub fn reorder_profiles(&self, order: &[ProfileId]) -> Result<(), AppError> {
        todo!()
    }

    pub fn save_action(&self, action: Action, settings: &ActionSettings) -> Result<(), AppError> {
        todo!()
    }

    /// Переопределение настроек действия в проекте.
    pub fn save_project_action(&self, project: ProjectId, action: Action, settings: &ActionSettings) -> Result<(), AppError> {
        todo!()
    }
}
