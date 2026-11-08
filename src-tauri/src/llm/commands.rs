//! Команды настроек LLM: провайдеры, ключи, профили, настройки действий, список моделей.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::model::{DEFAULT_REASONING_TOKENS, DEFAULT_TIMEOUT_SECS};
use super::service::ProviderEntry;
use super::{
    Action, ActionSettings, ApiKey, BatchSize, LlmClient, LlmSettingsService, Params, Profile, ProfileId, Provider, ProviderId,
    ProviderKind, Temperature,
};
use crate::common::{AppError, ValidationError};
use crate::state::blocking;

/// Провайдер для окна. Ключа нет, только признак, что он задан.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub url: String,
    pub has_key: bool,
    /// Без ключа запрос не уйдёт.
    pub needs_key: bool,
    /// ChatGPT температуру не принимает.
    pub takes_temperature: bool,
}

impl ProviderDto {
    fn new(provider: &Provider, has_key: bool) -> Self {
        Self {
            id: provider.id().to_string(),
            name: provider.name().to_string(),
            kind: provider.kind(),
            url: provider.url().to_string(),
            has_key,
            needs_key: provider.needs_key(),
            takes_temperature: provider.takes_temperature(),
        }
    }
}

impl From<ProviderEntry> for ProviderDto {
    fn from(entry: ProviderEntry) -> Self {
        Self::new(&entry.provider, entry.has_key)
    }
}

/// Профиль. Пустое поле значит значение из кода.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub model: String,
    pub context_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub timeout_secs: Option<u32>,
    /// Дополнительные поля тела запроса, JSON-объект с отступами. Пустая строка: полей нет.
    pub params: String,
}

impl From<&Profile> for ProfileDto {
    fn from(profile: &Profile) -> Self {
        let fields = profile.params().fields();
        let params = if fields.is_empty() {
            String::new()
        } else {
            serde_json::to_string_pretty(fields).unwrap_or_default()
        };
        Self {
            id: profile.id().to_string(),
            provider_id: profile.provider_id().to_string(),
            name: profile.name().to_string(),
            model: profile.model().to_string(),
            context_tokens: profile.context_tokens(),
            reasoning_tokens: profile.reasoning_limit(),
            timeout_secs: profile.timeout_limit(),
            params,
        }
    }
}

/// Настройки действия в приложении и значения из кода, которые действуют при пустом поле.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionDto {
    pub id: Action,
    pub title: String,
    pub profile: Option<String>,
    pub temperature: Option<f64>,
    pub batch_size: Option<u32>,
    pub default_temperature: f64,
    pub default_batch_size: u32,
}

impl ActionDto {
    fn new(action: Action, settings: &ActionSettings) -> Self {
        Self {
            id: action,
            title: action.title().to_string(),
            profile: settings.profile.map(|id| id.to_string()),
            temperature: settings.temperature.map(Temperature::as_f64),
            batch_size: settings.batch_size.map(|size| size.get() as u32),
            default_temperature: action.default_temperature().as_f64(),
            default_batch_size: action.default_batch_size().get() as u32,
        }
    }
}

/// Всё окно настроек LLM.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsDto {
    pub providers: Vec<ProviderDto>,
    /// Общий список по порядку: первый берут действия без своего профиля.
    pub profiles: Vec<ProfileDto>,
    /// Все действия в порядке меню.
    pub actions: Vec<ActionDto>,
    /// Ключи лежат в хранилище системы. Иначе они живут до закрытия приложения.
    pub keys_persistent: bool,
    pub default_reasoning_tokens: u32,
    pub default_timeout_secs: u32,
}

/// Провайдер из окна. Без `id` создаётся новый.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub kind: ProviderKind,
    pub url: String,
    /// Новый ключ. Пусто: прежний ключ остаётся.
    pub key: Option<String>,
}

/// Профиль из окна. Без `id` создаётся новый в конце списка.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub id: Option<String>,
    pub provider_id: String,
    pub name: String,
    pub model: String,
    pub context_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub timeout_secs: Option<u32>,
    /// JSON-объект или пустая строка.
    pub params: String,
}

/// Настройки действия из окна. Пустое поле: значение из кода.
#[derive(Debug, Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionInput {
    pub profile: Option<String>,
    pub temperature: Option<f64>,
    pub batch_size: Option<u32>,
}

impl ActionInput {
    fn settings(&self) -> Result<ActionSettings, ValidationError> {
        Ok(ActionSettings {
            profile: self.profile.as_deref().map(ProfileId::parse).transpose()?,
            temperature: self.temperature.map(Temperature::new).transpose()?,
            batch_size: self.batch_size.map(|size| BatchSize::new(size.into())).transpose()?,
        })
    }
}

fn settings_dto(service: &LlmSettingsService) -> Result<LlmSettingsDto, AppError> {
    let settings = service.settings()?;
    Ok(LlmSettingsDto {
        providers: settings.providers.into_iter().map(ProviderDto::from).collect(),
        profiles: settings.profiles.iter().map(ProfileDto::from).collect(),
        actions: settings.actions.iter().map(|(action, settings)| ActionDto::new(*action, settings)).collect(),
        keys_persistent: service.state.secrets.is_persistent(),
        default_reasoning_tokens: DEFAULT_REASONING_TOKENS,
        default_timeout_secs: DEFAULT_TIMEOUT_SECS,
    })
}

/// Провайдер из полей окна: прежний сохраняет место в списке, новый встаёт в конец.
fn provider_from(service: &LlmSettingsService, input: &ProviderInput) -> Result<Provider, AppError> {
    match &input.id {
        Some(id) => {
            let old = service.provider(ProviderId::parse(id)?)?;
            Ok(Provider::restore(old.id(), old.position(), &input.name, input.kind, &input.url)?)
        }
        None => Ok(Provider::new(&input.name, input.kind, &input.url, service.next_provider_position()?)?),
    }
}

fn params_from(text: &str) -> Result<Params, ValidationError> {
    if text.trim().is_empty() {
        return Ok(Params::default());
    }
    match serde_json::from_str(text) {
        Ok(serde_json::Value::Object(fields)) => Ok(Params::new(fields)),
        Ok(_) => Err(ValidationError::BadParams("нужен объект в фигурных скобках".to_string())),
        Err(error) => Err(ValidationError::BadParams(error.to_string())),
    }
}

/// Провайдеры, профили и настройки действий.
#[tauri::command]
#[specta::specta]
pub async fn llm_settings(app: AppHandle) -> Result<LlmSettingsDto, String> {
    blocking(app, |state| settings_dto(&LlmSettingsService { state })).await
}

/// Создаёт или меняет провайдера. Ключ сохраняется, только если передан.
#[tauri::command]
#[specta::specta]
pub async fn save_llm_provider(app: AppHandle, provider: ProviderInput) -> Result<ProviderDto, String> {
    blocking(app, move |state| {
        let service = LlmSettingsService { state };
        let saved = provider_from(&service, &provider)?;
        let key = provider.key.as_deref().and_then(ApiKey::new);
        service.save_provider(&saved, key.as_ref())?;
        let has_key = key.is_some() || state.secrets.get(saved.id())?.is_some();
        Ok(ProviderDto::new(&saved, has_key))
    })
    .await
}

/// Удаляет провайдера с ключом и его профилями.
#[tauri::command]
#[specta::specta]
pub async fn delete_llm_provider(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| LlmSettingsService { state }.delete_provider(ProviderId::parse(&id)?)).await
}

/// Убирает ключ провайдера.
#[tauri::command]
#[specta::specta]
pub async fn forget_llm_key(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| LlmSettingsService { state }.forget_key(ProviderId::parse(&id)?)).await
}

/// Создаёт или меняет профиль.
#[tauri::command]
#[specta::specta]
pub async fn save_llm_profile(app: AppHandle, profile: ProfileInput) -> Result<ProfileDto, String> {
    blocking(app, move |state| {
        let service = LlmSettingsService { state };
        let provider = ProviderId::parse(&profile.provider_id)?;
        let mut saved = match &profile.id {
            Some(id) => {
                let old = service.profile(ProfileId::parse(id)?)?;
                Profile::restore(old.id(), provider, old.position(), &profile.name, &profile.model, None, None, None, Params::default())?
            }
            None => Profile::new(provider, &profile.name, &profile.model, service.next_profile_position()?)?,
        };
        saved.set_limits(profile.context_tokens, profile.reasoning_tokens, profile.timeout_secs)?;
        saved.set_params(params_from(&profile.params)?);
        service.save_profile(&saved)?;
        Ok(ProfileDto::from(&saved))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_llm_profile(app: AppHandle, id: String) -> Result<(), String> {
    blocking(app, move |state| LlmSettingsService { state }.delete_profile(ProfileId::parse(&id)?)).await
}

/// Новый порядок профилей: первый берут действия без своего профиля.
#[tauri::command]
#[specta::specta]
pub async fn reorder_llm_profiles(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    blocking(app, move |state| {
        let order = ids.iter().map(|id| ProfileId::parse(id)).collect::<Result<Vec<_>, _>>()?;
        LlmSettingsService { state }.reorder_profiles(&order)
    })
    .await
}

/// Настройки действия в приложении.
#[tauri::command]
#[specta::specta]
pub async fn save_llm_action(app: AppHandle, action: Action, settings: ActionInput) -> Result<ActionDto, String> {
    blocking(app, move |state| {
        let parsed = settings.settings()?;
        LlmSettingsService { state }.save_action(action, &parsed)?;
        Ok(ActionDto::new(action, &parsed))
    })
    .await
}

/// Модели провайдера: и сохранённого, и того, что ещё правится в окне.
/// Без нового ключа берётся сохранённый ключ провайдера.
#[tauri::command]
#[specta::specta]
pub async fn llm_models(app: AppHandle, provider: ProviderInput) -> Result<Vec<String>, String> {
    let (target, key) = blocking(app, move |state| {
        let id = provider.id.as_deref().map(ProviderId::parse).transpose()?;
        let target = match id {
            Some(id) => Provider::restore(id, 0, &provider.name, provider.kind, &provider.url)?,
            None => Provider::new(&provider.name, provider.kind, &provider.url, 0)?,
        };
        let key = match (provider.key.as_deref().and_then(ApiKey::new), id) {
            (Some(key), _) => Some(key),
            (None, Some(id)) => state.secrets.get(id)?,
            (None, None) => None,
        };
        Ok((target, key))
    })
    .await?;
    LlmClient::models(&target, key.as_ref()).await.map_err(|error| error.to_string())
}

/// Останавливает текущий запуск модели между запросами: готовое остаётся.
#[tauri::command]
#[specta::specta]
pub fn cancel_llm_run(state: tauri::State<'_, crate::state::AppState>) {
    state.llm_run.cancel();
}

/// Идёт ли запуск модели: окно, открытое заново, узнаёт, что место занято.
#[tauri::command]
#[specta::specta]
pub fn llm_run_active(state: tauri::State<'_, crate::state::AppState>) -> bool {
    state.llm_run.is_active()
}
