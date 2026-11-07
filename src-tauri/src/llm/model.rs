//! Провайдер (куда идёт запрос) и профиль (какая модель отвечает и с какими настройками).

use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::common::{ValidationError, uuid_id};

uuid_id!(
    /// Id провайдера, он же имя ключа в хранилище учётных данных.
    ProviderId
);
uuid_id!(ProfileId);

/// Запас лимита ответа на размышления, если в профиле не задан.
pub const DEFAULT_REASONING_TOKENS: u32 = 8192;
/// Сколько ждать ответа, если в профиле не задано.
pub const DEFAULT_TIMEOUT_SECS: u32 = 300;

/// Вид запроса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Локальный сервер (LM Studio, Ollama): Chat Completions без прокси.
    Local,
    /// ChatGPT: `max_completion_tokens` без `temperature`.
    OpenAi,
    /// Claude: Messages API.
    Claude,
    /// Gemini: OpenAI-совместимый адрес.
    Gemini,
    /// Другой OpenAI-совместимый сервер.
    Compatible,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::OpenAi => "openai",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Compatible => "compatible",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "local" => Ok(Self::Local),
            "openai" => Ok(Self::OpenAi),
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "compatible" => Ok(Self::Compatible),
            _ => Err(ValidationError::UnknownValue { field: "llm_providers.kind", value: text.to_string() }),
        }
    }
}

/// Провайдер: один на сервер или аккаунт. Уходит в окно, поэтому ключа в нём нет.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provider {
    id: ProviderId,
    position: u32,
    name: String,
    kind: ProviderKind,
    url: Url,
}

impl Provider {
    pub fn new(name: &str, kind: ProviderKind, url: &str, position: u32) -> Result<Self, ValidationError> {
        Self::restore(ProviderId::generate(), position, name, kind, url)
    }

    pub fn restore(id: ProviderId, position: u32, name: &str, kind: ProviderKind, url: &str) -> Result<Self, ValidationError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ValidationError::Empty("name"));
        }
        let url = Url::parse(url.trim())
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .ok_or_else(|| ValidationError::UnknownValue { field: "url", value: url.to_string() })?;
        Ok(Self { id, position, name: name.to_string(), kind, url })
    }

    /// Без ключа запрос не уйдёт. Локальному и совместимому серверу ключ не обязателен.
    pub fn needs_key(&self) -> bool {
        matches!(self.kind, ProviderKind::OpenAi | ProviderKind::Claude | ProviderKind::Gemini)
    }

    /// ChatGPT не принимает температуру: окно показывает поле неактивным.
    pub fn takes_temperature(&self) -> bool {
        self.kind != ProviderKind::OpenAi
    }

    pub fn id(&self) -> ProviderId {
        self.id
    }

    pub fn position(&self) -> u32 {
        self.position
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> ProviderKind {
        self.kind
    }

    pub fn url(&self) -> &Url {
        &self.url
    }
}

/// Поля тела запроса, которых приложение не задаёт: `top_p`, `reasoning_effort` и подобные.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Params(serde_json::Map<String, serde_json::Value>);

impl Params {
    /// Поля приложения, которые профиль не переопределяет.
    const RESERVED: &[&str] = &[
        "messages",
        "model",
        "system",
        "temperature",
        "max_tokens",
        "max_completion_tokens",
        "stream",
    ];

    /// Отбрасывает поля, совпадающие с полями приложения.
    pub fn new(mut fields: serde_json::Map<String, serde_json::Value>) -> Self {
        fields.retain(|key, _| !Self::RESERVED.contains(&key.as_str()));
        Self(fields)
    }

    pub fn fields(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
}

/// Профиль: модель на провайдере и то, что от неё зависит. Пустое поле значит значение из кода.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
    id: ProfileId,
    provider_id: ProviderId,
    position: u32,
    name: String,
    model: String,
    context_tokens: Option<u32>,
    reasoning_tokens: Option<u32>,
    timeout_secs: Option<u32>,
    params: Params,
}

impl Profile {
    pub fn new(provider: ProviderId, name: &str, model: &str, position: u32) -> Result<Self, ValidationError> {
        let name = name.trim();
        let model = model.trim();
        if model.is_empty() {
            return Err(ValidationError::Empty("model"));
        }
        let name = if name.is_empty() { model } else { name };
        Ok(Self {
            id: ProfileId::generate(),
            provider_id: provider,
            position,
            name: name.to_string(),
            model: model.to_string(),
            context_tokens: None,
            reasoning_tokens: None,
            timeout_secs: None,
            params: Params::default(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: ProfileId,
        provider_id: ProviderId,
        position: u32,
        name: &str,
        model: &str,
        context_tokens: Option<u32>,
        reasoning_tokens: Option<u32>,
        timeout_secs: Option<u32>,
        params: Params,
    ) -> Result<Self, ValidationError> {
        let mut profile = Self::new(provider_id, name, model, position)?;
        profile.id = id;
        profile.set_limits(context_tokens, reasoning_tokens, timeout_secs)?;
        profile.params = params;
        Ok(profile)
    }

    /// Окно контекста, запас на размышления и таймаут. Ноль в окне и таймауте недопустим.
    pub fn set_limits(&mut self, context: Option<u32>, reasoning: Option<u32>, timeout: Option<u32>) -> Result<(), ValidationError> {
        if context == Some(0) {
            return Err(ValidationError::OutOfRange("context_tokens"));
        }
        if timeout == Some(0) {
            return Err(ValidationError::OutOfRange("timeout_secs"));
        }
        self.context_tokens = context;
        self.reasoning_tokens = reasoning;
        self.timeout_secs = timeout;
        Ok(())
    }

    pub fn set_params(&mut self, params: Params) {
        self.params = params;
    }

    /// Окно контекста. Без него объём текста игры ограничивают нынешние пределы в символах.
    pub fn context_tokens(&self) -> Option<u32> {
        self.context_tokens
    }

    pub fn reasoning_tokens(&self) -> u32 {
        self.reasoning_tokens.unwrap_or(DEFAULT_REASONING_TOKENS)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).into())
    }

    pub fn id(&self) -> ProfileId {
        self.id
    }

    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    pub fn position(&self) -> u32 {
        self.position
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn params(&self) -> &Params {
        &self.params
    }
}
