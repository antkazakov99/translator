//! Шрифты: шрифты игры из снимка, замены для языка перевода, проверка символов.

pub mod commands;

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::common::{AppError, Timestamp};
use crate::engines::{self, FileKind, FontSwap};
use crate::files::ContentHash;
use crate::projects::{ProjectId, ProjectInfoRepo};
use crate::scan::{ScanRepo, ScanService};
use crate::state::AppState;
use crate::storage::DbError;

/// Самый большой шрифт, который читается в память.
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

/// Символы, которые проверяются для языка оригинала и языка перевода.
const ORIGINAL_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.,!?";
const TARGET_CHARS: &str = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя«»—…";

/// Замена шрифта игры (`font_replacements`). Сам шрифт лежит в `files/` и переезжает с папкой проекта.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontReplacement {
    /// Путь шрифта в игре, который заменяется.
    pub target_path: String,
    /// Имя загруженного файла, для показа.
    pub file_name: String,
    pub hash: ContentHash,
    pub added_at: Timestamp,
}

/// Каких символов нет в шрифте.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontCoverage {
    /// Шрифт не удалось прочитать: о символах ничего не известно.
    pub unreadable: bool,
    pub missing_original: String,
    pub missing_target: String,
}

impl FontCoverage {
    /// TrueType и OpenType, из коллекции первый шрифт. Шрифты игры недоверенные: битый файл даёт `unreadable`, а не панику.
    pub fn of(data: &[u8]) -> Self {
        let Some(tables) = unicode_tables(data) else { return Self { unreadable: true, ..Self::default() } };
        let missing =
            |chars: &str| chars.chars().filter(|&ch| !tables.iter().any(|table| table_has(table, ch as u32) == Some(true))).collect();
        Self { unreadable: false, missing_original: missing(ORIGINAL_CHARS), missing_target: missing(TARGET_CHARS) }
    }
}

/// Шрифт игры из снимка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameFont {
    pub path: String,
    pub hash: ContentHash,
    pub mentions: usize,
    pub coverage: FontCoverage,
    /// Почему замена в моде не подействует.
    pub locked: Option<String>,
    pub replacement: Option<(FontReplacement, FontCoverage)>,
}

pub struct FontRepo<'c> {
    pub conn: &'c Connection,
}

impl FontRepo<'_> {
    pub fn replacements(&self) -> Result<Vec<FontReplacement>, DbError> {
        let mut statement =
            self.conn.prepare("SELECT target_path, file_name, hash, added_at FROM font_replacements ORDER BY target_path")?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(target_path, file_name, hash, added_at)| {
                Ok(FontReplacement {
                    target_path,
                    file_name,
                    hash: ContentHash::parse(&hash).map_err(|error| DbError::bad_value("font_replacements", "hash", error))?,
                    added_at: Timestamp::from_secs(added_at),
                })
            })
            .collect()
    }

    pub fn save(&self, replacement: &FontReplacement) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO font_replacements (target_path, file_name, hash, added_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (target_path) DO UPDATE SET file_name = excluded.file_name, hash = excluded.hash, added_at = excluded.added_at",
            params![replacement.target_path, replacement.file_name, replacement.hash.as_str(), replacement.added_at.secs()],
        )?;
        Ok(())
    }

    pub fn delete(&self, target_path: &str) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM font_replacements WHERE target_path = ?1", [target_path])?;
        Ok(())
    }

    /// Есть ли такой хэш среди замен: окно читает файл замены для предпросмотра.
    fn has_hash(&self, hash: &ContentHash) -> Result<bool, DbError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM font_replacements WHERE hash = ?1 LIMIT 1", [hash.as_str()], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

pub struct FontService<'a> {
    pub state: &'a AppState,
}

impl FontService<'_> {
    /// Шрифты игры с недостающими символами и заменами: сначала упомянутые в скриптах. Без папки игры.
    pub fn list(&self, project: ProjectId) -> Result<Vec<GameFont>, AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let (info, files, replacements) = open.project.read(|tx| {
            Ok((ProjectInfoRepo { conn: tx }.get()?, ScanRepo { conn: tx }.current_files()?, FontRepo { conn: tx }.replacements()?))
        })?;
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        let coverage = |hash: &ContentHash| read_limited(&open.files.path_of(hash)).map(|data| FontCoverage::of(&data)).unwrap_or_else(|_| FontCoverage { unreadable: true, ..FontCoverage::default() });
        let mut fonts: Vec<GameFont> = engine
            .fonts(&files, &open.files)?
            .into_iter()
            .filter_map(|font| {
                let file = files.iter().find(|file| file.path == font.path)?;
                let replacement = replacements
                    .iter()
                    .find(|replacement| replacement.target_path == font.path)
                    .map(|replacement| (replacement.clone(), coverage(&replacement.hash)));
                Some(GameFont {
                    path: font.path,
                    hash: file.hash.clone(),
                    mentions: font.mentions,
                    coverage: coverage(&file.hash),
                    locked: font.locked,
                    replacement,
                })
            })
            .collect();
        fonts.sort_by_key(|font| (font.mentions == 0, font.path.to_lowercase()));
        Ok(fonts)
    }

    /// Под `snapshot_lock`: файл в `files/`, потом строка в базе. Коллекции TTC и не шрифты не принимаются.
    pub fn replace(&self, project: ProjectId, target_path: &str, font: &Path) -> Result<FontReplacement, AppError> {
        let target = self.list(project)?.into_iter().find(|game| game.path == target_path).ok_or(FontError::UnknownFont)?;
        if let Some(reason) = target.locked {
            return Err(FontError::Locked(reason).into());
        }
        let data = read_limited(font)?;
        if data.starts_with(b"ttcf") {
            return Err(FontError::Collection.into());
        }
        if FontCoverage::of(&data).unreadable {
            return Err(FontError::NotFont.into());
        }
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let blob = open.files.put(data.as_slice())?;
        let replacement = FontReplacement {
            target_path: target_path.to_string(),
            file_name: font.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            hash: blob.hash,
            added_at: Timestamp::now(),
        };
        open.project.write(|tx| FontRepo { conn: tx }.save(&replacement))?;
        // Прежняя замена этого шрифта больше не нужна.
        ScanService::clean_files(&open);
        Ok(replacement)
    }

    /// Убирает замену и её файл, если он больше ничему не нужен.
    pub fn remove(&self, project: ProjectId, target_path: &str) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        open.project.write(|tx| FontRepo { conn: tx }.delete(target_path))?;
        ScanService::clean_files(&open);
        Ok(())
    }

    /// Файл шрифта игры или замены для предпросмотра. Читаются только шрифты снимка и замены.
    pub fn data(&self, project: ProjectId, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        let open = self.state.open_project(project)?;
        let known = open.project.read(|tx| {
            let font = ScanRepo { conn: tx }.current_files()?.iter().any(|file| file.kind == FileKind::Font && &file.hash == hash);
            Ok(font || FontRepo { conn: tx }.has_hash(hash)?)
        })?;
        if !known {
            return Err(FontError::UnknownFont.into());
        }
        let _lock = open.snapshot_lock.lock();
        read_limited(&open.files.path_of(hash))
    }

    /// Замены для сборки мода: только шрифтов, которые есть в текущем снимке.
    pub fn swaps(tx: &Connection) -> Result<Vec<FontSwap>, DbError> {
        let current = ScanRepo { conn: tx }.current_files()?;
        Ok(FontRepo { conn: tx }
            .replacements()?
            .into_iter()
            .filter(|replacement| current.iter().any(|file| file.kind == FileKind::Font && file.path == replacement.target_path))
            .map(|replacement| FontSwap { target_path: replacement.target_path, hash: replacement.hash })
            .collect())
    }
}

/// Ошибки замены шрифта.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("Такого шрифта нет в игре")]
    UnknownFont,
    #[error("{0}. Замена в моде не подействует")]
    Locked(String),
    #[error("Коллекции шрифтов TTC не поддерживаются, выберите файл TTF или OTF")]
    Collection,
    #[error("Файл не похож на шрифт TTF или OTF")]
    NotFont,
    #[error("Шрифт больше 64 МБ")]
    TooLarge,
}

impl From<FontError> for AppError {
    fn from(error: FontError) -> Self {
        AppError::Font(error.to_string())
    }
}

fn read_limited(path: &Path) -> Result<Vec<u8>, AppError> {
    if std::fs::metadata(path)?.len() > MAX_FONT_BYTES {
        return Err(FontError::TooLarge.into());
    }
    Ok(std::fs::read(path)?)
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

/// Подтаблицы `cmap` с кодами Unicode. `None`: файл не разобран.
fn unicode_tables(data: &[u8]) -> Option<Vec<&[u8]>> {
    if !is_font_data(data) {
        return None;
    }
    // Из коллекции движок берёт первый шрифт.
    let start = if data.starts_with(b"ttcf") { u32_at(data, 12)? as usize } else { 0 };
    let tables = u16_at(data, start.checked_add(4)?)? as usize;
    let cmap = (0..tables).find_map(|index| {
        let record = start.checked_add(12)?.checked_add(index.checked_mul(16)?)?;
        (data.get(record..record.checked_add(4)?)? == b"cmap").then(|| u32_at(data, record + 8))?
    })? as usize;
    let count = u16_at(data, cmap.checked_add(2)?)? as usize;
    let mut result = Vec::new();
    for index in 0..count {
        let record = cmap.checked_add(4)?.checked_add(index.checked_mul(8)?)?;
        let platform = u16_at(data, record)?;
        let encoding = u16_at(data, record.checked_add(2)?)?;
        if platform == 0 || (platform == 3 && matches!(encoding, 1 | 10)) {
            let offset = u32_at(data, record.checked_add(4)?)? as usize;
            if let Some(table) = cmap.checked_add(offset).and_then(|at| data.get(at..)) {
                result.push(table);
            }
        }
    }
    Some(result)
}

/// Есть ли глиф у символа. `None`: таблица обрывается.
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
            let at = offsets.checked_add(segment)?.checked_add(range_offset)?.checked_add((code - start) as usize * 2)?;
            match u16_at(table, at)? {
                0 => 0,
                glyph => (glyph + delta) & 0xFFFF,
            }
        };
        return Some(glyph != 0);
    }
    Some(false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Шрифт с одной таблицей `cmap` формата 4: символы `chars` есть, остальных нет.
    pub(crate) fn font_with(chars: &str) -> Vec<u8> {
        let mut codes: Vec<u16> = chars.chars().map(|ch| ch as u16).collect();
        codes.sort_unstable();
        codes.dedup();
        // Каждый символ отдельным сегментом, плюс завершающий 0xFFFF.
        let segments: Vec<(u16, u16)> = codes.iter().map(|&code| (code, code)).chain([(0xFFFF, 0xFFFF)]).collect();
        let width = segments.len() * 2;
        let mut table = Vec::new();
        let push16 = |out: &mut Vec<u8>, value: u16| out.extend_from_slice(&value.to_be_bytes());
        push16(&mut table, 4);
        push16(&mut table, 0);
        push16(&mut table, 0);
        push16(&mut table, width as u16);
        for _ in 0..3 {
            push16(&mut table, 0);
        }
        for &(_, end) in &segments {
            push16(&mut table, end);
        }
        push16(&mut table, 0);
        for &(start, _) in &segments {
            push16(&mut table, start);
        }
        for (index, &(start, _)) in segments.iter().enumerate() {
            // Глиф index + 1: delta = glyph - code.
            let delta = if start == 0xFFFF { 1 } else { (index as u16 + 1).wrapping_sub(start) };
            push16(&mut table, delta);
        }
        for _ in &segments {
            push16(&mut table, 0);
        }
        let length = table.len() as u16;
        table[2..4].copy_from_slice(&length.to_be_bytes());

        let mut font = vec![0, 1, 0, 0];
        push16(&mut font, 1);
        font.extend_from_slice(&[0; 6]);
        font.extend_from_slice(b"cmap");
        font.extend_from_slice(&[0; 4]);
        let cmap_at = (font.len() + 8) as u32;
        font.extend_from_slice(&cmap_at.to_be_bytes());
        font.extend_from_slice(&(12 + table.len() as u32).to_be_bytes());
        push16(&mut font, 0);
        push16(&mut font, 1);
        push16(&mut font, 3);
        push16(&mut font, 1);
        font.extend_from_slice(&12u32.to_be_bytes());
        font.extend_from_slice(&table);
        font
    }

    #[test]
    fn coverage_lists_missing_characters() {
        let latin = font_with(ORIGINAL_CHARS);
        let coverage = FontCoverage::of(&latin);
        assert!(!coverage.unreadable);
        assert_eq!(coverage.missing_original, "");
        assert_eq!(coverage.missing_target, TARGET_CHARS);

        let both = font_with(&format!("{ORIGINAL_CHARS}{TARGET_CHARS}"));
        assert_eq!(FontCoverage::of(&both), FontCoverage::default());

        let partial = font_with(&format!("{ORIGINAL_CHARS}АБВ"));
        assert!(FontCoverage::of(&partial).missing_target.starts_with("ГДЕ"));
    }

    #[test]
    fn replacement_goes_into_the_mod_under_the_translation_folder() {
        use crate::export::ExportService;
        use crate::projects::ProjectService;
        use crate::secrets::SecretStore;
        use crate::translation::TranslationService;

        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().join("data"), SecretStore::Memory(parking_lot::Mutex::new(Default::default()))).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.path().join("Game"));
        let projects = ProjectService { state: &state };
        let id = projects.create(root).unwrap().id();
        projects.open(id).unwrap();
        let open = state.open_project(id).unwrap();
        let line: i64 = open.project.read(|tx| Ok(tx.query_row("SELECT id FROM segments LIMIT 1", [], |row| row.get(0))?)).unwrap();
        TranslationService { state: &state }.save_text(id, crate::scan::SegmentId::from_row(line), "Привет.").unwrap();

        let service = FontService { state: &state };
        let fonts = service.list(id).unwrap();
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].path, "game/fonts/Main.ttf");
        assert!(fonts[0].coverage.unreadable, "в образце не шрифт, а четыре байта");
        assert!(service.data(id, &fonts[0].hash).is_ok());

        let picked = dir.path().join("Cyrillic.ttf");
        std::fs::write(&picked, font_with(&format!("{ORIGINAL_CHARS}{TARGET_CHARS}"))).unwrap();
        let not_font = dir.path().join("notes.ttf");
        std::fs::write(&not_font, b"text").unwrap();
        assert!(matches!(service.replace(id, "game/fonts/Main.ttf", &not_font), Err(AppError::Font(_))));
        assert!(matches!(service.replace(id, "game/fonts/Other.ttf", &picked), Err(AppError::Font(_))));
        let replacement = service.replace(id, "game/fonts/Main.ttf", &picked).unwrap();
        assert_eq!(replacement.file_name, "Cyrillic.ttf");
        let (saved, coverage) = service.list(id).unwrap().remove(0).replacement.unwrap();
        assert_eq!((saved.hash.clone(), coverage), (replacement.hash.clone(), FontCoverage::default()));

        let archive = ExportService { state: &state }.build(id).unwrap();
        let entries = crate::engines::renpy::rpa::RpaArchive::open(&archive.path).unwrap().entries;
        assert!(entries.contains_key("tl/russian/fonts/Main.ttf"), "{:?}", entries.keys().collect::<Vec<_>>());
        assert!(!entries.contains_key("fonts/Main.ttf"));

        service.remove(id, "game/fonts/Main.ttf").unwrap();
        assert!(service.list(id).unwrap()[0].replacement.is_none());
        assert!(!open.files.contains(&replacement.hash), "файл замены убран");
    }

    /// Реальная игра из `TRANSLATOR_TEST_RESCAN_GAME` (корень игры): шрифты читаются, упоминания находятся.
    #[test]
    #[ignore]
    fn real_game_fonts_are_listed() {
        let Some(path) = std::env::var_os("TRANSLATOR_TEST_RESCAN_GAME") else { return };
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_path_buf(), crate::secrets::SecretStore::Memory(parking_lot::Mutex::new(Default::default()))).unwrap();
        let projects = crate::projects::ProjectService { state: &state };
        let root = crate::projects::ProjectService::resolve_root(Path::new(&path)).unwrap();
        let id = projects.create(root).unwrap().id();
        projects.open(id).unwrap();
        let fonts = FontService { state: &state }.list(id).unwrap();
        for font in &fonts {
            eprintln!("{} · упоминаний {} · нет русских: {:?} · {:?}", font.path, font.mentions, font.coverage.missing_target.chars().count(), font.locked);
        }
        assert!(fonts.iter().any(|font| font.mentions > 0), "игра упоминает хотя бы один свой шрифт");
        assert!(fonts.iter().any(|font| !font.coverage.unreadable));
    }

    #[test]
    fn broken_fonts_are_unreadable_not_a_panic() {
        assert!(FontCoverage::of(b"not a font").unreadable);
        let mut cut = font_with("abc");
        cut.truncate(30);
        let coverage = FontCoverage::of(&cut);
        assert!(coverage.unreadable || coverage.missing_original.len() == ORIGINAL_CHARS.len());
        for size in 0..font_with("abc").len() {
            FontCoverage::of(&font_with("abc")[..size]);
        }
    }
}
