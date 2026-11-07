//! Настройки LLM приложения: провайдеры, ключи, профили, настройки действий.

use super::{Action, ActionSettings, ApiKey, Profile, ProfileId, Provider, ProviderId, ProviderRepo};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::AppState;

pub struct LlmSettingsService<'a> {
    pub state: &'a AppState,
}

impl LlmSettingsService<'_> {
    /// Сначала строка, потом ключ: сбой между ними оставит провайдера без ключа, а не ключ без провайдера.
    /// `None` в `key` оставляет прежний ключ.
    pub fn save_provider(&self, provider: &Provider, key: Option<&ApiKey>) -> Result<(), AppError> {
        self.state.app.write(|tx| ProviderRepo { conn: tx }.save(provider))?;
        if let Some(key) = key {
            self.state.secrets.set(provider.id(), key)?;
        }
        Ok(())
    }

    /// Убирает ключ провайдера, строка остаётся.
    pub fn forget_key(&self, id: ProviderId) -> Result<(), AppError> {
        Ok(self.state.secrets.delete(id)?)
    }

    /// Сначала ключ, потом строка: ключ никогда не переживает провайдера, поэтому уборка ключей при запуске не нужна.
    /// Профили удаляются каскадом, у действий профиль обнуляется.
    pub fn delete_provider(&self, id: ProviderId) -> Result<(), AppError> {
        self.state.secrets.delete(id)?;
        self.state.app.write(|tx| ProviderRepo { conn: tx }.delete(id))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderKind;
    use crate::state::tests::memory_secrets;
    use crate::storage::DbError;

    #[test]
    fn provider_and_its_key_live_and_die_together() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let service = LlmSettingsService { state: &state };
        let provider = Provider::new("Claude", ProviderKind::Claude, "https://api.anthropic.com", 0).unwrap();

        service.save_provider(&provider, Some(&ApiKey::new("sk-ключ").unwrap())).unwrap();
        service.save_provider(&provider, None).unwrap();
        assert_eq!(state.secrets.get(provider.id()).unwrap().unwrap().expose(), "sk-ключ");
        assert_eq!(state.app.read(|tx| ProviderRepo { conn: tx }.get(provider.id())).unwrap(), provider);

        service.delete_provider(provider.id()).unwrap();
        assert!(state.secrets.get(provider.id()).unwrap().is_none());
        let missing = state.app.read(|tx| ProviderRepo { conn: tx }.get(provider.id()));
        assert!(matches!(missing, Err(DbError::NotFound)));
    }
}
