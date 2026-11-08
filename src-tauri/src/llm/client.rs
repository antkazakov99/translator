//! HTTP к моделям: `ChatRequest` в запрос нужного протокола и ответ обратно в текст.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;
use thiserror::Error;

use super::{ApiKey, ChatRequest, LlmTarget, Provider, ProviderKind};

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("Нет ключа API для «{0}»")]
    NoKey(String),
    #[error("Сервер не ответил: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Сервер вернул {status}: {body}")]
    Status { status: u16, body: String },
    #[error("Пустой ответ модели")]
    Empty,
    #[error("Модель потратила лимит ответа до текста: увеличьте запас на размышления в профиле")]
    Truncated,
    #[error("Модель отказалась обрабатывать этот текст")]
    Refused,
    #[error("Запрос не помещается в контекст модели: увеличьте окно контекста на сервере или уменьшите пакет")]
    ContextOverflow,
}

impl LlmError {
    /// Сбой, после которого есть смысл спросить ещё раз: сеть, перегрузка сервера, пустой ответ.
    /// Нет ключа, отказ модели, переполненный контекст и неверный запрос повтор не исправит.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Http(_) | Self::Empty | Self::Truncated => true,
            Self::Status { status, .. } => *status == 429 || *status >= 500,
            Self::NoKey(_) | Self::Refused | Self::ContextOverflow => false,
        }
    }
}

impl From<LlmError> for crate::common::AppError {
    fn from(error: LlmError) -> Self {
        Self::Llm(error.to_string())
    }
}

/// Протокол запроса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmClient {
    /// Claude Messages API.
    Messages,
    /// OpenAI-совместимый Chat Completions: ChatGPT, Gemini, локальные и совместимые серверы.
    ChatCompletions,
}

impl LlmClient {
    pub fn for_kind(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Claude => Self::Messages,
            ProviderKind::Local | ProviderKind::OpenAi | ProviderKind::Gemini | ProviderKind::Compatible => Self::ChatCompletions,
        }
    }

    /// Лимит ответа включает запас профиля на размышления, температуру не получает ChatGPT,
    /// поля `params` профиля добавляются в тело, локальный сервер идёт без прокси.
    pub async fn send(self, target: &LlmTarget, request: &ChatRequest) -> Result<String, LlmError> {
        let provider = &target.provider;
        if target.key.is_none() && provider.needs_key() {
            return Err(LlmError::NoKey(provider.name().to_string()));
        }
        let (url, body) = match self {
            Self::Messages => (format!("{}/messages", versioned_url(provider.url())), messages_body(target, request)),
            Self::ChatCompletions => (completions_url(provider.url()), completions_body(target, request)),
        };
        let client = http(provider.kind(), target.profile.timeout())?;
        let response = authorize(client.post(url), provider.kind(), target.key.as_ref()).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if body.contains("exceed_context_size") || body.contains("exceeds the available context size") {
                return Err(LlmError::ContextOverflow);
            }
            return Err(LlmError::Status { status: status.as_u16(), body: short(body) });
        }
        match self {
            Self::Messages => messages_text(response.json().await?),
            Self::ChatCompletions => completions_text(response.json().await?),
        }
    }

    /// Модели провайдера по алфавиту. Заодно проверка адреса и ключа.
    pub async fn models(provider: &Provider, key: Option<&ApiKey>) -> Result<Vec<String>, LlmError> {
        if key.is_none() && provider.needs_key() {
            return Err(LlmError::NoKey(provider.name().to_string()));
        }
        let url = models_url(provider);
        let request = http(provider.kind(), MODELS_TIMEOUT)?.get(url);
        let response = authorize(request, provider.kind(), key).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(LlmError::Status { status: status.as_u16(), body: short(response.text().await.unwrap_or_default()) });
        }
        let parsed: ModelsResponse = response.json().await?;
        // Gemini отдаёт полное имя ресурса, а в запросе ждёт короткое.
        let mut models: Vec<String> =
            parsed.data.into_iter().map(|model| model.id.trim_start_matches("models/").to_string()).collect();
        models.sort();
        models.dedup();
        Ok(models)
    }
}

/// Версия Messages API в заголовке `anthropic-version`.
const CLAUDE_API_VERSION: &str = "2026-10-21";
/// Список моделей отвечает быстро: долгое ожидание значит неверный адрес.
const MODELS_TIMEOUT: Duration = Duration::from_secs(6);
/// Сколько символов тела ошибки показывать.
const ERROR_BODY_CHARS: usize = 500;

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Deserialize)]
struct ModelItem {
    id: String,
}

/// Локальный сервер идёт мимо прокси: системный прокси не знает про 127.0.0.1. Внешние идут через него.
fn http(kind: ProviderKind, timeout: Duration) -> Result<reqwest::Client, LlmError> {
    let builder = reqwest::Client::builder().timeout(timeout);
    let builder = if kind == ProviderKind::Local { builder.no_proxy() } else { builder };
    Ok(builder.build()?)
}

/// Адрес с версией API: сервер можно указать и без неё, а у Gemini путь свой (`/v1beta/openai`).
/// `localhost` становится `127.0.0.1`: LM Studio и Ollama в Windows слушают только IPv4, а `localhost` уходит в `::1`.
fn versioned_url(url: &Url) -> String {
    let base = plain_url(url);
    if base.ends_with("/v1") || base.ends_with("/openai") { base } else { format!("{base}/v1") }
}

/// Адрес без запроса, фрагмента и `/` в конце, `localhost` заменён на `127.0.0.1`.
fn plain_url(url: &Url) -> String {
    let mut url = url.clone();
    if url.host_str() == Some("localhost") {
        let _ = url.set_host(Some("127.0.0.1"));
    }
    url.set_query(None);
    url.set_fragment(None);
    url.as_str().trim_end_matches('/').to_string()
}

/// Адрес списка моделей. Claude отдаёт его постранично, а нужен весь.
fn models_url(provider: &Provider) -> String {
    let base = versioned_url(provider.url());
    match provider.kind() {
        ProviderKind::Claude => format!("{base}/models?limit=1000"),
        _ => format!("{base}/models"),
    }
}

/// Подписывает запрос ключом так, как принято у сервиса.
fn authorize(request: reqwest::RequestBuilder, kind: ProviderKind, key: Option<&ApiKey>) -> reqwest::RequestBuilder {
    match (kind, key) {
        (_, None) => request,
        (ProviderKind::Claude, Some(key)) => {
            request.header("x-api-key", key.expose()).header("anthropic-version", CLAUDE_API_VERSION)
        }
        (_, Some(key)) => request.bearer_auth(key.expose()),
    }
}

/// Адрес Chat Completions. Адрес провайдера может уже вести на него целиком.
fn completions_url(url: &Url) -> String {
    let plain = plain_url(url);
    if plain.ends_with("/chat/completions") { plain } else { format!("{}/chat/completions", versioned_url(url)) }
}

/// Модели Claude до 4.6 не принимают `output_config.effort`.
const CLAUDE_WITHOUT_EFFORT: &[&str] = &["claude-3", "haiku-4", "sonnet-4-5", "sonnet-4-2", "opus-4-1", "opus-4-2"];

/// Тело Messages API. Параметры выборки Claude больше не принимает. Модели разбирают текст, а не решают задачи,
/// поэтому глубина размышления низкая, если профиль не задал свою.
fn messages_body(target: &LlmTarget, request: &ChatRequest) -> serde_json::Value {
    let model = target.profile.model();
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": request.max_tokens + target.profile.reasoning_tokens(),
        "system": request.system,
        "messages": [{ "role": "user", "content": request.user }],
    });
    if !CLAUDE_WITHOUT_EFFORT.iter().any(|old| model.contains(old)) {
        body["output_config"] = serde_json::json!({ "effort": "low" });
    }
    with_params(body, target)
}

/// Тело Chat Completions. ChatGPT не принимает у новых моделей `max_tokens` и температуру. Размышления входят
/// в лимит ответа и у локальных моделей: без запаса Gemma тратила его целиком и не писала ответ.
fn completions_body(target: &LlmTarget, request: &ChatRequest) -> serde_json::Value {
    let limit = request.max_tokens + target.profile.reasoning_tokens();
    let mut body = serde_json::json!({
        "model": target.profile.model(),
        "messages": [
            { "role": "system", "content": request.system },
            { "role": "user", "content": request.user },
        ],
    });
    if target.provider.kind() == ProviderKind::OpenAi {
        body["max_completion_tokens"] = limit.into();
    } else {
        body["max_tokens"] = limit.into();
    }
    if target.provider.takes_temperature() {
        body["temperature"] = request.temperature.as_f64().into();
    }
    with_params(body, target)
}

/// Поля профиля поверх тела. Поля приложения `Params` отбросил при сохранении.
fn with_params(mut body: serde_json::Value, target: &LlmTarget) -> serde_json::Value {
    if let Some(fields) = body.as_object_mut() {
        for (key, value) in target.profile.params().fields() {
            fields.insert(key.clone(), value.clone());
        }
    }
    body
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<MessagesBlock>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct MessagesBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

/// Текст ответа Claude: блоки размышлений в него не входят.
fn messages_text(response: MessagesResponse) -> Result<String, LlmError> {
    if response.stop_reason.as_deref() == Some("refusal") {
        return Err(LlmError::Refused);
    }
    let text: String =
        response.content.into_iter().filter(|block| block.kind == "text").filter_map(|block| block.text).collect();
    answer_text(text, response.stop_reason.as_deref())
}

#[derive(Deserialize)]
struct CompletionsResponse {
    #[serde(default)]
    choices: Vec<CompletionsChoice>,
}

#[derive(Deserialize)]
struct CompletionsChoice {
    message: CompletionsMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct CompletionsMessage {
    content: Option<String>,
}

fn completions_text(response: CompletionsResponse) -> Result<String, LlmError> {
    let choice = response.choices.into_iter().next().ok_or(LlmError::Empty)?;
    if choice.finish_reason.as_deref() == Some("content_filter") {
        return Err(LlmError::Refused);
    }
    answer_text(choice.message.content.unwrap_or_default(), choice.finish_reason.as_deref())
}

/// Обрезанный, но не пустой ответ отдаётся как есть: разбор ответа возьмёт из него готовые реплики.
fn answer_text(text: String, stop: Option<&str>) -> Result<String, LlmError> {
    let text = text.trim();
    match (text.is_empty(), stop) {
        (true, Some("length" | "max_tokens")) => Err(LlmError::Truncated),
        (true, _) => Err(LlmError::Empty),
        (false, _) => Ok(text.to_string()),
    }
}

fn short(body: String) -> String {
    match body.char_indices().nth(ERROR_BODY_CHARS) {
        Some((end, _)) => format!("{}…", &body[..end]),
        None => body,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::llm::{Action, ActionSettings, Profile, Temperature};

    /// Сервер модели для тестов: на каждый запрос отвечает `reply(тело запроса)` со статусом 200.
    /// Возвращает адрес с `/v1` и тела всех запросов. Поток живёт до конца тестов.
    pub(crate) fn fake_model(reply: impl Fn(&serde_json::Value) -> String + Send + 'static) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://localhost:{}/v1", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let answer = reply(&body);
                log.lock().unwrap().push(body);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (url, seen)
    }

    /// Ответ Chat Completions с текстом модели.
    pub(crate) fn completion(text: &str) -> String {
        serde_json::json!({ "choices": [{ "message": { "content": text }, "finish_reason": "stop" }] }).to_string()
    }

    fn target(kind: ProviderKind, url: &str, model: &str, key: Option<&str>) -> LlmTarget {
        let provider = provider(kind, url);
        let mut profile = Profile::new(provider.id(), "", model, 0).unwrap();
        profile.set_limits(None, Some(100), Some(5)).unwrap();
        let mut params = serde_json::Map::new();
        params.insert("top_p".to_string(), 0.9.into());
        profile.set_params(crate::llm::Params::new(params));
        LlmTarget { provider, profile, key: key.and_then(ApiKey::new), settings: ActionSettings::default().resolve(Action::Lines) }
    }

    fn request() -> ChatRequest {
        ChatRequest { system: "Система".to_string(), user: "Реплики".to_string(), temperature: Temperature::new(0.3).unwrap(), max_tokens: 1000 }
    }

    #[test]
    fn completions_get_limits_temperature_and_profile_params() {
        let (url, seen) = fake_model(|_| completion("  [{\"id\": \"1\"}]  "));
        let answer = tauri::async_runtime::block_on(LlmClient::ChatCompletions.send(&target(ProviderKind::Local, &url, "gemma", None), &request()));
        assert_eq!(answer.unwrap(), "[{\"id\": \"1\"}]");
        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(body["model"], "gemma");
        assert_eq!(body["max_tokens"], 1100, "лимит ответа с запасом на размышления");
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "Реплики");
    }

    #[test]
    fn chatgpt_gets_completion_limit_without_temperature() {
        let (url, seen) = fake_model(|_| completion("Ответ"));
        let target = target(ProviderKind::OpenAi, &url, "gpt-5", Some("sk-test"));
        tauri::async_runtime::block_on(LlmClient::ChatCompletions.send(&target, &request())).unwrap();
        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(body["max_completion_tokens"], 1100);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn claude_gets_system_apart_and_text_blocks_are_joined() {
        let (url, seen) = fake_model(|_| {
            serde_json::json!({
                "content": [{ "type": "thinking", "thinking": "..." }, { "type": "text", "text": "Ответ" }],
                "stop_reason": "end_turn"
            })
            .to_string()
        });
        let target = target(ProviderKind::Claude, &url, "claude-sonnet-5-5", Some("sk-ant"));
        let answer = tauri::async_runtime::block_on(LlmClient::Messages.send(&target, &request()));
        assert_eq!(answer.unwrap(), "Ответ");
        let body = seen.lock().unwrap()[0].clone();
        assert_eq!(body["system"], "Система");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["output_config"]["effort"], "low");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn empty_answers_and_missing_keys_are_errors() {
        let (url, _) = fake_model(|_| serde_json::json!({ "choices": [{ "message": { "content": "" }, "finish_reason": "length" }] }).to_string());
        let error = tauri::async_runtime::block_on(LlmClient::ChatCompletions.send(&target(ProviderKind::Local, &url, "gemma", None), &request()));
        assert!(matches!(error, Err(LlmError::Truncated)));
        let error = tauri::async_runtime::block_on(LlmClient::Messages.send(&target(ProviderKind::Claude, &url, "claude", None), &request()));
        assert!(matches!(error, Err(LlmError::NoKey(_))));
    }

    #[test]
    fn completions_url_keeps_a_full_endpoint() {
        let url = |text: &str| completions_url(&Url::parse(text).unwrap());
        assert_eq!(url("http://localhost:1234"), "http://127.0.0.1:1234/v1/chat/completions");
        assert_eq!(url("https://example.com/api/chat/completions"), "https://example.com/api/chat/completions");
        assert_eq!(url("https://generativelanguage.googleapis.com/v1beta/openai/"), "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions");
    }

    fn provider(kind: ProviderKind, url: &str) -> Provider {
        Provider::new("Сервер", kind, url, 0).unwrap()
    }

    #[test]
    fn models_url_adds_version_and_claude_page_size() {
        assert_eq!(
            models_url(&provider(ProviderKind::Claude, "https://api.anthropic.com")),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert_eq!(models_url(&provider(ProviderKind::OpenAi, "https://api.openai.com/v1/")), "https://api.openai.com/v1/models");
        assert_eq!(
            models_url(&provider(ProviderKind::Gemini, "https://generativelanguage.googleapis.com/v1beta/openai")),
            "https://generativelanguage.googleapis.com/v1beta/openai/models"
        );
        assert_eq!(models_url(&provider(ProviderKind::Local, "http://localhost:1234")), "http://127.0.0.1:1234/v1/models");
    }

    #[test]
    fn models_need_key_where_service_wants_one() {
        let error = tauri::async_runtime::block_on(LlmClient::models(&provider(ProviderKind::OpenAi, "https://api.openai.com/v1"), None));
        assert!(matches!(error, Err(LlmError::NoKey(_))));
    }

    /// Сервер на один ответ: возвращает адрес и поток, который отдаёт запрос целиком.
    fn serve_once(status: &'static str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://localhost:{}/v1", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).unwrap();
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..read]).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn models_come_sorted_with_short_names_and_bearer_key() {
        let (url, server) = serve_once("200 OK", r#"{"data":[{"id":"models/gemma"},{"id":"alpha"},{"id":"gemma"}]}"#);
        let key = ApiKey::new("sk-локальный").unwrap();
        let models = tauri::async_runtime::block_on(LlmClient::models(&provider(ProviderKind::Compatible, &url), Some(&key)));
        let request = server.join().unwrap();
        assert_eq!(models.unwrap(), ["alpha", "gemma"]);
        assert!(request.starts_with("GET /v1/models HTTP/1.1"));
        assert!(request.to_lowercase().contains("authorization: bearer sk-"));
    }

    #[test]
    fn models_report_server_error_with_body() {
        let (url, server) = serve_once("401 Unauthorized", r#"{"error":"bad key"}"#);
        let models = tauri::async_runtime::block_on(LlmClient::models(&provider(ProviderKind::Local, &url), None));
        server.join().unwrap();
        match models {
            Err(LlmError::Status { status, body }) => {
                assert_eq!(status, 401);
                assert!(body.contains("bad key"));
            }
            other => panic!("ждали ошибку статуса, пришло {:?}", other.map_err(|error| error.to_string())),
        }
    }
}
