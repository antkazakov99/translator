pub mod glossary;
pub mod graph;
pub mod parser;
pub mod pickle;
pub mod rpa;
pub mod rpyc;
pub mod rpyc_writer;
pub mod translation;
pub mod llm;

use crate::graph::ScenarioNode;
use crate::parser::GameParser;
use crate::parser::renpy::{ChoiceBlockDto, RenpyParser, SceneEdgeDto, ScriptStructure};
use rayon::prelude::*;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Instant, SystemTime};

/// Язык перевода: имя папки `tl/<язык>` и блоков `translate <язык>`.
pub(crate) const TL_LANG: &str = "russian";

/// Стабильный FNV-1a хэш для воспроизводимых имён кэш-папок между
/// запусками и версиями компилятора (в отличие от DefaultHasher).
pub fn stable_hash_str(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Путь к папке `tl/russian` внутри директории игры.
fn tl_lang_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("tl").join(TL_LANG)
}

fn tl_segment_position(parts: &[Component]) -> Option<usize> {
    parts.windows(2).position(|pair| {
        pair[0].as_os_str().eq_ignore_ascii_case("tl")
            && pair[1].as_os_str().eq_ignore_ascii_case(TL_LANG)
    })
}

/// Путь к исходному скрипту: убирает сегмент `tl/<язык>` из пути файла перевода.
fn original_script_path(path: &Path) -> PathBuf {
    let parts: Vec<Component> = path.components().collect();
    match tl_segment_position(&parts) {
        Some(pos) => parts[..pos].iter().chain(&parts[pos + 2..]).collect(),
        None => path.to_path_buf(),
    }
}

/// Исходный скрипт файла: рядом с файлами игры или в кэше скриптов из архивов.
fn resolve_original_script(path: &Path, game_root: &Path) -> PathBuf {
    let direct = original_script_path(path);
    if !direct.exists()
        && let Some(name) = tl_relative_name(path)
    {
        let cached = get_project_scripts_cache_dir(game_root).join(name);
        if cached.exists() {
            return cached;
        }
    }
    direct
}

/// Путь файла перевода относительно `tl/<язык>`; `None` для остальных файлов.
pub(crate) fn tl_relative_name(path: &Path) -> Option<String> {
    let parts: Vec<Component> = path.components().collect();
    let pos = tl_segment_position(&parts)?;
    let relative: PathBuf = parts[pos + 2..].iter().collect();
    Some(relative.to_string_lossy().replace('\\', "/"))
}

static PATH_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Блокировка на путь: сериализует запись одного файла или папки, не мешая остальным.
pub(crate) fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    PATH_LOCKS.lock().entry(path.to_path_buf()).or_default().clone()
}

#[derive(Clone)]
struct ParsedFileCacheEntry {
    file_mtime: SystemTime,
    orig_mtime: Option<SystemTime>,
    result: ParsedFileDto,
}

static PARSED_FILE_CACHE: LazyLock<Mutex<HashMap<String, ParsedFileCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static SCRIPT_STRUCTURE_CACHE: LazyLock<Mutex<HashMap<PathBuf, (SystemTime, ScriptStructure)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

type SpeakerMap = HashMap<String, String>;

/// Данные просканированных проектов. Поля обновляются под одной блокировкой,
/// поэтому сканирование публикует список скриптов и персонажей одновременно.
#[derive(Default)]
struct ProjectCache {
    scripts: HashMap<PathBuf, (Instant, Vec<PathBuf>)>,
    speakers: HashMap<PathBuf, SpeakerMap>,
    cached_script_projects: HashMap<PathBuf, PathBuf>,
}

static PROJECT_CACHE: LazyLock<Mutex<ProjectCache>> =
    LazyLock::new(|| Mutex::new(ProjectCache::default()));

fn get_cached_script_structure(path: &Path) -> Option<ScriptStructure> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    {
        let cache = SCRIPT_STRUCTURE_CACHE.lock();
        if let Some((cached_mtime, cached_struct)) = cache.get(path)
            && *cached_mtime == mtime
        {
            return Some(cached_struct.clone());
        }
    }
    let content = std::fs::read_to_string(path).ok()?;
    let structure = RenpyParser::parse_script_structure(&content);
    {
        let mut cache = SCRIPT_STRUCTURE_CACHE.lock();
        cache.insert(path.to_path_buf(), (mtime, structure.clone()));
    }
    Some(structure)
}

fn get_project_scripts(search_dir: &Path, cache_dir: &Path) -> Vec<PathBuf> {
    {
        let cache = PROJECT_CACHE.lock();
        if let Some((instant, files)) = cache.scripts.get(search_dir)
            && instant.elapsed() < std::time::Duration::from_secs(60)
        {
            return files.clone();
        }
    }
    let mut all_scripts = Vec::new();
    collect_rpy_files(search_dir, &mut all_scripts);
    if cache_dir.exists() {
        collect_rpy_files(cache_dir, &mut all_scripts);
    }
    {
        let mut cache = PROJECT_CACHE.lock();
        cache.scripts.insert(
            search_dir.to_path_buf(),
            (Instant::now(), all_scripts.clone()),
        );
    }
    all_scripts
}

/// Максимальная глубина рекурсивного обхода каталогов скриптов Ren'Py
pub(crate) const MAX_SCAN_DEPTH: usize = 20;

/// Ограничение на количество сканируемых скриптов при быстром извлечении спикеров,
/// чтобы не блокировать первичный скан проекта чтением сотен файлов.
const MAX_SPEAKER_SCAN_FILES: usize = 40;

/// Рекурсивно собирает все файлы `.rpy`, строго игнорируя:
/// - скрытые директории и файлы (.codex-backups, .editor-backups, .git)
/// - файлы бэкапов (*.bak, *.backup, *.tmp)
fn collect_rpy_files(dir: &Path, files: &mut Vec<PathBuf>) {
    collect_rpy_files_depth(dir, files, 0);
}

fn collect_rpy_files_depth(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            // Игнорируем скрытые папки (.codex-backups, .editor-backups, .git), папки с бэкапами и сторонние инжекторы _mods
            if file_name.starts_with('.')
                || file_name == "_mods"
                || file_name.contains("backup")
                || file_name.contains("tmp")
            {
                continue;
            }

            if path.is_dir() {
                collect_rpy_files_depth(&path, files, depth + 1);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rpy") {
                // Исключаем любые бэкапы вида *.bak или *.bak.*
                if !file_name.ends_with(".bak") && !file_name.contains(".bak") {
                    files.push(path);
                }
            }
        }
    }
}

/// Проверяет наличие архивов `.rpa` в директории игры
fn has_rpa_archives(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("rpa") {
                return true;
            }
        }
    }
    false
}

/// Папка данных приложения вне каталогов игр.
fn app_data_dir() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("TranslatorApp")
}

/// Возвращает путь к изолированной папке кэша в AppData для конкретной игры,
/// чтобы оригинальная папка игры никогда не засорялась распакованными файлами.
/// Использует FNV-1a — стабильный хэш, воспроизводимый между версиями компилятора.
pub fn get_project_cache_dir(game_dir: &Path) -> PathBuf {
    let hash = format!("{:012x}", stable_hash_str(&game_dir.to_string_lossy().to_lowercase()));

    let folder_name = game_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "game".to_string());

    app_data_dir()
        .join("projects_cache")
        .join(format!("{}_{}", folder_name, hash))
}

fn get_project_scripts_cache_dir(game_dir: &Path) -> PathBuf {
    get_project_cache_dir(game_dir).join("scripts")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationFileItem {
    pub name: String,
    pub path: String,
    pub size_kb: f64,
    pub total_lines: usize,
    pub translated_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub game_name: String,
    pub characters: Vec<(String, String)>,
    pub files: Vec<TranslationFileItem>,
    pub total_lines: usize,
    pub total_translated: usize,
}

/// Файл `tl/<язык>` другого языка. Путь задаётся относительно папки скриптов игры.
fn is_other_language_file(relative: &Path) -> bool {
    let mut parts = relative.components();
    parts.next().is_some_and(|part| part.as_os_str().eq_ignore_ascii_case("tl"))
        && parts.next().is_some_and(|part| !part.as_os_str().eq_ignore_ascii_case(TL_LANG))
}

/// Параллельно строит список `TranslationFileItem` из набора путей.
/// Фильтрует файлы без диалоговых строк (`total == 0`) и переводы на другие
/// языки: их реплики дублируют оригинал, а перевод идёт в `tl/<язык>` проекта.
/// `base_dir` используется для получения относительного имени файла.
fn build_translation_file_items(
    paths: Vec<PathBuf>,
    base_dir: &Path,
) -> Vec<TranslationFileItem> {
    paths
        .into_par_iter()
        .filter(|path| !path.strip_prefix(base_dir).is_ok_and(is_other_language_file))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            let size_kb = content.len() as f64 / 1024.0;
            let (total, translated) = RenpyParser::quick_translation_stats(&content);
            if total == 0 {
                return None;
            }
            let relative_name = path
                .strip_prefix(base_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
            Some(TranslationFileItem {
                name: relative_name,
                path: path.to_string_lossy().to_string(),
                size_kb,
                total_lines: total,
                translated_lines: translated,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueEntryDto {
    pub id: String,
    pub translation_key: String,
    pub line_number: usize,
    pub target_lines: Vec<usize>,
    pub speaker: Option<String>,
    pub speaker_alias: Option<String>,
    pub original_text: String,
    pub translated_text: Option<String>,
    pub status: String,
    pub scene_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneDto {
    pub id: String,
    pub name: String,
    pub total_lines: usize,
    pub translated_lines: usize,
    pub choices: Vec<ChoiceBlockDto>,
    pub jumps: Vec<String>,
    pub entries: Vec<DialogueEntryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFileDto {
    pub file_path: String,
    pub file_name: String,
    pub total_lines: usize,
    pub translated_lines: usize,
    pub scenes: Vec<SceneDto>,
    pub edges: Vec<SceneEdgeDto>,
}

#[tauri::command]
fn resolve_game_directory(path: String) -> Result<String, String> {
    let selected_path = PathBuf::from(&path);
    if !selected_path.is_dir() {
        return Err(format!("Папка игры не найдена: {}", path));
    }

    Ok(effective_game_directory(&selected_path)
        .to_string_lossy()
        .to_string())
}

fn effective_game_directory(selected_path: &Path) -> PathBuf {
    let nested_game_path = selected_path.join("game");
    if nested_game_path.is_dir() {
        nested_game_path
    } else {
        selected_path.to_path_buf()
    }
}

fn is_dir_named(dir: &Path, name: &str) -> bool {
    dir.file_name()
        .is_some_and(|dir_name| dir_name.eq_ignore_ascii_case(name))
}

/// Признаки корня игры. Папка языка `tl/<язык>` повторяет файлы игры
/// (options.rpy, screens.rpy), поэтому корнем считаться не может.
fn is_game_root(dir: &Path) -> bool {
    let is_language_dir = dir.parent().is_some_and(|parent| is_dir_named(parent, "tl"));
    is_dir_named(dir, "game")
        || (!is_language_dir
            && (dir.join("tl").is_dir()
                || dir.join("options.rpy").is_file()
                || dir.join("screens.rpy").is_file()
                || dir.join("gui.rpy").is_file()))
}

fn known_game_directory(file_path: &Path) -> Option<PathBuf> {
    file_path
        .ancestors()
        .find(|dir| dir.is_dir() && is_game_root(dir))
        .map(Path::to_path_buf)
}

fn ensure_output_outside_game(output: &Path, game_dirs: &[PathBuf]) -> Result<(), String> {
    if !output.is_absolute()
        || output
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("Укажите абсолютный путь без . или .. для вывода файлов".to_string());
    }

    let mut existing = output;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or("Не удалось проверить целевой путь")?;
    }
    let existing_real = existing
        .canonicalize()
        .map_err(|e| format!("Не удалось проверить целевой путь {:?}: {}", existing, e))?;
    let resolved_output = existing_real.join(
        output
            .strip_prefix(existing)
            .map_err(|e| format!("Не удалось проверить целевой путь: {}", e))?,
    );

    let mut protected_dirs = game_dirs.to_vec();
    if let Some(game_dir) = known_game_directory(output) {
        protected_dirs.push(game_dir);
    }
    for game_dir in &protected_dirs {
        let game_real = game_dir
            .canonicalize()
            .map_err(|e| format!("Не удалось проверить папку игры {:?}: {}", game_dir, e))?;
        if resolved_output.starts_with(&game_real) || game_real.starts_with(&resolved_output) {
            return Err("Вывод в папку игры запрещён. Выберите папку данных приложения или другое место вне игры".to_string());
        }
    }
    Ok(())
}

/// Находит базовую директорию игры (`game`), поднимаясь по дереву каталогов от файла перевода или скрипта.
fn find_game_root(file_path: &Path) -> PathBuf {
    if let Some(game_dir) = {
        let cache = PROJECT_CACHE.lock();
        cache
            .cached_script_projects
            .iter()
            .find_map(|(cache_dir, game_dir)| {
                file_path.starts_with(cache_dir).then(|| game_dir.clone())
            })
    } {
        return game_dir;
    }

    known_game_directory(file_path).unwrap_or_else(|| {
        file_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

#[tauri::command]
async fn scan_game_project(game_dir: String) -> Result<ProjectSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        if !base_path.exists() {
            return Err(format!("Папка игры не найдена: {}", game_dir));
        }

        // 1. Ищем персонажей в оригинальных скриптах игры в параллельных потоках (Rayon)
        let mut script_files = Vec::new();
        collect_rpy_files(&base_path, &mut script_files);

        // 2. Если в папке игры есть архивы .rpa — распаковываем ТОЛЬКО скрипты в изолированный AppData кэш
        let cache_dir = get_project_scripts_cache_dir(&base_path);
        if has_rpa_archives(&base_path) {
            rpa::RpaArchive::extract_scripts_from_game_archives(&base_path, &cache_dir)?;
            PROJECT_CACHE
                .lock()
                .cached_script_projects
                .insert(cache_dir.clone(), base_path.clone());
            collect_rpy_files(&cache_dir, &mut script_files);
        } else {
            let cache_lock = path_lock(&cache_dir);
            let _guard = cache_lock.lock();
            if cache_dir.exists() {
                std::fs::remove_dir_all(&cache_dir).map_err(|e| {
                    format!(
                        "Не удалось удалить устаревший кэш скриптов {:?}: {}",
                        cache_dir, e
                    )
                })?;
            }
        }

        let mut characters_map: std::collections::HashMap<String, String> = script_files
            .par_iter()
            .filter_map(|path| {
                let path_str = path.to_string_lossy();
                if !path_str.contains(r"\tl\") && !path_str.contains("/tl/") {
                    std::fs::read_to_string(path)
                        .ok()
                        .map(|content| RenpyParser::extract_speakers(&content))
                } else {
                    None
                }
            })
            .reduce(std::collections::HashMap::new, |mut acc, map| {
                acc.extend(map);
                acc
            });

        // Подставляем имя главного героя вместо шаблонных переменных вроде [player.name]
        if let Some(player_name) = characters_map.get("player").cloned() {
            for v in characters_map.values_mut() {
                if v == "[player.name]" || v == "[player]" {
                    *v = player_name.clone();
                }
            }
        }

        let mut characters: Vec<(String, String)> = characters_map.into_iter().collect();
        characters.sort_by(|a, b| a.1.cmp(&b.1));

        // Сохраняем персонажей и список скриптов в глобальный кэш проекта атомарно под одной блокировкой
        {
            let mut cache = PROJECT_CACHE.lock();
            let mut map = HashMap::new();
            for (alias, name) in &characters {
                map.insert(alias.clone(), name.clone());
            }
            cache.speakers.insert(base_path.clone(), map);
            cache.scripts.insert(base_path.clone(), (Instant::now(), script_files.clone()));
        }

        // 3. Ищем файлы переводов в game/tl/russian в параллельных потоках (сверхбыстрый подсчет строк).
        // Запакованный перевод к этому моменту восстановлен в кэше
        let disk_tl_dir = tl_lang_dir(&base_path);
        let packed_tl_dir = cache_dir.join("tl").join(TL_LANG);
        let tl_dir = if disk_tl_dir.exists() || !packed_tl_dir.exists() {
            disk_tl_dir
        } else {
            packed_tl_dir
        };
        let mut files = Vec::new();
        let mut grand_total = 0;
        let mut grand_translated = 0;

        if tl_dir.exists() {
            let mut tl_files = Vec::new();
            collect_rpy_files(&tl_dir, &mut tl_files);

            let mut parsed_files = build_translation_file_items(tl_files, &tl_dir);

            for f in &parsed_files {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_files.sort_by(|a, b| {
                b.size_kb
                    .partial_cmp(&a.size_kb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            files = parsed_files;
        }

        // 4. Если в tl/russian пусто (новый перевод), но в кэше есть скрипты из RPA — выводим их как базу для перевода
        if files.is_empty() && cache_dir.exists() {
            let mut cached_files = Vec::new();
            collect_rpy_files(&cache_dir, &mut cached_files);

            let mut parsed_cached = build_translation_file_items(cached_files, &cache_dir);

            for f in &parsed_cached {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_cached.sort_by(|a, b| {
                b.size_kb
                    .partial_cmp(&a.size_kb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            files = parsed_cached;
        }

        // 5. Если в tl/russian пусто и нет RPA, но есть распакованные .rpy в папке игры
        if files.is_empty() && !script_files.is_empty() {
            let loose: Vec<PathBuf> = script_files
                .into_iter()
                .filter(|p| !p.starts_with(&cache_dir))
                .collect();
            let mut parsed_loose = build_translation_file_items(loose, &base_path);

            for f in &parsed_loose {
                grand_total += f.total_lines;
                grand_translated += f.translated_lines;
            }

            parsed_loose.sort_by(|a, b| {
                b.size_kb
                    .partial_cmp(&a.size_kb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            files = parsed_loose;
        }

        Ok(ProjectSummary {
            game_name: base_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            characters,
            files,
            total_lines: grand_total,
            total_translated: grand_translated,
        })
    })
    .await
    .map_err(|e| format!("Ошибка задачи сканирования: {}", e))?
}

#[tauri::command]
async fn parse_file_entries(file_path: String) -> Result<ParsedFileDto, String> {
    tauri::async_runtime::spawn_blocking(move || parse_project_file(file_path))
        .await
        .map_err(|e| format!("Ошибка задачи парсинга: {}", e))?
}

/// Какой сцене принадлежит текст выбора. Один текст может встречаться в разных
/// сценах, а порядок обхода `scene_choices` случаен, поэтому берётся первая сцена
/// по порядку объявления. Иначе ключ строки менялся бы между запусками.
fn choice_scenes(structure: &ScriptStructure) -> HashMap<String, String> {
    let mut scenes = HashMap::new();
    for label in &structure.labels {
        let Some(choices) = structure.scene_choices.get(&label.name) else {
            continue;
        };
        for option in choices.iter().flat_map(|choice| &choice.options) {
            scenes
                .entry(option.text.trim().to_string())
                .or_insert_with(|| label.name.clone());
        }
    }
    scenes
}

/// Разбирает файл проекта на сцены и подставляет сохранённые правки перевода.
pub(crate) fn parse_project_file(file_path: String) -> Result<ParsedFileDto, String> {
    let path = Path::new(&file_path);
    let file_mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("Не удалось получить свойства файла: {}", e))?;

    let search_dir = find_game_root(path);
    let cache_dir = get_project_scripts_cache_dir(&search_dir);

    let orig_path = resolve_original_script(path, &search_dir);
    let orig_path = orig_path.as_path();
    let orig_mtime = if orig_path.exists() {
        std::fs::metadata(orig_path).and_then(|m| m.modified()).ok()
    } else {
        None
    };

    // 1. Проверяем кэш готового распарсенного файла (мгновенный отклик при повторном клике)
    {
        let cache = PARSED_FILE_CACHE.lock();
        if let Some(entry) = cache.get(&file_path)
            && entry.file_mtime == file_mtime
            && entry.orig_mtime == orig_mtime
        {
            return Ok(entry.result.clone());
        }
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Не удалось прочитать файл: {}", e))?;

    // 2. Персонажи / speakers: берем из кэша проекта, если есть
    let speakers_map = {
        let cache = PROJECT_CACHE.lock();
        cache.speakers.get(&search_dir).cloned()
    };
    let speakers_map = match speakers_map {
        Some(map) => map,
        None => {
            let mut map = HashMap::new();
            let mut check_files = Vec::new();
            collect_rpy_files(&search_dir, &mut check_files);
            collect_rpy_files(&cache_dir, &mut check_files);
            for f in check_files.into_iter().take(MAX_SPEAKER_SCAN_FILES) {
                if let Ok(c) = std::fs::read_to_string(&f) {
                    map.extend(RenpyParser::extract_speakers(&c));
                }
            }
            // Полная карта от параллельного сканирования важнее этой быстрой выборки
            let mut cache = PROJECT_CACHE.lock();
            cache
                .speakers
                .entry(search_dir.clone())
                .or_insert(map)
                .clone()
        }
    };

    let parser = RenpyParser::new().with_speakers(speakers_map);
    let graph = parser
        .parse_file(path, &content)
        .map_err(|e| format!("Ошибка парсинга: {}", e))?;
    let saved_translations = translation::load_for_graph(path, &graph)?;

    // 3. Извлекаем граф ветвлений, выборов и переходов из оригинального скрипта игры (с кэшированием структуры)
    let script_struct = if orig_path.exists() {
        get_cached_script_structure(orig_path).unwrap_or_default()
    } else {
        get_cached_script_structure(path).unwrap_or_default()
    };

    // Быстрый поиск: текст выбора -> имя сцены
    let choice_text_to_scene = choice_scenes(&script_struct);

    let mut scenes: Vec<SceneDto> = Vec::new();
    let mut file_total = 0;
    let mut file_translated = 0;
    let mut current_scene_name = "Общие диалоги".to_string();
    let mut line_repeats = HashMap::new();

    for idx in graph.node_indices() {
        if let Some(node) = graph.get_node(idx) {
            match node {
                ScenarioNode::Label(label_name) => {
                    current_scene_name = label_name.clone();
                }
                ScenarioNode::Dialogue(d) => {
                    let (target_scene_name, is_menu_choice) =
                        if d.speaker.as_deref() == Some("UI / Choice") {
                            let trimmed_orig = d.original_text.trim();
                            if let Some(scene) = choice_text_to_scene.get(trimmed_orig) {
                                (scene.clone(), true)
                            } else {
                                ("Интерфейс и меню (strings)".to_string(), false)
                            }
                        } else {
                            (current_scene_name.clone(), false)
                        };

                    let translation_key =
                        translation::LineKey::for_line(d, &mut line_repeats).encode()?;
                    let translated_text = saved_translations
                        .get(&translation_key)
                        .map(|text| Some(text.clone()))
                        .unwrap_or_else(|| d.translated_text.clone());
                    let is_trans = translated_text
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty());
                    file_total += 1;
                    if is_trans {
                        file_translated += 1;
                    }

                    // Находим существующую сцену с таким именем (чтобы реплики склеивались в одну сцену)
                    let scene_idx = if let Some(pos) =
                        scenes.iter().position(|s| s.name == target_scene_name)
                    {
                        pos
                    } else {
                        let new_id = format!("scene_{}", scenes.len());
                        scenes.push(SceneDto {
                            id: new_id,
                            name: target_scene_name.clone(),
                            total_lines: 0,
                            translated_lines: 0,
                            choices: Vec::new(),
                            jumps: Vec::new(),
                            entries: Vec::new(),
                        });
                        scenes.len() - 1
                    };

                    let scene = &mut scenes[scene_idx];
                    scene.total_lines += 1;
                    if is_trans {
                        scene.translated_lines += 1;
                    }

                    let mut speaker = d.speaker.clone();
                    let mut speaker_alias = d.speaker_alias.clone();
                    if is_menu_choice {
                        speaker = Some("Выбор в меню".to_string());
                        speaker_alias = Some("Choice".to_string());
                    }

                    scene.entries.push(DialogueEntryDto {
                        id: d.id.clone(),
                        translation_key,
                        line_number: d.line_number,
                        target_lines: d.target_lines.clone(),
                        speaker,
                        speaker_alias,
                        original_text: d.original_text.clone(),
                        translated_text,
                        status: format!("{:?}", d.status),
                        scene_name: target_scene_name,
                    });
                }
                _ => {}
            }
        }
    }

    // Гарантируем, что ВСЕ сюжетные сцены/метки из оригинального файла присутствуют в списке сцен
    for label_info in &script_struct.labels {
        if !scenes.iter().any(|s| s.name == label_info.name) {
            scenes.push(SceneDto {
                id: format!("scene_{}", scenes.len()),
                name: label_info.name.clone(),
                total_lines: 0,
                translated_lines: 0,
                choices: Vec::new(),
                jumps: Vec::new(),
                entries: Vec::new(),
            });
        }
    }

    // Упорядочиваем сцены в порядке объявления в оригинальном скрипте
    scenes.sort_by_key(|s| {
        if s.name == "Интерфейс и меню (strings)" {
            usize::MAX
        } else {
            script_struct
                .labels
                .iter()
                .position(|l| l.name == s.name)
                .unwrap_or(usize::MAX - 1)
        }
    });

    // Переиндексируем ID сцен
    for (idx, scene) in scenes.iter_mut().enumerate() {
        scene.id = format!("scene_{}", idx);
    }

    // Привязываем найденные выборы (меню) и переходы к каждой сцене
    for scene in &mut scenes {
        if let Some(choices) = script_struct.scene_choices.get(&scene.name) {
            scene.choices = choices.clone();
        }
        if let Some(jumps) = script_struct.scene_jumps.get(&scene.name) {
            scene.jumps = jumps.iter().map(|(target, _)| target.clone()).collect();
        }
    }

    let mut edges = script_struct.edges;
    // Если переходов в файле не обнаружено, связываем сцены последовательно
    if edges.is_empty() && scenes.len() > 1 {
        for i in 0..scenes.len() - 1 {
            if scenes[i].name != "Интерфейс и меню (strings)"
                && scenes[i + 1].name != "Интерфейс и меню (strings)"
            {
                edges.push(SceneEdgeDto {
                    from: scenes[i].name.clone(),
                    to: scenes[i + 1].name.clone(),
                    kind: "linear".to_string(),
                    condition: None,
                });
            }
        }
    }

    // Поиск входящих переходов из ДРУГИХ скриптов проекта в сцены текущего файла
    let current_scene_names: std::collections::HashSet<String> = scenes
        .iter()
        .map(|s| s.name.clone())
        .filter(|name| name != "Интерфейс и меню (strings)")
        .collect();

    if !current_scene_names.is_empty() {
        let all_scripts = get_project_scripts(&search_dir, &cache_dir);

        let cur_file_norm = path.to_string_lossy().to_lowercase().replace('\\', "/");
        let cur_orig_norm = orig_path
            .to_string_lossy()
            .to_lowercase()
            .replace('\\', "/");

        let external_incoming_edges: Vec<SceneEdgeDto> = all_scripts
            .into_par_iter()
            .filter(|p| {
                let s = p.to_string_lossy().replace('\\', "/");
                let s_lower = s.to_lowercase();
                if s_lower.contains("/tl/") {
                    return false;
                }
                if s_lower == cur_file_norm || s_lower == cur_orig_norm {
                    return false;
                }
                true
            })
            .filter_map(|other_path| {
                let other_struct = get_cached_script_structure(&other_path)?;
                let mut matched = Vec::new();
                for edge in other_struct.edges {
                    if current_scene_names.contains(&edge.to) {
                        matched.push(edge);
                    }
                }
                if matched.is_empty() {
                    None
                } else {
                    Some(matched)
                }
            })
            .flatten()
            .collect();

        for ext_edge in external_incoming_edges {
            if !edges.iter().any(|e| {
                e.from == ext_edge.from
                    && e.to == ext_edge.to
                    && e.kind == ext_edge.kind
                    && e.condition == ext_edge.condition
            }) {
                edges.push(ext_edge);
            }
        }
    }

    let parsed_result = ParsedFileDto {
        file_path: file_path.clone(),
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        total_lines: file_total,
        translated_lines: file_translated,
        scenes,
        edges,
    };

    // Сохраняем в кэш распарсенных файлов для мгновенного отклика при навигации
    {
        let mut cache = PARSED_FILE_CACHE.lock();
        cache.insert(
            file_path,
            ParsedFileCacheEntry {
                file_mtime,
                orig_mtime,
                result: parsed_result.clone(),
            },
        );
    }

    Ok(parsed_result)
}

#[tauri::command]
async fn save_file_translations(
    file_path: String,
    changes: Vec<translation::TranslationEdit>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        translation::save(Path::new(&file_path), &changes)?;
        PARSED_FILE_CACHE.lock().remove(&file_path);
        Ok(())
    })
    .await
    .map_err(|e| format!("Ошибка задачи сохранения перевода: {}", e))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTargetResult {
    pub file_name: String,
    pub short_name: String,
    pub file_path: String,
    pub original_path: String,
    pub line_number: usize,
    pub scene_name: String,
}

#[tauri::command]
async fn find_scene_target(
    game_dir: String,
    target_name: String,
) -> Result<Option<SceneTargetResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        if !base_path.exists() {
            return Err(format!("Путь к игре не существует: {}", game_dir));
        }

        let tl_dir = tl_lang_dir(&base_path);
        let cache_dir = get_project_scripts_cache_dir(&base_path);

        let mut script_files = Vec::new();
        collect_rpy_files(&base_path, &mut script_files);
        if cache_dir.exists() {
            collect_rpy_files(&cache_dir, &mut script_files);
        }

        // 1. Сначала ищем в оригинальных скриптах игры (быстрее всего)
        let orig_scripts: Vec<_> = script_files
            .iter()
            .filter(|p| {
                let s = p.to_string_lossy();
                !s.contains(r"\tl\") && !s.contains("/tl/")
            })
            .cloned()
            .collect();

        let pattern_label_colon = format!("label {}:", target_name);
        let pattern_label_space = format!("label {} ", target_name);
        let pattern_screen_colon = format!("screen {}:", target_name);
        let pattern_screen_paren = format!("screen {}(", target_name);

        let found = orig_scripts.into_par_iter().find_map_any(|orig_path| {
            if let Ok(content) = std::fs::read_to_string(&orig_path) {
                for (idx, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(&pattern_label_colon)
                        || trimmed.starts_with(&pattern_label_space)
                        || trimmed.starts_with(&pattern_screen_colon)
                        || trimmed.starts_with(&pattern_screen_paren)
                    {
                        let rel_path = if orig_path.starts_with(&cache_dir) {
                            orig_path.strip_prefix(&cache_dir).unwrap_or(&orig_path)
                        } else {
                            orig_path.strip_prefix(&base_path).unwrap_or(&orig_path)
                        };
                        let tl_path = tl_dir.join(rel_path);
                        let resolved_tl_path = if tl_path.exists() {
                            tl_path.to_string_lossy().to_string()
                        } else {
                            orig_path.to_string_lossy().to_string()
                        };

                        let file_name = rel_path.to_string_lossy().replace('\\', "/");
                        let short_name = orig_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        return Some(SceneTargetResult {
                            file_name,
                            short_name,
                            file_path: resolved_tl_path,
                            original_path: orig_path.to_string_lossy().to_string(),
                            line_number: idx + 1,
                            scene_name: target_name.clone(),
                        });
                    }
                }
            }
            None
        });

        if found.is_some() {
            return Ok(found);
        }

        // 2. Если не нашли в оригинале, ищем в tl/russian (по блокам translate russian <target>:)
        let tl_scripts: Vec<_> = script_files
            .into_iter()
            .filter(|p| {
                let s = p.to_string_lossy();
                s.contains(r"\tl\") || s.contains("/tl/")
            })
            .collect();

        let tl_prefix_hash = format!("translate {} {}_", TL_LANG, target_name);
        let tl_prefix_colon = format!("translate {} {}:", TL_LANG, target_name);

        let tl_found = tl_scripts.into_par_iter().find_map_any(|tl_path| {
            if let Ok(content) = std::fs::read_to_string(&tl_path) {
                for (idx, line) in content.lines().enumerate() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(&tl_prefix_hash) || trimmed.starts_with(&tl_prefix_colon)
                    {
                        let rel_path = tl_path.strip_prefix(&tl_dir).unwrap_or(&tl_path);
                        let file_name = rel_path.to_string_lossy().replace('\\', "/");
                        let short_name = tl_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let orig_path = base_path.join(rel_path);

                        return Some(SceneTargetResult {
                            file_name,
                            short_name,
                            file_path: tl_path.to_string_lossy().to_string(),
                            original_path: orig_path.to_string_lossy().to_string(),
                            line_number: idx + 1,
                            scene_name: target_name.clone(),
                        });
                    }
                }
            }
            None
        });

        Ok(tl_found)
    })
    .await
    .map_err(|e| format!("Ошибка задачи поиска сцены: {}", e))?
}

#[derive(Debug, Clone, Serialize)]
struct ModExportResult {
    message: String,
    output_path: String,
}

/// Собирает архив мода: сохранённые правки, файлы `tl/<язык>` и включение языка.
fn build_mod_archive(
    base_path: &Path,
    out_path: &Path,
    format: rpyc_writer::ScriptFormat,
) -> Result<String, String> {
    let tl_dir = tl_lang_dir(base_path);
    let packed = rpa::RpaArchive::packed_translation_files(base_path)?;
    let export = translation::prepare_mod_export(base_path, &tl_dir, !packed.is_empty())?;
    if !tl_dir.exists() && packed.is_empty() && export.files.is_empty() {
        return Err(if export.skipped > 0 {
            format!(
                "Мод не собран: не удалось перенести сохранённые правки (правок: {})",
                export.skipped
            )
        } else {
            "Нечего экспортировать: нет ни папки tl/russian, ни сохранённых правок".to_string()
        });
    }

    let files = translation::mod_archive_files(&tl_dir, packed, export.files, format)?;
    rpa::RpaArchive::create_archive(out_path, &files)?;

    let mut message = format!(
        "Мод собран: {} файлов. Положите {} в папку game игры",
        files.len(),
        out_path.file_name().unwrap_or_default().to_string_lossy()
    );
    if export.skipped > 0 {
        message.push_str(&format!(
            ". Не перенесено правок: {} (их строки изменились или конфликтуют с tl/russian)",
            export.skipped
        ));
    }
    Ok(message)
}

#[tauri::command]
async fn export_mod(
    game_dir: String,
    output_path: Option<String>,
) -> Result<ModExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        let out_path = match &output_path {
            Some(path) => PathBuf::from(path),
            None => get_project_cache_dir(&base_path)
                .join("exports")
                .join(rpa::MOD_ARCHIVE_NAME),
        };
        ensure_output_outside_game(&out_path, std::slice::from_ref(&base_path))?;

        let format = rpyc_writer::ScriptFormat::detect(&base_path)?;
        let message = build_mod_archive(&base_path, &out_path, format)?;
        Ok(ModExportResult {
            message,
            output_path: out_path.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("Ошибка экспорта мода: {}", e))?
}

#[derive(Debug, Clone, Serialize)]
struct UnpackResult {
    count: usize,
    target_dir: String,
}

#[tauri::command]
async fn unpack_rpa(
    rpa_path: String,
    target_dir: String,
    game_dir: Option<String>,
) -> Result<UnpackResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let src = Path::new(&rpa_path);
        let mut protected = Vec::new();
        if let Some(game_dir) = game_dir {
            protected.push(effective_game_directory(Path::new(&game_dir)));
        }
        if let Some(game_dir) = known_game_directory(src) {
            protected.push(game_dir);
        }
        let default_target = get_project_cache_dir(
            protected
                .first()
                .map(PathBuf::as_path)
                .unwrap_or_else(|| src.parent().unwrap_or(src)),
        )
        .join("unpacked")
        .join(src.file_stem().unwrap_or_default());
        let dst = if target_dir.trim().is_empty() {
            default_target.as_path()
        } else {
            Path::new(&target_dir)
        };
        ensure_output_outside_game(dst, &protected)?;
        let count = rpa::RpaArchive::extract_all(src, dst)?;
        Ok(UnpackResult {
            count,
            target_dir: dst.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("Ошибка распаковки RPA: {}", e))?
}

#[tauri::command]
async fn clear_project_script_cache(game_dir: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        if !base_path.is_dir() {
            return Err(format!("Папка игры не найдена: {}", game_dir));
        }
        let cache_dir = get_project_scripts_cache_dir(&base_path);
        {
            let cache_lock = path_lock(&cache_dir);
            let _guard = cache_lock.lock();
            if cache_dir.exists() {
                std::fs::remove_dir_all(&cache_dir).map_err(|e| {
                    format!("Не удалось очистить кэш скриптов {:?}: {}", cache_dir, e)
                })?;
            }
        }
        {
            let mut cache = PROJECT_CACHE.lock();
            cache.cached_script_projects.remove(&cache_dir);
            cache.scripts.remove(&base_path);
            cache.speakers.remove(&base_path);
        }
        SCRIPT_STRUCTURE_CACHE
            .lock()
            .retain(|path, _| !path.starts_with(&cache_dir));
        PARSED_FILE_CACHE
            .lock()
            .retain(|path, _| !Path::new(path).starts_with(&cache_dir));
        Ok(())
    })
    .await
    .map_err(|e| format!("Ошибка очистки временного кэша: {}", e))?
}

#[tauri::command]
async fn decompile_rpyc_file(
    rpyc_path: String,
    output_rpy_path: Option<String>,
    game_dir: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&rpyc_path);
        if let Some(ref out_p) = output_rpy_path {
            let mut protected = Vec::new();
            if let Some(game_dir) = game_dir {
                protected.push(effective_game_directory(Path::new(&game_dir)));
            }
            if let Some(game_dir) = known_game_directory(path) {
                protected.push(game_dir);
            }
            ensure_output_outside_game(Path::new(out_p), &protected)?;
        }
        let stmts = rpyc::RpycParser::parse_rpyc_file(path)?;
        let rpy_text = rpyc::RpycParser::decompile_to_rpy_text(&stmts);

        if let Some(ref out_p) = output_rpy_path {
            let out_path = Path::new(out_p);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Не удалось создать папку вывода {:?}: {}", parent, e))?;
            }
            std::fs::write(out_path, &rpy_text)
                .map_err(|e| format!("Не удалось сохранить декомпилированный .rpy: {}", e))?;
        }

        Ok(rpy_text)
    })
    .await
    .map_err(|e| format!("Ошибка декомпиляции: {}", e))?
}

#[tauri::command]
async fn get_project_glossary(
    game_dir: String,
    scanned_characters: Option<Vec<(String, String)>>,
) -> Result<glossary::ProjectGlossary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        let mut gl = glossary::ProjectGlossary::load_for_project(&base_path)?;
        if let Some(ref chars) = scanned_characters {
            gl.merge_with_scanned_characters(chars);
        }
        Ok(gl)
    })
    .await
    .map_err(|e| format!("Ошибка загрузки глоссария: {}", e))?
}

#[tauri::command]
async fn save_project_glossary(
    game_dir: String,
    glossary: glossary::ProjectGlossary,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base_path = effective_game_directory(Path::new(&game_dir));
        let saved_path = glossary.save_for_project(&base_path)?;
        Ok(format!(
            "Глоссарий сохранен ({:?})",
            saved_path.file_name().unwrap_or_default()
        ))
    })
    .await
    .map_err(|e| format!("Ошибка сохранения глоссария: {}", e))?
}

fn read_text_file_safe(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Не удалось прочитать файл {:?}: {}", path, e))?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Ok(String::from_utf8_lossy(&bytes[3..]).into_owned())
    } else {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFileDetails {
    pub file_path: String,
    pub file_name: String,
    pub original_path: Option<String>,
    pub translation_path: Option<String>,
    pub content: String,
    pub original_content: Option<String>,
    pub total_lines: usize,
    pub original_total_lines: Option<usize>,
}

#[tauri::command]
async fn read_source_file(file_path: String) -> Result<SourceFileDetails, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(format!("Файл не существует: {}", file_path));
        }

        let content = read_text_file_safe(path)?;
        let total_lines = content.lines().count();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "script.rpy".to_string());

        let path_str = path.to_string_lossy();
        let is_translation = path_str.contains(r"\tl\") || path_str.contains("/tl/");

        let mut original_path = None;
        let mut original_content = None;
        let mut original_total_lines = None;
        let mut translation_path = None;

        if is_translation {
            translation_path = Some(file_path.clone());
            // Пробуем найти оригинальный файл скрипта
            let original = resolve_original_script(path, &find_game_root(path));

            if original.exists()
                && original != path
                && let Ok(orig_txt) = read_text_file_safe(&original)
            {
                original_total_lines = Some(orig_txt.lines().count());
                original_content = Some(orig_txt);
                original_path = Some(original.to_string_lossy().to_string());
            }
        } else {
            // Файл сам является оригинальным скриптом
            original_path = Some(file_path.clone());
            original_total_lines = Some(total_lines);
            let game_root = find_game_root(path);
            if let Ok(rel) = path.strip_prefix(&game_root) {
                let tl_cand = tl_lang_dir(&game_root).join(rel);
                if tl_cand.exists() {
                    translation_path = Some(tl_cand.to_string_lossy().to_string());
                }
            }
        }

        Ok(SourceFileDetails {
            file_path,
            file_name,
            original_path,
            translation_path,
            content,
            original_content,
            total_lines,
            original_total_lines,
        })
    })
    .await
    .map_err(|e| format!("Ошибка чтения файла: {}", e))?
}

#[tauri::command]
async fn open_in_system_editor(file_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(format!("Файл не существует: {}", file_path));
        }
        tauri_plugin_opener::open_path(path, None::<&str>)
            .map_err(|e| format!("Не удалось открыть файл: {}", e))
    })
    .await
    .map_err(|e| format!("Ошибка задачи открытия: {}", e))?
}

#[tauri::command]
async fn reveal_in_system_explorer(file_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&file_path);
        if !path.exists() {
            return Err(format!("Файл не существует: {}", file_path));
        }
        #[cfg(target_os = "windows")]
        {
            use std::process::Command;
            Command::new("explorer")
                .arg(format!("/select,{}", file_path))
                .spawn()
                .map_err(|e| format!("Не удалось открыть проводник: {}", e))?;
            Ok(())
        }
        #[cfg(not(target_os = "windows"))]
        {
            let folder = path.parent().unwrap_or(path);
            open::that(folder).map_err(|e| format!("Не удалось открыть папку: {}", e))
        }
    })
    .await
    .map_err(|e| format!("Ошибка задачи проводника: {}", e))?
}

#[tauri::command]
async fn check_llm_models(api_url: String) -> Result<Vec<String>, String> {
    llm::check_models(&api_url).await
}

#[tauri::command]
async fn autofill_glossary_characters(
    game_dir: String,
    api_url: String,
    model: String,
    characters: Vec<glossary::CharacterGlossaryEntry>,
    only_empty: bool,
) -> Result<llm::AutofillResult<glossary::CharacterGlossaryEntry>, String> {
    let base_path = effective_game_directory(Path::new(&game_dir));
    let aliases: Vec<String> = characters.iter().map(|c| c.alias.clone()).collect();
    let samples = tauri::async_runtime::spawn_blocking(move || {
        llm::collect_character_dialogue_samples(&base_path, &aliases)
    })
    .await
    .map_err(|e| format!("Ошибка сбора контекста персонажей: {}", e))?;

    llm::autofill_characters(&api_url, &model, characters, only_empty, samples).await
}

#[tauri::command]
async fn autofill_glossary_terms(
    api_url: String,
    model: String,
    terms: Vec<glossary::TermGlossaryEntry>,
    only_empty: bool,
) -> Result<llm::AutofillResult<glossary::TermGlossaryEntry>, String> {
    llm::autofill_terms(&api_url, &model, terms, only_empty).await
}

#[tauri::command]
async fn extract_glossary_terms_from_game(
    game_dir: String,
    api_url: String,
    model: String,
    max_terms: Option<usize>,
) -> Result<Vec<glossary::TermGlossaryEntry>, String> {
    let base_path = effective_game_directory(Path::new(&game_dir));
    llm::extract_terms_from_game_script(&api_url, &model, &base_path, max_terms.unwrap_or(20)).await
}

/// Подключает логи: в debug — консоль, в release — файл, потому что у приложения
/// нет консоли (`windows_subsystem = "windows"`). Файл перезаписывается при запуске.
fn init_logging() {
    if cfg!(debug_assertions) {
        tracing_subscriber::fmt::init();
        return;
    }
    let log_dir = app_data_dir();
    if std::fs::create_dir_all(&log_dir).is_ok()
        && let Ok(file) = std::fs::File::create(log_dir.join("translator.log"))
    {
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            resolve_game_directory,
            scan_game_project,
            parse_file_entries,
            find_scene_target,
            export_mod,
            unpack_rpa,
            clear_project_script_cache,
            decompile_rpyc_file,
            get_project_glossary,
            save_project_glossary,
            save_file_translations,
            read_source_file,
            open_in_system_editor,
            reveal_in_system_explorer,
            check_llm_models,
            autofill_glossary_characters,
            autofill_glossary_terms,
            extract_glossary_terms_from_game
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Игра с папкой `tl/russian` и несжатыми `.rpy` (папка `game`).
#[cfg(test)]
pub(crate) const TEST_TRANSLATED_GAME: &str = "TRANSLATOR_TEST_TRANSLATED_GAME";
/// Игра с `archive.rpa` и файлом `brothel_keyhole.rpyc` внутри (папка `game`).
#[cfg(test)]
pub(crate) const TEST_RPA_GAME: &str = "TRANSLATOR_TEST_RPA_GAME";
/// Игра с персонажами в `___sys/_vars/*.rpyc` внутри `archive.rpa` (папка `game`).
#[cfg(test)]
pub(crate) const TEST_CHARACTERS_GAME: &str = "TRANSLATOR_TEST_CHARACTERS_GAME";
/// Путь к файлу архива формата RPA-2.0.
#[cfg(test)]
pub(crate) const TEST_RPA2_ARCHIVE: &str = "TRANSLATOR_TEST_RPA2_ARCHIVE";

/// Путь к файлу архива с индексом в pickle протокола 4 или новее.
#[cfg(test)]
pub(crate) const TEST_FRAMED_ARCHIVE: &str = "TRANSLATOR_TEST_FRAMED_ARCHIVE";

/// Папка `tl/<язык>` реальной игры для проверки компилятора.
#[cfg(test)]
pub(crate) const TEST_TL_DIR: &str = "TRANSLATOR_TEST_TL_DIR";
/// Папка `game` игры с переводом внутри архивов RPA.
#[cfg(test)]
pub(crate) const TEST_PACKED_TL_GAME: &str = "TRANSLATOR_TEST_PACKED_TL_GAME";

/// Папка игры для тестов на реальных данных: `cargo test -- --ignored`
/// с заданной переменной окружения. Без неё такие тесты ничего не проверяют.
#[cfg(test)]
pub(crate) fn local_test_game(env_var: &str) -> Option<String> {
    std::env::var(env_var).ok().filter(|path| Path::new(path).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_root_of_translation_files() {
        let temp_dir = std::env::temp_dir()
            .join(format!("translator_game_root_{}", uuid::Uuid::new_v4()));
        for game_dir in [
            temp_dir.join("VN").join("game"),
            temp_dir.join("NoGameFolder"),
            temp_dir.join("tl").join("VN").join("game"),
        ] {
            let tl_dir = tl_lang_dir(&game_dir);
            std::fs::create_dir_all(tl_dir.join("script")).unwrap();
            // Такие файлы создаёт `renpy generate translations`
            for name in ["options.rpy", "screens.rpy", "gui.rpy", "story.rpy"] {
                std::fs::write(tl_dir.join(name), "").unwrap();
            }
            std::fs::write(tl_dir.join("script").join("arc.rpy"), "").unwrap();
            std::fs::write(game_dir.join("story.rpy"), "").unwrap();

            assert_eq!(find_game_root(&tl_dir.join("story.rpy")), game_dir);
            assert_eq!(find_game_root(&tl_dir.join("script").join("arc.rpy")), game_dir);
            assert_eq!(find_game_root(&game_dir.join("story.rpy")), game_dir);
        }
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_original_script_path_strips_language_folder() {
        assert_eq!(
            original_script_path(Path::new(r"D:\VN\game\tl\russian\script\arc.rpy")),
            Path::new(r"D:\VN\game\script\arc.rpy")
        );
        assert_eq!(
            original_script_path(Path::new("D:/VN/game/TL/Russian/story.rpy")),
            Path::new("D:/VN/game/story.rpy")
        );
        assert_eq!(
            original_script_path(Path::new(r"D:\tl\russianish\story.rpy")),
            Path::new(r"D:\tl\russianish\story.rpy")
        );
    }

    #[test]
    fn test_repeated_choice_text_belongs_to_first_declared_scene() {
        let script = (0..32)
            .map(|i| format!("label scene_{i}:\n    menu:\n        \"Yes\":\n            pass\n\n"))
            .collect::<String>();
        let structure = RenpyParser::parse_script_structure(&script);
        assert_eq!(choice_scenes(&structure).get("Yes").unwrap(), "scene_0");
    }

    /// Сохраняет перевод для реплик файла так же, как это делает интерфейс.
    fn save_edits(file: &Path, edits: &[(&str, &str)]) {
        let parsed = parse_project_file(file.to_string_lossy().to_string()).unwrap();
        let changes: Vec<_> = edits
            .iter()
            .map(|(original, text)| translation::TranslationEdit {
                key: parsed
                    .scenes
                    .iter()
                    .flat_map(|scene| &scene.entries)
                    .find(|entry| entry.original_text == *original)
                    .unwrap()
                    .translation_key
                    .clone(),
                text: text.to_string(),
            })
            .collect();
        translation::save(file, &changes).unwrap();
        PARSED_FILE_CACHE.lock().remove(file.to_string_lossy().as_ref());
    }

    fn translated_text(file: &Path, original: &str) -> Option<String> {
        PARSED_FILE_CACHE.lock().remove(file.to_string_lossy().as_ref());
        let parsed = parse_project_file(file.to_string_lossy().to_string()).unwrap();
        parsed
            .scenes
            .iter()
            .flat_map(|scene| &scene.entries)
            .find(|entry| entry.original_text == original)
            .unwrap()
            .translated_text
            .clone()
    }

    #[test]
    fn test_saved_edits_survive_line_shifts() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_stable_key_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        let tl_dir = tl_lang_dir(&game_dir);
        std::fs::create_dir_all(&tl_dir).unwrap();

        let translation = concat!(
            "translate russian start_abcd1234:\n\n",
            "    # e \"Hello\"\n",
            "    e \"\"\n\n",
            "translate russian strings:\n\n",
            "    old \"Yes\"\n",
            "    new \"\"\n",
        );
        let tl_file = tl_dir.join("story.rpy");
        std::fs::write(&tl_file, translation).unwrap();
        save_edits(&tl_file, &[("Hello", "Привет"), ("Yes", "Да")]);
        std::fs::write(&tl_file, format!("# Обновлено\n\n\n{translation}")).unwrap();
        assert_eq!(translated_text(&tl_file, "Hello").as_deref(), Some("Привет"));
        assert_eq!(translated_text(&tl_file, "Yes").as_deref(), Some("Да"));

        let script = "label start:\n    e \"Same\"\n    e \"Same\"\n";
        let original = game_dir.join("story.rpy");
        std::fs::write(&original, script).unwrap();
        let parsed = parse_project_file(original.to_string_lossy().to_string()).unwrap();
        let second = parsed.scenes[0].entries[1].translation_key.clone();
        translation::save(
            &original,
            &[translation::TranslationEdit { key: second, text: "Второй".into() }],
        )
        .unwrap();
        std::fs::write(&original, format!("define a = 1\n\n{script}")).unwrap();
        PARSED_FILE_CACHE.lock().remove(original.to_string_lossy().as_ref());
        let parsed = parse_project_file(original.to_string_lossy().to_string()).unwrap();
        assert_eq!(parsed.scenes[0].entries[0].translated_text, None);
        assert_eq!(parsed.scenes[0].entries[1].translated_text.as_deref(), Some("Второй"));

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_legacy_keys_are_migrated_on_load() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_legacy_key_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        let tl_dir = tl_lang_dir(&game_dir);
        std::fs::create_dir_all(&tl_dir).unwrap();
        let story = tl_dir.join("story.rpy");
        std::fs::write(
            &story,
            concat!(
                "# Строки сдвинулись после сохранения правок\n",
                "translate russian start_abcd1234:\n",
                "    # e \"Unique\"\n",
                "    e \"\"\n",
                "translate russian start_abcd1235:\n",
                "    # e \"Twice\"\n",
                "    e \"\"\n",
                "translate russian start_abcd1236:\n",
                "    # e \"Twice\"\n",
                "    e \"\"\n",
                "translate russian start_abcd1237:\n",
                "    # e \"Exact\"\n",
                "    e \"\"\n",
            ),
        )
        .unwrap();

        let legacy = |line: usize, scene: &str, text: &str| {
            serde_json::to_string(&(line, scene, text, 0)).unwrap()
        };
        let unmatched = legacy(2, "start", "Twice");
        let edits = [
            (legacy(1, "start", "Unique"), "Единственная"),
            (legacy(12, "другая сцена", "Exact"), "Точная"),
            (unmatched.clone(), "Неоднозначная"),
        ]
        .map(|(key, text)| translation::TranslationEdit { key, text: text.into() });
        translation::save(&story, &edits).unwrap();

        assert_eq!(translated_text(&story, "Unique").as_deref(), Some("Единственная"));
        assert_eq!(translated_text(&story, "Exact").as_deref(), Some("Точная"));
        assert_eq!(translated_text(&story, "Twice"), None);

        // Перенос записан на диск, а неоднозначная правка не потеряна
        let out_dir = temp_dir.join("unpacked");
        let message = pack_and_unpack(&game_dir, &out_dir);
        assert!(message.contains("Не перенесено правок: 1"), "{message}");
        let packed = compiled_text(&out_dir.join("tl").join(TL_LANG).join("story.rpyc"));
        assert!(packed.contains("Единственная") && packed.contains("Точная"), "{packed}");
        let stored = std::fs::read_dir(get_project_cache_dir(&game_dir).join("translations"))
            .unwrap()
            .flatten()
            .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
            .collect::<String>();
        assert!(stored.contains("Неоднозначная"));
        assert!(stored.contains("Block"));

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    const TEST_FORMAT: rpyc_writer::ScriptFormat = rpyc_writer::ScriptFormat {
        layout: rpyc_writer::AstLayout::Python2,
        script_version: 5003000,
    };

    fn pack_and_unpack(game_dir: &Path, out_dir: &Path) -> String {
        let archive = out_dir.with_extension("rpa");
        let message = build_mod_archive(game_dir, &archive, TEST_FORMAT).unwrap();
        rpa::RpaArchive::extract_all(&archive, out_dir).unwrap();
        message
    }

    /// Текстовое представление дерева скрипта из `.rpyc`.
    fn compiled_text(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap();
        let slots = rpyc::RpycParser::read_slots(&bytes).unwrap();
        let tree = rpyc::RpycParser::decompress_slot(&bytes, &slots[0]).unwrap();
        format!("{:?}", pickle::PickleDecoder::new(&tree).decode().unwrap())
    }

    #[test]
    fn test_saved_edits_of_translation_file_reach_mod() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_saved_edit_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        let tl_dir = tl_lang_dir(&game_dir);
        std::fs::create_dir_all(&tl_dir).unwrap();
        let source = concat!(
            "translate russian start_abcd1234:\r\n",
            "\r\n",
            "    # e happy \"Hello\"\r\n",
            "    e happy \"\"\r\n",
            "\r\n",
            "translate russian strings:\r\n",
            "\r\n",
            "    old \"Yes\"\r\n",
            "    new \"\"\r\n",
            "\r\n",
            "translate russian strings:\r\n",
            "\r\n",
            "    old \"Yes\"\r\n",
            "    new \"\"\r\n",
        );
        let story = tl_dir.join("story.rpy");
        std::fs::write(&story, source).unwrap();
        std::fs::write(tl_dir.join("story.rpyc"), b"stale").unwrap();
        std::fs::write(tl_dir.join("other.rpyc"), b"kept").unwrap();

        save_edits(&story, &[("Hello", "Привет \"всем\""), ("Yes", "Да")]);
        let out_dir = temp_dir.join("unpacked");
        let message = pack_and_unpack(&game_dir, &out_dir);
        assert!(!message.contains("Не перенесено"), "{message}");

        let packed = out_dir.join("tl").join(TL_LANG);
        let patched = compiled_text(&packed.join("story.rpyc"));
        assert!(patched.contains("Привет \\\"всем\\\""), "{patched}");
        assert_eq!(patched.matches("Да").count(), 1);
        assert_eq!(std::fs::read(packed.join("other.rpyc")).unwrap(), b"kept");
        assert!(!packed.join("story.rpy").exists());
        assert_eq!(std::fs::read_to_string(&story).unwrap(), source);
        assert!(compiled_text(&packed.join("translator_language.rpyc")).contains("config.language"));

        // Язык, который игра предлагает сама, выбирает игрок
        let screens = game_dir.join("screens.rpy");
        std::fs::write(
            &screens,
            "screen preferences():\n    textbutton \"RU\" action Language(\"russian\")\n",
        )
        .unwrap();
        let out_dir = temp_dir.join("selectable");
        pack_and_unpack(&game_dir, &out_dir);
        assert!(!out_dir.join("tl").join(TL_LANG).join("translator_language.rpyc").exists());

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_other_language_files_are_hidden_and_their_edits_reach_mod() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_other_lang_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        rpa::RpaArchive::create_archive(
            &game_dir.join("scripts.rpa"),
            &[
                ("story.rpy".to_string(), b"label start:\n    e \"Hello\"\n".to_vec()),
                (
                    "tl/PortuguesBR/story.rpy".to_string(),
                    b"translate PortuguesBR start_abcd1234:\n\n    # e \"Hello\"\n    e \"Ola\"\n".to_vec(),
                ),
            ],
        )
        .unwrap();

        let summary = tauri::async_runtime::block_on(scan_game_project(
            game_dir.to_string_lossy().to_string(),
        ))
        .unwrap();
        let names: Vec<_> = summary.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["story.rpy"]);
        assert_eq!(summary.total_lines, 1);

        // Правка, сохранённая до того, как такие файлы убрали из списка
        let foreign = get_project_scripts_cache_dir(&game_dir)
            .join("tl")
            .join("PortuguesBR")
            .join("story.rpy");
        save_edits(&foreign, &[("Hello", "Привет")]);

        let out_dir = temp_dir.join("unpacked");
        let message = pack_and_unpack(&game_dir, &out_dir);
        assert!(!message.contains("Не перенесено"), "{message}");
        let strings = compiled_text(
            &out_dir.join("tl").join(TL_LANG).join("translator_saved_strings.rpyc"),
        );
        assert!(strings.contains("Hello") && strings.contains("Привет"), "{strings}");

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_saved_edit_with_repeated_spaces_matches_engine_text() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_spaces_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let story = game_dir.join("story.rpy");
        std::fs::write(&story, "label start:\n    e \"One  two\"\n").unwrap();

        // Ключ хранит текст со двумя пробелами: так правки сохранялись раньше
        let key = translation::LineKey::Script {
            label: "start".to_string(),
            text: "One  two".to_string(),
            repeat: 0,
        };
        let edit = translation::TranslationEdit { key: key.encode().unwrap(), text: "Раз два".into() };
        translation::save(&story, &[edit]).unwrap();

        // Без открытия файла правка тоже доходит до мода
        let out_dir = temp_dir.join("unpacked");
        let message = pack_and_unpack(&game_dir, &out_dir);
        assert!(!message.contains("Не перенесено"), "{message}");
        let strings = compiled_text(
            &out_dir.join("tl").join(TL_LANG).join("translator_saved_strings.rpyc"),
        );
        assert!(strings.contains("One two") && !strings.contains("One  two"), "{strings}");
        assert_eq!(translated_text(&story, "One two").as_deref(), Some("Раз два"));

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_packed_translation_becomes_standalone_mod() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_packed_tl_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let story = concat!(
            "translate russian start_abcd1234:\n",
            "    e \"Старый текст\" (\"base\")\n",
            "translate russian strings:\n",
            "    old \"Start\"\n",
            "    new \"Начать\"\n",
        );
        let compile = |name: &str, source: &str| {
            rpyc_writer::compile_script(TEST_FORMAT, TL_LANG, name, source).unwrap()
        };
        let other = compile("other.rpy", "translate russian strings:\n    old \"a\"\n    new \"б\"\n");
        let prefix = format!("tl/{TL_LANG}");
        rpa::RpaArchive::create_archive(
            &game_dir.join("translation.rpa"),
            &[
                (format!("{prefix}/story.rpyc"), compile("story.rpy", story)),
                (format!("{prefix}/other.rpyc"), other.clone()),
                (format!("{prefix}/fonts/ui.ttf"), b"font".to_vec()),
            ],
        )
        .unwrap();

        let summary = tauri::async_runtime::block_on(scan_game_project(
            game_dir.to_string_lossy().into_owned(),
        ))
        .unwrap();
        let file = summary.files.iter().find(|file| file.name == "story.rpy").unwrap();
        assert!(Path::new(&file.path).starts_with(get_project_scripts_cache_dir(&game_dir)));
        let restored = std::fs::read_to_string(&file.path).unwrap();
        assert!(restored.contains("old \"Start\""), "{restored}");

        let parsed = parse_project_file(file.path.clone()).unwrap();
        let entry = parsed
            .scenes
            .iter()
            .flat_map(|scene| &scene.entries)
            .find(|entry| entry.translated_text.as_deref() == Some("Старый текст"))
            .unwrap();
        let edit = translation::TranslationEdit {
            key: entry.translation_key.clone(),
            text: "Новый текст".to_string(),
        };
        translation::save(Path::new(&file.path), &[edit]).unwrap();

        let out_dir = temp_dir.join("mod");
        pack_and_unpack(&game_dir, &out_dir);
        let packed = out_dir.join("tl").join(TL_LANG);
        let story = compiled_text(&packed.join("story.rpyc"));
        assert!(story.contains("Новый текст") && !story.contains("Старый"), "{story}");
        assert!(story.contains("ArgumentInfo"), "{story}");
        assert_eq!(std::fs::read(packed.join("other.rpyc")).unwrap(), other);
        assert_eq!(std::fs::read(packed.join("fonts").join("ui.ttf")).unwrap(), b"font");

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_mod_archive_contains_only_compiled_scripts_and_assets() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_compiled_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        let tl_dir = tl_lang_dir(&game_dir);
        std::fs::create_dir_all(tl_dir.join("fonts")).unwrap();
        let story = tl_dir.join("story.rpy");
        std::fs::write(&story, "translate russian strings:\n").unwrap();
        std::fs::write(tl_dir.join("fonts").join("ui.ttf"), b"font").unwrap();
        std::fs::write(tl_dir.join("menu.rpy"), "label custom:\n").unwrap();
        std::fs::write(tl_dir.join("menu.rpyc"), b"compiled by game").unwrap();
        let archive = temp_dir.join("mod").join(rpa::MOD_ARCHIVE_NAME);

        // Скрипт с неподдерживаемой конструкцией и устаревшей компиляцией не собрать
        let menu = std::fs::File::options()
            .write(true)
            .open(tl_dir.join("menu.rpy"))
            .unwrap();
        menu.set_modified(SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let error = build_mod_archive(&game_dir, &archive, TEST_FORMAT).unwrap_err();
        assert!(error.contains("menu.rpy, строка 1"), "{error}");
        assert!(!archive.exists());

        menu.set_modified(SystemTime::now() - std::time::Duration::from_secs(60))
            .unwrap();
        drop(menu);
        build_mod_archive(&game_dir, &archive, TEST_FORMAT).unwrap();
        let entries = rpa::RpaArchive::open(&archive).unwrap().entries;
        let mut names: Vec<_> = entries.keys().cloned().collect();
        names.sort();
        assert_eq!(
            names,
            [
                format!("tl/{TL_LANG}/fonts/ui.ttf"),
                format!("tl/{TL_LANG}/menu.rpyc"),
                format!("tl/{TL_LANG}/story.rpyc"),
                format!("tl/{TL_LANG}/translator_language.rpyc"),
            ]
        );

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_mod_archive_is_not_treated_as_game_archive() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_mod_archive_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let file = |name: &str, text: &str| (name.to_string(), text.as_bytes().to_vec());
        rpa::RpaArchive::create_archive(
            &game_dir.join("scripts.rpa"),
            &[file("story.rpy", "label start:\n    e \"Hello\"\n")],
        )
        .unwrap();
        rpa::RpaArchive::create_archive(
            &game_dir.join(rpa::MOD_ARCHIVE_NAME),
            &[file("tl/russian/mod.rpy", "label modded:\n    e \"Mod\"\n")],
        )
        .unwrap();

        let summary = tauri::async_runtime::block_on(scan_game_project(
            game_dir.to_string_lossy().to_string(),
        ))
        .unwrap();
        let names: Vec<_> = summary.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["story.rpy"]);

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_pack_explains_when_no_edit_could_be_transferred() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_no_edits_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let story = game_dir.join("story.rpy");
        std::fs::write(&story, "label start:\n    e \"Hello\"\n").unwrap();
        let edit = translation::TranslationEdit {
            key: "line-1".into(),
            text: "Привет".into(),
        };
        translation::save(&story, &[edit]).unwrap();

        let archive = temp_dir.join(rpa::MOD_ARCHIVE_NAME);
        let error = build_mod_archive(&game_dir, &archive, TEST_FORMAT).unwrap_err();
        assert!(error.contains("правок: 1"), "{error}");

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_saved_edits_of_original_script_become_string_translations() {
        let temp_dir =
            std::env::temp_dir().join(format!("translator_saved_strings_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        std::fs::create_dir_all(&game_dir).unwrap();
        let story = game_dir.join("story.rpy");
        std::fs::write(&story, "label start:\n    e \"Say \\\"hi\\\"\"\n    e \"Taken\"\n").unwrap();

        // Без папки tl/russian мод собирается только из правок
        save_edits(&story, &[("Say \"hi\"", "Скажи \"привет\""), ("Taken", "Занято")]);
        let out_dir = temp_dir.join("first");
        pack_and_unpack(&game_dir, &out_dir);
        let strings = compiled_text(
            &out_dir.join("tl").join(TL_LANG).join("translator_saved_strings.rpyc"),
        );
        assert!(strings.contains(r#"Say \"hi\""#) && strings.contains(r#"Скажи \"привет\""#), "{strings}");

        // Повтор `old` в одном языке Ren'Py считает ошибкой загрузки
        let tl_dir = tl_lang_dir(&game_dir);
        std::fs::create_dir_all(&tl_dir).unwrap();
        std::fs::write(
            tl_dir.join("menu.rpy"),
            "translate russian strings:\n    old \"Taken\"\n    new \"Уже есть\"\n",
        )
        .unwrap();
        let out_dir = temp_dir.join("second");
        let message = pack_and_unpack(&game_dir, &out_dir);
        assert!(message.contains("Не перенесено правок: 1"), "{message}");
        let strings = compiled_text(
            &out_dir.join("tl").join(TL_LANG).join("translator_saved_strings.rpyc"),
        );
        assert!(!strings.contains("Taken"));

        let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_parse_file_entries_benchmark() {
        tauri::async_runtime::block_on(async {
            let Some(game_dir) = local_test_game(TEST_TRANSLATED_GAME) else {
                return;
            };
            let file_path = Path::new(&game_dir).join("celia.rpy").to_string_lossy().to_string();
            let file_path2 = Path::new(&game_dir).join("folk arc.rpy").to_string_lossy().to_string();
            if !Path::new(&file_path).exists() || !Path::new(&file_path2).exists() {
                return;
            }
            let start = std::time::Instant::now();
            let parsed1 = parse_file_entries(file_path.to_string()).await.unwrap();
            let dur1 = start.elapsed();
            println!("Первый файл (celia.rpy): {:?}", dur1);

            let start_other = std::time::Instant::now();
            let _parsed_other = parse_file_entries(file_path2.to_string()).await.unwrap();
            let dur_other = start_other.elapsed();
            println!(
                "Второй файл (folk arc.rpy, скрипты проекта уже в кэше): {:?}",
                dur_other
            );

            let start2 = std::time::Instant::now();
            let parsed2 = parse_file_entries(file_path.to_string()).await.unwrap();
            let dur2 = start2.elapsed();
            println!("Повторный клик на первый файл (из кэша файла): {:?}", dur2);

            assert_eq!(parsed1.total_lines, parsed2.total_lines);
            assert!(
                dur2.as_millis() <= 5,
                "Повторный клик должен быть мгновенным из кэша!"
            );
        });
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_scan_and_parse_story_rpy() {
        tauri::async_runtime::block_on(async {
            if let Some(game_dir) = local_test_game(TEST_TRANSLATED_GAME) {
                let game_dir = game_dir.as_str();
                let summary = scan_game_project(game_dir.to_string()).await.unwrap();
                println!("Найдены файлы ({} шт):", summary.files.len());
                // Проверяем, что в списке нет бэкапов!
                for f in &summary.files {
                    assert!(
                        !f.name.contains(".bak"),
                        "Файл бэкапа не должен попадать в проект: {}",
                        f.name
                    );
                    assert!(
                        !f.path.contains(".codex-backups"),
                        "Папка .codex-backups не должна сканироваться: {}",
                        f.path
                    );
                }

                let story_path = format!(r"{}\tl\russian\story.rpy", game_dir);
                if Path::new(&story_path).exists() {
                    let parsed = parse_file_entries(story_path).await.unwrap();
                    println!(
                        "story.rpy распарсен: {} сцен, всего строк: {}, переведено: {}",
                        parsed.scenes.len(),
                        parsed.total_lines,
                        parsed.translated_lines
                    );

                    for (i, s) in parsed.scenes.iter().enumerate().take(10) {
                        println!(
                            "Сцена #{}: {} ({} строк, переведено: {})",
                            i, s.name, s.total_lines, s.translated_lines
                        );
                    }

                    // Проверяем, что intro и vera_intro не фрагментированы на одиночные строки
                    let intro_scene = parsed.scenes.iter().find(|s| s.name == "intro");
                    assert!(intro_scene.is_some(), "Сцена intro должна существовать");
                    let intro_lines = intro_scene.unwrap().total_lines;
                    assert!(
                        intro_lines >= 50,
                        "Сцена intro должна содержать не менее 50 реплик, а не по 1 строке! Найдено: {}",
                        intro_lines
                    );

                    // Проверяем, что "Интерфейс и меню (strings)" содержит дедуплицированные строки
                    let ui_scene = parsed
                        .scenes
                        .iter()
                        .find(|s| s.name == "Интерфейс и меню (strings)");
                    assert!(ui_scene.is_some(), "Сцена UI строк должна существовать");
                    let ui_lines = ui_scene.unwrap().total_lines;
                    println!("Уникальных строк UI в story.rpy: {}", ui_lines);
                    assert!(
                        ui_lines < 66,
                        "Строки UI должны быть дедуплицированы, а не 66 дубликатов! Найдено: {}",
                        ui_lines
                    );
                    // Проверяем поиск целевой сцены "house" (должна найтись в script.rpy)
                    let found_house = find_scene_target(game_dir.to_string(), "house".to_string())
                        .await
                        .unwrap();
                    assert!(
                        found_house.is_some(),
                        "Целевая сцена 'house' должна быть найдена в проекте"
                    );
                    let house_loc = found_house.unwrap();
                    println!(
                        "Найдена сцена 'house': файл {}, строка {}",
                        house_loc.file_name, house_loc.line_number
                    );
                    assert_eq!(
                        house_loc.file_name, "script.rpy",
                        "Сцена 'house' должна находиться в script.rpy"
                    );

                    // Проверяем поиск сцены "PPF" в подпапке "script/locations/PPF HQ.rpy"
                    let found_ppf = find_scene_target(game_dir.to_string(), "PPF".to_string())
                        .await
                        .unwrap();
                    assert!(found_ppf.is_some(), "Сцена 'PPF' должна быть найдена");
                    let ppf_loc = found_ppf.unwrap();
                    println!(
                        "Найдена сцена 'PPF': file_name={}, short_name={}, line={}",
                        ppf_loc.file_name, ppf_loc.short_name, ppf_loc.line_number
                    );
                    assert_eq!(ppf_loc.file_name, "script/locations/PPF HQ.rpy");
                    assert_eq!(ppf_loc.short_name, "PPF HQ.rpy");
                    assert!(
                        ppf_loc
                            .file_path
                            .contains(r"tl\russian\script\locations\PPF HQ.rpy")
                    );
                }
            }
        });
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_rpa_and_rpyc_tauri_commands() {
        tauri::async_runtime::block_on(async {
            let Some(game_dir) = local_test_game(TEST_TRANSLATED_GAME) else {
                return;
            };
            let game_dir = game_dir.as_str();

            // 1. Тест декомпиляции celia.rpyc через команду decompile_rpyc_file
            let celia_rpyc = format!(r"{}\celia.rpyc", game_dir);
            let decompiled = decompile_rpyc_file(celia_rpyc, None, None).await.unwrap();
            assert!(decompiled.contains("label end_scene_nei:"));
            println!("Команда decompile_rpyc_file успешно декомпилировала celia.rpyc!");

            let tl_russian = tl_lang_dir(Path::new(game_dir));
            if !tl_russian.exists() {
                println!(
                    "tl/russian не существует в {:?}, пропускаем часть упаковки",
                    game_dir
                );
                return;
            }

            // 2. Тест сборки мода во временный архив
            let temp_mod = std::env::temp_dir().join("test_command_mod.rpa");
            let result = export_mod(
                game_dir.to_string(),
                Some(temp_mod.to_string_lossy().to_string()),
            )
            .await
            .unwrap();
            println!("Результат export_mod: {}", result.message);
            assert!(temp_mod.exists(), "Архив мода должен быть создан");

            let _ = std::fs::remove_file(&temp_mod);
        });
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_scan_packed_game_of_whores() {
        tauri::async_runtime::block_on(async {
            let Some(game_dir) = local_test_game(TEST_RPA_GAME) else {
                return;
            };
            let game_dir = game_dir.as_str();

            let start = std::time::Instant::now();
            let summary = scan_game_project(game_dir.to_string()).await.unwrap();
            println!(
                "Game of Whores отсканирована за {:?}! Найдено файлов: {}, всего строк: {}, персонажей: {}",
                start.elapsed(),
                summary.files.len(),
                summary.total_lines,
                summary.characters.len()
            );

            assert!(
                !summary.files.is_empty(),
                "Должны быть обнаружены файлы из archive.rpa"
            );
            assert!(
                summary.total_lines > 0,
                "Должны быть подсчитаны строки диалогов"
            );

            for f in summary.files.iter().take(10) {
                println!(
                    "  [Файл] {} ({:.1} КБ, строк: {})",
                    f.name, f.size_kb, f.total_lines
                );
            }

            // Проверяем парсинг одного из файлов
            let test_file = &summary.files[0];
            let parsed = parse_file_entries(test_file.path.clone()).await.unwrap();
            println!(
                "Скрипт {} распарсен: {} сцен, {} реплик",
                parsed.file_name,
                parsed.scenes.len(),
                parsed.total_lines
            );
            assert!(!parsed.scenes.is_empty(), "Сцены должны быть сформированы");

            // Проверяем, что в самой папке игры НЕ появилось распакованных .rpy файлов!
            assert!(
                !Path::new(game_dir).join("bquest1.rpy").exists(),
                "Архив не должен распаковываться в папку игры"
            );
            assert!(
                !Path::new(game_dir).join("mgquest1.rpy").exists(),
                "Архив не должен распаковываться в папку игры"
            );
            println!("Папка игры чиста: 0 файлов распаковано в корень игры!");
        });
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_scan_lust_and_power() {
        tauri::async_runtime::block_on(async {
            let Some(game_dir) = local_test_game(TEST_CHARACTERS_GAME) else {
                return;
            };
            let game_dir = game_dir.as_str();

            let start = std::time::Instant::now();
            let summary = scan_game_project(game_dir.to_string()).await.unwrap();
            println!(
                "Lust and Power отсканирована за {:?}! Файлов: {}, всего строк: {}, персонажей: {}",
                start.elapsed(),
                summary.files.len(),
                summary.total_lines,
                summary.characters.len()
            );

            let empty_count = summary.files.iter().filter(|f| f.total_lines == 0).count();
            println!("Файлов с 0 строк диалогов: {}", empty_count);
            assert_eq!(
                empty_count, 0,
                "Все пустые файлы с 0 строк диалогов должны быть отфильтрованы!"
            );

            println!(
                "Извлеченные персонажи (всего {}):",
                summary.characters.len()
            );
            for (alias, name) in summary.characters.iter().take(25) {
                println!("  [{}] -> {}", alias, name);
            }
            assert!(
                !summary.characters.is_empty(),
                "Персонажи должны быть успешно извлечены!"
            );
            assert!(
                summary
                    .characters
                    .iter()
                    .any(|(a, n)| a == "lis" && n == "Lisa"),
                "Lisa должна быть найдена"
            );
            assert!(
                summary
                    .characters
                    .iter()
                    .any(|(a, n)| a == "emm" && n == "Emma"),
                "Emma должна быть найдена"
            );
        });
    }

    #[test]
    fn test_glossary_commands() {
        tauri::async_runtime::block_on(async {
            let unique_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let temp_game_dir =
                std::env::temp_dir().join(format!("test_glossary_game_{}", unique_id));
            let _ = std::fs::create_dir_all(&temp_game_dir);

            let scanned = vec![
                ("lis".to_string(), "Lisa".to_string()),
                ("dh".to_string(), "The Demon Hunter".to_string()),
            ];

            let mut gl =
                get_project_glossary(temp_game_dir.to_string_lossy().to_string(), Some(scanned))
                    .await
                    .unwrap();

            assert_eq!(gl.characters.len(), 2);
            let dh = gl.characters.iter_mut().find(|c| c.alias == "dh").unwrap();
            dh.translated_name = "Охотник на демонов".to_string();

            gl.terms.push(glossary::TermGlossaryEntry {
                id: "term_1".to_string(),
                source_term: "Demon Blade".to_string(),
                translated_term: "Клинок демона".to_string(),
                category: "item".to_string(),
                notes: "Реликвия".to_string(),
            });

            let save_msg = save_project_glossary(temp_game_dir.to_string_lossy().to_string(), gl)
                .await
                .unwrap();
            println!("{}", save_msg);
            assert!(!temp_game_dir.join("translator_glossary.json").exists());

            // Загружаем повторно и сверяем сохраненные данные
            let reloaded = get_project_glossary(temp_game_dir.to_string_lossy().to_string(), None)
                .await
                .unwrap();

            assert_eq!(reloaded.characters.len(), 2);
            assert_eq!(reloaded.terms.len(), 1);
            assert_eq!(reloaded.terms[0].translated_term, "Клинок демона");

            // Старый файл в игре не должен перекрывать данные приложения.
            std::fs::write(
                temp_game_dir.join("translator_glossary.json"),
                r#"{"characters":[],"terms":[]}"#,
            )
            .unwrap();
            let reloaded = get_project_glossary(temp_game_dir.to_string_lossy().to_string(), None)
                .await
                .unwrap();
            assert_eq!(reloaded.terms[0].translated_term, "Клинок демона");

            let _ = std::fs::remove_dir_all(&temp_game_dir);
            let _ = std::fs::remove_dir_all(get_project_cache_dir(&temp_game_dir));
        });
    }

    #[test]
    fn test_game_files_remain_untouched_by_cache_and_export_commands() {
        tauri::async_runtime::block_on(async {
            let temp_dir = std::env::temp_dir()
                .join(format!("translator_readonly_game_{}", uuid::Uuid::new_v4()));
            let game_dir = temp_dir.join("game");
            let tl_dir = game_dir.join("tl").join("russian");
            std::fs::create_dir_all(&tl_dir).unwrap();
            std::fs::write(tl_dir.join("story.rpy"), "translate russian strings:\n").unwrap();
            let compiled = game_dir.join("story.rpyc");
            std::fs::write(&compiled, b"original bytecode").unwrap();
            let game_path = game_dir.to_string_lossy().to_string();
            let script_cache = get_project_scripts_cache_dir(&game_dir);
            std::fs::create_dir_all(&script_cache).unwrap();
            std::fs::write(script_cache.join("cached.rpy"), b"cached").unwrap();

            clear_project_script_cache(game_path.clone()).await.unwrap();
            assert!(!script_cache.exists());
            assert_eq!(std::fs::read(&compiled).unwrap(), b"original bytecode");

            let in_game_mod = game_dir.join(rpa::MOD_ARCHIVE_NAME);
            assert!(
                export_mod(
                    game_path.clone(),
                    Some(in_game_mod.to_string_lossy().to_string()),
                )
                .await
                .is_err()
            );
            assert!(!in_game_mod.exists());
            let default_mod = get_project_cache_dir(&game_dir)
                .join("exports")
                .join(rpa::MOD_ARCHIVE_NAME);
            build_mod_archive(&game_dir, &default_mod, TEST_FORMAT).unwrap();
            assert!(default_mod.exists());

            assert!(
                unpack_rpa(
                    game_dir.join("archive.rpa").to_string_lossy().to_string(),
                    game_dir.join("extracted").to_string_lossy().to_string(),
                    Some(game_path.clone()),
                )
                .await
                .is_err()
            );
            let outside_archive = temp_dir.join("outside.rpa");
            rpa::RpaArchive::create_archive(
                &outside_archive,
                &[("text.txt".to_string(), b"test".to_vec())],
            )
            .unwrap();
            assert!(
                unpack_rpa(
                    outside_archive.to_string_lossy().to_string(),
                    game_dir.join("extracted").to_string_lossy().to_string(),
                    None,
                )
                .await
                .is_err()
            );
            assert!(!game_dir.join("extracted").exists());
            let unpacked = unpack_rpa(
                outside_archive.to_string_lossy().to_string(),
                String::new(),
                Some(game_path.clone()),
            )
            .await
            .unwrap();
            assert_eq!(unpacked.count, 1);
            assert!(unpacked.target_dir.contains("unpacked"));
            assert_eq!(
                std::fs::read(
                    get_project_cache_dir(&game_dir)
                        .join("unpacked")
                        .join("outside")
                        .join("text.txt")
                )
                .unwrap(),
                b"test"
            );
            assert!(
                decompile_rpyc_file(
                    compiled.to_string_lossy().to_string(),
                    Some(game_dir.join("story.rpy").to_string_lossy().to_string()),
                    Some(game_path),
                )
                .await
                .is_err()
            );
            assert!(!game_dir.join("story.rpy").exists());

            let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
            let _ = std::fs::remove_dir_all(&temp_dir);
        });
    }

    #[test]
    #[ignore = "Требуются локальные тестовые файлы игры"]
    fn test_real_ppf_and_hilde_cross_file_flow() {
        tauri::async_runtime::block_on(async {
            let Some(game_dir) = local_test_game(TEST_TRANSLATED_GAME) else {
                return;
            };
            let locations = tl_lang_dir(Path::new(&game_dir)).join("script").join("locations");
            let hilde_path = locations.join("arc1 hilde").join("hilde arc I.rpy");
            let ppf_path = locations.join("PPF HQ.rpy");
            if !hilde_path.exists() || !ppf_path.exists() {
                return;
            }
            let hilde_path = hilde_path.to_string_lossy();
            let ppf_path = ppf_path.to_string_lossy();

            let hilde = parse_file_entries(hilde_path.to_string()).await.unwrap();
            assert!(
                hilde
                    .edges
                    .iter()
                    .any(|e| e.from == "PPF_menu" && e.to == "hilde_ar1c_selector"),
                "Входящий переход PPF_menu -> hilde_ar1c_selector должен присутствовать в графе hilde arc I"
            );

            let ppf = parse_file_entries(ppf_path.to_string()).await.unwrap();
            assert!(
                ppf.edges
                    .iter()
                    .any(|e| e.from == "PPF_menu" && e.to == "hilde_ar1c_selector"),
                "Исходящий переход PPF_menu -> hilde_ar1c_selector должен присутствовать в графе PPF HQ"
            );
        });
    }

    #[test]
    fn test_translation_file_uses_original_from_archive() {
        tauri::async_runtime::block_on(async {
            let temp_dir = std::env::temp_dir()
                .join(format!("translator_archived_original_{}", uuid::Uuid::new_v4()));
            let game_dir = temp_dir.join("game");
            let tl_dir = tl_lang_dir(&game_dir);
            std::fs::create_dir_all(&tl_dir).unwrap();
            let script = concat!(
                "label start:\n",
                "    menu:\n",
                "        \"Go\":\n",
                "            jump next_scene\n\n",
                "label next_scene:\n",
                "    mc \"Next\"\n",
            );
            rpa::RpaArchive::create_archive(
                &game_dir.join("scripts.rpa"),
                &[("story.rpy".to_string(), script.as_bytes().to_vec())],
            )
            .unwrap();
            let translation = tl_dir.join("story.rpy");
            std::fs::write(
                &translation,
                concat!(
                    "translate russian strings:\n",
                    "    old \"Go\"\n",
                    "    new \"Идти\"\n\n",
                    "translate russian next_scene_55667788:\n",
                    "    # mc \"Next\"\n",
                    "    mc \"Дальше\"\n",
                ),
            )
            .unwrap();

            scan_game_project(game_dir.to_string_lossy().to_string()).await.unwrap();
            let file_path = translation.to_string_lossy().to_string();
            let parsed = parse_file_entries(file_path.clone()).await.unwrap();

            let start = parsed.scenes.iter().find(|s| s.name == "start").unwrap();
            assert_eq!(start.entries[0].original_text, "Go");
            assert_eq!(start.entries[0].speaker.as_deref(), Some("Выбор в меню"));
            assert!(
                parsed
                    .edges
                    .iter()
                    .any(|e| e.from == "start" && e.to == "next_scene" && e.kind == "choice")
            );

            let details = read_source_file(file_path).await.unwrap();
            assert_eq!(details.original_content.as_deref(), Some(script));

            let _ = std::fs::remove_dir_all(get_project_cache_dir(&game_dir));
            let _ = std::fs::remove_dir_all(&temp_dir);
        });
    }

    #[test]
    fn test_menu_selector_scene_and_choices() {
        tauri::async_runtime::block_on(async {
            let unique_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let temp_game_dir =
                std::env::temp_dir().join(format!("test_menu_selector_{}", unique_id));
            let tl_dir = temp_game_dir.join("tl").join("russian");
            std::fs::create_dir_all(&tl_dir).unwrap();

            let orig_script = r#"
label start:
    mc "Hello"

label my_selector:
    menu:
        "Go forward":
            jump next_scene
        "Go back":
            jump prev_scene

label next_scene:
    mc "Next"
"#;
            let orig_path = temp_game_dir.join("story.rpy");
            std::fs::write(&orig_path, orig_script).unwrap();

            let trans_script = r#"
translate russian start_11223344:
    mc "Привет"

translate russian strings:
    old "Go forward"
    new "Идти вперед"

    old "Go back"
    new "Идти назад"

    old "Start Game"
    new "Начать игру"

translate russian next_scene_55667788:
    mc "Дальше"
"#;
            let trans_path = tl_dir.join("story.rpy");
            std::fs::write(&trans_path, trans_script).unwrap();

            let parsed = parse_file_entries(trans_path.to_string_lossy().to_string())
                .await
                .unwrap();

            // Проверяем, что сцена my_selector присутствует, несмотря на отсутствие реплик персонажей
            let selector_scene = parsed.scenes.iter().find(|s| s.name == "my_selector");
            assert!(
                selector_scene.is_some(),
                "Сцена с выбором (my_selector) должна существовать в списке сцен"
            );
            let sel = selector_scene.unwrap();
            assert_eq!(
                sel.entries.len(),
                2,
                "Два пункта меню должны быть привязаны к сцене my_selector"
            );
            assert!(
                sel.entries.iter().any(|e| e.original_text == "Go forward"
                    && e.speaker.as_deref() == Some("Выбор в меню"))
            );
            assert!(
                sel.entries.iter().any(|e| e.original_text == "Go back"
                    && e.speaker.as_deref() == Some("Выбор в меню"))
            );

            // Проверяем, что общие строки UI остались в 'Интерфейс и меню (strings)'
            let ui_scene = parsed
                .scenes
                .iter()
                .find(|s| s.name == "Интерфейс и меню (strings)")
                .unwrap();
            assert_eq!(ui_scene.entries.len(), 1);
            assert_eq!(ui_scene.entries[0].original_text, "Start Game");

            // Проверяем переходы из my_selector
            assert!(
                parsed
                    .edges
                    .iter()
                    .any(|e| e.from == "my_selector" && e.to == "next_scene" && e.kind == "choice")
            );
            assert!(
                parsed
                    .edges
                    .iter()
                    .any(|e| e.from == "my_selector" && e.to == "prev_scene" && e.kind == "choice")
            );

            let _ = std::fs::remove_dir_all(&temp_game_dir);
        });
    }
}
