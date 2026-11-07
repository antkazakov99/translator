//! Цель запроса: провайдер, профиль, ключ и итог настроек действия. Заменяет `ChatTarget` из 1.0.

use super::{
    Action, ActionRepo, ActionSettings, Profile, ProfileId, ProfileRepo, Provider, ProviderRepo, ResolvedSettings, Temperature,
};
use crate::common::AppError;
use crate::state::{AppState, OpenProject};

/// Ключ API. Без `Debug` и `Serialize`: не попадёт ни в лог, ни в окно.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Пустой ключ после обрезки пробелов не ключ.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (!text.is_empty()).then(|| Self(text.to_string()))
    }

    /// Только для заголовка запроса.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Куда и как уходит запрос. Собирается один раз: правки настроек посреди запуска его не трогают.
pub struct LlmTarget {
    pub provider: Provider,
    pub profile: Profile,
    pub key: Option<ApiKey>,
    pub settings: ResolvedSettings,
}

/// Запрос без протокола.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub temperature: Temperature,
    pub max_tokens: u32,
}

/// Выбор профиля: разовый, потом профиль действия, потом первый в списке.
pub struct ProfileChoice;

impl ProfileChoice {
    /// `profiles` в порядке `position`. Профиль действия, которого уже нет, пропускается.
    pub fn resolve(one_off: Option<ProfileId>, settings: &ResolvedSettings, profiles: &[Profile]) -> Result<ProfileId, AppError> {
        let exists = |id: ProfileId| profiles.iter().any(|profile| profile.id() == id);
        if let Some(id) = one_off {
            return if exists(id) { Ok(id) } else { Err(AppError::ProfileNotFound) };
        }
        settings
            .profile
            .filter(|id| exists(*id))
            .or_else(|| profiles.first().map(Profile::id))
            .ok_or(AppError::NoProfiles)
    }
}

/// Собирает `LlmTarget` перед запуском: настройки действия из app.db и переопределения проекта.
pub struct LlmTargetService<'a> {
    pub state: &'a AppState,
}

impl LlmTargetService<'_> {
    pub fn target(&self, project: Option<&OpenProject>, action: Action, one_off: Option<ProfileId>) -> Result<LlmTarget, AppError> {
        let (profiles, app_settings) =
            self.state.app.read(|tx| Ok((ProfileRepo { conn: tx }.list()?, ActionRepo { conn: tx }.settings(action)?)))?;
        let project_settings = match project {
            Some(project) => self.project_settings(project, action, &profiles)?,
            None => ActionSettings::default(),
        };
        let settings = project_settings.or(app_settings).resolve(action);
        let id = ProfileChoice::resolve(one_off, &settings, &profiles)?;
        let profile = profiles.into_iter().find(|profile| profile.id() == id).ok_or(AppError::ProfileNotFound)?;
        let provider = self.state.app.read(|tx| ProviderRepo { conn: tx }.get(profile.provider_id()))?;
        // Нет ключа у провайдера, которому он нужен: запрос не уйдёт, `LlmClient` вернёт `LlmError::NoKey`.
        let key = self.state.secrets.get(provider.id())?;
        Ok(LlmTarget { provider, profile, key, settings: ResolvedSettings { profile: Some(id), ..settings } })
    }

    /// Переопределения проекта: неизвестный профиль читается как `None`, остальные поля действуют.
    fn project_settings(&self, project: &OpenProject, action: Action, profiles: &[Profile]) -> Result<ActionSettings, AppError> {
        let mut settings = project.project.read(|tx| ActionRepo { conn: tx }.settings(action))?;
        settings.profile = settings.profile.filter(|id| profiles.iter().any(|profile| profile.id() == *id));
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{BatchSize, LlmSettingsService, ProviderKind};
    use crate::projects::ProjectService;
    use crate::state::tests::{memory_secrets, ready_project};

    #[test]
    fn project_overrides_app_settings_and_unknown_profile_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), memory_secrets()).unwrap();
        let settings = LlmSettingsService { state: &state };
        let provider = Provider::new("LM Studio", ProviderKind::Local, "http://127.0.0.1:1234/v1", 0).unwrap();
        settings.save_provider(&provider, None).unwrap();
        let first = Profile::new(provider.id(), "", "first", 0).unwrap();
        let second = Profile::new(provider.id(), "", "second", 1).unwrap();
        settings.save_profile(&first).unwrap();
        settings.save_profile(&second).unwrap();
        let app = ActionSettings { profile: Some(second.id()), batch_size: Some(BatchSize::new(7).unwrap()), ..Default::default() };
        settings.save_action(Action::Lines, &app).unwrap();

        let project = ready_project(&state, "Игра");
        ProjectService { state: &state }.open(project.id()).unwrap();
        let open = state.open_project(project.id()).unwrap();
        let service = LlmTargetService { state: &state };

        let target = service.target(Some(&open), Action::Lines, None).unwrap();
        assert_eq!(target.profile.id(), second.id());
        assert_eq!(target.settings.batch_size.get(), 7);
        assert_eq!(target.settings.temperature, Action::Lines.default_temperature());
        assert!(target.key.is_none());

        // Профиль проекта, которого уже нет в приложении, пропускается, а размер пакета проекта действует.
        let gone = ActionSettings { profile: Some(ProfileId::generate()), batch_size: Some(BatchSize::new(2).unwrap()), ..Default::default() };
        open.project.write(|tx| ActionRepo { conn: tx }.save(Action::Lines, &gone)).unwrap();
        let target = service.target(Some(&open), Action::Lines, None).unwrap();
        assert_eq!(target.profile.id(), second.id());
        assert_eq!(target.settings.batch_size.get(), 2);

        // Без своего профиля действие берёт первый, разовый выбор важнее всего.
        assert_eq!(service.target(None, Action::Names, None).unwrap().profile.id(), first.id());
        assert_eq!(service.target(None, Action::Names, Some(second.id())).unwrap().profile.id(), second.id());
    }
}
