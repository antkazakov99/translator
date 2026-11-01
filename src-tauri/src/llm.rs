#[cfg(test)]
use crate::glossary::NameForm;
use crate::glossary::{CharacterGlossaryEntry, TermGlossaryEntry, TokenGlossaryEntry};
use crate::scenes::{self, GameLines, ScriptLine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, warn};

/// Предел текста реплик на персонажа и на запрос: окно контекста модели не бесконечно.
const MAX_SAMPLE_TEXT: usize = 12_000;
/// Подробные описания длинные: больше персонажей в ответе не успевают и не помещаются.
const MAX_BATCH_CHARACTERS: usize = 3;
const CHARACTER_ANSWER_TOKENS: u32 = 3072;
/// Сколько имён переводится за запрос: локальная модель пишет 40 имён около трёх минут,
/// а таймаут запроса пять. Весь состав игры модель видит в каждом пакете.
const MAX_BATCH_NAMES: usize = 30;
const NAME_ANSWER_TOKENS: u32 = 3072;
/// Сколько токенов разбирается за запрос и сколько реплик модель видит на каждый.
const TOKEN_BATCH: usize = 10;
const TOKEN_SAMPLES: usize = 8;
const TOKEN_ANSWER_TOKENS: u32 = 1536;
/// Что видит модель на месте разбираемого токена.
const TOKEN_GAP: &str = "[___]";
/// Локальная модель пишет подробный ответ минутами.
const REQUEST_TIMEOUT_SECS: u64 = 300;
/// Запас для облачных моделей: рассуждения считаются в том же лимите, что и ответ, и могут съесть его целиком.
const REASONING_TOKENS: u32 = 8192;
const CLAUDE_API_VERSION: &str = "2026-10-21";
const SCENE_NOTE_TOKENS: u32 = 768;
const MAX_CONDENSE_ROUNDS: usize = 3;

/// Теги оформления вроде `{i}`. Подстановки `[name]` остаются: без них фраза теряет смысл.
static RE_TEXT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").unwrap());
static RE_TEXT_VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([A-Za-z_][\w.]*)(?:![a-z]+)?\]").unwrap());
static RE_STRING_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:(?:default|define)\s+|\$\s*)?(?:store\.)?([A-Za-z_][\w.]*)\s*=\s*_?\(?\s*(?:"([^"\\]*)"|'([^'\\]*)')\s*\)?\s*(?:#.*)?$"#).unwrap()
});
const MAX_VARIABLE_VALUES: usize = 3;
const MAX_VARIABLE_CHARS: usize = 40;
const MAX_PART_START: usize = 140;
const MAX_TERM_HINT_CHARS: usize = 1500;

/// Результат пакетного автозаполнения. Сбой отдельного пакета не отменяет
/// уже полученные переводы, поэтому число сбоев передаётся интерфейсу.
#[derive(Debug, Serialize)]
pub struct AutofillResult<T> {
    pub items: Vec<T>,
    pub failed_batches: usize,
}

/// Итог одного пакета: записи приходят в интерфейс сразу, не дожидаясь остальных.
#[derive(Debug, Clone, Serialize)]
pub struct BatchProgress<T> {
    pub batch: usize,
    pub total_batches: usize,
    pub failed_batches: usize,
    pub items: Vec<T>,
}

/// Персонаж после ответа модели и реплики, которые она видела.
#[derive(Debug, Clone, Serialize)]
pub struct CharacterResult {
    #[serde(flatten)]
    pub character: CharacterGlossaryEntry,
    pub samples: Vec<String>,
}

/// Что за имя пишет модель. Фамилия по-русски может зависеть от пола, а форма строится от готового написания.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NameKind {
    Name,
    Surname,
    Form,
}

/// Имя в запросе к модели. Повторы убирает интерфейс: только он знает, чьё это имя.
#[derive(Debug, Clone, Deserialize)]
pub struct NameRequest {
    pub id: String,
    pub name: String,
    pub kind: NameKind,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub base: String, // Готовое написание, от которого строится форма ("Анна Каранович")
}

/// Написание имени от модели. В глоссарий попадает после проверки пользователем.
#[derive(Debug, Clone, Serialize)]
pub struct NameSuggestion {
    pub id: String,
    pub translated: String,
    pub origin: String,
}

/// Предположение модели о токене. В глоссарий попадает после проверки пользователем.
#[derive(Debug, Clone, Serialize)]
pub struct TokenSuggestion {
    pub name: String,
    pub value: String,
    pub note: String,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<ClaudeOutputConfig>,
}

#[derive(Debug, Serialize)]
struct ClaudeOutputConfig {
    effort: &'static str,
}

#[derive(Debug, Deserialize)]
struct ClaudeBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeBlock>,
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Debug, Deserialize)]
struct CharacterLlmItem {
    alias: String,
    translated_name: Option<String>,
    gender: Option<String>,
    notes: Option<String>,
    speech: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NameLlmItem {
    id: Option<String>,
    origin: Option<String>,
    translated_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenLlmItem {
    id: Option<String>,
    value: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TermLlmItem {
    id: Option<String>,
    translated_term: Option<String>,
    category: Option<String>,
    notes: Option<String>,
}

/// Создаёт HTTP клиент с таймаутом и отключенным прокси для надёжной работы с локальными серверами
fn make_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Не удалось инициализировать HTTP-клиент: {}", e))
}

/// Клиент для внешнего сервиса: он может быть доступен только через системный прокси.
fn make_external_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Не удалось инициализировать HTTP-клиент: {}", e))
}

/// Нормализует базовый URL API (убирает завершающий слэш, преобразует localhost в 127.0.0.1 для избежания проблем с IPv6)
fn normalize_api_url(raw: &str) -> String {
    let mut trimmed = raw.trim().trim_end_matches('/').to_string();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        trimmed = format!("http://{}", trimmed);
    }
    // Локальные серверы LM Studio и Ollama на Windows часто слушают только IPv4 (127.0.0.1).
    // Резолв 'localhost' в ::1 (IPv6) приводит к ошибкам WSAECONNREFUSED / error sending request.
    if trimmed.starts_with("http://localhost:") {
        trimmed = trimmed.replacen("http://localhost:", "http://127.0.0.1:", 1);
    } else if trimmed == "http://localhost" {
        trimmed = "http://127.0.0.1".to_string();
    }
    trimmed
}

/// Адрес с версией API: сервер можно указать и без неё, а у Gemini путь свой (`/v1beta/openai`).
fn versioned_url(api_url: &str) -> String {
    let base_url = normalize_api_url(api_url);
    if base_url.ends_with("/v1") || base_url.ends_with("/openai") {
        base_url
    } else {
        format!("{}/v1", base_url)
    }
}

/// Подписывает запрос ключом так, как принято у сервиса.
fn authorize(request: reqwest::RequestBuilder, provider: Provider, key: Option<&str>) -> reqwest::RequestBuilder {
    match (provider, key) {
        (_, None) => request,
        (Provider::Claude, Some(key)) => {
            request.header("x-api-key", key).header("anthropic-version", CLAUDE_API_VERSION)
        }
        (_, Some(key)) => request.bearer_auth(key),
    }
}

/// Адрес списка моделей. Claude отдаёт его постранично, а нужен весь.
fn models_url(api_url: &str, provider: Provider) -> String {
    let base_url = versioned_url(api_url);
    match provider {
        Provider::Claude => format!("{}/models?limit=1000", base_url),
        _ => format!("{}/models", base_url),
    }
}

/// Проверяет доступность сервера и возвращает список доступных моделей
pub async fn check_models(api_url: &str, api_key: Option<&str>, provider: Provider) -> Result<Vec<String>, String> {
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    let client = if api_key.is_some() { make_external_client(6)? } else { make_client(6)? };
    let models_url = models_url(api_url, provider);

    let resp = authorize(client.get(&models_url), provider, api_key)
        .send()
        .await
        .map_err(|e| format!("Ошибка подключения к {}: {}", models_url, e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Сервер ответил ошибкой HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }

    let parsed: ModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Не удалось разобрать список моделей: {}", e))?;

    // Gemini может вернуть полное имя ресурса, а в запросе ждёт короткое
    let mut models: Vec<String> = parsed
        .data
        .into_iter()
        .map(|m| m.id.trim_start_matches("models/").to_string())
        .collect();
    models.sort();
    Ok(models)
}

/// Тело запроса chat completions. ChatGPT не принимает у новых моделей `max_tokens`, а `temperature`
/// поддерживают не все его модели, поэтому там остаётся значение по умолчанию.
fn chat_request(target: &ChatTarget, system_prompt: &str, user_prompt: &str, temperature: f32, max_tokens: u32) -> ChatCompletionRequest {
    let (temperature, max_tokens, max_completion_tokens) = match target.provider {
        Provider::Openai => (None, None, Some(max_tokens + REASONING_TOKENS)),
        Provider::Gemini => (Some(temperature), Some(max_tokens + REASONING_TOKENS), None),
        _ => (Some(temperature), Some(max_tokens), None),
    };
    ChatCompletionRequest {
        model: target.model.clone(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            },
        ],
        temperature,
        max_tokens,
        max_completion_tokens,
    }
}

/// Тело запроса к Claude. Параметры выборки API больше не принимает. Модели разбирают текст, а не решают
/// задачи, поэтому глубина размышления низкая; старые модели этот параметр отвергают.
fn claude_request(target: &ChatTarget, system_prompt: &str, user_prompt: &str, max_tokens: u32) -> ClaudeRequest {
    let takes_effort = !["claude-3", "haiku-4", "sonnet-4-5", "sonnet-4-2", "opus-4-1", "opus-4-2"]
        .iter()
        .any(|old| target.model.contains(old));
    ClaudeRequest {
        model: target.model.clone(),
        max_tokens: max_tokens + REASONING_TOKENS,
        system: system_prompt.to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: user_prompt.to_string(),
        }],
        output_config: takes_effort.then_some(ClaudeOutputConfig { effort: "low" }),
    }
}

/// Текст ответа Claude: блоки рассуждений в него не входят.
fn claude_text(response: ClaudeResponse) -> Result<String, String> {
    if response.stop_reason.as_deref() == Some("refusal") {
        return Err("Модель отказалась обрабатывать этот текст".to_string());
    }
    let text: String = response
        .content
        .into_iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text)
        .collect();
    Ok(text.trim().to_string())
}

/// Отправляет запрос к модели и превращает сбой сети или ответ с ошибкой в сообщение.
async fn send_to_model(request: reqwest::RequestBuilder, endpoint: &str) -> Result<reqwest::Response, String> {
    let response = request
        .send()
        .await
        .map_err(|e| {
            let mut msg = format!("Запрос к модели не удался ({}): {}", endpoint, e);
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                msg.push_str(&format!(" -> {}", s));
                source = s.source();
            }
            msg
        })?;

    if !response.status().is_success() {
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("LLM API вернул ошибку: {}", err_text));
    }
    Ok(response)
}

/// Выполняет один запрос к модели: Claude напрямую, остальные через chat completions
async fn call_chat_completion(
    target: &ChatTarget,
    system_prompt: &str,
    user_prompt: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, String> {
    if target.api_url.trim().is_empty() {
        return Err("Адрес сервера не задан в настройках LLM".to_string());
    }
    if target.model.trim().is_empty() {
        return Err("Модель не выбрана в настройках LLM".to_string());
    }
    let api_key = target.key();
    let client = if api_key.is_some() {
        make_external_client(REQUEST_TIMEOUT_SECS)?
    } else {
        make_client(REQUEST_TIMEOUT_SECS)?
    };

    if target.provider == Provider::Claude {
        let endpoint = format!("{}/messages", versioned_url(&target.api_url));
        let request = authorize(client.post(&endpoint), target.provider, api_key)
            .json(&claude_request(target, system_prompt, user_prompt, max_tokens));
        let answer: ClaudeResponse = send_to_model(request, &endpoint)
            .await?
            .json()
            .await
            .map_err(|e| format!("Не удалось прочитать ответ LLM: {}", e))?;
        return claude_text(answer);
    }

    let base_url = normalize_api_url(&target.api_url);
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url
    } else {
        format!("{}/chat/completions", versioned_url(&target.api_url))
    };
    let request = authorize(client.post(&endpoint), target.provider, api_key)
        .json(&chat_request(target, system_prompt, user_prompt, temperature, max_tokens));
    let response = send_to_model(request, &endpoint).await?;

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Не удалось прочитать ответ LLM: {}", e))?;

    let choice = completion
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "LLM вернул пустой список вариантов ответов (choices пуст)".to_string())?;

    let text = choice
        .message
        .content
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(text)
}

/// Обрабатывает сбой запроса: без готовых пакетов ошибка уходит пользователю, иначе
/// готовое сохраняется, а оставшиеся пакеты считаются сбойными. Дальше обработка
/// не идёт: у недоступного сервера каждый запрос ждал бы таймаут.
fn remaining_batches(error: String, batch: usize, total: usize) -> Result<usize, String> {
    if batch == 0 {
        return Err(error);
    }
    warn!("Запрос к LLM не удался на пакете {} из {}: {}", batch + 1, total, error);
    Ok(total - batch)
}

/// Находит в ответе нейросети первый JSON-массив объектов: вне зависимости от
/// Markdown-блоков, пояснений и объекта-обёртки вида `{"items":[...]}`
fn extract_json_content(raw: &str) -> &str {
    let trimmed = raw.trim();
    for (start, _) in trimmed.match_indices('[') {
        let mut values =
            serde_json::Deserializer::from_str(&trimmed[start..]).into_iter::<serde_json::Value>();
        if let Some(Ok(serde_json::Value::Array(items))) = values.next()
            && items.iter().all(serde_json::Value::is_object)
        {
            return &trimmed[start..start + values.byte_offset()];
        }
    }
    trimmed
}

/// Берёт строки равномерно по списку, чтобы они оказались из разных частей игры.
fn spread(lines: &[&String], count: usize) -> Vec<String> {
    let count = count.min(lines.len());
    (0..count).map(|i| lines[i * lines.len() / count].clone()).collect()
}

/// Значения строковых переменных из скриптов: `default`, `define` и присваивания.
/// Значений может быть несколько, если игра меняет переменную по ходу сюжета.
pub fn string_variables(scripts: &[PathBuf]) -> HashMap<String, Vec<String>> {
    let mut variables: HashMap<String, Vec<String>> = HashMap::new();
    for path in scripts {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        for line in content.lines() {
            let Some(caps) = RE_STRING_ASSIGNMENT.captures(line) else {
                continue;
            };
            let value = caps.get(2).or_else(|| caps.get(3)).map_or("", |value| value.as_str()).trim();
            if value.is_empty() || value.chars().count() > MAX_VARIABLE_CHARS {
                continue;
            }
            let values = variables.entry(caps[1].to_string()).or_default();
            if !values.iter().any(|known| known == value) {
                values.push(value.to_string());
            }
        }
    }
    variables
}

/// Что модель увидит вместо токенов при чтении сцен, по именам.
pub type TokenNotes = HashMap<String, String>;

/// Имя персонажа из глоссария.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnownName {
    pub original: String,
    pub translated: String,
    pub gender: String,
}

impl KnownName {
    fn gender_word(&self) -> Option<&'static str> {
        match self.gender.as_str() {
            "male" => Some("male"),
            "female" => Some("female"),
            "neuter" => Some("neuter"),
            _ => None,
        }
    }
}

/// Что модель получает сверх текста игры.
#[derive(Debug, Default, Deserialize)]
pub struct ModelHints {
    pub tokens: Vec<TokenGlossaryEntry>,
    pub names: Vec<KnownName>,
    #[serde(default)]
    pub terms: Vec<TermGlossaryEntry>,
}

/// Значение подставляется как есть, а описание идёт в скобках, как поясняющий текст.
/// Имени токена здесь нет в обоих случаях: модель верит ему больше, чем пояснению.
pub fn token_notes(tokens: &[TokenGlossaryEntry]) -> TokenNotes {
    tokens
        .iter()
        .filter_map(|token| match (token.value.trim(), token.note.trim()) {
            ("", "") => None,
            ("", note) => Some((token.name.clone(), format!("[{note}]"))),
            (value, _) => Some((token.name.clone(), value.to_string())),
        })
        .collect()
}

/// Токен из реплик игры.
#[derive(Debug, Clone, Serialize)]
pub struct TextToken {
    pub name: String,
    /// Сколько реплик его содержит.
    pub lines: usize,
    /// Значения из скриптов.
    pub values: Vec<String>,
    pub example: String,
}

/// Все токены из реплик, начиная с самых частых.
pub fn text_tokens(game: &GameLines) -> Vec<TextToken> {
    let mut found: HashMap<String, TextToken> = HashMap::new();
    for line in &game.lines {
        let text = RE_TEXT_TAG.replace_all(&line.text, "").replace('\n', " ");
        let names: HashSet<&str> = RE_TEXT_VARIABLE
            .captures_iter(&text)
            .filter_map(|caps| caps.get(1))
            .map(|name| name.as_str())
            .collect();
        for name in names {
            let token = found.entry(name.to_string()).or_insert_with(|| TextToken {
                name: name.to_string(),
                lines: 0,
                values: game.variables.get(name).cloned().unwrap_or_default(),
                example: text.trim().to_string(),
            });
            token.lines += 1;
        }
    }
    let mut tokens: Vec<TextToken> = found.into_values().collect();
    tokens.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.name.cmp(&b.name)));
    tokens
}

/// Подставляет значения переменных. Значение из глоссария важнее скриптов и заменяет токен
/// целиком, как в игре: если модель видит имя `[landlady]`, она верит ему больше, чем пояснению.
/// Переменная с неясным значением остаётся как есть: модели сказано не делать выводов из её имени.
fn resolve_variables(text: &str, variables: &HashMap<String, Vec<String>>, tokens: &TokenNotes) -> String {
    RE_TEXT_VARIABLE
        .replace_all(text, |caps: &regex::Captures| {
            if let Some(value) = tokens.get(&caps[1]) {
                return value.clone();
            }
            match variables.get(&caps[1]) {
                Some(values) if values.len() <= MAX_VARIABLE_VALUES => values.join("/"),
                _ => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// Делит персонажей на запросы так, чтобы их реплики уместились в окно контекста.
fn character_batches(
    indices: &[usize],
    characters: &[CharacterGlossaryEntry],
    samples: &HashMap<String, Vec<String>>,
) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut text = 0;
    for &idx in indices {
        let size: usize = samples
            .get(&characters[idx].alias.to_lowercase())
            .into_iter()
            .flatten()
            .map(|line| line.chars().count())
            .sum();
        match batches.last_mut() {
            Some(batch) if batch.len() < MAX_BATCH_CHARACTERS && text + size <= MAX_SAMPLE_TEXT => {
                batch.push(idx);
                text += size;
            }
            _ => {
                batches.push(vec![idx]);
                text = size;
            }
        }
    }
    batches
}

/// Упомянуто ли имя отдельным словом: Ana не должна находиться в Banana.
fn mentions(text: &str, name: &str) -> bool {
    !name.is_empty()
        && text.match_indices(name).any(|(start, _)| {
            let before = text[..start].chars().next_back();
            let after = text[start + name.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
}

/// Персонажи глоссария, упомянутые в тексте.
fn mentioned_names<'a>(text: &str, names: &'a [KnownName]) -> Vec<&'a KnownName> {
    let mut seen = HashSet::new();
    names
        .iter()
        .filter(|name| seen.insert(name.original.as_str()) && mentions(text, &name.original))
        .collect()
}

/// Переводы имён из глоссария для тех, кто упомянут в тексте. Строка не входит в части сцен:
/// с уточнённым переводом имени готовые заметки остаются действительными. Пол идёт отдельной
/// строкой: пометку рядом с именем модель переписывала в ответ.
fn name_hint(text: &str, names: &[KnownName]) -> Option<String> {
    let named = mentioned_names(text, names);
    if named.is_empty() {
        return None;
    }
    let pairs: Vec<String> = named.iter().map(|name| format!("{} = {}", name.original, name.translated)).collect();
    let genders: Vec<String> = named
        .iter()
        .filter_map(|name| name.gender_word().map(|gender| format!("{} {gender}", name.original)))
        .collect();
    let mut hint = format!("Russian names: {}", pairs.join(", "));
    if !genders.is_empty() {
        hint.push_str(&format!("\nGender: {}", genders.join(", ")));
    }
    Some(hint)
}

/// Заполненные термины глоссария, которые встречаются в тексте. Как и имена, в части сцен они
/// не входят. Объём ограничен: итоговому запросу достаются термины со всего текста персонажа.
fn mentioned_terms(text: &str, terms: &[TermGlossaryEntry]) -> Vec<String> {
    let text = text.to_lowercase();
    let mut size = 0;
    terms
        .iter()
        .flat_map(|term| {
            let main = term.translated_term.trim();
            std::iter::once((term.source_term.trim(), main, term)).chain(term.forms.iter().map(move |form| {
                let translated = form.translated.trim();
                (form.source.trim(), if translated.is_empty() { main } else { translated }, term)
            }))
        })
        .filter(|(source, translated, _)| !translated.is_empty() && mentions(&text, &source.to_lowercase()))
        .map(|(source, translated, term)| {
            let pair = format!("{source} = {translated}");
            match term.notes.trim() {
                "" => pair,
                notes => format!("{pair} ({notes})"),
            }
        })
        .take_while(|pair| {
            size += pair.chars().count() + 2;
            size <= MAX_TERM_HINT_CHARS
        })
        .collect()
}

fn term_hint(text: &str, terms: &[TermGlossaryEntry]) -> Option<String> {
    let pairs = mentioned_terms(text, terms);
    (!pairs.is_empty()).then(|| format!("Known terms: {}", pairs.join(", ")))
}

/// Подсказки из глоссария для этого текста.
fn glossary_hints(text: &str, hints: &ModelHints) -> Vec<String> {
    name_hint(text, &hints.names).into_iter().chain(term_hint(text, &hints.terms)).collect()
}

/// Текст реплики в том виде, в каком его видит модель.
fn sample_text(text: &str, variables: &HashMap<String, Vec<String>>, tokens: &TokenNotes) -> String {
    let text = RE_TEXT_TAG.replace_all(text, "").replace('\n', " ");
    resolve_variables(text.trim(), variables, tokens)
}

/// Тексты сцен в виде «Говорящий: реплика».
fn scene_transcripts(
    game: &GameLines,
    characters: &[CharacterGlossaryEntry],
    scenes: &HashSet<String>,
    tokens: &TokenNotes,
) -> HashMap<String, Vec<String>> {
    let names: HashMap<String, &str> = characters
        .iter()
        .flat_map(|c| c.aliases().into_iter().map(|alias| (alias.to_lowercase(), c.original_name.as_str())))
        .collect();
    let mut by_scene: HashMap<String, Vec<String>> = HashMap::new();
    for line in &game.lines {
        let key = line.key();
        if !scenes.contains(&key) {
            continue;
        }
        let alias = line.alias.as_ref().map(|alias| alias.to_lowercase());
        let speaker = alias
            .as_ref()
            .and_then(|alias| names.get(alias).copied())
            .or(line.speaker.as_deref())
            .or(line.alias.as_deref())
            .unwrap_or("Narrator");
        by_scene
            .entry(key)
            .or_default()
            .push(format!("{}: {}", speaker, sample_text(&line.text, &game.variables, tokens)));
    }
    by_scene
}

/// Строка между сценами внутри части: они могут быть далеко друг от друга в игре.
const SCENE_BREAK: &str = "---\n";

/// Сколько следующих сцен просматривается, чтобы заполнить часть до конца.
const SCENE_WINDOW: usize = 8;

/// Меньше этого в конце длинной сцены модели не хватает контекста.
const MIN_SCENE_TAIL: usize = 3000;

/// Раскладывает сцены по частям и возвращает их номера. Часть дополняется подходящими сценами
/// из ближайших: так частей меньше, а порядок сюжета нарушается только в пределах окна.
pub fn pack_scenes(sizes: &[usize], window: usize) -> Vec<Vec<usize>> {
    let mut pending: Vec<usize> = (0..sizes.len()).collect();
    let mut parts = Vec::new();
    while !pending.is_empty() {
        let first = pending.remove(0);
        let mut part = vec![first];
        let mut size = sizes[first];
        let mut position = 0;
        for _ in 0..window {
            let Some(&scene) = pending.get(position) else {
                break;
            };
            if size + SCENE_BREAK.len() + sizes[scene] <= MAX_SAMPLE_TEXT {
                size += SCENE_BREAK.len() + sizes[scene];
                part.push(pending.remove(position));
            } else {
                position += 1;
            }
        }
        part.sort_unstable();
        parts.push(part);
    }
    parts
}

/// Режет сцену на куски по репликам, если она не помещается в запрос. Короткий последний кусок
/// начинается с конца предыдущего: без начала разговора модель видела бы обрывок.
fn split_scene(lines: &[String]) -> Vec<String> {
    let length = |line: &String| line.chars().count() + 1;
    let mut parts: Vec<Vec<&String>> = vec![Vec::new()];
    let mut size = 0;
    for line in lines {
        if size > 0 && size + length(line) > MAX_SAMPLE_TEXT {
            parts.push(Vec::new());
            size = 0;
        }
        parts.last_mut().expect("список кусков не пуст").push(line);
        size += length(line);
    }
    if let [.., previous, tail] = parts.as_mut_slice() {
        let mut start = previous.len();
        while start > 0 && size < MIN_SCENE_TAIL {
            start -= 1;
            size += length(previous[start]);
        }
        tail.splice(0..0, previous[start..].iter().copied());
    }
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .map(|part| part.iter().map(|line| format!("{line}\n")).collect())
        .collect()
}

/// Сцена внутри части. Файл и путь нужны, чтобы открыть её в редакторе.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PartScene {
    pub scene: String,
    pub file: String,
    pub path: String,
}

/// Часть сцен персонажа: текст для модели, сцены, из которых он собран, и токены этих сцен.
pub struct ScenePart {
    pub text: String,
    pub scenes: Vec<PartScene>,
    pub tokens: Vec<TokenGlossaryEntry>,
}

/// Токены реплик по сценам. Пустое значение у тех, что модель увидит как есть.
fn scene_tokens(
    game: &GameLines,
    tokens: &TokenNotes,
    scenes: &HashSet<String>,
) -> HashMap<String, Vec<TokenGlossaryEntry>> {
    let mut found: HashMap<String, Vec<TokenGlossaryEntry>> = HashMap::new();
    for line in &game.lines {
        let key = line.key();
        if !scenes.contains(&key) {
            continue;
        }
        for caps in RE_TEXT_VARIABLE.captures_iter(&line.text) {
            let name = &caps[1];
            let note = tokens.get(name).cloned().unwrap_or_default();
            let resolved = game.variables.get(name).is_some_and(|values| values.len() <= MAX_VARIABLE_VALUES);
            if !note.is_empty() || !resolved {
                found.entry(key.clone()).or_default().push(TokenGlossaryEntry { name: name.to_string(), note, ..Default::default() });
            }
        }
    }
    found
}

/// Делит все включённые сцены персонажа на части, каждая из которых помещается в запрос.
/// Части состоят из целых сцен. По репликам режется только сцена, которая сама не помещается.
/// Пояснения к токенам лежат в самой части: с новым пояснением её заметка считается устаревшей.
pub fn scene_parts(game: &GameLines, tokens: &TokenNotes, character: &CharacterGlossaryEntry) -> Vec<ScenePart> {
    let included: Vec<_> = scenes::character_scenes(game, &character.aliases(), &character.names())
        .into_iter()
        .filter(|scene| !character.excluded_scenes.contains(&scene.key))
        .collect();
    parts_of_scenes(game, tokens, &included, std::slice::from_ref(character))
}

/// То же для термина: его сцены это реплики, где он встречается.
pub fn term_scene_parts(game: &GameLines, tokens: &TokenNotes, term: &TermGlossaryEntry) -> Vec<ScenePart> {
    let included: Vec<_> = scenes::term_scenes(game, &term.names())
        .into_iter()
        .filter(|scene| !term.excluded_scenes.contains(&scene.key))
        .collect();
    parts_of_scenes(game, tokens, &included, &[])
}

fn parts_of_scenes(
    game: &GameLines,
    tokens: &TokenNotes,
    included: &[scenes::CharacterScene],
    characters: &[CharacterGlossaryEntry],
) -> Vec<ScenePart> {
    let keys = included.iter().map(|scene| scene.key.clone()).collect();
    let transcripts = scene_transcripts(game, characters, &keys, tokens);
    let by_scene = scene_tokens(game, tokens, &keys);
    let pieces: Vec<(String, &scenes::CharacterScene)> = included
        .iter()
        .filter_map(|scene| transcripts.get(&scene.key).map(|lines| (lines, scene)))
        .flat_map(|(lines, scene)| split_scene(lines).into_iter().map(move |piece| (piece, scene)))
        .collect();
    let sizes: Vec<usize> = pieces.iter().map(|(piece, _)| piece.chars().count()).collect();
    pack_scenes(&sizes, SCENE_WINDOW)
        .into_iter()
        .map(|part| {
            let text = part.iter().map(|&index| pieces[index].0.as_str()).collect::<Vec<_>>().join(SCENE_BREAK);
            let mut scenes: Vec<PartScene> = Vec::new();
            let mut part_tokens: Vec<TokenGlossaryEntry> = Vec::new();
            for &index in &part {
                let source = pieces[index].1;
                part_tokens.extend(by_scene.get(&source.key).into_iter().flatten().cloned());
                let scene = PartScene {
                    scene: source.scene.clone(),
                    file: source.file.clone(),
                    path: source.path.clone(),
                };
                if !scenes.contains(&scene) {
                    scenes.push(scene);
                }
            }
            part_tokens.sort_by(|a, b| a.name.cmp(&b.name));
            part_tokens.dedup();
            ScenePart { text, scenes, tokens: part_tokens }
        })
        .collect()
}

pub fn scene_chunks(game: &GameLines, tokens: &TokenNotes, character: &CharacterGlossaryEntry) -> Vec<String> {
    scene_parts(game, tokens, character).into_iter().map(|part| part.text).collect()
}

pub fn term_scene_chunks(game: &GameLines, tokens: &TokenNotes, term: &TermGlossaryEntry) -> Vec<String> {
    term_scene_parts(game, tokens, term).into_iter().map(|part| part.text).collect()
}

/// Оставляет `count` частей, равномерно по игре, но не меньше готовых: прочитанное не пропадает.
/// Меньший набор всегда входит в больший, поэтому при сдвиге ползунка готовые заметки подходят снова.
pub fn pick_chunks(chunks: Vec<String>, count: usize, ready: &[bool]) -> Vec<String> {
    let is_ready = |index: usize| ready.get(index).copied().unwrap_or(false);
    let ready_count = (0..chunks.len()).filter(|index| is_ready(*index)).count();
    let mut picked: Vec<usize> = (0..chunks.len()).collect();
    picked.sort_by_key(|index| (!is_ready(*index), index.reverse_bits()));
    picked.truncate(count.max(ready_count).max(1));
    picked.sort_unstable();
    picked.into_iter().map(|index| chunks[index].clone()).collect()
}

/// Ход обработки сцен персонажа.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryProgress {
    /// `scenes`, `condense`, `final` или `single`, если всё делает один запрос.
    pub stage: String,
    pub done: usize,
    pub total: usize,
}

fn note_key(model: &str, alias: &str, chunk: &str) -> String {
    format!("{:016x}", crate::stable_hash_str(&format!("{model}\n{alias}\n{chunk}")))
}

/// Часть сцен персонажа с заметкой модели и тем, что уйдёт вместе с ней в запрос.
#[derive(Debug, Clone, Serialize)]
pub struct CharacterPart {
    pub key: String,
    /// Начало текста части, чтобы её можно было узнать.
    pub start: String,
    pub note: Option<String>,
    /// Модель не нашла в части сведений, в итог такая заметка не попадает.
    pub empty: bool,
    pub scenes: Vec<PartScene>,
    /// Персонажи, чьи имена получит модель.
    pub names: Vec<KnownName>,
    pub terms: Vec<String>,
    /// Токены части. Пустое значение: модель увидит токен как есть и не узнает, что за ним стоит.
    pub tokens: Vec<TokenGlossaryEntry>,
}

/// Части записи с их заметками от этой модели. `owner` это хозяин заметок, `name` имя записи в запросе.
pub fn character_parts(
    parts: Vec<ScenePart>,
    model: &str,
    owner: &str,
    name: &str,
    notes_file: &Path,
    hints: &ModelHints,
) -> Vec<CharacterPart> {
    let stored = read_scene_notes(notes_file);
    parts
        .into_iter()
        .map(|part| {
            let key = note_key(model, owner, &part.text);
            let note = stored.get(&key).map(|note| strip_old_marker(note).to_string());
            let empty = note.as_deref().is_some_and(|note| is_blank_note(note) || is_refusal(note));
            let start = part.text.lines().next().unwrap_or_default().chars().take(MAX_PART_START).collect();
            let sent = format!("{name}\n{}", part.text);
            let names = mentioned_names(&sent, &hints.names).into_iter().cloned().collect();
            let terms = mentioned_terms(&sent, &hints.terms);
            CharacterPart { key, start, note, empty, scenes: part.scenes, names, terms, tokens: part.tokens }
        })
        .collect()
}

/// Забывает заметки: следующий запуск прочитает эти части заново.
pub fn forget_notes(notes_file: &Path, keys: &[String]) -> Result<(), String> {
    let mut stored = read_scene_notes(notes_file);
    keys.iter().for_each(|key| {
        stored.remove(key);
    });
    write_scene_notes(notes_file, &stored)
}

/// Какие части уже прочитаны этой моделью.
pub fn ready_chunks(chunks: &[String], model: &str, alias: &str, notes_file: &Path) -> Vec<bool> {
    let stored = read_scene_notes(notes_file);
    chunks.iter().map(|chunk| stored.contains_key(&note_key(model, alias, chunk))).collect()
}

fn read_scene_notes(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_scene_notes(path: &Path, notes: &HashMap<String, String>) -> Result<(), String> {
    let json = serde_json::to_string(notes)
        .map_err(|e| format!("Не удалось подготовить заметки: {}", e))?;
    crate::rpa::write_atomic(path, |out: &mut File| {
        out.write_all(json.as_bytes())
            .map_err(|e| format!("Не удалось сохранить заметки {:?}: {}", path, e))
    })
}

/// Сжимает заметки, пока они не поместятся в итоговый запрос.
async fn condense_notes(
    target: &ChatTarget,
    mut notes: Vec<String>,
    on_progress: &(impl Fn(SummaryProgress) + Send + Sync),
) -> Result<Vec<String>, String> {
    let size = |notes: &[String]| notes.iter().map(|note| note.chars().count()).sum::<usize>();
    let system_prompt = "You merge notes about one character or term of a visual novel. Write in Russian. \
        Keep concrete facts: occupation, relations, events, manner of speech, what a thing is and what it does. Drop repetitions. \
        Respond with plain text only.";

    for _ in 0..MAX_CONDENSE_ROUNDS {
        if size(&notes) <= MAX_SAMPLE_TEXT {
            break;
        }
        let mut groups: Vec<Vec<String>> = Vec::new();
        let mut group_size = 0;
        for note in notes {
            let length = note.chars().count();
            match groups.last_mut() {
                Some(group) if group_size + length <= MAX_SAMPLE_TEXT => {
                    group.push(note);
                    group_size += length;
                }
                _ => {
                    groups.push(vec![note]);
                    group_size = length;
                }
            }
        }
        let total = groups.len();
        let mut condensed = Vec::new();
        for (done, group) in groups.iter().enumerate() {
            on_progress(SummaryProgress { stage: "condense".to_string(), done, total });
            let prompt = format!("Notes:\n\n{}\n\nMerge them:", group.join("\n\n"));
            condensed.push(call_chat_completion(target, system_prompt, &prompt, 0.2, CHARACTER_ANSWER_TOKENS).await?);
        }
        notes = condensed;
    }

    let total = size(&notes);
    if total > MAX_SAMPLE_TEXT {
        let keep = (notes.len() * MAX_SAMPLE_TEXT / total).max(1);
        notes = spread(&notes.iter().collect::<Vec<_>>(), keep);
    }
    Ok(notes)
}

/// Ответ модели, когда в части нет сведений о персонаже. Он виден в окне,
/// но в итоговый запрос не идёт.
const NO_NOTE: &str = "Нет сведений о персонаже.";
const NO_TERM_NOTE: &str = "Нет сведений о термине.";
/// Маркер из прежнего запроса: модель дописывала его и после настоящих заметок,
/// а такие заметки лежат в сохранённых.
const OLD_NOTE_MARKER: &str = "NONE";
/// Ответы без содержания, которые нечего показывать.
const BLANK_NOTES: [&str; 3] = ["none", "empty string", "пустая строка"];
const NOTE_REFUSALS: [&str; 5] = [
    "нет информации",
    "нет сведений",
    "не содержит информации",
    "ничего не сказано",
    "не упоминается",
];
/// Длинный текст с такими словами уже содержит факты о персонаже.
const MAX_REFUSAL_LENGTH: usize = 120;

fn note_text(note: &str) -> String {
    note.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn strip_old_marker(note: &str) -> &str {
    let body = note.trim_end_matches(|c: char| !c.is_alphanumeric());
    body.strip_suffix(OLD_NOTE_MARKER).unwrap_or(note).trim()
}

fn is_blank_note(note: &str) -> bool {
    let text = note_text(note);
    text.is_empty() || BLANK_NOTES.contains(&text.as_str())
}

fn is_refusal(note: &str) -> bool {
    let text = note_text(note);
    text.chars().count() <= MAX_REFUSAL_LENGTH
        && NOTE_REFUSALS.iter().any(|refusal| text.contains(refusal))
}

/// Итоговый запрос: сборка идёт и после остановки чтения, поэтому общий флаг её не касается.
async fn write_description(
    target: &ChatTarget,
    character: CharacterGlossaryEntry,
    lines: Vec<String>,
) -> Result<CharacterGlossaryEntry, String> {
    let samples = HashMap::from([(character.alias.to_lowercase(), lines)]);
    let cancel = AtomicBool::new(false);
    let result = autofill_characters(target, vec![character], false, samples, &cancel, |_| {}).await?;
    match result.items.into_iter().next() {
        Some(updated) if result.failed_batches == 0 => Ok(updated),
        _ => Err("Модель не вернула данные о персонаже, попробуйте ещё раз".to_string()),
    }
}

/// Чем сервис отличается от обычного OpenAI-совместимого сервера: у локальных серверов и прочих совместимых всё стандартно.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Compatible,
    Openai,
    Claude,
    Gemini,
}

/// Куда идёт запрос. У каждого действия своя цель из настроек LLM, а ключ нужен только внешним сервисам.
/// `Debug` не выводится намеренно: ключ не должен попасть в журнал.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTarget {
    pub api_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub provider: Provider,
}

impl ChatTarget {
    fn key(&self) -> Option<&str> {
        self.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty())
    }
}

/// Обновляет персонажа по частям его сцен: собирает заметки, а затем пишет описание по ним. Готовые заметки лежат на диске, поэтому остановленный запуск
/// продолжается с того же места. После остановки описание собирается из того, что уже прочитано.
/// Возвращает `None`, если запуск остановлен раньше первой заметки.
pub async fn summarize_character(
    target: &ChatTarget,
    character: CharacterGlossaryEntry,
    chunks: Vec<String>,
    hints: &ModelHints,
    notes_file: &Path,
    cancel: &AtomicBool,
    on_progress: impl Fn(SummaryProgress) + Send + Sync,
) -> Result<Option<CharacterGlossaryEntry>, String> {
    let model = target.model.as_str();
    let system_prompt = format!(
        "You read a part of a visual novel script and take notes about the character {}. \
        The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
        A line \"---\" separates scenes that may be far apart in the story. \
        Write in Russian, 2 to 5 short sentences, only what this text shows: what the character does, \
        occupation, relations with others, and how they speak (tone, slang, \"ты\" or \"вы\"). \
        If the character is neither named nor speaks anywhere in the text, answer only with: {NO_NOTE} \
        Otherwise always write notes, even if the character appears only briefly. \
        Text in [square brackets] is an in-game variable: never infer anything from its name. \
        A whole sentence in brackets explains such a variable: rely on it, but never copy it word for word. \
        A line starting with \"Russian names:\" gives the exact spelling of names: always use it, \
        declining the name as Russian grammar requires. \
        A line starting with \"Gender:\" tells who is male or female. \
        A line starting with \"Known terms:\" gives established translations: use them. \
        Respond with plain text only.",
        character.original_name
    );

    let mut stored = read_scene_notes(notes_file);
    let total = chunks.len();
    let own_name = character.original_name.clone();
    let hint_for = |text: &str| glossary_hints(&format!("{own_name}\n{text}"), hints);

    // Одна новая часть: запрос сразу пишет описание по её тексту, и оно же становится заметкой
    let new_single_part = match chunks.as_slice() {
        [chunk] => Some(note_key(model, &character.alias, chunk)).filter(|key| !stored.contains_key(key)),
        _ => None,
    };
    if let Some(key) = new_single_part {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        on_progress(SummaryProgress { stage: "single".to_string(), done: 0, total });
        let lines = hint_for(&chunks[0]).into_iter().chain(chunks[0].lines().map(str::to_string)).collect();
        let updated = write_description(target, character, lines).await?;
        stored.insert(key, updated.notes.clone());
        write_scene_notes(notes_file, &stored)?;
        return Ok(Some(updated));
    }

    let reader = NoteReader {
        owner: &character.alias,
        name: &character.original_name,
        excluded: &character.excluded_notes,
        system_prompt: &system_prompt,
    };
    let notes = read_notes(target, reader, &chunks, hints, notes_file, cancel, &on_progress).await?;
    if notes.is_empty() {
        return if cancel.load(Ordering::Relaxed) {
            Ok(None)
        } else {
            Err("В выбранных сценах модель не нашла сведений о персонаже".to_string())
        };
    }
    let mut notes = condense_notes(target, notes, &on_progress).await?;
    // В заметках имена уже по-русски, поэтому упомянутых ищем в самом тексте игры
    notes.splice(0..0, hint_for(&chunks.join("\n")));
    on_progress(SummaryProgress { stage: "final".to_string(), done: total, total });
    write_description(target, character, notes).await.map(Some)
}

/// Чьи заметки читаются по частям сцен.
struct NoteReader<'a> {
    owner: &'a str,
    name: &'a str,
    excluded: &'a [String],
    system_prompt: &'a str,
}

/// Читает части и возвращает заметки для описания. Каждая заметка сразу ложится на диск,
/// поэтому остановленный запуск продолжается с той же части.
async fn read_notes(
    target: &ChatTarget,
    reader: NoteReader<'_>,
    chunks: &[String],
    hints: &ModelHints,
    notes_file: &Path,
    cancel: &AtomicBool,
    on_progress: &(impl Fn(SummaryProgress) + Send + Sync),
) -> Result<Vec<String>, String> {
    let model = target.model.as_str();
    let mut stored = read_scene_notes(notes_file);
    let total = chunks.len();
    let mut notes = Vec::new();
    for (done, chunk) in chunks.iter().enumerate() {
        on_progress(SummaryProgress { stage: "scenes".to_string(), done, total });
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let key = note_key(model, reader.owner, chunk);
        let note = match stored.get(&key) {
            Some(note) => note.clone(),
            None => {
                let hints = glossary_hints(&format!("{}\n{chunk}", reader.name), hints);
                let hint: String = hints.iter().map(|hint| format!("{hint}\n\n")).collect();
                let prompt = format!("{hint}Script:\n\n{chunk}\n\nNotes about {}:", reader.name);
                let note = call_chat_completion(target, reader.system_prompt, &prompt, 0.2, SCENE_NOTE_TOKENS).await?;
                stored.insert(key.clone(), note.clone());
                write_scene_notes(notes_file, &stored)?;
                note
            }
        };
        let note = strip_old_marker(&note);
        if !reader.excluded.contains(&key) && !is_blank_note(note) && !is_refusal(note) {
            notes.push(note.to_string());
        }
    }
    Ok(notes)
}

const TERM_DESCRIPTION_PROMPT: &str = "You write a glossary entry for a translator of a visual novel into Russian. \
    You receive notes about one term collected from the game. Write in Russian, 2 to 4 sentences, plain text only: \
    what it is, who or what it is connected with, and its place in the story. \
    Use only the notes, do not invent facts, and do not translate the term itself in a new way. \
    A line starting with \"Russian names:\" gives the exact spelling of names: always use it, \
    declining the name as Russian grammar requires. \
    A line starting with \"Gender:\" tells who is male or female. \
    A line starting with \"Known terms:\" gives established translations: use them.";

/// Пишет описание термина так же, как персонажа: читает части его сцен, а затем собирает заметки.
/// Возвращает `None`, если запуск остановлен раньше первой заметки.
pub async fn summarize_term(
    target: &ChatTarget,
    term: &TermGlossaryEntry,
    chunks: Vec<String>,
    hints: &ModelHints,
    notes_file: &Path,
    cancel: &AtomicBool,
    on_progress: impl Fn(SummaryProgress) + Send + Sync,
) -> Result<Option<String>, String> {
    let name = term.source_term.trim();
    let system_prompt = format!(
        "You read a part of a visual novel script and take notes about the term {name}. \
        It may be a place, an organization, an item, a skill, a title or another concept of the game. \
        The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
        A line \"---\" separates scenes that may be far apart in the story. \
        Write in Russian, 1 to 3 short sentences, only what this text shows: what it is, \
        who or what it is connected with, and what happens with it. \
        If the text says nothing about it, answer only with: {NO_TERM_NOTE} \
        Text in [square brackets] is an in-game variable: never infer anything from its name. \
        A whole sentence in brackets explains such a variable: rely on it, but never copy it word for word. \
        A line starting with \"Russian names:\" gives the exact spelling of names: always use it, \
        declining the name as Russian grammar requires. \
        A line starting with \"Gender:\" tells who is male or female. \
        A line starting with \"Known terms:\" gives established translations: use them. \
        Respond with plain text only."
    );
    let owner = term.note_owner();
    let reader = NoteReader { owner: &owner, name, excluded: &term.excluded_notes, system_prompt: &system_prompt };
    let notes = read_notes(target, reader, &chunks, hints, notes_file, cancel, &on_progress).await?;
    if notes.is_empty() {
        return if cancel.load(Ordering::Relaxed) {
            Ok(None)
        } else {
            Err("В выбранных сценах модель не нашла сведений о термине".to_string())
        };
    }
    let mut lines = condense_notes(target, notes, &on_progress).await?;
    lines.splice(0..0, glossary_hints(&format!("{name}\n{}", chunks.join("\n")), hints));
    on_progress(SummaryProgress { stage: "final".to_string(), done: chunks.len(), total: chunks.len() });
    // Как и у персонажа, сборка идёт и после остановки чтения
    let prompt = format!("Notes about {name}:\n\n{}", lines.join("\n"));
    let answer = call_chat_completion(target, TERM_DESCRIPTION_PROMPT, &prompt, 0.2, SCENE_NOTE_TOKENS).await?;
    let description = answer.trim();
    if description.is_empty() {
        return Err("Модель не вернула описание термина, попробуйте ещё раз".to_string());
    }
    Ok(Some(description.to_string()))
}

/// Автозаполнение персонажей в глоссарии через LLM
pub async fn autofill_characters(
    target: &ChatTarget,
    mut characters: Vec<CharacterGlossaryEntry>,
    only_empty: bool,
    samples: HashMap<String, Vec<String>>,
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<CharacterResult>) + Send + Sync,
) -> Result<AutofillResult<CharacterGlossaryEntry>, String> {
    // Выбираем только те, что нуждаются в обработке
    let indices_to_process: Vec<usize> = characters
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            if !only_empty {
                true
            } else {
                c.translated_name.trim().is_empty()
                    || c.gender == "unknown"
                    || c.gender.trim().is_empty()
                    || c.notes.trim().is_empty()
                    || c.speech.trim().is_empty()
            }
        })
        .map(|(i, _)| i)
        .collect();

    if indices_to_process.is_empty() {
        return Ok(AutofillResult {
            items: characters,
            failed_batches: 0,
        });
    }

    info!(
        "Отправка {} персонажей на автозаполнение через LLM ({})",
        indices_to_process.len(),
        target.model
    );

    let system_prompt = r#"You are an expert game localizer and linguist specializing in translating visual novels and RPGs from English to Russian.
Your task is to localize character profiles for a translation glossary.

For each character provided, you must output:
1. "alias": EXACT same alias code as provided in input.
2. "translated_name": Natural, authentic Russian transliteration or translation of the character's name.
   - Standard names are transliterated: "Emma" -> "Эмма", "Lisa" -> "Лиза", "Jack" -> "Джек", "Chloe" -> "Хлоя".
   - Read a name by the rules of its own language, not English: "Anja" -> "Аня", "Dunja" -> "Дуня", "Giovanni" -> "Джованни".
   - Descriptive names are translated: "The warrior girl" -> "Девушка-воин", "Old Bartender" -> "Старый бармен", "Demon Beast" -> "Демонический зверь".
   - If the input already gives translated_name, return it unchanged and spell the name that way everywhere.
3. "gender": Grammatical gender in Russian:
   - "female": women, girls, female creatures (согласование: сказала, пришла).
   - "male": men, boys, male creatures (согласование: сказал, пришёл).
   - "neuter": genderless entities, monsters, things, spirits, constructs (согласование: пришло, сказало).
   - "unknown": only if completely impossible to infer from name and context lines.
4. "notes": Detailed description in Russian, 3 to 6 sentences, based only on the provided lines: occupation and place in the story, who they are to the protagonist (lines addressed to "you" are usually addressed to the protagonist), character and motives, how they speak, how they treat others, and what happens to them in the story. A translator will rely on it to keep the voice consistent. Do not invent facts. If there are few lines, write less.
5. "speech": Required separate field, 1 to 2 sentences in Russian on the manner of speech: tone, vocabulary or slang, and whether they use "ты" or "вы" with others. Fill it for every character, even with few lines. It may repeat what "notes" says: do not shorten "notes" because of it.

Text in [square brackets] is an in-game variable whose value is unknown. Never infer an occupation, kinship or name from the variable's own name. A whole sentence in brackets explains such a variable: rely on it, but never copy it word for word.
A line starting with "Russian names:" gives the exact spelling of names: always use it in "notes" and "speech", declining the name as Russian grammar requires, even if other lines spell it differently.
A line starting with "Gender:" tells who is male or female.
A line starting with "Known terms:" gives established translations of game terms: use them in "notes" and "speech".
Lines may come as a scene transcript in the form "Speaker: text". Only lines of that speaker show their own manner; lines of others show how they see the character.
Lines may also be notes collected from every scene of the game: treat them as reliable and use all of them.

Respond ONLY with a valid JSON array of objects without markdown fences or explanations:
[
  {"alias": "lis", "translated_name": "Лиза", "gender": "female", "notes": "Младшая сестра героя, студентка. Дерзкая и насмешливая, за колкостями скрывает неуверенность. Говорит коротко и с подколками. С героем спорит и поддразнивает его, но быстро приходит на помощь. По ходу истории становится мягче и откровеннее.", "speech": "Говорит коротко и с подколками, часто использует сленг, со всеми на 'ты'."}
]"#;

    // Небольшие пакеты: результат виден чаще, а сбой портит меньше записей
    let batches = character_batches(&indices_to_process, &characters, &samples);
    let total_batches = batches.len();
    let mut failed_batches = 0;
    let report = |batch: usize, failed_batches: usize, items: Vec<CharacterResult>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in batches.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut user_prompt = String::from("Characters to translate and determine gender:\n\n");
        for &idx in chunk {
            let c = &characters[idx];
            let alias_lower = c.alias.to_lowercase();
            let sample_str = if let Some(lines) = samples.get(&alias_lower) {
                if !lines.is_empty() {
                    format!(" | Sample dialogue: [{}]", lines.join("; "))
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let translated = match c.translated_name.trim() {
                "" => String::new(),
                name => format!(", translated_name: \"{name}\""),
            };
            user_prompt.push_str(&format!(
                "- alias: \"{}\", original_name: \"{}\"{}{}\n",
                c.alias, c.original_name, translated, sample_str
            ));
        }

        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = match call_chat_completion(
            target,
            system_prompt,
            &user_prompt,
            0.2, // низкая температура для точности и следования схеме
            CHARACTER_ANSWER_TOKENS,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                failed_batches += remaining_batches(error, batch, total_batches)?;
                report(total_batches - 1, failed_batches, Vec::new());
                break;
            }
        };

        let json_clean = extract_json_content(&raw_response);
        let items: Vec<CharacterLlmItem> = match serde_json::from_str(json_clean) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!(
                    "Не удалось распарсить JSON ответа LLM: {}. Сырой ответ:\n{}",
                    e, raw_response
                );
                // Пробуем продолжить без падения
                failed_batches += 1;
                report(batch, failed_batches, Vec::new());
                continue;
            }
        };

        // Сопоставляем результаты обратно по alias. Модель может потерять знак внутреннего номера
        let answer_key = |alias: &str| alias.trim_start_matches(NO_CODE_MARK).to_lowercase();
        let item_map: HashMap<String, CharacterLlmItem> = items
            .into_iter()
            .map(|item| (answer_key(&item.alias), item))
            .collect();

        for &idx in chunk {
            let c = &mut characters[idx];
            if let Some(item) = item_map.get(&answer_key(&c.alias)) {
                if let Some(name) = &item.translated_name {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() && (only_empty && c.translated_name.trim().is_empty() || !only_empty) {
                        c.translated_name = trimmed.to_string();
                    }
                }

                if let Some(g) = &item.gender {
                    let g_lower = g.trim().to_lowercase();
                    let valid_gender = match g_lower.as_str() {
                        "female" | "f" | "ж" | "женский" => "female",
                        "male" | "m" | "м" | "мужской" => "male",
                        "neuter" | "n" | "ср" | "средний" => "neuter",
                        _ => "unknown",
                    };
                    if valid_gender != "unknown" && (only_empty && (c.gender == "unknown" || c.gender.is_empty()) || !only_empty) {
                        c.gender = valid_gender.to_string();
                    }
                }

                let fill = |field: &mut String, value: &Option<String>| {
                    let trimmed = value.as_deref().unwrap_or_default().trim();
                    if !trimmed.is_empty() && (field.trim().is_empty() || !only_empty) {
                        *field = trimmed.to_string();
                    }
                };
                fill(&mut c.notes, &item.notes);
                fill(&mut c.speech, &item.speech);
            }
        }

        let finished = chunk
            .iter()
            .map(|&idx| CharacterResult {
                character: characters[idx].clone(),
                samples: samples.get(&characters[idx].alias.to_lowercase()).cloned().unwrap_or_default(),
            })
            .collect();
        report(batch, failed_batches, finished);
    }

    Ok(AutofillResult {
        items: characters,
        failed_batches,
    })
}

/// С этого знака начинается внутренний номер персонажа без кода в скриптах (`name-forms.ts`).
const NO_CODE_MARK: char = '~';

/// Автозаполнение и перевод терминов в глоссарии через LLM
pub async fn autofill_terms(
    target: &ChatTarget,
    mut terms: Vec<TermGlossaryEntry>,
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<TermGlossaryEntry>) + Send + Sync,
) -> Result<AutofillResult<TermGlossaryEntry>, String> {
    let indices_to_process: Vec<usize> = (0..terms.len()).collect();

    if indices_to_process.is_empty() {
        return Ok(AutofillResult {
            items: terms,
            failed_batches: 0,
        });
    }

    info!(
        "Отправка {} терминов на автозаполнение через LLM ({})",
        indices_to_process.len(),
        target.model
    );

    let system_prompt = r#"You are an expert game localization specialist translating English terms to Russian.
Translate in-game terms, locations, factions, artifacts, ranks and items into authentic Russian.

Rules:
1. "id": EXACT same id as input.
2. "translated_term": Accurate, authentic Russian translation.
   Examples: "Demon Hunter" -> "Охотник на демонов", "PPF Headquarters" -> "Штаб ППФ", "Ancient Rune" -> "Древняя руна", "Silver Key" -> "Серебряный ключ".
3. "category": One of "general", "location", "organization", "title", "item". Use "organization" for companies, factions, agencies, gangs, schools as institutions and other groups of people.
4. "notes": Brief context/commentary in Russian, or empty string.

Respond ONLY with a valid JSON array of objects:
[
  {"id": "term-1", "translated_term": "Штаб ППФ", "category": "location", "notes": "Главная база"}
]"#;

    const BATCH_SIZE: usize = 20;
    let total_batches = indices_to_process.len().div_ceil(BATCH_SIZE);
    let mut failed_batches = 0;
    let report = |batch: usize, failed_batches: usize, items: Vec<TermGlossaryEntry>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in indices_to_process.chunks(BATCH_SIZE).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut user_prompt = String::from("Terms to translate into Russian:\n\n");
        for &idx in chunk {
            let t = &terms[idx];
            user_prompt.push_str(&format!(
                "- id: \"{}\", source_term: \"{}\", current_category: \"{}\"\n",
                t.id, t.source_term, t.category
            ));
        }
        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = match call_chat_completion(
            target,
            system_prompt,
            &user_prompt,
            0.2,
            2048,
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                failed_batches += remaining_batches(error, batch, total_batches)?;
                report(total_batches - 1, failed_batches, Vec::new());
                break;
            }
        };

        let json_clean = extract_json_content(&raw_response);
        let items: Vec<TermLlmItem> = match serde_json::from_str(json_clean) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("Не удалось распарсить JSON терминов от LLM: {}", e);
                failed_batches += 1;
                report(batch, failed_batches, Vec::new());
                continue;
            }
        };

        let item_map: HashMap<String, TermLlmItem> = items
            .into_iter()
            .filter_map(|item| item.id.clone().map(|id| (id, item)))
            .collect();

        for &idx in chunk {
            let t = &mut terms[idx];
            if let Some(item) = item_map.get(&t.id) {
                if let Some(trans) = &item.translated_term {
                    let trimmed = trans.trim();
                    if !trimmed.is_empty() {
                        t.translated_term = trimmed.to_string();
                    }
                }
                if let Some(cat) = &item.category {
                    let trimmed = cat.trim().to_lowercase();
                    if matches!(trimmed.as_str(), "general" | "location" | "organization" | "title" | "item") {
                        t.category = trimmed;
                    }
                }
                if let Some(n) = &item.notes {
                    let trimmed = n.trim();
                    if !trimmed.is_empty() && t.notes.trim().is_empty() {
                        t.notes = trimmed.to_string();
                    }
                }
            }
        }

        report(batch, failed_batches, chunk.iter().map(|&idx| terms[idx].clone()).collect());
    }

    Ok(AutofillResult {
        items: terms,
        failed_batches,
    })
}

/// Тексты всех реплик скриптов.
pub fn script_texts(game: &GameLines) -> Vec<String> {
    game.lines.iter().map(|line| line.text.clone()).collect()
}

const NAMES_SYSTEM_PROMPT: &str = r#"You are an expert in personal names and their established Russian spellings, preparing a translation glossary for a visual novel or RPG.
You receive every character name of one game and the names to spell in Russian.

Work in this order:
1. Read the whole list and work out which languages and countries the names come from. Names of one game usually share a setting, so use the others to settle a name that could belong to several languages.
2. Decide the language of each name on its own: a game can mix several. A name may lack diacritics: "Dzenana" is "Dženana".
3. Spell the name the way speakers of that language say it, using the established Russian form if there is one. Never read a name by English rules unless it is English. Final "-ia" becomes "-ия": "Maria" -> "Мария". Answer in Cyrillic letters only.

Spelling by language:
- Serbian, Croatian, Bosnian: "lj" and "nj" are one soft sound, "j" is "й", "dž" and "đ" are "дж", "c" is "ц": "Anja" -> "Аня", "Dunja" -> "Дуня", "Ljubica" -> "Любица", "Ljiljana" -> "Лиляна", "Mirjana" -> "Мирьяна", "Jovan" -> "Йован", "Dženana" -> "Дженана".
- German: "ei" is "ай", "eu" is "ой", "j" is "й": "Heike" -> "Хайке", "Jürgen" -> "Юрген", "Hilde" -> "Хильде", "Marlene" -> "Марлен".
- Italian: "Giovanni" -> "Джованни", "Chiara" -> "Кьяра".
- French: final consonants are silent: "Claire" -> "Клер", "Jacques" -> "Жак".
- Spanish: "j" is "х": "Jorge" -> "Хорхе", "Juana" -> "Хуана".
- Polish, Czech, Hungarian: "Wojciech" -> "Войцех", "Jiří" -> "Иржи", "Erzsébet" -> "Эржебет".
- Japanese: "Yuki" -> "Юки", "Shoji" -> "Сёдзи".
- English: "Chloe" -> "Хлоя", "Jack" -> "Джек", "Katy" -> "Кэти".

Roles, descriptions and abbreviations are translated by meaning: "Old Bartender" -> "Старый бармен", "Agent Lisa" -> "Агент Лиза", "MC" -> "Главный герой".
Text in [square brackets] is an in-game variable: return it unchanged.

A line may say more about the name:
- "surname of a woman" or "surname of a man": make the Russian surname agree with the gender if its language changes it: "Kowalski" of a woman -> "Ковальская", "Ivanov" of a woman -> "Иванова". Other surnames stay the same: "Karanovic" -> "Каранович".
- "form of Анна Каранович": another way to call that character. Keep that spelling for the parts they share and translate titles by meaning: "Mrs. Karanovic" -> "Миссис Каранович", "Annie" -> "Энни".

Respond ONLY with a valid JSON array, one object per name to spell, without markdown fences or explanations:
[
  {"id": "n0", "origin": "сербский", "translated_name": "Аня"}
]
"id" is the EXACT id as given. "origin" is the language of the name as one Russian word, or "описание" for roles and descriptions."#;

/// Строка запроса: что это за имя и от какого написания оно строится.
fn name_line(request: &NameRequest) -> String {
    let person = match request.gender.as_str() {
        "female" => "a woman",
        "male" => "a man",
        _ => "a character",
    };
    let detail = match request.kind {
        NameKind::Name => String::new(),
        NameKind::Surname => format!(", surname of {person}"),
        NameKind::Form if request.base.is_empty() => format!(", name of {person}"),
        NameKind::Form => format!(", name of {person}, form of {}", request.base),
    };
    format!("- id: \"{}\", name: \"{}\"{detail}\n", request.id, request.name)
}

/// Имена без повторов и пустых строк: регистр не считается отличием, порядок сохраняется.
fn unique_names(names: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty() && seen.insert(name.to_lowercase()))
        .collect()
}

/// Слово из смеси кириллицы и латиницы: модель сбилась посреди имени.
fn is_garbled(name: &str) -> bool {
    name.split_whitespace().any(|word| {
        word.chars().any(|c| c.is_ascii_alphabetic()) && word.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
    })
}

/// Предлагает написание имён `names`. Модель видит весь состав игры `cast`, потому что
/// происхождение одного имени часто понятно только по остальным.
pub async fn translate_names(
    target: &ChatTarget,
    names: &[NameRequest],
    cast: &[String],
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<NameSuggestion>) + Send + Sync,
) -> Result<AutofillResult<NameSuggestion>, String> {
    let cast = unique_names(cast).join(", ");
    let total_batches = names.len().div_ceil(MAX_BATCH_NAMES);
    let mut failed_batches = 0;
    let mut items: Vec<NameSuggestion> = Vec::new();
    let report = |batch: usize, failed_batches: usize, items: Vec<NameSuggestion>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in names.chunks(MAX_BATCH_NAMES).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let list: String = chunk.iter().map(name_line).collect();
        let user_prompt = format!("All names in this game: {cast}\n\nNames to spell:\n{list}\nProvide JSON array now:");

        let raw_response = match call_chat_completion(target, NAMES_SYSTEM_PROMPT, &user_prompt, 0.2, NAME_ANSWER_TOKENS).await {
            Ok(response) => response,
            Err(error) => {
                failed_batches += remaining_batches(error, batch, total_batches)?;
                report(total_batches - 1, failed_batches, Vec::new());
                break;
            }
        };

        let answer: Vec<NameLlmItem> = match serde_json::from_str(extract_json_content(&raw_response)) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("Не удалось распарсить JSON имён от LLM: {}. Сырой ответ:\n{}", e, raw_response);
                failed_batches += 1;
                report(batch, failed_batches, Vec::new());
                continue;
            }
        };

        let kept_before = items.len();
        for item in answer {
            let id = item.id.as_deref().unwrap_or_default().trim();
            let translated = item.translated_name.as_deref().unwrap_or_default().trim();
            // Имени, которого не было в списке, модель придумала сама
            let Some(request) = chunk.iter().find(|request| request.id == id) else {
                continue;
            };
            if translated.is_empty() || is_garbled(translated) || items.iter().any(|known| known.id == id) {
                continue;
            }
            items.push(NameSuggestion {
                id: request.id.clone(),
                translated: translated.to_string(),
                origin: item.origin.as_deref().unwrap_or_default().trim().to_string(),
            });
        }
        report(batch, failed_batches, items[kept_before..].to_vec());
    }

    Ok(AutofillResult { items, failed_batches })
}

const TOKENS_SYSTEM_PROMPT: &str = r#"You study text variables of a visual novel that is being translated into Russian.
The game replaces each variable with a string at run time. In the lines below the variable is shown as [___].

Work out from the lines what each variable stands for. Rely only on the lines and on "Script values", the values the scripts really assign to it. If the lines do not show what it means, leave the field empty instead of guessing.

For each variable output:
1. "id": EXACT id as given.
2. "note": One short sentence in Russian for a translator: what the variable stands for and who it refers to.
3. "value": A Russian word or phrase in the nominative case that reads correctly in place of [___]. Empty string if the player types it in, if it changes during the game, or if the lines do not settle it.

A line starting with "Russian names:" gives the exact spelling of names. A line starting with "Known terms:" gives established translations.

Respond ONLY with a valid JSON array, without markdown fences or explanations:
[
  {"id": "t0", "note": "Кем Селия приходится главному герою", "value": "мать"},
  {"id": "t1", "note": "Имя главного героя, его вводит игрок", "value": ""}
]"#;

/// Токен с материалом для модели.
#[derive(Debug)]
pub struct TokenSample {
    name: String,
    values: Vec<String>,
    lines: Vec<String>,
}

/// Реплики с токенами `names`, равномерно по игре. Имени разбираемого токена в них нет: модель верит
/// ему больше, чем репликам. Токены без реплик пропускаются: судить модели не о чем.
pub fn token_samples(game: &GameLines, tokens: &TokenNotes, names: &[String]) -> Vec<TokenSample> {
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut found: HashMap<&str, Vec<&ScriptLine>> = HashMap::new();
    for line in &game.lines {
        let used: HashSet<&str> = RE_TEXT_VARIABLE
            .captures_iter(&line.text)
            .filter_map(|caps| caps.get(1))
            .map(|name| name.as_str())
            .filter(|name| wanted.contains(name))
            .collect();
        for name in used {
            found.entry(name).or_default().push(line);
        }
    }
    names
        .iter()
        .filter_map(|name| {
            let lines = found.get(name.as_str())?;
            let mut hidden = tokens.clone();
            hidden.insert(name.clone(), TOKEN_GAP.to_string());
            let lines = (0..TOKEN_SAMPLES.min(lines.len())).map(|i| {
                let line = lines[i * lines.len() / TOKEN_SAMPLES.min(lines.len())];
                let text = sample_text(&line.text, &game.variables, &hidden);
                match line.speaker.as_deref().or(line.alias.as_deref()) {
                    Some(speaker) => format!("{speaker}: {text}"),
                    None => text,
                }
            });
            Some(TokenSample {
                name: name.clone(),
                values: game.variables.get(name).cloned().unwrap_or_default(),
                lines: lines.collect(),
            })
        })
        .collect()
}

/// Предлагает пояснения и значения токенов. Окончательно их выбирает пользователь.
pub async fn suggest_tokens(
    target: &ChatTarget,
    samples: &[TokenSample],
    hints: &ModelHints,
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<TokenSuggestion>) + Send + Sync,
) -> Result<AutofillResult<TokenSuggestion>, String> {
    let total_batches = samples.len().div_ceil(TOKEN_BATCH);
    let mut failed_batches = 0;
    let mut items: Vec<TokenSuggestion> = Vec::new();
    let report = |batch: usize, failed_batches: usize, items: Vec<TokenSuggestion>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in samples.chunks(TOKEN_BATCH).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let text: String = chunk.iter().flat_map(|sample| &sample.lines).map(|line| format!("{line}\n")).collect();
        let mut user_prompt: String = glossary_hints(&text, hints).iter().map(|hint| format!("{hint}\n")).collect();
        for (index, sample) in chunk.iter().enumerate() {
            user_prompt.push_str(&format!("\nid: t{index}\n"));
            if !sample.values.is_empty() {
                user_prompt.push_str(&format!("Script values: {}\n", sample.values.join(", ")));
            }
            user_prompt.push_str("Lines:\n");
            for line in &sample.lines {
                user_prompt.push_str(&format!("{line}\n"));
            }
        }
        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = match call_chat_completion(target, TOKENS_SYSTEM_PROMPT, &user_prompt, 0.2, TOKEN_ANSWER_TOKENS).await {
            Ok(response) => response,
            Err(error) => {
                failed_batches += remaining_batches(error, batch, total_batches)?;
                report(total_batches - 1, failed_batches, Vec::new());
                break;
            }
        };

        let answer: Vec<TokenLlmItem> = match serde_json::from_str(extract_json_content(&raw_response)) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("Не удалось распарсить JSON токенов от LLM: {}. Сырой ответ:\n{}", e, raw_response);
                failed_batches += 1;
                report(batch, failed_batches, Vec::new());
                continue;
            }
        };

        let kept_before = items.len();
        for item in answer {
            let id = item.id.as_deref().unwrap_or_default().trim();
            let Some(sample) = id.strip_prefix('t').and_then(|index| index.parse::<usize>().ok()).and_then(|index| chunk.get(index)) else {
                continue;
            };
            let value = item.value.as_deref().unwrap_or_default().trim();
            let note = item.note.as_deref().unwrap_or_default().trim();
            if (value.is_empty() && note.is_empty()) || items.iter().any(|known| known.name == sample.name) {
                continue;
            }
            items.push(TokenSuggestion { name: sample.name.clone(), value: value.to_string(), note: note.to_string() });
        }
        report(batch, failed_batches, items[kept_before..].to_vec());
    }

    Ok(AutofillResult { items, failed_batches })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn http_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        )
    }

    /// Полученная длина запроса покрывает заголовки и тело из `Content-Length`.
    fn request_is_complete(request: &[u8]) -> bool {
        let text = String::from_utf8_lossy(request);
        let Some(headers_end) = text.find("\r\n\r\n") else {
            return false;
        };
        let body_length = text[..headers_end]
            .lines()
            .find_map(|line| line.to_lowercase().strip_prefix("content-length:")?.trim().parse().ok())
            .unwrap_or(0usize);
        request.len() >= headers_end + 4 + body_length
    }

    /// Запускает сервер, который отвечает на запросы по порядку, и возвращает его адрес.
    fn serve_responses(responses: Vec<String>) -> String {
        record_requests(responses).0
    }

    /// Как `serve_responses`, но ещё отдаёт текст полученных запросов.
    fn record_requests(responses: Vec<String>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                while !request_is_complete(&request) {
                    let read = stream.read(&mut chunk).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                }
                let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{}", address), requests)
    }

    #[test]
    fn test_failed_request_keeps_finished_batches() {
        let content = r#"[{"id":"t0","translated_term":"Первый"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let failure = http_response("500 Internal Server Error", "сбой");
        let terms = || -> Vec<TermGlossaryEntry> {
            (0..45)
                .map(|i| TermGlossaryEntry { id: format!("t{i}"), ..Default::default() })
                .collect()
        };

        let cancel = AtomicBool::new(false);
        let url = serve_responses(vec![http_response("200 OK", &body), failure.clone()]);
        let result = tauri::async_runtime::block_on(autofill_terms(&chat(&url), terms(), &cancel, |_| {}))
            .unwrap();
        assert_eq!(result.failed_batches, 2);
        assert_eq!(result.items[0].translated_term, "Первый");

        // Если готового ещё нет, пользователь видит саму ошибку сервера
        let url = serve_responses(vec![failure]);
        let error = tauri::async_runtime::block_on(autofill_terms(&chat(&url), terms(), &cancel, |_| {}))
            .unwrap_err();
        assert!(error.contains("сбой"), "{error}");
    }

    fn chat_answer(content: &str) -> String {
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        http_response("200 OK", &body)
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn name_requests(names: &[&str]) -> Vec<NameRequest> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| NameRequest {
                id: format!("n{index}"),
                name: name.to_string(),
                kind: NameKind::Name,
                gender: String::new(),
                base: String::new(),
            })
            .collect()
    }

    #[test]
    fn test_names_are_spelled_against_the_whole_cast() {
        let content = r#"[
            {"id":"n0","origin":"сербский","translated_name":"Аня"},
            {"id":"n1","translated_name":"Хильде"},
            {"id":"n1","translated_name":"Хильда"},
            {"id":"n9","translated_name":"Выдумка"},
            {"id":"n2","translated_name":" "},
            {"id":"n3","translated_name":"Саarah"}
        ]"#;
        let (url, requests) = record_requests(vec![chat_answer(content)]);
        let cancel = AtomicBool::new(false);
        let names = name_requests(&["Anja", "Hilde", "Dunja", "Sarah"]);
        let cast = strings(&["Anja", "Hilde", "Dunja", "Sarah", "Vlado"]);

        let result = tauri::async_runtime::block_on(translate_names(&chat(&url), &names, &cast, &cancel, |_| {}))
            .unwrap();

        let request = requests.recv().unwrap();
        assert!(!request.to_lowercase().contains("authorization:"), "{request}");
        assert!(request.contains("All names in this game: Anja, Hilde, Dunja, Sarah, Vlado"), "{request}");
        let found: Vec<_> = result
            .items
            .iter()
            .map(|item| (item.id.as_str(), item.translated.as_str(), item.origin.as_str()))
            .collect();
        assert_eq!(found, [("n0", "Аня", "сербский"), ("n1", "Хильде", "")]);
        assert_eq!(result.failed_batches, 0);
    }

    #[test]
    fn test_name_lines_say_what_the_name_is() {
        let [mut name, mut surname, mut form]: [NameRequest; 3] = name_requests(&["Anna", "Kowalski", "Mrs. K"]).try_into().unwrap();
        assert_eq!(name_line(&name), "- id: \"n0\", name: \"Anna\"\n");

        surname.kind = NameKind::Surname;
        surname.gender = "female".to_string();
        assert_eq!(name_line(&surname), "- id: \"n1\", name: \"Kowalski\", surname of a woman\n");

        form.kind = NameKind::Form;
        form.gender = "male".to_string();
        assert_eq!(name_line(&form), "- id: \"n2\", name: \"Mrs. K\", name of a man\n");
        form.base = "Анна Ковальская".to_string();
        assert_eq!(name_line(&form), "- id: \"n2\", name: \"Mrs. K\", name of a man, form of Анна Ковальская\n");

        name.kind = NameKind::Surname;
        assert_eq!(name_line(&name), "- id: \"n0\", name: \"Anna\", surname of a character\n");
    }

    #[test]
    fn test_name_request_kinds_are_read_from_the_interface() {
        let request: NameRequest = serde_json::from_str(r#"{"id": "n0", "name": "Annie", "kind": "form", "base": "Анна"}"#).unwrap();
        assert!(matches!(request.kind, NameKind::Form));
        assert_eq!(request.gender, "");
    }

    #[test]
    fn test_every_name_batch_sees_the_whole_cast() {
        let cast: Vec<String> = (0..MAX_BATCH_NAMES + 1).map(|i| format!("Name{i}")).collect();
        let names = name_requests(&cast.iter().map(String::as_str).collect::<Vec<_>>());
        let (url, requests) = record_requests(vec![chat_answer("[]"), chat_answer("не JSON")]);
        let cancel = AtomicBool::new(false);

        let result = tauri::async_runtime::block_on(translate_names(&chat(&url), &names, &cast, &cancel, |_| {}))
            .unwrap();

        assert_eq!(result.failed_batches, 1);
        for _ in 0..2 {
            let request = requests.recv().unwrap();
            assert!(request.contains(&cast[0]) && request.contains(&cast[MAX_BATCH_NAMES]), "{request}");
        }
    }

    #[test]
    fn test_model_never_sees_the_token_name() {
        let script = "label one:\n    k \"Ask [landlady] about [other].\"\n    \"[landlady!c] is home.\"\n";
        let (dir, scripts) = script_with(script);
        let variables = HashMap::from([("landlady".to_string(), vec!["Mary".to_string()])]);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), variables);

        let samples = token_samples(&game, &TokenNotes::new(), &strings(&["landlady", "absent"]));
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].lines, ["k: Ask [___] about [other].", "[___] is home."]);

        let content = r#"[
            {"id":"t0","value":"мать","note":"Кем она приходится герою"},
            {"id":"t7","value":"выдумка","note":""}
        ]"#;
        let (url, requests) = record_requests(vec![chat_answer(content)]);
        let cancel = AtomicBool::new(false);
        let result = tauri::async_runtime::block_on(suggest_tokens(
            &chat(&url), &samples, &ModelHints::default(), &cancel, |_| {},
        ))
        .unwrap();

        let request = requests.recv().unwrap();
        assert!(!request.contains("landlady"), "{request}");
        assert!(request.contains("Script values: Mary"), "{request}");
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].name, "landlady");
        assert_eq!(result.items[0].value, "мать");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_target_needs_a_server_and_a_model() {
        let cancel = AtomicBool::new(false);
        let names = name_requests(&["Anja"]);
        let run = |target: ChatTarget| {
            tauri::async_runtime::block_on(translate_names(&target, &names, &[], &cancel, |_| {})).unwrap_err()
        };
        assert!(run(ChatTarget { api_url: " ".to_string(), ..chat("http://127.0.0.1:1") }).contains("Адрес"));
        assert!(run(ChatTarget { model: String::new(), ..chat("http://127.0.0.1:1") }).contains("Модель"));
    }

    #[test]
    fn test_external_server_gets_the_api_key() {
        let (url, requests) = record_requests(vec![chat_answer("[]")]);
        let cancel = AtomicBool::new(false);
        let target = ChatTarget { api_key: Some("sec\nret".to_string()), ..chat(&url) };
        let names = name_requests(&["Anja"]);

        // Такой ключ нельзя записать в заголовок: это ошибка запроса, а не паника
        let error = tauri::async_runtime::block_on(translate_names(&target, &names, &[], &cancel, |_| {}))
            .unwrap_err();
        assert!(!error.contains("sec"), "{error}");

        let target = ChatTarget { api_key: Some("secret".to_string()), ..target };
        tauri::async_runtime::block_on(translate_names(&target, &names, &[], &cancel, |_| {})).unwrap();
        let request = requests.recv().unwrap();
        assert!(request.to_lowercase().contains("authorization: bearer secret"), "{request}");
    }

    #[test]
    fn test_claude_is_called_directly() {
        let answer = r#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"[]"}],"stop_reason":"end_turn"}"#;
        let (url, requests) = record_requests(vec![http_response("200 OK", answer)]);
        let cancel = AtomicBool::new(false);
        let target = ChatTarget { api_key: Some("secret".to_string()), provider: Provider::Claude, ..chat(&url) };
        let names = name_requests(&["Anja"]);

        tauri::async_runtime::block_on(translate_names(&target, &names, &[], &cancel, |_| {})).unwrap();
        let request = requests.recv().unwrap();
        let lowered = request.to_lowercase();
        assert!(request.starts_with("POST /v1/messages "), "{request}");
        assert!(lowered.contains("x-api-key: secret"), "{request}");
        assert!(lowered.contains("anthropic-version:"), "{request}");
        assert!(!lowered.contains("authorization:"), "{request}");
    }

    #[test]
    fn test_claude_request_has_no_sampling_and_a_top_level_system() {
        let target = ChatTarget { model: "claude-haiku-5-5".to_string(), ..chat("https://api.anthropic.com") };
        let body = serde_json::to_value(claude_request(&target, "system", "user", 100)).unwrap();
        assert_eq!(body["system"], "system");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["output_config"]["effort"], "low");
        assert_eq!(body["max_tokens"], 100 + REASONING_TOKENS);
        assert!(body.get("temperature").is_none());

        let old = ChatTarget { model: "claude-haiku-4-5-20251001".to_string(), ..target };
        let body = serde_json::to_value(claude_request(&old, "system", "user", 100)).unwrap();
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn test_claude_text_skips_thinking_and_reports_a_refusal() {
        let answer = |json: &str| claude_text(serde_json::from_str(json).unwrap());
        let text = answer(r#"{"content":[{"type":"thinking"},{"type":"text","text":" ответ "}],"stop_reason":"end_turn"}"#);
        assert_eq!(text.unwrap(), "ответ");
        assert!(answer(r#"{"content":[],"stop_reason":"refusal"}"#).unwrap_err().contains("отказалась"));
    }

    #[test]
    fn test_each_service_gets_its_token_limit() {
        let body = |provider| {
            let target = ChatTarget { provider, ..chat("http://server") };
            serde_json::to_value(chat_request(&target, "system", "user", 0.2, 100)).unwrap()
        };

        let local = body(Provider::Compatible);
        assert_eq!(local["max_tokens"], 100);
        assert!(local.get("temperature").is_some());
        assert!(local.get("max_completion_tokens").is_none());

        let openai = body(Provider::Openai);
        assert_eq!(openai["max_completion_tokens"], 100 + REASONING_TOKENS);
        assert!(openai.get("max_tokens").is_none());
        assert!(openai.get("temperature").is_none());

        assert_eq!(body(Provider::Gemini)["max_tokens"], 100 + REASONING_TOKENS);
    }

    #[test]
    fn test_service_addresses() {
        assert_eq!(versioned_url("http://127.0.0.1:1234"), "http://127.0.0.1:1234/v1");
        assert_eq!(versioned_url("https://api.openai.com/v1/"), "https://api.openai.com/v1");
        assert_eq!(
            versioned_url("https://generativelanguage.googleapis.com/v1beta/openai/"),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(models_url("https://api.anthropic.com", Provider::Claude), "https://api.anthropic.com/v1/models?limit=1000");
        assert_eq!(models_url("https://api.openai.com/v1", Provider::Openai), "https://api.openai.com/v1/models");
    }

    fn script_with(content: &str) -> (PathBuf, Vec<PathBuf>) {
        let dir = std::env::temp_dir().join(format!("translator_llm_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.rpy");
        fs::write(&path, content).unwrap();
        (dir, vec![path])
    }

    #[test]
    fn test_batches_respect_text_limit() {
        let characters = characters(8);
        let long = vec!["x".repeat(5_000)];
        let samples: HashMap<String, Vec<String>> = [
            ("c0".to_string(), long.clone()),
            ("c1".to_string(), long.clone()),
            ("c2".to_string(), long),
        ]
        .into();
        let batches = character_batches(&(0..8).collect::<Vec<_>>(), &characters, &samples);
        assert_eq!(batches, [vec![0, 1], vec![2, 3, 4], vec![5, 6, 7]]);
    }

    #[test]
    fn test_variables_are_replaced_with_their_values() {
        let (dir, scripts) = script_with(concat!(
            "default houselady = \"mom\"\n",
            "define nick = _(\"sis\")\n",
            "default empty_name = \"\"\n",
            "label start:\n",
            "    $ place = \"bar\"\n",
            "    $ place = 'club'\n",
            "    k \"I am your [houselady], [nick!c].\"\n",
            "    k \"Going to the [place] with [friend.name] now.\"\n",
        ));
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), string_variables(&scripts));
        assert_eq!(scene_chunks(&game, &TokenNotes::new(), &kate(&[])), [
            "Kate: I am your mom, sis.\nKate: Going to the bar/club with [friend.name] now.\n"
        ]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_token_value_replaces_the_token() {
        let (dir, scripts) = script_with(concat!(
            "default landlady = \"landlady\"\n",
            "default place = \"bar\"\n",
            "label home:\n",
            "    k \"Ask [landlady!c] about the rent.\"\n",
            "    k \"Ask [landlady] again, and [nick] too, at the [place].\"\n",
        ));
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), string_variables(&scripts));
        assert_eq!(scene_chunks(&game, &TokenNotes::new(), &kate(&[])), [
            "Kate: Ask landlady about the rent.\nKate: Ask landlady again, and [nick] too, at the bar.\n"
        ]);

        let notes = token_notes(&[
            TokenGlossaryEntry { name: "landlady".to_string(), value: " мать ".to_string(), note: "Кем она приходится".to_string() },
            TokenGlossaryEntry { name: "nick".to_string(), note: " ".to_string(), ..Default::default() },
            TokenGlossaryEntry { name: "absent".to_string(), note: "нет в тексте".to_string(), ..Default::default() },
        ]);
        let parts = scene_parts(&game, &notes, &kate(&[]));
        assert_eq!(parts[0].text, "Kate: Ask мать about the rent.\nKate: Ask мать again, and [nick] too, at the bar.\n");
        assert_eq!(parts[0].tokens, [
            TokenGlossaryEntry { name: "landlady".to_string(), note: "мать".to_string(), ..Default::default() },
            TokenGlossaryEntry { name: "nick".to_string(), note: String::new(), ..Default::default() },
        ]);

        // Без значения модель видит описание, но не имя токена
        let described = token_notes(&[TokenGlossaryEntry { name: "landlady".to_string(), note: " Кем она приходится ".to_string(), ..Default::default() }]);
        assert_eq!(
            scene_parts(&game, &described, &kate(&[]))[0].text,
            "Kate: Ask [Кем она приходится] about the rent.\nKate: Ask [Кем она приходится] again, and [nick] too, at the bar.\n"
        );

        assert!(scene_chunks(&game, &notes, &kate(&[]))[0].starts_with("Kate: Ask мать about the rent."));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_term_hint_covers_filled_terms_in_the_text() {
        let term = |source: &str, translated: &str, notes: &str| TermGlossaryEntry {
            source_term: source.to_string(),
            translated_term: translated.to_string(),
            notes: notes.to_string(),
            ..Default::default()
        };
        let terms = [
            term("PPF HQ", "Штаб ППФ", "главная база"),
            term("Rune", "Руна", ""),
            term("Blade", "", "без перевода"),
            term("Absent", "Нет в тексте", ""),
        ];
        assert_eq!(
            term_hint("Back to the ppf hq with a RUNE, a blade and runes.", &terms).as_deref(),
            Some("Known terms: PPF HQ = Штаб ППФ (главная база), Rune = Руна")
        );
        assert_eq!(term_hint("Only runes here.", &terms), None);

        // Форма идёт со своим переводом, а без него с переводом самого термина
        let mut foundation = term("Pleasure Palace Foundation", "Фонд дворца", "");
        foundation.forms = vec![
            NameForm { source: "PPF".to_string(), translated: "ФД".to_string() },
            NameForm { source: "Foundation".to_string(), translated: String::new() },
        ];
        assert_eq!(
            term_hint("The PPF and the Foundation.", &[foundation]).as_deref(),
            Some("Known terms: PPF = ФД, Foundation = Фонд дворца")
        );

        // Запрос не раздувается, сколько бы терминов ни нашлось
        let many: Vec<TermGlossaryEntry> = (0..400).map(|i| term(&format!("Term{i}"), &format!("Термин {i}"), "")).collect();
        let text = many.iter().map(|term| term.source_term.as_str()).collect::<Vec<_>>().join(" ");
        assert!(term_hint(&text, &many).unwrap().chars().count() <= MAX_TERM_HINT_CHARS + "Known terms: ".len());

        let hints = ModelHints {
            names: vec![KnownName { original: "Kate".to_string(), translated: "Кейт".to_string(), gender: "female".to_string() }],
            terms: vec![term("Rune", "Руна", "")],
            ..Default::default()
        };
        assert_eq!(glossary_hints("Kate has a Rune", &hints), ["Russian names: Kate = Кейт\nGender: Kate female", "Known terms: Rune = Руна"]);
    }

    #[test]
    fn test_name_hint_covers_only_names_in_the_text() {
        let name = |original: &str, translated: &str, gender: &str| KnownName {
            original: original.to_string(),
            translated: translated.to_string(),
            gender: gender.to_string(),
        };
        let names = [
            name("Celia", "Селия", "female"),
            name("Ana", "Ана", "unknown"),
            name("Celia", "Селия", "female"),
            name("Vera", "Вера", ""),
            name("Vlado", "Владо", "male"),
            name("Thing", "Нечто", "neuter"),
        ];
        assert_eq!(
            name_hint("Celia: Ask Ana, not the banana.\nCelia: Bye.", &names).as_deref(),
            Some("Russian names: Celia = Селия, Ana = Ана\nGender: Celia female")
        );
        assert_eq!(
            name_hint("Vlado saw the Thing.", &names).as_deref(),
            Some("Russian names: Vlado = Владо, Thing = Нечто\nGender: Vlado male, Thing neuter")
        );
        assert_eq!(name_hint("Banana and Celiac.", &names), None);

        let hints = ModelHints { names: names.to_vec(), ..Default::default() };
        assert_eq!(glossary_hints("Vera is here", &hints), ["Russian names: Vera = Вера"]);
        assert!(glossary_hints("Nobody", &hints).is_empty());
    }

    #[test]
    fn test_text_tokens_list_lines_values_and_example() {
        let (dir, scripts) = script_with(concat!(
            "default landlady = \"mom\"\n",
            "label home:\n",
            "    k \"{i}Hi{/i} [nick], [nick]!\"\n",
            "    k \"Ask [landlady!c] and [nick].\"\n",
        ));
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), string_variables(&scripts));
        let tokens = text_tokens(&game);
        assert_eq!(tokens.len(), 2);
        assert_eq!((tokens[0].name.as_str(), tokens[0].lines, tokens[0].example.as_str()), ("nick", 2, "Hi [nick], [nick]!"));
        assert_eq!((tokens[1].name.as_str(), tokens[1].lines), ("landlady", 1));
        assert_eq!(tokens[1].values, ["mom"]);
        let _ = fs::remove_dir_all(dir);
    }

    const TWO_SCENES: &str = concat!(
        "label home:\n",
        "    k \"I am staying home tonight.\"\n",
        "    m \"Then I will cook, Kate.\"\n",
        "label bar:\n",
        "    k \"One more drink for me.\"\n",
        "    b \"Last one, Kate.\"\n",
    );

    fn kate(excluded: &[&str]) -> CharacterGlossaryEntry {
        CharacterGlossaryEntry {
            alias: "k".to_string(),
            original_name: "Kate".to_string(),
            excluded_scenes: excluded.iter().map(|key| key.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_nearby_scenes_fill_a_part() {
        assert_eq!(pack_scenes(&[7000, 6000, 5500, 4900], 8), [vec![0, 3], vec![1, 2]]);
        // Подходящая сцена за пределами окна не трогается
        assert_eq!(pack_scenes(&[7000, 6000, 5500, 4900], 1), [vec![0], vec![1, 2], vec![3]]);
    }

    #[test]
    fn test_short_scene_tail_starts_with_previous_lines() {
        let line = |index: usize| format!("Kate: {index:03} {}", "word ".repeat(19));
        let lines: Vec<String> = (0..120).map(line).collect();
        let pieces = split_scene(&lines);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|piece| piece.chars().count() <= MAX_SAMPLE_TEXT));
        assert!(pieces[1].chars().count() >= MIN_SCENE_TAIL);
        assert!(pieces[1].contains("Kate: 119 "));
        // Начало дополненного куска уже есть в предыдущем, а реплики не теряются
        let first_line = pieces[1].lines().next().unwrap();
        assert!(pieces[0].contains(first_line));
        assert!(lines.iter().all(|line| pieces.iter().any(|piece| piece.contains(line.as_str()))));

        // Длинный хвост и короткая сцена не меняются
        let long_tail: Vec<String> = (0..150).map(line).collect();
        let pieces = split_scene(&long_tail);
        assert!(!pieces[0].contains(pieces[1].lines().next().unwrap()));
        assert_eq!(split_scene(&lines[..2]), [format!("{}\n{}\n", lines[0], lines[1])]);
        assert!(split_scene(&[]).is_empty());
    }

    #[test]
    fn test_parts_keep_scenes_whole() {
        let scene = |name: &str, lines: usize| {
            let mut text = format!("label {name}:\n");
            for line in 0..lines {
                text.push_str(&format!("    k \"{name} line {line} {}\"\n", "word ".repeat(30)));
            }
            text
        };
        // Вторая сцена не помещается в запрос, а её остаток может идти с соседними
        let script = [scene("first", 40), scene("second", 100), scene("third", 5)].concat();
        let (dir, scripts) = script_with(&script);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());

        let parts = scene_parts(&game, &TokenNotes::new(), &kate(&[]));
        let listed: Vec<&str> = parts.iter().flat_map(|part| &part.scenes).map(|scene| scene.scene.as_str()).collect();
        assert!(parts.iter().flat_map(|part| &part.scenes).all(|scene| scene.file == "story.rpy" && !scene.path.is_empty()));
        assert!(["first", "second", "third"].iter().all(|scene| listed.contains(scene)));
        let chunks = scene_chunks(&game, &TokenNotes::new(), &kate(&[]));
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= MAX_SAMPLE_TEXT));
        let whole = |name: &str, last: usize| {
            chunks.iter().any(|chunk| chunk.contains(&format!("{name} line 0 ")) && chunk.contains(&format!("{name} line {last} ")))
        };
        assert!(whole("first", 39) && whole("third", 4));
        assert!(!whole("second", 99));
        assert!(chunks.iter().any(|chunk| chunk.contains("---\nKate: ")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_excluded_scene_is_not_sent_to_model() {
        let (dir, scripts) = script_with(TWO_SCENES);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());
        let chunks = scene_chunks(&game, &TokenNotes::new(), &kate(&["story.rpy::bar"])).concat();
        assert!(chunks.contains("I am staying home tonight."), "{chunks}");
        assert!(!chunks.contains("One more drink"), "{chunks}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_scenes_join_merged_codes() {
        let (dir, scripts) = script_with("label one:\n    hil \"First line of her.\"\n    hi \"Second line of her.\"\n");
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());
        let hilde = CharacterGlossaryEntry {
            alias: "hil".to_string(),
            original_name: "Hilde".to_string(),
            merged: vec![crate::glossary::MergedCharacter {
                alias: "hi".to_string(),
                original_name: "Hilde".to_string(),
            }],
            ..Default::default()
        };
        assert_eq!(scene_chunks(&game, &TokenNotes::new(), &hilde), ["Hilde: First line of her.\nHilde: Second line of her.\n"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_whole_scene_keeps_all_speakers() {
        let (dir, scripts) = script_with(TWO_SCENES);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());

        let all = scene_chunks(&game, &TokenNotes::new(), &kate(&["story.rpy::bar"]));
        assert_eq!(all, ["Kate: I am staying home tonight.\nm: Then I will cook, Kate.\n"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_chunks_cover_all_enabled_scenes_within_limit() {
        let mut script = String::new();
        for scene in 0..3 {
            script.push_str(&format!("label scene_{scene}:\n"));
            for line in 0..120 {
                script.push_str(&format!("    k \"Scene {scene} line {line} {}\"\n", "word ".repeat(30)));
            }
        }
        let (dir, scripts) = script_with(&script);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());

        let chunks = scene_chunks(&game, &TokenNotes::new(), &kate(&[]));
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= MAX_SAMPLE_TEXT));
        let text = chunks.concat();
        assert!(text.contains("Scene 0 line 0 ") && text.contains("Scene 2 line 119 "));

        let without = scene_chunks(&game, &TokenNotes::new(), &kate(&["story.rpy::scene_2"]));
        assert!(!without.concat().contains("Scene 2 line"));
        let _ = fs::remove_dir_all(dir);
    }

    fn chat(url: &str) -> ChatTarget {
        ChatTarget { api_url: url.to_string(), model: "model".to_string(), api_key: None, provider: Provider::Compatible }
    }

    fn completion(content: &str) -> String {
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        http_response("200 OK", &body)
    }

    #[test]
    fn test_summary_resumes_from_saved_notes() {
        let dir = std::env::temp_dir().join(format!("translator_notes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one".to_string(), "Kate: two".to_string()];
        let answer = r#"[{"alias":"k","notes":"Итог","speech":"На 'ты'"}]"#;
        let cancel = AtomicBool::new(false);
        let stages = std::sync::Mutex::new(Vec::new());

        // Первая часть готова, на второй запуск остановлен: описание собирается из неё
        let url = serve_responses(vec![completion("Заметка 1"), completion(answer)]);
        let stopped = tauri::async_runtime::block_on(summarize_character(
            &chat(&url), kate(&[]), chunks.clone(), &ModelHints::default(), &notes_file, &cancel,
            |p| {
                if p.done == 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        ))
        .unwrap();
        assert_eq!(stopped.unwrap().notes, "Итог");

        // Новый запуск не спрашивает готовую часть заново: у сервера всего два ответа
        cancel.store(false, Ordering::Relaxed);
        let url = serve_responses(vec![completion("Заметка 2"), completion(answer)]);
        let updated = tauri::async_runtime::block_on(summarize_character(
            &chat(&url), kate(&[]), chunks.clone(), &ModelHints::default(), &notes_file, &cancel,
            |p| stages.lock().unwrap().push(p.stage),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(updated.notes, "Итог");
        assert_eq!(updated.speech, "На 'ты'");
        assert_eq!(stages.into_inner().unwrap(), ["scenes", "scenes", "final"]);
        assert_eq!(ready_chunks(&chunks, "model", &kate(&[]).alias, &notes_file), [true, true]);
        assert_eq!(ready_chunks(&chunks, "other", &kate(&[]).alias, &notes_file), [false, false]);
        let _ = fs::remove_dir_all(dir);
    }

    fn academy() -> TermGlossaryEntry {
        TermGlossaryEntry {
            id: "t1".to_string(),
            source_term: "Ninja Academy".to_string(),
            forms: vec![crate::glossary::NameForm { source: "NA".to_string(), translated: String::new() }],
            ..Default::default()
        }
    }

    #[test]
    fn test_term_parts_come_from_its_scenes_and_all_its_forms() {
        let (dir, scripts) = script_with(concat!(
            "label first:\n    k \"I study at the Ninja Academy.\"\n",
            "label second:\n    k \"The NA is closed.\"\n",
            "label third:\n    k \"Nothing here.\"\n",
        ));
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());

        let text = term_scene_chunks(&game, &TokenNotes::new(), &academy()).concat();
        assert!(text.contains("Ninja Academy") && text.contains("The NA is closed"));
        assert!(!text.contains("Nothing here"));

        let without = TermGlossaryEntry { excluded_scenes: vec!["story.rpy::second".to_string()], ..academy() };
        assert!(!term_scene_chunks(&game, &TokenNotes::new(), &without).concat().contains("The NA is closed"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_term_summary_collects_notes_and_skips_empty_parts() {
        let dir = std::env::temp_dir().join(format!("translator_term_notes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one".to_string(), "Kate: two".to_string()];
        let cancel = AtomicBool::new(false);
        let stages = std::sync::Mutex::new(Vec::new());

        let (url, requests) = record_requests(vec![completion("Школа ниндзя"), completion(NO_TERM_NOTE), completion("  Итог  ")]);
        let description = tauri::async_runtime::block_on(summarize_term(
            &chat(&url), &academy(), chunks.clone(), &ModelHints::default(), &notes_file, &cancel,
            |p| stages.lock().unwrap().push(p.stage),
        ))
        .unwrap();
        assert_eq!(description.as_deref(), Some("Итог"));
        assert_eq!(stages.into_inner().unwrap(), ["scenes", "scenes", "final"]);
        let last = requests.try_iter().last().unwrap();
        assert!(last.contains("Школа ниндзя") && !last.contains(NO_TERM_NOTE));

        // Заметки термина не пересекаются с заметками персонажа и не читаются заново
        let owner = academy().note_owner();
        assert_eq!(ready_chunks(&chunks, "model", &owner, &notes_file), [true, true]);
        assert_eq!(ready_chunks(&chunks, "model", "t1", &notes_file), [false, false]);
        let url = serve_responses(vec![completion("Снова итог")]);
        let again = tauri::async_runtime::block_on(summarize_term(
            &chat(&url), &academy(), chunks.clone(), &ModelHints::default(), &notes_file, &cancel, |_| {},
        ))
        .unwrap();
        assert_eq!(again.as_deref(), Some("Снова итог"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_term_summary_without_notes_stops_or_fails() {
        let dir = std::env::temp_dir().join(format!("translator_term_empty_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one".to_string()];
        let cancel = AtomicBool::new(true);
        let stopped = tauri::async_runtime::block_on(summarize_term(
            &chat("http://127.0.0.1:9"), &academy(), chunks.clone(), &ModelHints::default(), &notes_file, &cancel, |_| {},
        ));
        assert_eq!(stopped.unwrap(), None);

        cancel.store(false, Ordering::Relaxed);
        let url = serve_responses(vec![completion(NO_TERM_NOTE)]);
        let empty = tauri::async_runtime::block_on(summarize_term(
            &chat(&url), &academy(), chunks, &ModelHints::default(), &notes_file, &cancel, |_| {},
        ));
        assert!(empty.unwrap_err().contains("термине"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_chunks_are_picked_evenly() {
        let chunks: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert_eq!(pick_chunks(chunks.clone(), 1, &[]), ["0"]);
        assert_eq!(pick_chunks(chunks.clone(), 3, &[]), ["0", "4", "8"]);
        assert_eq!(pick_chunks(chunks.clone(), 5, &[]), ["0", "2", "4", "6", "8"]);
        assert_eq!(pick_chunks(chunks.clone(), 50, &[]), chunks);
    }

    #[test]
    fn test_ready_chunks_are_always_picked() {
        let chunks: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let ready: Vec<bool> = (0..10).map(|i| [1, 3, 7].contains(&i)).collect();
        assert_eq!(pick_chunks(chunks.clone(), 1, &ready), ["1", "3", "7"]);
        assert_eq!(pick_chunks(chunks.clone(), 5, &ready), ["0", "1", "3", "7", "8"]);
    }

    #[test]
    fn test_smaller_pick_is_part_of_larger_one() {
        let chunks: Vec<String> = (0..40).map(|i| i.to_string()).collect();
        for count in 1..40 {
            let smaller = pick_chunks(chunks.clone(), count, &[]);
            let larger = pick_chunks(chunks.clone(), count + 1, &[]);
            assert!(smaller.iter().all(|chunk| larger.contains(chunk)), "{count}");
        }
    }

    #[test]
    fn test_parts_show_notes_and_can_be_forgotten() {
        let dir = std::env::temp_dir().join(format!("translator_parts_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = ["Kate: one\nKate: two\n", "Kate: three\n", "Kate: four\n"].map(str::to_string);
        let stored = HashMap::from([
            (note_key("model", "k", &chunks[0]), "Кейт смеётся.".to_string()),
            (note_key("model", "k", &chunks[1]), NO_NOTE.to_string()),
        ]);
        write_scene_notes(&notes_file, &stored).unwrap();

        let source = |text: &String| ScenePart {
            text: text.clone(),
            scenes: vec![PartScene { scene: "home".to_string(), ..Default::default() }],
            tokens: vec![TokenGlossaryEntry { name: "nick".to_string(), ..Default::default() }],
        };
        let hints = ModelHints {
            names: vec![KnownName { original: "Kate".to_string(), translated: "Кейт".to_string(), gender: "female".to_string() }],
            terms: vec![TermGlossaryEntry {
                source_term: "Three".to_string(),
                translated_term: "Три".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let character = kate(&[]);
        let parts = character_parts(chunks.iter().map(source).collect(), "model", &character.alias, &character.original_name, &notes_file, &hints);
        assert_eq!(parts[1].scenes[0].scene, "home");
        assert_eq!(parts[1].names[0].translated, "Кейт");
        assert_eq!(parts[1].terms, ["Three = Три"]);
        assert_eq!(parts[1].tokens[0].name, "nick");
        assert!(parts[0].terms.is_empty());
        assert_eq!(parts[0].start, "Kate: one");
        assert_eq!((parts[0].note.as_deref(), parts[0].empty), (Some("Кейт смеётся."), false));
        assert_eq!((parts[1].note.as_deref(), parts[1].empty), (Some(NO_NOTE), true));
        assert_eq!((parts[2].note.as_deref(), parts[2].empty), (None, false));

        forget_notes(&notes_file, std::slice::from_ref(&parts[1].key)).unwrap();
        assert_eq!(ready_chunks(&chunks, "model", "k", &notes_file), [true, false, false]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_excluded_note_stays_out_of_the_description() {
        let dir = std::env::temp_dir().join(format!("translator_excluded_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one\n".to_string(), "Kate: two\n".to_string()];
        let keys: Vec<String> = chunks.iter().map(|chunk| note_key("model", "k", chunk)).collect();
        let stored = HashMap::from([
            (keys[0].clone(), "Первая.".to_string()),
            (keys[1].clone(), "Вторая.".to_string()),
        ]);
        write_scene_notes(&notes_file, &stored).unwrap();

        let content = r#"[{"alias":"k","translated_name":"Кейт","gender":"female","notes":"Итог","speech":"На 'ты'"}]"#;
        let (url, requests) = record_requests(vec![completion(content)]);
        let character = CharacterGlossaryEntry { excluded_notes: vec![keys[0].clone()], ..kate(&[]) };
        let cancel = AtomicBool::new(false);
        let updated = tauri::async_runtime::block_on(summarize_character(
            &chat(&url), character, chunks, &ModelHints::default(), &notes_file, &cancel,
            |_| {},
        ))
        .unwrap()
        .unwrap();
        assert_eq!(updated.notes, "Итог");
        let description_request = requests.recv().unwrap();
        assert!(description_request.contains("Вторая."));
        assert!(!description_request.contains("Первая."));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_refusals_are_not_notes() {
        for note in ["", "  ", "none.", "\"\"", "''", "Пустая строка"] {
            assert!(is_blank_note(note), "{note:?}");
        }
        for note in [NO_NOTE, "Пока что в тексте нет информации об Алёнке."] {
            assert!(!is_blank_note(note) && is_refusal(note), "{note:?}");
        }
        let long = format!("{} О её прошлом нет информации.", "Работает медсестрой в городской больнице. ".repeat(3));
        for note in ["Лиза командует отрядом.", long.as_str()] {
            assert!(!is_blank_note(note) && !is_refusal(note), "{note:?}");
        }
    }

    #[test]
    fn test_single_part_counts_as_read() {
        let dir = std::env::temp_dir().join(format!("translator_notes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one".to_string()];
        let alias = kate(&[]).alias;
        let answer = r#"[{"alias":"k","notes":"Итог","speech":"На 'ты'"}]"#;
        let cancel = AtomicBool::new(false);
        let run = |response: &str, stages: &std::sync::Mutex<Vec<String>>| {
            let url = serve_responses(vec![completion(response)]);
            tauri::async_runtime::block_on(summarize_character(
                &chat(&url), kate(&[]), chunks.clone(), &ModelHints::default(), &notes_file, &cancel,
                |p| stages.lock().unwrap().push(p.stage),
            ))
        };

        // Мусор вместо ответа: часть не засчитана, и окно узнаёт об ошибке
        let stages = std::sync::Mutex::new(Vec::new());
        assert!(run("мусор", &stages).is_err());
        assert_eq!(ready_chunks(&chunks, "model", &alias, &notes_file), [false]);

        // Один запрос и читает часть, и пишет описание
        assert_eq!(run(answer, &stages).unwrap().unwrap().notes, "Итог");
        assert_eq!(ready_chunks(&chunks, "model", &alias, &notes_file), [true]);
        assert_eq!(stages.into_inner().unwrap(), ["single", "single"]);

        // Следующий запуск собирает описание из сохранённой части
        let stages = std::sync::Mutex::new(Vec::new());
        assert!(run(answer, &stages).unwrap().is_some());
        assert_eq!(stages.into_inner().unwrap(), ["scenes", "final"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_old_marker_keeps_facts_of_note() {
        assert_eq!(strip_old_marker("Alenka makes serum. NONE"), "Alenka makes serum.");
        assert_eq!(strip_old_marker("NONE."), "");
        assert_eq!(strip_old_marker(" Лиза командует. "), "Лиза командует.");
    }

    #[test]
    fn test_refusals_stay_out_of_description() {
        let dir = std::env::temp_dir().join(format!("translator_notes_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let notes_file = dir.join("scene_notes.json");
        let chunks = vec!["Kate: one".to_string(), "Kate: two".to_string(), "Kate: three".to_string()];
        let answer = r#"[{"alias":"k","notes":"Итог","speech":"На 'ты'"}]"#;
        let cancel = AtomicBool::new(false);

        let (url, requests) = record_requests(vec![
            completion("Пустая строка"),
            completion("Заметка. NONE"),
            completion(NO_NOTE),
            completion(answer),
        ]);
        let updated = tauri::async_runtime::block_on(summarize_character(
            &chat(&url), kate(&[]), chunks, &ModelHints::default(), &notes_file, &cancel,
            |_| {},
        ))
        .unwrap()
        .unwrap();
        assert_eq!(updated.notes, "Итог");
        let description_request = requests.iter().nth(3).unwrap();
        assert!(description_request.contains("Заметка."));
        assert!(!description_request.contains(NO_NOTE));

        // Заметок нет: описание нечему обновлять, модель не должна придумывать его по имени
        let url = serve_responses(vec![completion("NONE"), completion(NO_NOTE)]);
        let result = tauri::async_runtime::block_on(summarize_character(
            &chat(&url), kate(&[]), vec!["Kate: four".to_string(), "Kate: six".to_string()], &ModelHints::default(), &notes_file, &cancel,
            |_| {},
        ));
        assert!(result.is_err());

        // Остановка до первой заметки: собирать нечего, и это не ошибка
        cancel.store(true, Ordering::Relaxed);
        let stopped = tauri::async_runtime::block_on(summarize_character(
            &chat("http://127.0.0.1:1"), kate(&[]), vec!["Kate: five".to_string()], &ModelHints::default(), &notes_file, &cancel,
            |_| {},
        ));
        assert_eq!(stopped, Ok(None));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_long_notes_are_condensed_for_final_request() {
        let long = "заметка ".repeat(1200);
        let url = serve_responses(vec![completion("Сжато 1"), completion("Сжато 2")]);
        let notes = tauri::async_runtime::block_on(condense_notes(
            &chat(&url), vec![long.clone(), long], &|_| {},
        ))
        .unwrap();
        assert_eq!(notes, ["Сжато 1", "Сжато 2"]);
    }

    #[test]
    fn test_model_fills_description() {
        let content = r#"[{"alias":"c0","translated_name":"Катя","gender":"female","notes":"Вежливая","speech":"На 'вы'"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let url = serve_responses(vec![http_response("200 OK", &body)]);
        let cancel = AtomicBool::new(false);
        let result = tauri::async_runtime::block_on(autofill_characters(
            &chat(&url), characters(1), true, HashMap::new(), &cancel, |_| {},
        ))
        .unwrap();
        assert_eq!(result.items[0].notes, "Вежливая");
        assert_eq!(result.items[0].speech, "На 'вы'");
    }

    #[test]
    fn test_character_without_code_survives_a_lost_mark() {
        let content = r#"[{"alias":"miss_granger","gender":"female","notes":"Отличница"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let url = serve_responses(vec![http_response("200 OK", &body)]);
        let mut mentioned = characters(1);
        mentioned[0].alias = "~miss_granger".to_string();
        let cancel = AtomicBool::new(false);
        let result = tauri::async_runtime::block_on(autofill_characters(
            &chat(&url), mentioned, true, HashMap::new(), &cancel, |_| {},
        ))
        .unwrap();
        assert_eq!(result.items[0].notes, "Отличница");
        assert_eq!(result.items[0].alias, "~miss_granger");
    }

    #[test]
    fn test_script_texts_take_dialogue_instead_of_code() {
        let (dir, scripts) = script_with(concat!(
            "define e = Character(\"Eileen\")\n",
            "image room = \"Images/Room/day.png\"\n",
            "label start:\n",
            "    $ hero = \"Default\"\n",
            "    scene room\n",
            "    e \"Welcome to Jack's Bar, friend.\"\n",
            "    \"The Demon Hunter Association is here.\"\n",
        ));
        assert_eq!(
            script_texts(&GameLines::new(scenes::load_lines(&scripts, |_| String::new()), HashMap::new())),
            ["Welcome to Jack's Bar, friend.", "The Demon Hunter Association is here."]
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn characters(count: usize) -> Vec<CharacterGlossaryEntry> {
        (0..count)
            .map(|i| CharacterGlossaryEntry {
                alias: format!("c{i}"),
                original_name: format!("Name {i}"),
                gender: "unknown".to_string(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn test_characters_arrive_by_batches_with_their_lines() {
        let answer = |from: usize, to: usize| {
            let items: Vec<_> = (from..to)
                .map(|i| serde_json::json!({"alias": format!("c{i}"), "translated_name": format!("Имя {i}"), "gender": "female"}))
                .collect();
            let content = serde_json::Value::Array(items).to_string();
            let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
            http_response("200 OK", &body)
        };
        let url = serve_responses(vec![answer(0, 3), answer(3, 5)]);
        let samples = HashMap::from([("c4".to_string(), vec!["Don't go".to_string()])]);
        let cancel = AtomicBool::new(false);
        let reports = std::sync::Mutex::new(Vec::new());

        let result = tauri::async_runtime::block_on(autofill_characters(
            &chat(&url), characters(5), true, samples, &cancel,
            |progress| reports.lock().unwrap().push(progress),
        ))
        .unwrap();

        let reports = reports.into_inner().unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!((reports[0].batch, reports[0].total_batches, reports[0].items.len()), (1, 2, 3));
        assert_eq!(reports[0].items[0].character.translated_name, "Имя 0");
        assert!(reports[0].items[0].samples.is_empty());
        assert_eq!(reports[1].items[1].samples, ["Don't go"]);
        assert_eq!(result.items[4].translated_name, "Имя 4");
    }

    #[test]
    fn test_stop_keeps_finished_batches() {
        let content = r#"[{"alias":"c0","translated_name":"Первый"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let url = serve_responses(vec![http_response("200 OK", &body)]);
        let cancel = AtomicBool::new(false);

        let result = tauri::async_runtime::block_on(autofill_characters(
            &chat(&url), characters(12), true, HashMap::new(), &cancel,
            |_| cancel.store(true, Ordering::Relaxed),
        ))
        .unwrap();

        assert_eq!(result.failed_batches, 0, "остановка это не сбой");
        assert_eq!(result.items[0].translated_name, "Первый");
        assert_eq!(result.items[5].translated_name, "");
    }

    #[test]
    fn test_extract_json_content() {
        let clean = r#"[{"alias":"lis","translated_name":"Лиза","gender":"female"}]"#;
        assert_eq!(extract_json_content(clean), clean);

        let markdown = format!("```json\n{}\n```", clean);
        assert_eq!(extract_json_content(&markdown), clean);

        let markdown_no_lang = format!("```\n{}\n```", clean);
        assert_eq!(extract_json_content(&markdown_no_lang), clean);

        let with_text = format!("Here is the JSON:\n{}\nHope it helps!", clean);
        assert_eq!(extract_json_content(&with_text), clean);

        let fenced_with_text = format!("Response:\n```json\n{}\n```\nEnjoy!", clean);
        assert_eq!(extract_json_content(&fenced_with_text), clean);

        let wrapped = format!(r#"{{"items":{},"total":1}}"#, clean);
        assert_eq!(extract_json_content(&wrapped), clean);

        let numbered_note = format!("Found [3] names [1]:\n{}\nSee [2].", clean);
        assert_eq!(extract_json_content(&numbered_note), clean);

        assert_eq!(extract_json_content("  no json here "), "no json here");
    }

    #[test]
    fn test_normalize_api_url() {
        assert_eq!(normalize_api_url("localhost:1234"), "http://127.0.0.1:1234");
        assert_eq!(
            normalize_api_url("http://localhost:1234/v1/"),
            "http://127.0.0.1:1234/v1"
        );
        assert_eq!(
            normalize_api_url("http://127.0.0.1:1234/v1/"),
            "http://127.0.0.1:1234/v1"
        );
        assert_eq!(
            normalize_api_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn test_term_llm_json_deserialization() {
        let json = r#"[
            {"id": "t1", "source_term": "Demon Hunter", "translated_term": "Охотник на демонов", "category": "title", "notes": "Звание"},
            {"id": "t2", "source_term": "PPF HQ", "translated_term": "Штаб ППФ", "category": "location", "notes": ""}
        ]"#;
        let items: Vec<TermLlmItem> = serde_json::from_str(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].translated_term.as_deref(), Some("Охотник на демонов"));
        assert_eq!(items[1].category.as_deref(), Some("location"));
    }
}
