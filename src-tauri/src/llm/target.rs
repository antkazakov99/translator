//! Цель запроса: провайдер, профиль, ключ и итог настроек действия. Заменяет `ChatTarget` из 1.0.

use super::{Action, ActionSettings, Profile, ProfileId, Provider, ResolvedSettings, Temperature};
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
        todo!()
    }

    /// Переопределения проекта: неизвестный профиль читается как `None`, остальные поля действуют.
    fn project_settings(&self, project: &OpenProject, action: Action, profiles: &[Profile]) -> Result<ActionSettings, AppError> {
        todo!()
    }
}
