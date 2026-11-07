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
    #[error("Ответ модели обрезан лимитом")]
    Truncated,
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
        todo!()
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
    let mut url = url.clone();
    if url.host_str() == Some("localhost") {
        let _ = url.set_host(Some("127.0.0.1"));
    }
    url.set_query(None);
    url.set_fragment(None);
    let base = url.as_str().trim_end_matches('/');
    if base.ends_with("/v1") || base.ends_with("/openai") { base.to_string() } else { format!("{base}/v1") }
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

fn short(body: String) -> String {
    match body.char_indices().nth(ERROR_BODY_CHARS) {
        Some((end, _)) => format!("{}…", &body[..end]),
        None => body,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

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
