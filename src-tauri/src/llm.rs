use crate::glossary::{CharacterGlossaryEntry, TermGlossaryEntry};
use crate::parser::renpy::RenpyParser;
use crate::terms::TermCandidate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{info, warn};

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

/// Термин, который модель оставила в списке кандидатов.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewedTerm {
    #[serde(flatten)]
    pub term: TermGlossaryEntry,
    pub count: usize,
    pub example: String,
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
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
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
}

#[derive(Debug, Deserialize)]
struct TermLlmItem {
    id: Option<String>,
    source_term: Option<String>,
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

/// Проверяет доступность сервера и возвращает список доступных моделей
pub async fn check_models(api_url: &str) -> Result<Vec<String>, String> {
    let base_url = normalize_api_url(api_url);
    let client = make_client(6)?;

    // Если в URL уже есть /v1, то /models, если нет — пробуем /v1/models
    let models_url = if base_url.ends_with("/v1") {
        format!("{}/models", base_url)
    } else {
        format!("{}/v1/models", base_url)
    };

    let resp = client
        .get(&models_url)
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

    let mut models: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
    models.sort();
    Ok(models)
}

/// Выполняет один запрос в OpenAI-совместимый chat completions endpoint
async fn call_chat_completion(
    api_url: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, String> {
    let base_url = normalize_api_url(api_url);
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.clone()
    } else if base_url.ends_with("/v1") {
        format!("{}/chat/completions", base_url)
    } else {
        format!("{}/v1/chat/completions", base_url)
    };

    let client = make_client(180)?;
    let request_body = ChatCompletionRequest {
        model: model.to_string(),
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
        max_tokens: Some(max_tokens),
    };

    let response = client
        .post(&endpoint)
        .json(&request_body)
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

/// Реплики скрипта по порядку: псевдоним говорящего и текст. Разбор сам отделяет
/// их от кода и не обрывает текст на апострофах и экранированных кавычках.
fn script_lines(path: &Path) -> Vec<(Option<String>, String)> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(graph) = RenpyParser::new().parse_original_script(path, &content) else {
        return Vec::new();
    };
    graph
        .dialogues()
        .map(|line| (line.speaker_alias.clone(), line.original_text.clone()))
        .collect()
}

/// Собирает до 2 коротких контекстных реплик для каждого алиаса персонажа
pub fn collect_character_dialogue_samples(
    scripts: &[PathBuf],
    aliases: &[String],
) -> HashMap<String, Vec<String>> {
    let mut samples: HashMap<String, Vec<String>> = HashMap::new();
    let alias_set: std::collections::HashSet<String> =
        aliases.iter().map(|a| a.to_lowercase()).collect();

    for path in scripts {
        for (speaker, text) in script_lines(path) {
            let Some(speaker) = speaker.map(|s| s.to_lowercase()) else {
                continue;
            };
            let text = text.trim().to_string();
            if alias_set.contains(&speaker) && (3..=120).contains(&text.chars().count()) {
                let list = samples.entry(speaker).or_default();
                if list.len() < 2 && !list.contains(&text) {
                    list.push(text);
                }
            }
        }
        // Если для всех нашли по 2 реплики, можно остановиться раньше
        if aliases
            .iter()
            .all(|a| samples.get(&a.to_lowercase()).map_or(0, |l| l.len()) >= 2)
        {
            break;
        }
    }

    samples
}

/// Автозаполнение персонажей в глоссарии через LLM
pub async fn autofill_characters(
    api_url: &str,
    model: &str,
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
        model
    );

    let system_prompt = r#"You are an expert game localizer and linguist specializing in translating visual novels and RPGs from English to Russian.
Your task is to localize character profiles for a translation glossary.

For each character provided, you must output:
1. "alias": EXACT same alias code as provided in input.
2. "translated_name": Natural, authentic Russian transliteration or translation of the character's name.
   - Standard names are transliterated: "Emma" -> "Эмма", "Lisa" -> "Лиза", "Jack" -> "Джек", "Chloe" -> "Хлоя".
   - Descriptive names are translated: "The warrior girl" -> "Девушка-воин", "Old Bartender" -> "Старый бармен", "Demon Beast" -> "Демонический зверь".
3. "gender": Grammatical gender in Russian:
   - "female": women, girls, female creatures (согласование: сказала, пришла).
   - "male": men, boys, male creatures (согласование: сказал, пришёл).
   - "neuter": genderless entities, monsters, things, spirits, constructs (согласование: пришло, сказало).
   - "unknown": only if completely impossible to infer from name and context lines.
4. "notes": Brief helpful note in Russian on role, demeanor or speech tone (e.g. "дерзкая, на 'ты'", "вежливый тон", "главный герой"), or empty string if standard.

Respond ONLY with a valid JSON array of objects without markdown fences or explanations:
[
  {"alias": "lis", "translated_name": "Лиза", "gender": "female", "notes": "Подруга"}
]"#;

    // Небольшие пакеты: результат виден чаще, а сбой портит меньше записей
    const BATCH_SIZE: usize = 5;
    let total_batches = indices_to_process.len().div_ceil(BATCH_SIZE);
    let mut failed_batches = 0;
    let report = |batch: usize, failed_batches: usize, items: Vec<CharacterResult>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in indices_to_process.chunks(BATCH_SIZE).enumerate() {
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

            user_prompt.push_str(&format!(
                "- alias: \"{}\", original_name: \"{}\"{}\n",
                c.alias, c.original_name, sample_str
            ));
        }

        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = match call_chat_completion(
            api_url,
            model,
            system_prompt,
            &user_prompt,
            0.2, // низкая температура для точности и следования схеме
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

        // Сопоставляем результаты обратно по alias
        let item_map: HashMap<String, CharacterLlmItem> = items
            .into_iter()
            .map(|item| (item.alias.to_lowercase(), item))
            .collect();

        for &idx in chunk {
            let c = &mut characters[idx];
            let key = c.alias.to_lowercase();
            if let Some(item) = item_map.get(&key) {
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

                if let Some(n) = &item.notes {
                    let trimmed = n.trim();
                    if !trimmed.is_empty() && (c.notes.trim().is_empty() || !only_empty) {
                        c.notes = trimmed.to_string();
                    }
                }
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

/// Автозаполнение и перевод терминов в глоссарии через LLM
pub async fn autofill_terms(
    api_url: &str,
    model: &str,
    mut terms: Vec<TermGlossaryEntry>,
    only_empty: bool,
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<TermGlossaryEntry>) + Send + Sync,
) -> Result<AutofillResult<TermGlossaryEntry>, String> {
    let indices_to_process: Vec<usize> = terms
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            if !only_empty {
                true
            } else {
                t.translated_term.trim().is_empty()
            }
        })
        .map(|(i, _)| i)
        .collect();

    if indices_to_process.is_empty() {
        return Ok(AutofillResult {
            items: terms,
            failed_batches: 0,
        });
    }

    info!(
        "Отправка {} терминов на автозаполнение через LLM ({})",
        indices_to_process.len(),
        model
    );

    let system_prompt = r#"You are an expert game localization specialist translating English terms to Russian.
Translate in-game terms, locations, factions, artifacts, ranks and items into authentic Russian.

Rules:
1. "id": EXACT same id as input.
2. "translated_term": Accurate, authentic Russian translation.
   Examples: "Demon Hunter" -> "Охотник на демонов", "PPF Headquarters" -> "Штаб ППФ", "Ancient Rune" -> "Древняя руна", "Silver Key" -> "Серебряный ключ".
3. "category": One of "general", "location", "title", "item".
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
            api_url,
            model,
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
                    if matches!(trimmed.as_str(), "general" | "location" | "title" | "item") {
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
pub fn script_texts(scripts: &[PathBuf]) -> Vec<String> {
    scripts
        .iter()
        .flat_map(|path| script_lines(path))
        .map(|(_, text)| text)
        .collect()
}

/// Проверяет кандидатов из `terms`: модель отсеивает мусор и переводит остальное.
/// Окончательно термины выбирает пользователь, поэтому результат не попадает в глоссарий сам.
pub async fn check_term_candidates(
    api_url: &str,
    model: &str,
    candidates: &[TermCandidate],
    cancel: &AtomicBool,
    on_batch: impl Fn(BatchProgress<ReviewedTerm>) + Send + Sync,
) -> Result<AutofillResult<ReviewedTerm>, String> {
    let system_prompt = r#"You are an expert game localizer preparing a translation glossary into Russian.
You receive candidate terms found automatically in a visual novel or RPG, each with a line of game text that shows its use. The game text may be in any language.

Keep only names worth a glossary: places, organizations, factions, unique items, skills, ranks, titles and other world-specific concepts.
Reject interjections, ordinary words and phrases, interface labels, character names, and fragments of sentences.

Rules:
1. "source_term": EXACT candidate text.
2. "translated_term": Natural, authentic Russian translation.
3. "category": One of "general", "location", "title", "item".
4. "notes": Brief context/commentary in Russian, or empty string.

Respond ONLY with a valid JSON array of the kept candidates, or [] if none qualify:
[
  {"source_term": "Demon Hunter Association", "translated_term": "Ассоциация охотников на демонов", "category": "title", "notes": "Организация"}
]"#;

    const BATCH_SIZE: usize = 20;
    let total_batches = candidates.len().div_ceil(BATCH_SIZE);
    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut failed_batches = 0;
    let mut items: Vec<ReviewedTerm> = Vec::new();
    let report = |batch: usize, failed_batches: usize, items: Vec<ReviewedTerm>| {
        on_batch(BatchProgress { batch: batch + 1, total_batches, failed_batches, items })
    };
    for (batch, chunk) in candidates.chunks(BATCH_SIZE).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut user_prompt = String::from("Candidate terms:\n\n");
        for candidate in chunk {
            user_prompt.push_str(&format!(
                "- source_term: \"{}\", example: \"{}\"\n",
                candidate.term, candidate.example
            ));
        }
        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = match call_chat_completion(
            api_url,
            model,
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
        let answer: Vec<TermLlmItem> = match serde_json::from_str(json_clean) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("Не удалось распарсить JSON кандидатов от LLM: {}", e);
                failed_batches += 1;
                report(batch, failed_batches, Vec::new());
                continue;
            }
        };

        let kept_before = items.len();
        for item in answer {
            let Some(source) = item.source_term.as_deref().map(str::trim) else {
                continue;
            };
            let translated = item.translated_term.as_deref().unwrap_or_default().trim();
            // Термин, которого не было в списке, модель придумала сама
            let Some(candidate) = chunk.iter().find(|c| c.term.eq_ignore_ascii_case(source)) else {
                continue;
            };
            if translated.is_empty() || items.iter().any(|known| known.term.source_term == candidate.term) {
                continue;
            }
            let category = item.category.as_deref().map(|c| c.trim().to_lowercase()).unwrap_or_default();
            items.push(ReviewedTerm {
                term: TermGlossaryEntry {
                    id: format!("term_auto_{}_{}", timestamp, items.len()),
                    source_term: candidate.term.clone(),
                    translated_term: translated.to_string(),
                    category: if matches!(category.as_str(), "location" | "title" | "item") {
                        category
                    } else {
                        "general".to_string()
                    },
                    notes: item.notes.unwrap_or_default(),
                },
                count: candidate.count,
                example: candidate.example.clone(),
            });
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
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
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        format!("http://{}", address)
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
        let result = tauri::async_runtime::block_on(autofill_terms(&url, "model", terms(), false, &cancel, |_| {}))
            .unwrap();
        assert_eq!(result.failed_batches, 2);
        assert_eq!(result.items[0].translated_term, "Первый");

        // Если готового ещё нет, пользователь видит саму ошибку сервера
        let url = serve_responses(vec![failure]);
        let error = tauri::async_runtime::block_on(autofill_terms(&url, "model", terms(), false, &cancel, |_| {}))
            .unwrap_err();
        assert!(error.contains("сбой"), "{error}");
    }

    fn script_with(content: &str) -> (PathBuf, Vec<PathBuf>) {
        let dir = std::env::temp_dir().join(format!("translator_llm_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("story.rpy");
        fs::write(&path, content).unwrap();
        (dir, vec![path])
    }

    #[test]
    fn test_character_samples_keep_whole_lines() {
        let (dir, scripts) = script_with(concat!(
            "label start:\n",
            "    # e \"Commented out\"\n",
            "    e \"Don't go to Jack's Bar\"\n",
            "    m \"Hi\"\n",
            "    e \"She said \\\"no\\\" twice\"\n",
            "    e \"One more\"\n",
        ));
        let samples = collect_character_dialogue_samples(&scripts, &["e".to_string(), "m".to_string()]);
        assert_eq!(samples["e"], ["Don't go to Jack's Bar", "She said \"no\" twice"]);
        assert!(!samples.contains_key("m"), "короткая реплика не даёт контекста");
        let _ = fs::remove_dir_all(dir);
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
            script_texts(&scripts),
            ["Welcome to Jack's Bar, friend.", "The Demon Hunter Association is here."]
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn candidate(term: &str) -> TermCandidate {
        TermCandidate { term: term.to_string(), count: 7, example: format!("See {term} here") }
    }

    #[test]
    fn test_model_keeps_only_listed_candidates() {
        let content = r#"[
            {"source_term":"konoha","translated_term":"Коноха","category":"location","notes":"Деревня"},
            {"source_term":"Invented Place","translated_term":"Выдумка"},
            {"source_term":"Oh God","translated_term":""},
            {"source_term":"Rare Thing","translated_term":"Редкость","category":"weapon"}
        ]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let url = serve_responses(vec![http_response("200 OK", &body)]);
        let candidates = [candidate("Konoha"), candidate("Oh God"), candidate("Rare Thing")];

        let cancel = AtomicBool::new(false);
        let result = tauri::async_runtime::block_on(check_term_candidates(&url, "model", &candidates, &cancel, |_| {}))
            .unwrap();

        assert_eq!(result.failed_batches, 0);
        let kept: Vec<_> = result.items.iter().map(|item| item.term.source_term.as_str()).collect();
        assert_eq!(kept, ["Konoha", "Rare Thing"]);
        assert_eq!(result.items[0].term.category, "location");
        assert_eq!(result.items[0].count, 7);
        assert_eq!(result.items[0].example, "See Konoha here");
        assert_eq!(result.items[1].term.category, "general");
        assert_ne!(result.items[0].term.id, result.items[1].term.id);
    }

    #[test]
    fn test_failed_check_keeps_finished_batches() {
        let content = r#"[{"source_term":"Term 0","translated_term":"Термин"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let failure = http_response("500 Internal Server Error", "сбой");
        let candidates: Vec<_> = (0..45).map(|i| candidate(&format!("Term {i}"))).collect();

        let url = serve_responses(vec![http_response("200 OK", &body), failure]);
        let cancel = AtomicBool::new(false);
        let reports = std::sync::Mutex::new(Vec::new());
        let result = tauri::async_runtime::block_on(check_term_candidates(&url, "model", &candidates, &cancel, |p| {
            reports.lock().unwrap().push((p.batch, p.total_batches, p.failed_batches, p.items.len()));
        }))
        .unwrap();
        assert_eq!(result.failed_batches, 2);
        assert_eq!(result.items.len(), 1);
        // Сбой не теряет отчёт: интерфейс узнаёт, что обработка дошла до конца
        assert_eq!(reports.into_inner().unwrap(), [(1, 3, 0, 1), (3, 3, 2, 0)]);
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
        let url = serve_responses(vec![answer(0, 5), answer(5, 7)]);
        let samples = HashMap::from([("c6".to_string(), vec!["Don't go".to_string()])]);
        let cancel = AtomicBool::new(false);
        let reports = std::sync::Mutex::new(Vec::new());

        let result = tauri::async_runtime::block_on(autofill_characters(
            &url, "model", characters(7), true, samples, &cancel,
            |progress| reports.lock().unwrap().push(progress),
        ))
        .unwrap();

        let reports = reports.into_inner().unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!((reports[0].batch, reports[0].total_batches, reports[0].items.len()), (1, 2, 5));
        assert_eq!(reports[0].items[0].character.translated_name, "Имя 0");
        assert!(reports[0].items[0].samples.is_empty());
        assert_eq!(reports[1].items[1].samples, ["Don't go"]);
        assert_eq!(result.items[6].translated_name, "Имя 6");
    }

    #[test]
    fn test_stop_keeps_finished_batches() {
        let content = r#"[{"alias":"c0","translated_name":"Первый"}]"#;
        let body = serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string();
        let url = serve_responses(vec![http_response("200 OK", &body)]);
        let cancel = AtomicBool::new(false);

        let result = tauri::async_runtime::block_on(autofill_characters(
            &url, "model", characters(12), true, HashMap::new(), &cancel,
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
