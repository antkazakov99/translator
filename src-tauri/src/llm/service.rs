//! Настройки LLM приложения: провайдеры, ключи, профили, настройки действий.

use super::{Action, ActionRepo, ActionSettings, ApiKey, Profile, ProfileId, ProfileRepo, Provider, ProviderId, ProviderRepo};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::AppState;
use crate::storage::DbError;

/// Провайдер для окна: без ключа, только признак, что он задан.
pub struct ProviderEntry {
    pub provider: Provider,
    pub has_key: bool,
}

/// Всё, что показывает окно настроек.
pub struct LlmSettings {
    pub providers: Vec<ProviderEntry>,
    pub profiles: Vec<Profile>,
    /// Все действия в порядке меню, без строки в базе с пустыми полями.
    pub actions: Vec<(Action, ActionSettings)>,
}

pub struct LlmSettingsService<'a> {
    pub state: &'a AppState,
}

impl LlmSettingsService<'_> {
    pub fn settings(&self) -> Result<LlmSettings, AppError> {
        let (providers, profiles, saved) = self.state.app.read(|tx| {
            Ok((ProviderRepo { conn: tx }.list()?, ProfileRepo { conn: tx }.list()?, ActionRepo { conn: tx }.all()?))
        })?;
        let providers = providers
            .into_iter()
            .map(|provider| {
                // Сбой хранилища ключей не закрывает настройки: провайдер показывается без ключа.
                let has_key = match self.state.secrets.get(provider.id()) {
                    Ok(key) => key.is_some(),
                    Err(error) => {
                        tracing::warn!("Ключ провайдера {} не прочитан: {error}", provider.id());
                        false
                    }
                };
                ProviderEntry { provider, has_key }
            })
            .collect();
        Ok(LlmSettings { providers, profiles, actions: with_all_actions(saved) })
    }

    /// Место нового провайдера: в конце списка.
    pub fn next_provider_position(&self) -> Result<u32, AppError> {
        let providers = self.state.app.read(|tx| ProviderRepo { conn: tx }.list())?;
        Ok(providers.iter().map(|provider| provider.position() + 1).max().unwrap_or(0))
    }

    /// Место нового профиля: в конце общего списка.
    pub fn next_profile_position(&self) -> Result<u32, AppError> {
        let profiles = self.state.app.read(|tx| ProfileRepo { conn: tx }.list())?;
        Ok(profiles.iter().map(|profile| profile.position() + 1).max().unwrap_or(0))
    }

    pub fn provider(&self, id: ProviderId) -> Result<Provider, AppError> {
        Ok(self.state.app.read(|tx| ProviderRepo { conn: tx }.get(id))?)
    }

    pub fn profile(&self, id: ProfileId) -> Result<Profile, AppError> {
        self.state.app.read(|tx| ProfileRepo { conn: tx }.get(id)).map_err(profile_error)
    }

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

    /// Провайдер профиля должен существовать.
    pub fn save_profile(&self, profile: &Profile) -> Result<(), AppError> {
        self.state.app.write(|tx| {
            ProviderRepo { conn: tx }.get(profile.provider_id())?;
            ProfileRepo { conn: tx }.save(profile)
        })?;
        Ok(())
    }

    pub fn delete_profile(&self, id: ProfileId) -> Result<(), AppError> {
        self.state.app.write(|tx| ProfileRepo { conn: tx }.delete(id))?;
        Ok(())
    }

    /// Меняет порядок профилей: первый берут действия без своего профиля.
    /// Профили, которых нет в `order`, встают следом в прежнем порядке, незнакомые id пропускаются.
    pub fn reorder_profiles(&self, order: &[ProfileId]) -> Result<(), AppError> {
        self.state.app.write(|tx| {
            let repo = ProfileRepo { conn: tx };
            let current: Vec<ProfileId> = repo.list()?.iter().map(Profile::id).collect();
            let listed = order.iter().filter(|id| current.contains(id));
            let rest = current.iter().filter(|id| !order.contains(id));
            for (position, id) in listed.chain(rest).enumerate() {
                repo.set_position(*id, position as u32)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn save_action(&self, action: Action, settings: &ActionSettings) -> Result<(), AppError> {
        self.state.app.write(|tx| {
            if let Some(profile) = settings.profile {
                ProfileRepo { conn: tx }.get(profile)?;
            }
            ActionRepo { conn: tx }.save(action, settings)
        })
        .map_err(profile_error)
    }

    /// Переопределения настроек действий в проекте, все действия в порядке меню.
    pub fn project_actions(&self, project: ProjectId) -> Result<Vec<(Action, ActionSettings)>, AppError> {
        let open = self.state.open_project(project)?;
        let saved = open.project.read(|tx| ActionRepo { conn: tx }.all())?;
        Ok(with_all_actions(saved))
    }

    /// Переопределение настроек действия в проекте. Профиль проверяется по app.db: в проекте внешних ключей нет.
    pub fn save_project_action(&self, project: ProjectId, action: Action, settings: &ActionSettings) -> Result<(), AppError> {
        if let Some(profile) = settings.profile {
            self.profile(profile)?;
        }
        let open = self.state.open_project(project)?;
        open.project.write(|tx| ActionRepo { conn: tx }.save(action, settings))?;
        Ok(())
    }
}

/// Строки базы, разложенные по всем действиям в порядке меню.
fn with_all_actions(saved: Vec<(Action, ActionSettings)>) -> Vec<(Action, ActionSettings)> {
    Action::ALL
        .into_iter()
        .map(|action| {
            let settings = saved.iter().find(|(saved, _)| *saved == action).map(|(_, settings)| *settings);
            (action, settings.unwrap_or_default())
        })
        .collect()
}

fn profile_error(error: DbError) -> AppError {
    match error {
        DbError::NotFound => AppError::ProfileNotFound,
        error => error.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{BatchSize, Params, ProviderKind, Temperature};
    use crate::projects::ProjectService;
    use crate::state::tests::{memory_secrets, ready_project};

    fn start() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        (dir, state)
    }

    fn local(service: &LlmSettingsService) -> Provider {
        let provider = Provider::new("LM Studio", ProviderKind::Local, "http://127.0.0.1:1234/v1", 0).unwrap();
        service.save_provider(&provider, None).unwrap();
        provider
    }

    fn profile(service: &LlmSettingsService, provider: &Provider, model: &str) -> Profile {
        let profile = Profile::new(provider.id(), "", model, service.next_profile_position().unwrap()).unwrap();
        service.save_profile(&profile).unwrap();
        profile
    }

    #[test]
    fn provider_and_its_key_live_and_die_together() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let provider = Provider::new("Claude", ProviderKind::Claude, "https://api.anthropic.com", 0).unwrap();

        service.save_provider(&provider, Some(&ApiKey::new("sk-ключ").unwrap())).unwrap();
        service.save_provider(&provider, None).unwrap();
        assert_eq!(state.secrets.get(provider.id()).unwrap().unwrap().expose(), "sk-ключ");
        assert_eq!(service.provider(provider.id()).unwrap(), provider);
        assert!(service.settings().unwrap().providers[0].has_key);

        service.delete_provider(provider.id()).unwrap();
        assert!(state.secrets.get(provider.id()).unwrap().is_none());
        assert!(matches!(service.provider(provider.id()), Err(AppError::Db(DbError::NotFound))));
    }

    #[test]
    fn profile_keeps_limits_and_params() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let provider = local(&service);
        let mut saved = Profile::new(provider.id(), "Быстрая", "gemma-3", 0).unwrap();
        saved.set_limits(Some(32_000), Some(0), None).unwrap();
        let fields = serde_json::json!({ "top_p": 0.9, "temperature": 1.0 });
        saved.set_params(Params::new(fields.as_object().unwrap().clone()));
        service.save_profile(&saved).unwrap();

        let loaded = service.profile(saved.id()).unwrap();
        assert_eq!(loaded, saved);
        assert_eq!(loaded.reasoning_tokens(), 0);
        assert_eq!(loaded.timeout_limit(), None);
        assert_eq!(loaded.params().fields().keys().collect::<Vec<_>>(), ["top_p"]);
    }

    #[test]
    fn profile_needs_existing_provider() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let profile = Profile::new(ProviderId::generate(), "", "gpt", 0).unwrap();
        assert!(matches!(service.save_profile(&profile), Err(AppError::Db(DbError::NotFound))));
    }

    #[test]
    fn reorder_puts_unlisted_profiles_after() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let provider = local(&service);
        let a = profile(&service, &provider, "a");
        let b = profile(&service, &provider, "b");
        let c = profile(&service, &provider, "c");

        service.reorder_profiles(&[c.id(), ProfileId::generate(), a.id()]).unwrap();
        let order: Vec<_> = service.settings().unwrap().profiles.iter().map(|profile| profile.model().to_string()).collect();
        assert_eq!(order, ["c", "a", "b"]);
        assert_eq!(state.app.read(|tx| ProfileRepo { conn: tx }.first()).unwrap().unwrap().id(), c.id());
        assert_eq!(service.next_profile_position().unwrap(), 3);
        let _ = b;
    }

    #[test]
    fn action_settings_round_trip_and_empty_row_goes_away() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let provider = local(&service);
        let model = profile(&service, &provider, "gemma");
        let settings = ActionSettings {
            profile: Some(model.id()),
            temperature: Some(Temperature::new(0.3).unwrap()),
            batch_size: Some(BatchSize::new(12).unwrap()),
        };

        service.save_action(Action::Lines, &settings).unwrap();
        let actions = service.settings().unwrap().actions;
        assert_eq!(actions.iter().map(|(action, _)| *action).collect::<Vec<_>>(), Action::ALL);
        assert_eq!(actions[0], (Action::Lines, settings));
        let stored: f64 = state
            .app
            .read(|tx| Ok(tx.query_row("SELECT temperature FROM llm_action_settings", [], |row| row.get(0))?))
            .unwrap();
        assert_eq!(stored, 0.3);

        // Удаление профиля обнуляет его в настройках действия, остальное остаётся.
        service.delete_profile(model.id()).unwrap();
        let after = state.app.read(|tx| ActionRepo { conn: tx }.settings(Action::Lines)).unwrap();
        assert_eq!(after, ActionSettings { profile: None, ..settings });

        service.save_action(Action::Lines, &ActionSettings::default()).unwrap();
        let rows: i64 = state
            .app
            .read(|tx| Ok(tx.query_row("SELECT COUNT(*) FROM llm_action_settings", [], |row| row.get(0))?))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn action_rejects_unknown_profile() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let settings = ActionSettings { profile: Some(ProfileId::generate()), ..Default::default() };
        assert!(matches!(service.save_action(Action::Names, &settings), Err(AppError::ProfileNotFound)));
    }

    #[test]
    fn project_overrides_live_in_project_db() {
        let (_dir, state) = start();
        let service = LlmSettingsService { state: &state };
        let project = ready_project(&state, "Игра");
        ProjectService { state: &state }.open(project.id()).unwrap();
        let settings = ActionSettings { batch_size: Some(BatchSize::new(5).unwrap()), ..Default::default() };

        service.save_project_action(project.id(), Action::Review, &settings).unwrap();
        let actions = service.project_actions(project.id()).unwrap();
        assert_eq!(actions.iter().find(|(action, _)| *action == Action::Review).unwrap().1, settings);
        assert_eq!(service.settings().unwrap().actions[1].1, ActionSettings::default());

        let unknown = ActionSettings { profile: Some(ProfileId::generate()), ..Default::default() };
        let error = service.save_project_action(project.id(), Action::Review, &unknown);
        assert!(matches!(error, Err(AppError::ProfileNotFound)));
    }

    #[test]
    fn project_skips_actions_unknown_to_this_version() {
        let (_dir, state) = start();
        let project = ready_project(&state, "Игра");
        ProjectService { state: &state }.open(project.id()).unwrap();
        let open = state.open_project(project.id()).unwrap();
        open.project
            .write(|tx| Ok(tx.execute("INSERT INTO llm_action_settings (action_id, batch_size) VALUES ('future', 3)", [])?))
            .unwrap();
        let actions = LlmSettingsService { state: &state }.project_actions(project.id()).unwrap();
        assert!(actions.iter().all(|(_, settings)| *settings == ActionSettings::default()));
    }
}
