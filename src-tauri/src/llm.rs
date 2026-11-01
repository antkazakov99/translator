use crate::glossary::{CharacterGlossaryEntry, TermGlossaryEntry};
use crate::scenes::{self, GameLines, ScriptLine};
use crate::terms::{TermCandidate, is_stuttering};
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

/// Сколько реплик персонажа получает модель, если не задано другое.
pub const DEFAULT_CHARACTER_SAMPLES: usize = 64;
pub const MAX_CHARACTER_SAMPLES: usize = 256;
/// Предел текста реплик на персонажа и на запрос: окно контекста модели не бесконечно.
const MAX_SAMPLE_TEXT: usize = 12_000;
/// Подробные описания длинные: больше персонажей в ответе не успевают и не помещаются.
const MAX_BATCH_CHARACTERS: usize = 3;
const CHARACTER_ANSWER_TOKENS: u32 = 3072;
/// Локальная модель пишет подробный ответ минутами.
const REQUEST_TIMEOUT_SECS: u64 = 300;
const SCENE_NOTE_TOKENS: u32 = 768;
const MAX_CONDENSE_ROUNDS: usize = 3;
const MAX_SAMPLE_CHARS: usize = 200;
const MIN_SAMPLE_WORDS: usize = 4;
const EXPLICIT_WORDS: &[&str] = &[
    "anal", "ass", "balls", "blowjob", "boobs", "breasts", "clit", "cock", "cum", "cumming", "dick",
    "fuck", "fucking", "horny", "moan", "nipples", "orgasm", "pussy", "slut", "suck", "tits", "wet",
    "whore", "киска", "киску", "киски", "кончаю", "кончи", "член", "члена", "члену", "сиськи",
    "сиски", "соси", "трахни", "трахай",
];

/// Теги оформления вроде `{i}`. Подстановки `[name]` остаются: без них фраза теряет смысл.
static RE_TEXT_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").unwrap());
static RE_TEXT_VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([A-Za-z_][\w.]*)(?:![a-z]+)?\]").unwrap());
static RE_STRING_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*(?:(?:default|define)\s+|\$\s*)?(?:store\.)?([A-Za-z_][\w.]*)\s*=\s*_?\(?\s*(?:"([^"\\]*)"|'([^'\\]*)')\s*\)?\s*(?:#.*)?$"#).unwrap()
});
const MAX_VARIABLE_VALUES: usize = 3;
const MAX_VARIABLE_CHARS: usize = 40;

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
    speech: Option<String>,
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

    let client = make_client(REQUEST_TIMEOUT_SECS)?;
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

/// Подходит ли строка для понимания персонажа: короткие возгласы и стоны о нём не говорят.
fn is_informative(text: &str) -> bool {
    text.split_whitespace().count() >= MIN_SAMPLE_WORDS && !is_stuttering(text)
}

/// Откровенная реплика мало говорит о характере, поэтому берётся только при нехватке других.
fn is_explicit(text: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .any(|word| EXPLICIT_WORDS.contains(&word))
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

/// Подставляет значения переменных. Переменная с неясным значением остаётся как есть:
/// модели сказано не делать выводов из её имени.
fn resolve_variables(text: &str, variables: &HashMap<String, Vec<String>>) -> String {
    RE_TEXT_VARIABLE
        .replace_all(text, |caps: &regex::Captures| match variables.get(&caps[1]) {
            Some(values) if values.len() <= MAX_VARIABLE_VALUES => values.join("/"),
            _ => caps[0].to_string(),
        })
        .into_owned()
}

/// Выбирает реплики персонажа: сначала полезные и обычные, затем остальные.
fn pick_samples(lines: &[String], count: usize) -> Vec<String> {
    let usable = |line: &&String| is_informative(line);
    let plain: Vec<&String> = lines.iter().filter(usable).filter(|line| !is_explicit(line)).collect();
    let explicit: Vec<&String> = lines.iter().filter(usable).filter(|line| is_explicit(line)).collect();
    let rest: Vec<&String> = lines.iter().filter(|line| !is_informative(line)).collect();

    let mut picked = spread(&plain, count);
    for pool in [explicit, rest] {
        let missing = count - picked.len();
        picked.extend(spread(&pool, missing));
    }

    let text: usize = picked.iter().map(|line| line.chars().count()).sum();
    if text > MAX_SAMPLE_TEXT {
        let keep = (picked.len() * MAX_SAMPLE_TEXT / text).max(1);
        return spread(&picked.iter().collect::<Vec<_>>(), keep);
    }
    picked
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

/// Текст реплики в том виде, в каком его видит модель.
fn sample_text(text: &str, variables: &HashMap<String, Vec<String>>) -> String {
    let text = RE_TEXT_TAG.replace_all(text, "").replace('\n', " ");
    resolve_variables(text.trim(), variables)
}

/// Собирает реплики каждого персонажа из разных частей игры, кроме исключённых сцен
pub fn collect_character_dialogue_samples(
    script_lines: &[ScriptLine],
    variables: &HashMap<String, Vec<String>>,
    characters: &[CharacterGlossaryEntry],
    count: usize,
) -> HashMap<String, Vec<String>> {
    // Реплики всех кодов персонажа копятся под основным
    let mut owners: HashMap<String, (String, HashSet<&str>)> = HashMap::new();
    for character in characters {
        let excluded: HashSet<&str> = character.excluded_scenes.iter().map(String::as_str).collect();
        for alias in character.aliases() {
            owners.insert(alias.to_lowercase(), (character.alias.to_lowercase(), excluded.clone()));
        }
    }
    let mut lines: HashMap<String, Vec<String>> = HashMap::new();

    for line in script_lines {
        let Some((owner, skipped)) = line.alias.as_ref().and_then(|s| owners.get(&s.to_lowercase())) else {
            continue;
        };
        if skipped.contains(line.key().as_str()) {
            continue;
        }
        let text = sample_text(&line.text, variables);
        if (3..=MAX_SAMPLE_CHARS).contains(&text.chars().count()) {
            let list = lines.entry(owner.clone()).or_default();
            if !list.contains(&text) {
                list.push(text);
            }
        }
    }

    lines
        .into_iter()
        .map(|(speaker, list)| (speaker, pick_samples(&list, count)))
        .collect()
}

/// Тексты сцен в виде «Говорящий: реплика».
fn scene_transcripts(
    game: &GameLines,
    characters: &[CharacterGlossaryEntry],
    scenes: &HashSet<String>,
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
            .push(format!("{}: {}", speaker, sample_text(&line.text, &game.variables)));
    }
    by_scene
}

/// Собирает сцены персонажа целиком, со всеми участниками: так видно, кто с кем говорит.
/// Сцены берутся равномерно в пределах объёма.
pub fn collect_scene_samples(
    game: &GameLines,
    characters: &[CharacterGlossaryEntry],
) -> HashMap<String, Vec<String>> {
    let character_keys: Vec<Vec<String>> = characters
        .iter()
        .map(|character| {
            scenes::character_scenes(game, &character.aliases(), &character.names())
                .into_iter()
                .map(|scene| scene.key)
                .filter(|key| !character.excluded_scenes.contains(key))
                .collect()
        })
        .collect();
    let by_scene = scene_transcripts(game, characters, &character_keys.iter().flatten().cloned().collect());
    let mut samples = HashMap::new();
    for (character, keys) in characters.iter().zip(character_keys) {
        let text: usize = keys
            .iter()
            .flat_map(|key| by_scene.get(key).into_iter().flatten())
            .map(|line| line.chars().count())
            .sum();
        let scene_count = (keys.len() * MAX_SAMPLE_TEXT / text.max(1)).clamp(1, keys.len().max(1));

        let mut picked = Vec::new();
        let mut size = 0;
        'scenes: for i in 0..scene_count.min(keys.len()) {
            let key = &keys[i * keys.len() / scene_count];
            for line in by_scene.get(key).into_iter().flatten() {
                size += line.chars().count();
                if size > MAX_SAMPLE_TEXT {
                    break 'scenes;
                }
                picked.push(line.clone());
            }
        }
        if !picked.is_empty() {
            samples.insert(character.alias.to_lowercase(), picked);
        }
    }
    samples
}

/// Делит все включённые сцены персонажа на части, каждая из которых помещается в запрос.
pub fn scene_chunks(game: &GameLines, character: &CharacterGlossaryEntry) -> Vec<String> {
    let keys: Vec<String> = scenes::character_scenes(game, &character.aliases(), &character.names())
        .into_iter()
        .map(|scene| scene.key)
        .filter(|key| !character.excluded_scenes.contains(key))
        .collect();
    let transcripts = scene_transcripts(game, std::slice::from_ref(character), &keys.iter().cloned().collect());
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut size = 0;
    for key in &keys {
        for line in transcripts.get(key).into_iter().flatten() {
            let length = line.chars().count() + 1;
            if size > 0 && size + length > MAX_SAMPLE_TEXT {
                chunks.push(std::mem::take(&mut current));
                size = 0;
            }
            current.push_str(line);
            current.push('\n');
            size += length;
        }
    }
    if size > 0 {
        chunks.push(current);
    }
    chunks
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
    /// `scenes`, `note`, `condense`, `final` или `single`, если всё делает один запрос.
    pub stage: String,
    pub done: usize,
    pub total: usize,
    /// Заметка готовой части: окно показывает её сразу, не дожидаясь итога.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn note_key(model: &str, alias: &str, chunk: &str) -> String {
    format!("{:016x}", crate::stable_hash_str(&format!("{model}\n{alias}\n{chunk}")))
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
    api_url: &str,
    model: &str,
    mut notes: Vec<String>,
    on_progress: &(impl Fn(SummaryProgress) + Send + Sync),
) -> Result<Vec<String>, String> {
    let size = |notes: &[String]| notes.iter().map(|note| note.chars().count()).sum::<usize>();
    let system_prompt = "You merge notes about one character of a visual novel. Write in Russian. \
        Keep concrete facts: occupation, relations, events, manner of speech. Drop repetitions. \
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
            on_progress(SummaryProgress { stage: "condense".to_string(), done, total, note: None });
            let prompt = format!("Notes:\n\n{}\n\nMerge them:", group.join("\n\n"));
            condensed.push(call_chat_completion(api_url, model, system_prompt, &prompt, 0.2, CHARACTER_ANSWER_TOKENS).await?);
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
    api_url: &str,
    model: &str,
    character: CharacterGlossaryEntry,
    lines: Vec<String>,
) -> Result<CharacterGlossaryEntry, String> {
    let samples = HashMap::from([(character.alias.to_lowercase(), lines)]);
    let cancel = AtomicBool::new(false);
    let result = autofill_characters(api_url, model, vec![character], false, samples, &cancel, |_| {}).await?;
    match result.items.into_iter().next() {
        Some(updated) if result.failed_batches == 0 => Ok(updated),
        _ => Err("Модель не вернула данные о персонаже, попробуйте ещё раз".to_string()),
    }
}

/// Обновляет персонажа по частям его сцен: собирает заметки, а затем пишет описание по ним. Готовые заметки лежат на диске, поэтому остановленный запуск
/// продолжается с того же места. После остановки описание собирается из того, что уже прочитано.
/// Возвращает `None`, если запуск остановлен раньше первой заметки.
pub async fn summarize_character(
    api_url: &str,
    model: &str,
    character: CharacterGlossaryEntry,
    chunks: Vec<String>,
    notes_file: &Path,
    cancel: &AtomicBool,
    on_progress: impl Fn(SummaryProgress) + Send + Sync,
) -> Result<Option<CharacterGlossaryEntry>, String> {
    let system_prompt = format!(
        "You read a part of a visual novel script and take notes about the character {}. \
        The text has the form \"Speaker: text\", and \"you\" usually means the protagonist. \
        Write in Russian, 2 to 5 short sentences, only what this text shows: what the character does, \
        occupation, relations with others, and how they speak (tone, slang, \"ты\" or \"вы\"). \
        If the text says nothing about the character, answer only with: {NO_NOTE} \
        Text in [square brackets] is an in-game variable: never infer anything from its name. \
        Respond with plain text only.",
        character.original_name
    );

    let mut stored = read_scene_notes(notes_file);
    let total = chunks.len();

    // Одна новая часть: запрос сразу пишет описание по её тексту, и оно же становится заметкой
    let new_single_part = match chunks.as_slice() {
        [chunk] => Some(note_key(model, &character.alias, chunk)).filter(|key| !stored.contains_key(key)),
        _ => None,
    };
    if let Some(key) = new_single_part {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        on_progress(SummaryProgress { stage: "single".to_string(), done: 0, total, note: None });
        let lines = chunks[0].lines().map(str::to_string).collect();
        let updated = write_description(api_url, model, character, lines).await?;
        stored.insert(key, updated.notes.clone());
        write_scene_notes(notes_file, &stored)?;
        return Ok(Some(updated));
    }

    let mut notes = Vec::new();
    for (done, chunk) in chunks.iter().enumerate() {
        on_progress(SummaryProgress { stage: "scenes".to_string(), done, total, note: None });
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let key = note_key(model, &character.alias, chunk);
        let note = match stored.get(&key) {
            Some(note) => note.clone(),
            None => {
                let prompt = format!("Script:\n\n{chunk}\n\nNotes about {}:", character.original_name);
                let note = call_chat_completion(api_url, model, &system_prompt, &prompt, 0.2, SCENE_NOTE_TOKENS).await?;
                stored.insert(key, note.clone());
                write_scene_notes(notes_file, &stored)?;
                note
            }
        };
        let note = strip_old_marker(&note);
        if is_blank_note(note) {
            continue;
        }
        on_progress(SummaryProgress { stage: "note".to_string(), done, total, note: Some(note.to_string()) });
        if !is_refusal(note) {
            notes.push(note.to_string());
        }
    }

    if notes.is_empty() {
        return if cancel.load(Ordering::Relaxed) {
            Ok(None)
        } else {
            Err("В выбранных сценах модель не нашла сведений о персонаже".to_string())
        };
    }
    let notes = condense_notes(api_url, model, notes, &on_progress).await?;
    on_progress(SummaryProgress { stage: "final".to_string(), done: total, total, note: None });
    write_description(api_url, model, character, notes).await.map(Some)
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
4. "notes": Detailed description in Russian, 3 to 6 sentences, based only on the provided lines: occupation and place in the story, who they are to the protagonist (lines addressed to "you" are usually addressed to the protagonist), character and motives, how they speak, how they treat others, and what happens to them in the story. A translator will rely on it to keep the voice consistent. Do not invent facts. If there are few lines, write less.
5. "speech": Required separate field, 1 to 2 sentences in Russian on the manner of speech: tone, vocabulary or slang, and whether they use "ты" or "вы" with others. Fill it for every character, even with few lines. It may repeat what "notes" says: do not shorten "notes" because of it.

Text in [square brackets] is an in-game variable whose value is unknown. Never infer an occupation, kinship or name from the variable's own name.
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
    scenes::load_lines(scripts, |_| String::new())
        .into_iter()
        .map(|line| line.text)
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

    /// Реплики персонажей из скрипта теми же шагами, что и в команде.
    fn samples_from(scripts: &[PathBuf], aliases: &[&str], count: usize) -> HashMap<String, Vec<String>> {
        let lines = scenes::load_lines(scripts, |_| "story.rpy".to_string());
        let characters: Vec<CharacterGlossaryEntry> = aliases
            .iter()
            .map(|alias| CharacterGlossaryEntry { alias: alias.to_string(), ..Default::default() })
            .collect();
        collect_character_dialogue_samples(&lines, &string_variables(scripts), &characters, count)
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
            "    # e \"Commented out here for now\"\n",
            "    e \"Don't go to Jack's Bar tonight\"\n",
            "    m \"Hi\"\n",
            "    e \"{i}She said \\\"no\\\" to me twice{/i}\"\n",
        ));
        let samples = samples_from(&scripts, &["e", "m"], 5);
        assert_eq!(samples["e"], ["Don't go to Jack's Bar tonight", "She said \"no\" to me twice"]);
        assert!(!samples.contains_key("m"), "короткая реплика не даёт контекста");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_samples_come_from_whole_game() {
        let lines: Vec<String> = (0..100).map(|i| format!("Line number {i} of the story")).collect();
        let samples = pick_samples(&lines, 5);
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[0], "Line number 0 of the story");
        assert_eq!(samples[4], "Line number 80 of the story");
    }

    #[test]
    fn test_samples_prefer_lines_about_the_character() {
        let plain = "I was the best student in my class";
        let lines: Vec<String> = [
            "Aaaah... mmmmh...",
            "Oh!",
            "Please fuck me harder right now",
            plain,
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            pick_samples(&lines, 5),
            [plain, "Please fuck me harder right now", "Aaaah... mmmmh...", "Oh!"]
        );
    }

    #[test]
    fn test_samples_fit_context_window() {
        let lines: Vec<String> = (0..300).map(|i| format!("{i} {}", "word ".repeat(30))).collect();
        let samples = pick_samples(&lines, 256);
        let text: usize = samples.iter().map(|line| line.chars().count()).sum();
        assert!(text <= MAX_SAMPLE_TEXT, "{text}");
        assert!(samples.len() > 50);
        assert!(samples.last().unwrap().starts_with("29") || samples.last().unwrap().starts_with("2"));
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
        let samples = samples_from(&scripts, &["k"], 5);
        assert_eq!(
            samples["k"],
            ["I am your mom, sis.", "Going to the bar/club with [friend.name] now."]
        );
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
    fn test_excluded_scene_is_not_sent_to_model() {
        let (dir, scripts) = script_with(TWO_SCENES);
        let lines = scenes::load_lines(&scripts, |_| "story.rpy".to_string());
        let samples =
            collect_character_dialogue_samples(&lines, &HashMap::new(), &[kate(&["story.rpy::bar"])], 5);
        assert_eq!(samples["k"], ["I am staying home tonight."]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_samples_join_merged_codes() {
        let (dir, scripts) = script_with("label one:\n    hil \"First line of her.\"\n    hi \"Second line of her.\"\n");
        let lines = scenes::load_lines(&scripts, |_| "story.rpy".to_string());
        let hilde = CharacterGlossaryEntry {
            alias: "hil".to_string(),
            original_name: "Hilde".to_string(),
            merged: vec![crate::glossary::MergedCharacter {
                alias: "hi".to_string(),
                original_name: "Hilde".to_string(),
            }],
            ..Default::default()
        };
        let samples = collect_character_dialogue_samples(&lines, &HashMap::new(), &[hilde], 5);
        assert_eq!(samples["hil"], ["First line of her.", "Second line of her."]);
        assert!(!samples.contains_key("hi"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_whole_scene_keeps_all_speakers() {
        let (dir, scripts) = script_with(TWO_SCENES);
        let game = GameLines::new(scenes::load_lines(&scripts, |_| "story.rpy".to_string()), HashMap::new());

        let all = collect_scene_samples(&game, &[kate(&["story.rpy::bar"])]);
        assert_eq!(all["k"], ["Kate: I am staying home tonight.", "m: Then I will cook, Kate."]);
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

        let chunks = scene_chunks(&game, &kate(&[]));
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= MAX_SAMPLE_TEXT));
        let text = chunks.concat();
        assert!(text.contains("Scene 0 line 0 ") && text.contains("Scene 2 line 119 "));

        let without = scene_chunks(&game, &kate(&["story.rpy::scene_2"]));
        assert!(!without.concat().contains("Scene 2 line"));
        let _ = fs::remove_dir_all(dir);
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
            &url, "model", kate(&[]), chunks.clone(), &notes_file, &cancel,
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
            &url, "model", kate(&[]), chunks.clone(), &notes_file, &cancel,
            |p| stages.lock().unwrap().push(p.stage),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(updated.notes, "Итог");
        assert_eq!(updated.speech, "На 'ты'");
        assert_eq!(stages.into_inner().unwrap(), ["scenes", "note", "scenes", "note", "final"]);
        assert_eq!(ready_chunks(&chunks, "model", &kate(&[]).alias, &notes_file), [true, true]);
        assert_eq!(ready_chunks(&chunks, "other", &kate(&[]).alias, &notes_file), [false, false]);
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
                &url, "model", kate(&[]), chunks.clone(), &notes_file, &cancel,
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
        assert_eq!(stages.into_inner().unwrap(), ["scenes", "note", "final"]);
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
        let shown = std::sync::Mutex::new(Vec::new());

        let url = serve_responses(vec![
            completion("Пустая строка"),
            completion("Заметка. NONE"),
            completion(NO_NOTE),
            completion(answer),
        ]);
        let updated = tauri::async_runtime::block_on(summarize_character(
            &url, "model", kate(&[]), chunks, &notes_file, &cancel,
            |p| shown.lock().unwrap().extend(p.note),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(updated.notes, "Итог");
        assert_eq!(shown.into_inner().unwrap(), ["Заметка.", NO_NOTE]);

        // Заметок нет: описание нечему обновлять, модель не должна придумывать его по имени
        let url = serve_responses(vec![completion("NONE"), completion(NO_NOTE)]);
        let result = tauri::async_runtime::block_on(summarize_character(
            &url, "model", kate(&[]), vec!["Kate: four".to_string(), "Kate: six".to_string()], &notes_file, &cancel,
            |_| {},
        ));
        assert!(result.is_err());

        // Остановка до первой заметки: собирать нечего, и это не ошибка
        cancel.store(true, Ordering::Relaxed);
        let stopped = tauri::async_runtime::block_on(summarize_character(
            "http://127.0.0.1:1", "model", kate(&[]), vec!["Kate: five".to_string()], &notes_file, &cancel,
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
            &url, "model", vec![long.clone(), long], &|_| {},
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
            &url, "model", characters(1), true, HashMap::new(), &cancel, |_| {},
        ))
        .unwrap();
        assert_eq!(result.items[0].notes, "Вежливая");
        assert_eq!(result.items[0].speech, "На 'вы'");
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
        let url = serve_responses(vec![answer(0, 3), answer(3, 5)]);
        let samples = HashMap::from([("c4".to_string(), vec!["Don't go".to_string()])]);
        let cancel = AtomicBool::new(false);
        let reports = std::sync::Mutex::new(Vec::new());

        let result = tauri::async_runtime::block_on(autofill_characters(
            &url, "model", characters(5), true, samples, &cancel,
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
