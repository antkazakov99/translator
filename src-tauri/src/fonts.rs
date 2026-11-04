//! Шрифты игры и их подмена в моде. Движок ищет любой файл сначала в `tl/<язык>`,
//! поэтому шрифт с тем же именем в архиве мода заменяет исходный и в настройках,
//! и в тегах `{font=...}`, причём только для этого языка. Содержимое файла движок
//! определяет сам, так что формат замены может отличаться от формата исходного.
//! Сами замены лежат в данных проекта: файлы игры не меняются.

use crate::rpa::{self, RpaArchive, RpaEntry};
use crate::rpyc::RpycParser;
use crate::{TL_LANG, get_project_cache_dir, path_lock};
use parking_lot::Mutex;
use regex::bytes::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const FONT_EXTENSIONS: [&str; 6] = ["ttf", "otf", "ttc", "TTF", "OTF", "TTC"];
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const REPLACEMENTS_DIR: &str = "fonts";
const REPLACEMENTS_FILE: &str = "replacements.json";

/// Символы, которые проверяются для языка оригинала и языка перевода.
const ORIGINAL_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.,!?";
const TARGET_CHARS: &str = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя«»—…";

/// Где движок найдёт шрифт, если выбран язык перевода.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontOrigin {
    /// Файл в папке игры.
    Game,
    /// Файл в архиве игры.
    Archive,
    /// Файл из поставки движка, `renpy/common`.
    Engine,
    /// Шрифт уже лежит в переводе `tl/<язык>`.
    Translation,
}

#[derive(Debug, Clone)]
enum FontSource {
    Disk(PathBuf),
    Packed { archive: PathBuf, entry: RpaEntry },
}

impl FontSource {
    fn read(&self) -> Result<Vec<u8>, String> {
        match self {
            FontSource::Disk(path) => read_font_file(path),
            FontSource::Packed { archive, entry } => {
                let mut file = File::open(archive)
                    .map_err(|e| format!("Не удалось открыть архив {:?}: {}", archive, e))?;
                let size = file
                    .metadata()
                    .map_err(|e| format!("Не удалось определить размер архива: {}", e))?
                    .len();
                entry.read_up_to(&mut file, size, MAX_FONT_BYTES)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct LocatedFont {
    name: String,
    origin: FontOrigin,
    source: FontSource,
    /// Почему файл из архива мода не подействует.
    locked: Option<String>,
}

/// Шрифты последнего списка: предпросмотр не открывает индексы архивов заново.
static LOCATED_FONTS: LazyLock<Mutex<HashMap<PathBuf, BTreeMap<String, LocatedFont>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FontCoverage {
    /// Шрифт не удалось прочитать: о символах ничего не известно.
    pub unreadable: bool,
    pub missing_original: String,
    pub missing_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredReplacement {
    /// Файл в папке замен проекта.
    file: String,
    /// Имя выбранного пользователем файла.
    source_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FontReplacementInfo {
    pub source_name: String,
    pub coverage: FontCoverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameFont {
    pub name: String,
    pub origin: FontOrigin,
    /// Сколько скриптов упоминают файл. Имя, собранное в коде, поиск не найдёт.
    pub references: usize,
    pub coverage: FontCoverage,
    pub locked: Option<String>,
    pub replacement: Option<FontReplacementInfo>,
}

fn replacements_dir(game_dir: &Path) -> PathBuf {
    get_project_cache_dir(game_dir).join(REPLACEMENTS_DIR)
}

fn read_font_file(path: &Path) -> Result<Vec<u8>, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("Не удалось прочитать шрифт {:?}: {}", path, e))?
        .len();
    if size > MAX_FONT_BYTES {
        return Err(format!("Шрифт {:?} слишком велик", path));
    }
    fs::read(path).map_err(|e| format!("Не удалось прочитать шрифт {:?}: {}", path, e))
}

fn load_replacements(game_dir: &Path) -> Result<BTreeMap<String, StoredReplacement>, String> {
    let file = replacements_dir(game_dir).join(REPLACEMENTS_FILE);
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(&file)
        .map_err(|e| format!("Не удалось прочитать замены шрифтов: {}", e))?;
    let stored: BTreeMap<String, StoredReplacement> = serde_json::from_str(&text)
        .map_err(|e| format!("Повреждён файл замен шрифтов {:?}: {}", file, e))?;
    // Имя станет путём в архиве мода, а файл читается из папки замен
    Ok(stored
        .into_iter()
        .filter(|(name, stored)| is_safe_name(name) && is_plain_file_name(&stored.file))
        .collect())
}

fn save_replacements(
    game_dir: &Path,
    replacements: &BTreeMap<String, StoredReplacement>,
) -> Result<(), String> {
    let text = serde_json::to_string_pretty(replacements)
        .map_err(|e| format!("Не удалось записать замены шрифтов: {}", e))?;
    let file = replacements_dir(game_dir).join(REPLACEMENTS_FILE);
    rpa::write_atomic(&file, |out| {
        out.write_all(text.as_bytes())
            .map_err(|e| format!("Не удалось записать замены шрифтов: {}", e))
    })
}

fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', ':'])
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn is_plain_file_name(name: &str) -> bool {
    is_safe_name(name) && !name.contains('/')
}

fn is_font_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| FONT_EXTENSIONS.contains(&extension))
}

/// Все шрифты, которые движок может загрузить для языка перевода. Записи идут от менее
/// важных к более важным, как при поиске в движке: `tl/<язык>` раньше остальных
/// файлов, диск раньше архивов, архив с большим именем раньше архива с меньшим.
fn locate_fonts(game_dir: &Path) -> Result<BTreeMap<String, LocatedFont>, String> {
    let mut fonts = BTreeMap::new();
    let mut add = |name: &str, origin, source, locked| {
        fonts.insert(
            name.to_lowercase(),
            LocatedFont { name: name.to_string(), origin, source, locked },
        );
    };

    if let Some(common) = game_dir.parent().map(|root| root.join("renpy").join("common"))
        && let Ok(entries) = fs::read_dir(common)
    {
        for path in entries.flatten().map(|entry| entry.path()) {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if path.is_file() && is_font_name(&name) {
                add(&name, FontOrigin::Engine, FontSource::Disk(path), None);
            }
        }
    }

    let mut packed = Vec::new();
    for archive in rpa::game_archives(game_dir)? {
        match RpaArchive::open(&archive) {
            Ok(opened) => {
                let mut names: Vec<_> =
                    opened.entries.into_iter().filter(|(name, _)| is_font_name(name)).collect();
                names.sort_by(|a, b| a.0.cmp(&b.0));
                packed.push((archive, names));
            }
            Err(error) => tracing::warn!(archive = %archive.display(), %error, "Архив не прочитан"),
        }
    }
    for (archive, names) in &packed {
        for (name, entry) in names.iter().filter(|(name, _)| !name.starts_with("tl/")) {
            let source = FontSource::Packed { archive: archive.clone(), entry: entry.clone() };
            add(name, FontOrigin::Archive, source, None);
        }
    }

    let mut paths = Vec::new();
    crate::collect_files_depth(game_dir, &FONT_EXTENSIONS, &mut paths, 0);
    let mut on_disk: Vec<_> = paths
        .into_iter()
        .filter_map(|path| {
            let name = path.strip_prefix(game_dir).ok()?.to_string_lossy().replace('\\', "/");
            Some((name, path))
        })
        .collect();
    on_disk.sort();
    for (name, path) in on_disk.iter().filter(|(name, _)| !name.starts_with("tl/")) {
        add(name, FontOrigin::Game, FontSource::Disk(path.clone()), None);
    }

    for (archive, names) in &packed {
        // Движок просматривает архивы от большего имени к меньшему
        let archive_name = archive.file_name().unwrap_or_default().to_string_lossy();
        let locked = (archive_name.to_lowercase().as_str() > rpa::MOD_ARCHIVE_NAME).then(|| {
            format!("Архив {} важнее {}: его шрифт останется", archive_name, rpa::MOD_ARCHIVE_NAME)
        });
        for (name, entry) in names {
            if let Some(relative) = rpa::translation_file_name(name) {
                let source = FontSource::Packed { archive: archive.clone(), entry: entry.clone() };
                add(relative, FontOrigin::Translation, source, locked.clone());
            }
        }
    }
    for (name, path) in &on_disk {
        if let Some(relative) = rpa::translation_file_name(name) {
            let reason = format!("Файл tl/{}/{} на диске важнее архива мода", TL_LANG, relative);
            add(relative, FontOrigin::Translation, FontSource::Disk(path.clone()), Some(reason));
        }
    }
    Ok(fonts)
}

fn base_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Сколько скриптов упоминают каждый шрифт. Имена ищутся и в скомпилированных файлах:
/// декомпилятор оставляет от экранов только связи, а шрифты задаются именно там.
fn count_references(
    game_dir: &Path,
    fonts: &BTreeMap<String, LocatedFont>,
) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut bases: Vec<String> = fonts.keys().map(|key| regex::escape(base_name(key))).collect();
    bases.sort();
    bases.dedup();
    if bases.is_empty() {
        return counts;
    }
    // Длинные имена первыми: `a.ttf` не должно найтись внутри `aa.ttf`
    bases.sort_by_key(|base| std::cmp::Reverse(base.len()));
    let Ok(pattern) = RegexBuilder::new(&bases.join("|")).case_insensitive(true).build() else {
        return counts;
    };

    let mut scan = |data: &[u8]| {
        let found: HashSet<String> = pattern
            .find_iter(data)
            .map(|hit| String::from_utf8_lossy(hit.as_bytes()).to_lowercase())
            .collect();
        for base in found {
            *counts.entry(base).or_default() += 1;
        }
    };
    let mut scan_script = |name: &str, data: &[u8]| {
        if !name.ends_with(".rpyc") {
            scan(data);
            return;
        }
        let Ok(slots) = RpycParser::read_slots(data) else {
            return;
        };
        let mut text = Vec::new();
        for slot in &slots {
            if let Ok(slot_data) = RpycParser::decompress_slot(data, slot) {
                text.extend_from_slice(&slot_data);
            }
        }
        scan(&text);
    };
    let script_base = |name: &str| {
        name.strip_suffix(".rpyc").or_else(|| name.strip_suffix(".rpy")).unwrap_or(name).to_string()
    };

    let loose = rpa::loose_scripts(game_dir);
    let mut seen: HashSet<String> = HashSet::new();
    // Исходник идёт раньше скомпилированного файла: `.rpy` < `.rpyc`
    for (name, path) in &loose {
        if seen.insert(script_base(name))
            && let Some(data) = rpa::read_loose_script(name, path)
        {
            scan_script(name, &data);
        }
    }
    let Ok(archives) = rpa::game_archives(game_dir) else {
        return counts;
    };
    for path in archives.iter().rev() {
        let Ok(archive) = RpaArchive::open(path) else {
            continue;
        };
        let Ok(mut file) = File::open(path) else {
            continue;
        };
        let Ok(size) = file.metadata().map(|metadata| metadata.len()) else {
            continue;
        };
        let mut entries: Vec<_> = archive
            .entries
            .iter()
            .filter(|(name, _)| {
                (name.ends_with(".rpy") || name.ends_with(".rpyc"))
                    && (!name.starts_with("tl/") || rpa::translation_file_name(name).is_some())
            })
            .collect();
        entries.sort_by_key(|(name, _)| name.as_str());
        for (name, entry) in entries {
            if seen.insert(script_base(name))
                && let Ok(data) = entry.read_compiled(&mut file, size)
            {
                scan_script(name, &data);
            }
        }
    }
    counts
}

fn u16_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u32::from(bytes[0]) << 8 | u32::from(bytes[1]))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn is_font_data(data: &[u8]) -> bool {
    matches!(data.get(..4), Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf"))
}

/// Подтаблицы `cmap` с кодами Unicode; `None`, если файл не разобран.
fn unicode_tables(data: &[u8]) -> Option<Vec<&[u8]>> {
    if !is_font_data(data) {
        return None;
    }
    // Из коллекции движок берёт первый шрифт
    let start = if data.starts_with(b"ttcf") { u32_at(data, 12)? as usize } else { 0 };
    let tables = u16_at(data, start.checked_add(4)?)? as usize;
    let cmap = (0..tables).find_map(|index| {
        let record = start + 12 + index * 16;
        (data.get(record..record + 4)? == b"cmap").then(|| u32_at(data, record + 8))?
    })? as usize;
    let count = u16_at(data, cmap.checked_add(2)?)? as usize;
    let mut result = Vec::new();
    for index in 0..count {
        let record = cmap + 4 + index * 8;
        let platform = u16_at(data, record)?;
        let encoding = u16_at(data, record + 2)?;
        if platform == 0 || (platform == 3 && matches!(encoding, 1 | 10)) {
            let offset = u32_at(data, record + 4)? as usize;
            if let Some(table) = cmap.checked_add(offset).and_then(|at| data.get(at..)) {
                result.push(table);
            }
        }
    }
    Some(result)
}

/// Есть ли глиф у символа; `None`, если таблица обрывается.
fn table_has(table: &[u8], code: u32) -> Option<bool> {
    match u16_at(table, 0)? {
        0 => Some(code < 256 && *table.get(6 + code as usize)? != 0),
        4 => format4_has(table, code),
        6 => {
            let first = u16_at(table, 6)?;
            let count = u16_at(table, 8)?;
            if code < first || code - first >= count {
                return Some(false);
            }
            Some(u16_at(table, 10 + (code - first) as usize * 2)? != 0)
        }
        12 => {
            let groups = u32_at(table, 12)? as usize;
            let available = table.len().saturating_sub(16) / 12;
            for index in 0..groups.min(available) {
                let group = 16 + index * 12;
                if code >= u32_at(table, group)? && code <= u32_at(table, group + 4)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        _ => Some(false),
    }
}

fn format4_has(table: &[u8], code: u32) -> Option<bool> {
    if code > 0xFFFF {
        return Some(false);
    }
    let width = u16_at(table, 6)? as usize;
    let ends = 14;
    let starts = ends + width + 2;
    let deltas = starts + width;
    let offsets = deltas + width;
    for segment in (0..width).step_by(2) {
        if code > u16_at(table, ends + segment)? {
            continue;
        }
        let start = u16_at(table, starts + segment)?;
        if code < start {
            return Some(false);
        }
        let delta = u16_at(table, deltas + segment)?;
        let range_offset = u16_at(table, offsets + segment)? as usize;
        let glyph = if range_offset == 0 {
            (code + delta) & 0xFFFF
        } else {
            let at = offsets + segment + range_offset + (code - start) as usize * 2;
            match u16_at(table, at)? {
                0 => 0,
                glyph => (glyph + delta) & 0xFFFF,
            }
        };
        return Some(glyph != 0);
    }
    Some(false)
}

/// Каких символов нет в шрифте. Данные недоверенные: сбой разбора не должен закрыть приложение.
fn font_coverage(data: &[u8]) -> FontCoverage {
    let Some(tables) = unicode_tables(data) else {
        return FontCoverage { unreadable: true, ..FontCoverage::default() };
    };
    let missing = |chars: &str| -> String {
        chars
            .chars()
            .filter(|&ch| !tables.iter().any(|table| table_has(table, ch as u32) == Some(true)))
            .collect()
    };
    FontCoverage {
        unreadable: false,
        missing_original: missing(ORIGINAL_CHARS),
        missing_target: missing(TARGET_CHARS),
    }
}

fn source_coverage(source: &FontSource) -> FontCoverage {
    match source.read() {
        Ok(data) => font_coverage(&data),
        Err(error) => {
            tracing::warn!(%error, "Шрифт не прочитан");
            FontCoverage { unreadable: true, ..FontCoverage::default() }
        }
    }
}

/// Шрифты игры для раздела подмены. Шрифт движка показывается, только если его упоминает игра.
pub fn list_fonts(game_dir: &Path) -> Result<Vec<GameFont>, String> {
    let located = locate_fonts(game_dir)?;
    let references = count_references(game_dir, &located);
    let replacements = load_replacements(game_dir)?;
    let dir = replacements_dir(game_dir);
    let replaced = |name: &str| replacements.iter().find(|(key, _)| key.eq_ignore_ascii_case(name));

    let mut fonts: Vec<GameFont> = located
        .values()
        .filter_map(|font| {
            let count = references.get(base_name(&font.name.to_lowercase())).copied().unwrap_or(0);
            let replacement = replaced(&font.name).map(|(_, stored)| FontReplacementInfo {
                source_name: stored.source_name.clone(),
                coverage: source_coverage(&FontSource::Disk(dir.join(&stored.file))),
            });
            if font.origin == FontOrigin::Engine && count == 0 && replacement.is_none() {
                return None;
            }
            Some(GameFont {
                name: font.name.clone(),
                origin: font.origin,
                references: count,
                coverage: source_coverage(&font.source),
                locked: font.locked.clone(),
                replacement,
            })
        })
        .collect();
    fonts.sort_by(|a, b| {
        (a.references == 0, a.name.to_lowercase()).cmp(&(b.references == 0, b.name.to_lowercase()))
    });
    LOCATED_FONTS.lock().insert(game_dir.to_path_buf(), located);
    Ok(fonts)
}

fn located_font(game_dir: &Path, name: &str) -> Result<LocatedFont, String> {
    let key = name.to_lowercase();
    if let Some(font) = LOCATED_FONTS.lock().get(game_dir).and_then(|fonts| fonts.get(&key)) {
        return Ok(font.clone());
    }
    locate_fonts(game_dir)?
        .remove(&key)
        .ok_or_else(|| format!("Шрифт {} не найден в игре", name))
}

/// Файл шрифта игры для предпросмотра. Читаются только шрифты из списка игры.
pub fn read_game_font(game_dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    located_font(game_dir, name)?.source.read()
}

/// Файл замены для предпросмотра.
pub fn read_replacement(game_dir: &Path, name: &str) -> Result<Vec<u8>, String> {
    let replacements = load_replacements(game_dir)?;
    let (_, stored) = replacements
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("Для шрифта {} нет замены", name))?;
    read_font_file(&replacements_dir(game_dir).join(&stored.file))
}

/// Копирует выбранный файл в данные проекта, поэтому мод соберётся и без него.
pub fn set_replacement(game_dir: &Path, name: &str, source_path: &Path) -> Result<(), String> {
    let font = located_font(game_dir, name)?;
    let data = read_font_file(source_path)?;
    if data.starts_with(b"ttcf") {
        return Err("Коллекции шрифтов TTC не поддерживаются, выберите файл TTF или OTF".to_string());
    }
    if font_coverage(&data).unreadable {
        return Err("Файл не похож на шрифт TTF или OTF".to_string());
    }
    let extension = if data.starts_with(b"OTTO") { "otf" } else { "ttf" };
    let file = format!("{:x}.{}", md5::compute(&data), extension);
    let source_name = source_path.file_name().unwrap_or_default().to_string_lossy().into_owned();

    let dir = replacements_dir(game_dir);
    let lock = path_lock(&dir);
    let _guard = lock.lock();
    rpa::write_atomic(&dir.join(&file), |out| {
        out.write_all(&data).map_err(|e| format!("Не удалось сохранить шрифт: {}", e))
    })?;
    let mut replacements = load_replacements(game_dir)?;
    let previous = forget_replacement(&mut replacements, &font.name);
    replacements.insert(font.name, StoredReplacement { file, source_name });
    save_replacements(game_dir, &replacements)?;
    remove_unused_files(&dir, &replacements, previous);
    Ok(())
}

pub fn remove_replacement(game_dir: &Path, name: &str) -> Result<(), String> {
    let dir = replacements_dir(game_dir);
    let lock = path_lock(&dir);
    let _guard = lock.lock();
    let mut replacements = load_replacements(game_dir)?;
    let previous = forget_replacement(&mut replacements, name);
    save_replacements(game_dir, &replacements)?;
    remove_unused_files(&dir, &replacements, previous);
    Ok(())
}

fn forget_replacement(
    replacements: &mut BTreeMap<String, StoredReplacement>,
    name: &str,
) -> Option<StoredReplacement> {
    let key = replacements.keys().find(|key| key.eq_ignore_ascii_case(name))?.clone();
    replacements.remove(&key)
}

/// Один файл могут использовать несколько шрифтов, поэтому удаляется только ничейный.
fn remove_unused_files(
    dir: &Path,
    replacements: &BTreeMap<String, StoredReplacement>,
    previous: Option<StoredReplacement>,
) {
    if let Some(previous) = previous
        && replacements.values().all(|stored| stored.file != previous.file)
    {
        let _ = fs::remove_file(dir.join(previous.file));
    }
}

/// Итог подстановки замен в файлы мода.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AppliedReplacements {
    pub applied: usize,
    /// Шрифты, чью замену движок не увидит.
    pub ineffective: Vec<String>,
}

/// Кладёт замены в файлы архива мода под именами исходных шрифтов.
pub fn apply_replacements(
    game_dir: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<AppliedReplacements, String> {
    let replacements = load_replacements(game_dir)?;
    if replacements.is_empty() {
        return Ok(AppliedReplacements::default());
    }
    let located = locate_fonts(game_dir)?;
    let dir = replacements_dir(game_dir);
    let mut result = AppliedReplacements::default();
    for (name, stored) in &replacements {
        let target = format!("tl/{}/{}", TL_LANG, name);
        let data = read_font_file(&dir.join(&stored.file))
            .map_err(|e| format!("Мод не собран: нет замены шрифта {}. {}", name, e))?;
        files.retain(|(existing, _)| !existing.eq_ignore_ascii_case(&target));
        files.push((target, data));
        result.applied += 1;
        if located.get(&name.to_lowercase()).is_some_and(|font| font.locked.is_some()) {
            result.ineffective.push(name.clone());
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

/// Копирует замены в другой проект. Замены этого проекта остаются. Возвращает число новых.
pub fn transfer_replacements(source: &Path, target: &Path) -> Result<usize, String> {
    let incoming = load_replacements(source)?;
    if incoming.is_empty() {
        return Ok(0);
    }
    let from = replacements_dir(source);
    let to = replacements_dir(target);
    let lock = path_lock(&to);
    let _guard = lock.lock();
    let mut replacements = load_replacements(target)?;
    let mut added = 0;
    for (name, stored) in incoming {
        if replacements.keys().any(|key| key.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let data = read_font_file(&from.join(&stored.file))?;
        rpa::write_atomic(&to.join(&stored.file), |out| {
            out.write_all(&data).map_err(|e| format!("Не удалось скопировать шрифт: {}", e))
        })?;
        replacements.insert(name, stored);
        added += 1;
    }
    if added > 0 {
        save_replacements(target, &replacements)?;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Минимальный TrueType с таблицей `cmap` формата 4 на заданные диапазоны.
    fn font_with(ranges: &[(u16, u16)]) -> Vec<u8> {
        let mut segments = ranges.to_vec();
        segments.push((0xFFFF, 0xFFFF));
        let width = segments.len() as u16 * 2;
        let mut table: Vec<u16> = vec![4, 0, 0, width, 0, 0, 0];
        table.extend(segments.iter().map(|segment| segment.1));
        table.push(0);
        table.extend(segments.iter().map(|segment| segment.0));
        table.extend(segments.iter().map(|_| 1));
        table.extend(segments.iter().map(|_| 0));

        let mut data = vec![0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        data.extend_from_slice(b"cmap");
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&28u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        for value in [0u16, 1, 3, 1] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data.extend_from_slice(&12u32.to_be_bytes());
        for value in table {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data
    }

    fn latin() -> Vec<u8> {
        font_with(&[(0x21, 0x7E)])
    }

    fn cyrillic() -> Vec<u8> {
        font_with(&[
            (0x21, 0x7E),
            (0xAB, 0xAB),
            (0xBB, 0xBB),
            (0x401, 0x401),
            (0x410, 0x44F),
            (0x451, 0x451),
            (0x2014, 0x2014),
            (0x2026, 0x2026),
        ])
    }

    fn temp_game(label: &str) -> PathBuf {
        let root = std::env::temp_dir()
            .join(format!("translator_fonts_{}_{}", label, uuid::Uuid::new_v4()));
        let game = root.join("game");
        fs::create_dir_all(game.join("fonts")).unwrap();
        game
    }

    fn cleanup(game: &Path) {
        let _ = fs::remove_dir_all(get_project_cache_dir(game));
        let _ = fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn coverage_reports_missing_letters_and_marks() {
        let coverage = font_coverage(&latin());
        assert!(!coverage.unreadable);
        assert_eq!(coverage.missing_original, "");
        assert_eq!(coverage.missing_target.chars().count(), TARGET_CHARS.chars().count());

        assert_eq!(font_coverage(&cyrillic()), FontCoverage::default());

        let coverage = font_coverage(&font_with(&[(0x401, 0x401), (0x410, 0x44F), (0x451, 0x451)]));
        assert_eq!(coverage.missing_target, "«»—…");
        assert!(coverage.missing_original.contains('A'));
    }

    #[test]
    fn broken_fonts_are_reported_without_panic() {
        assert!(font_coverage(b"not a font").unreadable);
        assert!(font_coverage(&[]).unreadable);
        let font = cyrillic();
        for length in 0..font.len() {
            font_coverage(&font[..length]);
        }
        let mut wild = font.clone();
        for at in 0..wild.len() {
            let saved = wild[at];
            wild[at] = 0xFF;
            font_coverage(&wild);
            wild[at] = saved;
        }
        let mut collection = b"ttcf".to_vec();
        collection.extend_from_slice(&[0xFF; 12]);
        assert!(font_coverage(&collection).unreadable);
    }

    #[test]
    fn lists_used_fonts_first_and_finds_translated_copies() {
        let game = temp_game("list");
        fs::write(game.join("veteran typewriter.ttf"), latin()).unwrap();
        fs::write(game.join("fonts").join("unused.ttf"), cyrillic()).unwrap();
        fs::write(
            game.join("gui.rpy"),
            "define gui.interface_text_font = \"Veteran Typewriter.ttf\"\n",
        )
        .unwrap();
        let common = game.parent().unwrap().join("renpy").join("common");
        fs::create_dir_all(&common).unwrap();
        fs::write(common.join("DejaVuSans.ttf"), cyrillic()).unwrap();
        let tl = game.join("tl").join(TL_LANG).join("fonts");
        fs::create_dir_all(&tl).unwrap();
        fs::write(tl.join("ui.ttf"), cyrillic()).unwrap();
        fs::write(game.join("fonts").join("ui.ttf"), latin()).unwrap();

        let fonts = list_fonts(&game).unwrap();
        let names: Vec<_> = fonts.iter().map(|font| font.name.as_str()).collect();
        assert_eq!(names, ["veteran typewriter.ttf", "fonts/ui.ttf", "fonts/unused.ttf"]);
        assert_eq!(fonts[0].references, 1);
        assert!(fonts[0].locked.is_none());
        // Для языка перевода движок возьмёт копию из tl, а её заменить архивом нельзя
        assert_eq!(fonts[1].origin, FontOrigin::Translation);
        assert_eq!(fonts[1].coverage, FontCoverage::default());
        assert!(fonts[1].locked.is_some());

        fs::write(game.join("gui2.rpy"), "style x:\n    font \"DejaVuSans.ttf\"\n").unwrap();
        let fonts = list_fonts(&game).unwrap();
        assert!(fonts.iter().any(|font| font.origin == FontOrigin::Engine));
        cleanup(&game);
    }

    #[test]
    fn replacement_goes_into_the_mod_under_the_original_name() {
        let game = temp_game("apply");
        fs::write(game.join("veteran typewriter.ttf"), latin()).unwrap();
        fs::write(game.join("fonts").join("ui.ttf"), latin()).unwrap();
        let chosen = game.parent().unwrap().join("Новый.otf");
        fs::write(&chosen, cyrillic()).unwrap();

        set_replacement(&game, "VETERAN typewriter.ttf", &chosen).unwrap();
        set_replacement(&game, "fonts/ui.ttf", &chosen).unwrap();
        fs::remove_file(&chosen).unwrap();

        let fonts = list_fonts(&game).unwrap();
        let info = fonts[0].replacement.as_ref().unwrap();
        assert_eq!(info.source_name, "Новый.otf");
        assert_eq!(info.coverage, FontCoverage::default());
        assert_eq!(read_replacement(&game, "fonts/ui.ttf").unwrap(), cyrillic());
        assert_eq!(read_game_font(&game, "fonts/ui.ttf").unwrap(), latin());

        let old_target = format!("tl/{}/fonts/ui.ttf", TL_LANG);
        let mut files = vec![(old_target.clone(), b"old".to_vec())];
        let applied = apply_replacements(&game, &mut files).unwrap();
        assert_eq!(applied, AppliedReplacements { applied: 2, ineffective: vec![] });
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|(_, data)| *data == cyrillic()));
        assert!(files.iter().any(|(name, _)| *name == old_target));
        let renamed = format!("tl/{}/veteran typewriter.ttf", TL_LANG);
        assert!(files.iter().any(|(name, _)| *name == renamed));

        // Общий файл остаётся, пока его использует другой шрифт
        remove_replacement(&game, "fonts/ui.ttf").unwrap();
        assert!(read_replacement(&game, "veteran typewriter.ttf").is_ok());
        remove_replacement(&game, "veteran typewriter.ttf").unwrap();
        let left: Vec<_> = fs::read_dir(replacements_dir(&game)).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "остался только файл списка");
        cleanup(&game);
    }

    #[test]
    fn rejects_files_that_are_not_fonts_and_unknown_fonts() {
        let game = temp_game("reject");
        fs::write(game.join("a.ttf"), latin()).unwrap();
        let bad = game.parent().unwrap().join("bad.ttf");
        fs::write(&bad, b"not a font").unwrap();
        assert!(set_replacement(&game, "a.ttf", &bad).is_err());
        let good = game.parent().unwrap().join("good.ttf");
        fs::write(&good, cyrillic()).unwrap();
        assert!(set_replacement(&game, "missing.ttf", &good).is_err());
        assert!(read_game_font(&game, "../../good.ttf").is_err());
        cleanup(&game);
    }

    #[test]
    fn missing_replacement_stops_the_build_and_unsafe_names_are_ignored() {
        let game = temp_game("missing");
        fs::write(game.join("a.ttf"), latin()).unwrap();
        let good = game.parent().unwrap().join("good.ttf");
        fs::write(&good, cyrillic()).unwrap();
        set_replacement(&game, "a.ttf", &good).unwrap();
        let stored = load_replacements(&game).unwrap();
        fs::remove_file(replacements_dir(&game).join(&stored["a.ttf"].file)).unwrap();
        assert!(apply_replacements(&game, &mut Vec::new()).is_err());

        let list = replacements_dir(&game).join(REPLACEMENTS_FILE);
        fs::write(
            &list,
            r#"{"../evil.ttf":{"file":"x.ttf","source_name":"x"},"b.ttf":{"file":"../x.ttf","source_name":"x"}}"#,
        )
        .unwrap();
        assert!(load_replacements(&game).unwrap().is_empty());
        cleanup(&game);
    }

    #[test]
    fn transfer_copies_replacements_and_keeps_existing() {
        let source = temp_game("from");
        let target = temp_game("to");
        for game in [&source, &target] {
            fs::write(game.join("a.ttf"), latin()).unwrap();
            fs::write(game.join("b.ttf"), latin()).unwrap();
        }
        let first = source.parent().unwrap().join("first.ttf");
        fs::write(&first, cyrillic()).unwrap();
        set_replacement(&source, "a.ttf", &first).unwrap();
        set_replacement(&source, "b.ttf", &first).unwrap();
        let own = target.parent().unwrap().join("own.ttf");
        fs::write(&own, font_with(&[(0x21, 0x7E), (0x410, 0x44F)])).unwrap();
        set_replacement(&target, "a.ttf", &own).unwrap();

        assert_eq!(transfer_replacements(&source, &target).unwrap(), 1);
        assert_eq!(read_replacement(&target, "b.ttf").unwrap(), cyrillic());
        assert_eq!(load_replacements(&target).unwrap()["a.ttf"].source_name, "own.ttf");
        assert_eq!(transfer_replacements(&source, &target).unwrap(), 0);
        cleanup(&source);
        cleanup(&target);
    }

    #[test]
    fn built_mod_archive_contains_the_replacement() {
        let game = temp_game("archive");
        fs::write(game.join("veteran typewriter.ttf"), latin()).unwrap();
        let tl = game.join("tl").join(TL_LANG);
        fs::create_dir_all(&tl).unwrap();
        fs::write(tl.join("story.rpy"), "translate russian strings:
").unwrap();
        let chosen = game.parent().unwrap().join("new.otf");
        fs::write(&chosen, cyrillic()).unwrap();
        set_replacement(&game, "veteran typewriter.ttf", &chosen).unwrap();

        let out = game.parent().unwrap().join("mod").join(rpa::MOD_ARCHIVE_NAME);
        let message = crate::build_mod_archive(&game, &out, crate::tests::TEST_FORMAT).unwrap();
        assert!(message.contains("Подменено шрифтов: 1"), "{message}");
        let archive = RpaArchive::open(&out).unwrap();
        let entry = &archive.entries[&format!("tl/{}/veteran typewriter.ttf", TL_LANG)];
        let mut file = File::open(&out).unwrap();
        let size = file.metadata().unwrap().len();
        assert_eq!(entry.read_up_to(&mut file, size, MAX_FONT_BYTES).unwrap(), cyrillic());
        // Файл игры не меняется
        assert_eq!(fs::read(game.join("veteran typewriter.ttf")).unwrap(), latin());
        cleanup(&game);
    }

    #[test]
    #[ignore = "Нужна настоящая игра из TRANSLATOR_TEST_FONTS_GAME"]
    fn lists_fonts_of_a_real_game() {
        let Ok(game) = std::env::var("TRANSLATOR_TEST_FONTS_GAME") else {
            return;
        };
        for font in list_fonts(Path::new(&game)).unwrap() {
            println!(
                "{:40} {:?} скриптов {:3} нет: {}",
                font.name,
                font.origin,
                font.references,
                font.coverage.missing_target.chars().count()
            );
        }
    }
}
