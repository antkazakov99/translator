use crate::glossary::{CharacterGlossaryEntry, TermGlossaryEntry};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfigDto {
    pub api_url: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Option<f32>,
}

impl Default for LlmConfigDto {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:1234/v1".to_string(),
            model: "mistral-small".to_string(),
            temperature: Some(0.2),
        }
    }
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

/// Очищает строку ответа нейросети от Markdown-блоков (```json ... ```)
fn extract_json_content(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }
    // Если внутри есть [ ... ], вырезаем JSON массив
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            return &trimmed[start..=end];
        }
    }
    trimmed
}

/// Собирает до 2 коротких контекстных реплик для каждого алиаса персонажа из .rpy файлов
pub fn collect_character_dialogue_samples(
    game_dir: &Path,
    aliases: &[String],
) -> HashMap<String, Vec<String>> {
    let mut samples: HashMap<String, Vec<String>> = HashMap::new();
    let alias_set: std::collections::HashSet<String> =
        aliases.iter().map(|a| a.to_lowercase()).collect();

    if !game_dir.exists() {
        return samples;
    }

    // Ищем в game_dir .rpy файлы
    let walker = walkdir::WalkDir::new(game_dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("rpy")
                && !e.path().to_string_lossy().contains(".editor-backups")
        });

    let re_dialogue = Regex::new(r#"^\s*([a-zA-Z0-9_]+)\s+["']([^"']{3,120})["']"#).unwrap();

    for entry in walker {
        if let Ok(content) = fs::read_to_string(entry.path()) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    continue;
                }
                if let Some(caps) = re_dialogue.captures(trimmed) {
                    let speaker = caps.get(1).unwrap().as_str().to_lowercase();
                    if alias_set.contains(&speaker) {
                        let text = caps.get(2).unwrap().as_str().trim().to_string();
                        let list = samples.entry(speaker).or_default();
                        if list.len() < 2 && !list.contains(&text) {
                            list.push(text);
                        }
                    }
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
) -> Result<Vec<CharacterGlossaryEntry>, String> {
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
        return Ok(characters);
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

    // Обрабатываем пачками по 15 персонажей, чтобы не перегружать контекст модели
    const BATCH_SIZE: usize = 15;
    for chunk in indices_to_process.chunks(BATCH_SIZE) {
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

        let raw_response = call_chat_completion(
            api_url,
            model,
            system_prompt,
            &user_prompt,
            0.2, // низкая температура для точности и следования схеме
            2048,
        )
        .await?;

        let json_clean = extract_json_content(&raw_response);
        let items: Vec<CharacterLlmItem> = match serde_json::from_str(json_clean) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!(
                    "Не удалось распарсить JSON ответа LLM: {}. Сырой ответ:\n{}",
                    e, raw_response
                );
                // Пробуем продолжить без падения
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
    }

    Ok(characters)
}

/// Автозаполнение и перевод терминов в глоссарии через LLM
pub async fn autofill_terms(
    api_url: &str,
    model: &str,
    mut terms: Vec<TermGlossaryEntry>,
    only_empty: bool,
) -> Result<Vec<TermGlossaryEntry>, String> {
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
        return Ok(terms);
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
    for chunk in indices_to_process.chunks(BATCH_SIZE) {
        let mut user_prompt = String::from("Terms to translate into Russian:\n\n");
        for &idx in chunk {
            let t = &terms[idx];
            user_prompt.push_str(&format!(
                "- id: \"{}\", source_term: \"{}\", current_category: \"{}\"\n",
                t.id, t.source_term, t.category
            ));
        }
        user_prompt.push_str("\nProvide JSON array now:");

        let raw_response = call_chat_completion(
            api_url,
            model,
            system_prompt,
            &user_prompt,
            0.2,
            2048,
        )
        .await?;

        let json_clean = extract_json_content(&raw_response);
        let items: Vec<TermLlmItem> = match serde_json::from_str(json_clean) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!("Не удалось распарсить JSON терминов от LLM: {}", e);
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
    }

    Ok(terms)
}

/// Сканирует текст игры и извлекает ключевые термины, локации и титулы через LLM
pub async fn extract_terms_from_game_script(
    api_url: &str,
    model: &str,
    game_dir: &Path,
    max_terms: usize,
) -> Result<Vec<TermGlossaryEntry>, String> {
    if !game_dir.exists() {
        return Err(format!("Папка игры не найдена: {:?}", game_dir));
    }

    // Собираем репрезентативную выборку уникальных строк диалогов
    let mut sample_lines: Vec<String> = Vec::new();
    let walker = walkdir::WalkDir::new(game_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("rpy")
                && !e.path().to_string_lossy().contains(".editor-backups")
        });

    let re_text = Regex::new(r#"["']([A-Z][^"']{5,90})["']"#).unwrap();

    for entry in walker {
        if let Ok(content) = fs::read_to_string(entry.path()) {
            for line in content.lines() {
                if sample_lines.len() >= 60 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    continue;
                }
                if let Some(caps) = re_text.captures(trimmed) {
                    let s = caps.get(1).unwrap().as_str().trim().to_string();
                    if !sample_lines.contains(&s) {
                        sample_lines.push(s);
                    }
                }
            }
        }
        if sample_lines.len() >= 60 {
            break;
        }
    }

    if sample_lines.is_empty() {
        return Ok(Vec::new());
    }

    let system_prompt = r#"You are an expert game localizer.
Analyze the following sample game lines from an English visual novel/RPG.
Identify important proper nouns, locations, organizations/factions, unique items, and official titles.
Do NOT extract ordinary common words or character first names (like Jack, Emma).
Translate each discovered term into natural Russian.

Respond ONLY with a valid JSON array of up to 25 items:
[
  {
    "source_term": "Demon Hunter Association",
    "translated_term": "Ассоциация охотников на демонов",
    "category": "title",
    "notes": "Организация"
  }
]"#;

    let user_prompt = format!(
        "Game text excerpts:\n{}\n\nExtract and translate key terms, locations, items, titles (up to {} terms) as JSON array:",
        sample_lines.join("\n"),
        max_terms
    );

    let raw_response = call_chat_completion(
        api_url,
        model,
        system_prompt,
        &user_prompt,
        0.3,
        2048,
    )
    .await?;

    let json_clean = extract_json_content(&raw_response);
    let items: Vec<TermLlmItem> = serde_json::from_str(json_clean)
        .map_err(|e| format!("Не удалось распарсить извлеченные термины: {}", e))?;

    let mut result = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if let (Some(src), Some(trans)) = (item.source_term, item.translated_term) {
            let src_trimmed = src.trim().to_string();
            let trans_trimmed = trans.trim().to_string();
            if !src_trimmed.is_empty() && !trans_trimmed.is_empty() {
                let cat = item.category.unwrap_or_else(|| "general".to_string());
                let category = if matches!(cat.as_str(), "location" | "title" | "item" | "general") {
                    cat
                } else {
                    "general".to_string()
                };
                result.push(TermGlossaryEntry {
                    id: format!("term_auto_{}_{}", chrono::Utc::now().timestamp_millis(), i),
                    source_term: src_trimmed,
                    translated_term: trans_trimmed,
                    category,
                    notes: item.notes.unwrap_or_default(),
                });
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
