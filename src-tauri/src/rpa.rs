use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::pickle::{PickleDecoder, PickleEncoder, PickleValue};

const CACHE_VERSION: &str = "v3_safe_scripts";

fn safe_output_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let relative = Path::new(&normalized);
    if normalized.is_empty() || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(format!("Недопустимый путь в RPA: {}", name));
    }

    let root_real = root.canonicalize()
        .map_err(|e| format!("Не удалось проверить папку {:?}: {}", root, e))?;
    let mut output = root.to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(Component::Normal(part)) = parts.next() {
        output.push(part);
        if output.symlink_metadata().is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(format!("Путь RPA содержит ссылку: {}", name));
        }
        if parts.peek().is_some() {
            if !output.exists() {
                fs::create_dir(&output)
                    .map_err(|e| format!("Не удалось создать папку {:?}: {}", output, e))?;
            }
            let real = output.canonicalize()
                .map_err(|e| format!("Не удалось проверить папку {:?}: {}", output, e))?;
            if !real.starts_with(&root_real) {
                return Err(format!("Путь RPA выходит за пределы целевой папки: {}", name));
            }
        }
    }
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpaEntryDto {
    pub name: String,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone)]
pub struct RpaEntry {
    pub name: String,
    pub offset: u64,
    pub length: u64,
    pub prefix: Vec<u8>,
}

pub struct RpaArchive {
    pub version: String,
    pub index_offset: u64,
    pub key: u64,
    pub entries: HashMap<String, RpaEntry>,
}

impl RpaArchive {
    /// Открывает и считывает оглавление архива RPA (поддерживает RPA-3.0 и RPA-2.0)
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path)
            .map_err(|e| format!("Не удалось открыть RPA архив {:?}: {}", path, e))?;

        // 1. Читаем заголовок (первая строка)
        let mut header_bytes = Vec::new();
        let mut byte_buf = [0u8; 1];
        while file.read_exact(&mut byte_buf).is_ok() {
            if byte_buf[0] == b'\n' {
                break;
            }
            header_bytes.push(byte_buf[0]);
            if header_bytes.len() > 1024 {
                return Err("Слишком длинный заголовок RPA архива".to_string());
            }
        }

        let header_str = String::from_utf8_lossy(&header_bytes).trim().to_string();
        let parts: Vec<&str> = header_str.split_whitespace().collect();
        if parts.is_empty() {
            return Err("Пустой заголовок RPA архива".to_string());
        }

        let version = parts[0].to_string();
        if version != "RPA-3.0" && version != "RPA-2.0" {
            return Err(format!("Неподдерживаемая версия RPA архива: {}", version));
        }

        if parts.len() < 3 {
            return Err(format!("Некорректный заголовок RPA: '{}'", header_str));
        }

        let index_offset = u64::from_str_radix(parts[1], 16)
            .map_err(|e| format!("Ошибка парсинга смещения индекса RPA: {}", e))?;
        let key = u64::from_str_radix(parts[2], 16)
            .map_err(|e| format!("Ошибка парсинга ключа RPA: {}", e))?;

        // 2. Переходим к оглавлению архива
        file.seek(SeekFrom::Start(index_offset))
            .map_err(|e| format!("Не удалось перейти к оглавлению RPA на {}: {}", index_offset, e))?;

        // 3. Читаем сжатые zlib данные индекса до конца файла
        let mut compressed_index = Vec::new();
        file.read_to_end(&mut compressed_index)
            .map_err(|e| format!("Ошибка чтения оглавления RPA: {}", e))?;

        // 4. Распаковываем zlib
        let mut decoder = ZlibDecoder::new(&compressed_index[..]);
        let mut decompressed_index = Vec::new();
        decoder
            .read_to_end(&mut decompressed_index)
            .map_err(|e| format!("Ошибка распаковки zlib оглавления RPA: {}", e))?;

        // 5. Декодируем Python Pickle
        let mut pickle_dec = PickleDecoder::new(&decompressed_index);
        let root_val = pickle_dec
            .decode()
            .map_err(|e| format!("Ошибка парсинга pickle индекса RPA: {}", e))?;

        let dict_entries = match root_val {
            PickleValue::Dict(entries) => entries,
            _ => return Err("Индекс RPA должен представлять собой словарь Python".to_string()),
        };

        let mut entries = HashMap::new();
        for (k, v) in dict_entries {
            let filename = match k.as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };

            // Значение — список сегментов (в 99.9% случаев один сегмент)
            let segments = match v.as_list() {
                Some(l) => l,
                None => continue,
            };

            if let Some(first_seg) = segments.first() {
                match first_seg {
                    PickleValue::Tuple(items) if items.len() >= 2 => {
                        let raw_offset = items[0].as_u64().unwrap_or(0);
                        let raw_len = items[1].as_u64().unwrap_or(0);
                        let offset = raw_offset ^ key;
                        let length = raw_len ^ key;

                        let prefix = if items.len() >= 3 {
                            items[2].as_bytes().unwrap_or_default().to_vec()
                        } else {
                            Vec::new()
                        };

                        entries.insert(
                            filename.clone(),
                            RpaEntry {
                                name: filename,
                                offset,
                                length,
                                prefix,
                            },
                        );
                    }
                    _ => {}
                }
            }
        }

        Ok(Self {
            version,
            index_offset,
            key,
            entries,
        })
    }

    /// Распаковывает конкретный файл из RPA архива на диск
    pub fn extract_file(
        rpa_path: &Path,
        entry: &RpaEntry,
        out_path: &Path,
    ) -> Result<(), String> {
        let mut file = File::open(rpa_path)
            .map_err(|e| format!("Не удалось открыть RPA: {}", e))?;

        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|e| format!("Ошибка перехода к смещению {}: {}", entry.offset, e))?;

        let mut payload = vec![0u8; entry.length as usize];
        file.read_exact(&mut payload)
            .map_err(|e| format!("Ошибка чтения тела файла {}: {}", entry.name, e))?;

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Ошибка создания папки {:?}: {}", parent, e))?;
        }

        let mut out = File::create(out_path)
            .map_err(|e| format!("Ошибка создания файла {:?}: {}", out_path, e))?;

        if !entry.prefix.is_empty() {
            out.write_all(&entry.prefix)
                .map_err(|e| format!("Ошибка записи префикса: {}", e))?;
        }

        out.write_all(&payload)
            .map_err(|e| format!("Ошибка записи файла: {}", e))?;

        Ok(())
    }

    /// Распаковывает все файлы из архива в указанную директорию
    pub fn extract_all(rpa_path: &Path, target_dir: &Path) -> Result<usize, String> {
        let archive = Self::open(rpa_path)?;
        fs::create_dir_all(target_dir)
            .map_err(|e| format!("Не удалось создать целевую папку {:?}: {}", target_dir, e))?;

        let mut extracted_count = 0;
        for entry in archive.entries.values() {
            let out_file_path = safe_output_path(target_dir, &entry.name)?;
            Self::extract_file(rpa_path, entry, &out_file_path)?;
            extracted_count += 1;
        }

        Ok(extracted_count)
    }

    /// Создает 100% валидный RPA-3.0 архив из набора файлов в памяти
    pub fn create_archive(
        out_path: &Path,
        files: &[(String, Vec<u8>)],
    ) -> Result<(), String> {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Не удалось создать директорию {:?}: {}", parent, e))?;
        }

        let mut out = File::create(out_path)
            .map_err(|e| format!("Не удалось создать файл RPA {:?}: {}", out_path, e))?;

        // Зарезервированный заголовок 34 байта: "RPA-3.0 0000000000000000 00000000\n"
        let placeholder_header = "RPA-3.0 0000000000000000 00000000\n";
        out.write_all(placeholder_header.as_bytes())
            .map_err(|e| format!("Ошибка записи заголовка RPA: {}", e))?;

        let mut index_entries: Vec<(&str, u64, u64, &[u8])> = Vec::new();

        // Записываем каждый файл
        for (name, data) in files {
            let offset = out
                .stream_position()
                .map_err(|e| format!("Ошибка определения позиции: {}", e))?;
            let len = data.len() as u64;

            out.write_all(data)
                .map_err(|e| format!("Ошибка записи файла '{}': {}", name, e))?;

            index_entries.push((name.as_str(), offset, len, &[]));
        }

        // Фиксируем смещение оглавления
        let index_offset = out
            .stream_position()
            .map_err(|e| format!("Ошибка позиции оглавления: {}", e))?;

        // Сериализуем оглавление в Python Pickle Protocol 2
        let mut encoder = PickleEncoder::new();
        let pickle_bytes = encoder.encode_rpa_index(&index_entries);

        // Сжимаем оглавление zlib
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&pickle_bytes)
            .map_err(|e| format!("Ошибка zlib сжатия оглавления: {}", e))?;
        let compressed_index = zlib
            .finish()
            .map_err(|e| format!("Ошибка завершения сжатия zlib: {}", e))?;

        // Записываем оглавление
        out.write_all(&compressed_index)
            .map_err(|e| format!("Ошибка записи оглавления в RPA: {}", e))?;

        // Возвращаемся в начало и перезаписываем заголовок с реальным смещением
        out.seek(SeekFrom::Start(0))
            .map_err(|e| format!("Ошибка перемотки в начало RPA: {}", e))?;

        let final_header = format!("RPA-3.0 {:016x} 00000000\n", index_offset);
        out.write_all(final_header.as_bytes())
            .map_err(|e| format!("Ошибка записи финального заголовка: {}", e))?;

        out.flush()
            .map_err(|e| format!("Ошибка сброса буфера на диск: {}", e))?;

        Ok(())
    }

    /// Собирает директорию мода/перевода (например tl/russian) в единый RPA-3.0 мод-архив
    pub fn pack_mod_from_folder(
        source_folder: &Path,
        out_rpa_path: &Path,
    ) -> Result<usize, String> {
        if !source_folder.exists() {
            return Err(format!("Папка источника не существует: {:?}", source_folder));
        }

        let mut collected_files: Vec<(String, Vec<u8>)> = Vec::new();
        collect_folder_files_for_mod(source_folder, source_folder, &mut collected_files)?;

        if collected_files.is_empty() {
            return Err("Не найдено файлов для упаковки в мод".to_string());
        }

        let count = collected_files.len();
        for (name, _) in &mut collected_files {
            *name = format!("tl/russian/{}", name);
        }
        Self::create_archive(out_rpa_path, &collected_files)?;
        Ok(count)
    }

    /// Извлекает ТОЛЬКО скрипты (.rpy и .rpyc) из архива в указанную папку кэша,
    /// и автоматически декомпилирует .rpyc в .rpy (если исходный .rpy отсутствует).
    /// Это гарантирует, что 2+ ГБ медиа-файлов НЕ распаковываются, а папка игры НЕ засоряется.
    pub fn extract_and_decompile_scripts(
        rpa_path: &Path,
        target_cache_dir: &Path,
    ) -> Result<usize, String> {
        let archive = Self::open(rpa_path)?;

        let version_marker = target_cache_dir.join(".cache_version");
        let need_refresh = !version_marker.exists()
            || std::fs::read_to_string(&version_marker).unwrap_or_default() != CACHE_VERSION;

        if need_refresh && target_cache_dir.exists() {
            fs::remove_dir_all(target_cache_dir)
                .map_err(|e| format!("Не удалось обновить кэш скриптов {:?}: {}", target_cache_dir, e))?;
        }

        fs::create_dir_all(target_cache_dir)
            .map_err(|e| format!("Ошибка создания папки кэша {:?}: {}", target_cache_dir, e))?;

        let mut script_entries: Vec<RpaEntry> = Vec::new();
        let mut existing_rpy_basenames: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (name, entry) in &archive.entries {
            if name.ends_with(".rpy") {
                let base = name.trim_end_matches(".rpy").to_string();
                existing_rpy_basenames.insert(base);
                script_entries.push(entry.clone());
            }
        }

        for (name, entry) in &archive.entries {
            if name.ends_with(".rpyc") {
                let base = name.trim_end_matches(".rpyc").to_string();
                if !existing_rpy_basenames.contains(&base) {
                    script_entries.push(entry.clone());
                }
            }
        }

        if script_entries.is_empty() {
            return Ok(0);
        }

        let mut file = File::open(rpa_path)
            .map_err(|e| format!("Не удалось открыть RPA архив {:?}: {}", rpa_path, e))?;

        let mut processed = 0;
        for entry in script_entries {
            let rel_clean = entry.name.replace('\\', "/");
            let is_rpyc = rel_clean.ends_with(".rpyc");
            let target_rel_path = if is_rpyc {
                format!("{}.rpy", rel_clean.trim_end_matches(".rpyc"))
            } else {
                rel_clean
            };

            let out_file = safe_output_path(target_cache_dir, &target_rel_path)?;
            if out_file.exists() {
                processed += 1;
                continue;
            }

            file.seek(SeekFrom::Start(entry.offset))
                .map_err(|e| format!("Ошибка перемещения в RPA: {}", e))?;
            let mut payload = vec![0u8; entry.length as usize];
            file.read_exact(&mut payload)
                .map_err(|e| format!("Ошибка чтения файла {}: {}", entry.name, e))?;

            if is_rpyc {
                let stmts = match crate::rpyc::RpycParser::parse_rpyc_bytes(&payload) {
                    Ok(stmts) => stmts,
                    Err(error) => {
                        tracing::warn!(file = %entry.name, %error, "RPYC не декомпилирован");
                        continue;
                    }
                };
                let rpy_code = crate::rpyc::RpycParser::decompile_to_rpy_text(&stmts);
                fs::write(&out_file, rpy_code)
                    .map_err(|e| format!("Не удалось записать {:?}: {}", out_file, e))?;
            } else {
                fs::write(&out_file, &payload)
                    .map_err(|e| format!("Не удалось записать {:?}: {}", out_file, e))?;
            }
            processed += 1;
        }

        fs::write(&version_marker, CACHE_VERSION)
            .map_err(|e| format!("Не удалось сохранить версию кэша {:?}: {}", version_marker, e))?;
        Ok(processed)
    }

    /// Проверяет наличие архивов .rpa в папке игры и извлекает/декомпилирует все скрипты в папку кэша
    pub fn extract_scripts_from_game_archives(
        game_dir: &Path,
        target_cache_dir: &Path,
    ) -> Result<usize, String> {
        let mut archives = Vec::new();
        for entry in fs::read_dir(game_dir)
            .map_err(|e| format!("Не удалось прочитать папку игры {:?}: {}", game_dir, e))?
        {
            let path = entry.map_err(|e| format!("Ошибка чтения папки игры: {}", e))?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpa") {
                archives.push(path);
            }
        }
        archives.sort();

        let mut fingerprint = format!("{}\n", CACHE_VERSION);
        for path in &archives {
            let metadata = fs::metadata(path)
                .map_err(|e| format!("Не удалось проверить архив {:?}: {}", path, e))?;
            fingerprint.push_str(&format!("{:?}|{}|{:?}\n", path, metadata.len(), metadata.modified().ok()));
        }
        let fingerprint_file = target_cache_dir.join(".archive_fingerprint");
        if fs::read_to_string(&fingerprint_file).ok().as_deref() != Some(&fingerprint)
            && target_cache_dir.exists()
        {
            fs::remove_dir_all(target_cache_dir)
                .map_err(|e| format!("Не удалось обновить кэш скриптов {:?}: {}", target_cache_dir, e))?;
        }

        let mut total_extracted = 0;
        for path in archives {
            total_extracted += Self::extract_and_decompile_scripts(&path, target_cache_dir)?;
        }
        if !target_cache_dir.exists() {
            fs::create_dir_all(target_cache_dir)
                .map_err(|e| format!("Не удалось создать кэш скриптов {:?}: {}", target_cache_dir, e))?;
        }
        fs::write(&fingerprint_file, fingerprint)
            .map_err(|e| format!("Не удалось сохранить состояние кэша {:?}: {}", fingerprint_file, e))?;
        Ok(total_extracted)
    }
}

/// Рекурсивно собирает файлы для создания мода
fn collect_folder_files_for_mod(
    root: &Path,
    current: &Path,
    acc: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    if let Ok(entries) = fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            // Пропускаем бэкапы и скрытые файлы
            if file_name.starts_with('.') || file_name.contains("backup") || file_name.ends_with(".bak") {
                continue;
            }

            if path.is_dir() {
                collect_folder_files_for_mod(root, &path, acc)?;
            } else if path.is_file() {
                let data = fs::read(&path)
                    .map_err(|e| format!("Не удалось прочитать файл мода {:?}: {}", path, e))?;
                let rel_path = path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .map_err(|e| format!("Не удалось определить путь файла мода {:?}: {}", path, e))?;
                acc.push((rel_path, data));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_read_rpa3() {
        let temp_dir = std::env::temp_dir().join("translator_rpa_test");
        let rpa_path = temp_dir.join("test_mod.rpa");
        let extract_dir = temp_dir.join("extracted");

        let files = vec![
            ("script.rpy".to_string(), b"label start:\n    'Hello World'".to_vec()),
            ("subfolder/dialogue.rpy".to_string(), b"label scene1:\n    'Test Branch'".to_vec()),
            ("image.png".to_string(), vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
        ];

        // 1. Создаем RPA архив
        RpaArchive::create_archive(&rpa_path, &files).expect("Ошибка создания RPA");

        // 2. Открываем и проверяем список файлов
        let archive = RpaArchive::open(&rpa_path).expect("Ошибка открытия RPA");
        assert_eq!(archive.version, "RPA-3.0");
        assert_eq!(archive.entries.len(), 3);
        assert!(archive.entries.contains_key("script.rpy"));
        assert!(archive.entries.contains_key("subfolder/dialogue.rpy"));
        assert!(archive.entries.contains_key("image.png"));

        // 3. Распаковываем все файлы
        let count = RpaArchive::extract_all(&rpa_path, &extract_dir).expect("Ошибка распаковки RPA");
        assert_eq!(count, 3);

        let s1 = fs::read_to_string(extract_dir.join("script.rpy")).unwrap();
        assert_eq!(s1, "label start:\n    'Hello World'");

        let s2 = fs::read_to_string(extract_dir.join("subfolder/dialogue.rpy")).unwrap();
        assert_eq!(s2, "label scene1:\n    'Test Branch'");

        let img = fs::read(extract_dir.join("image.png")).unwrap();
        assert_eq!(img, vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

        // Очищаем временную папку
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_reject_archive_path_traversal() {
        let temp_dir = std::env::temp_dir().join(format!("translator_rpa_path_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        let archive_path = temp_dir.join("malicious.rpa");
        RpaArchive::create_archive(&archive_path, &[("../escaped.txt".into(), b"bad".to_vec())]).unwrap();

        assert!(RpaArchive::extract_all(&archive_path, &temp_dir.join("output")).is_err());
        assert!(!temp_dir.join("escaped.txt").exists());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_mod_archive_keeps_translation_prefix() {
        let temp_dir = std::env::temp_dir().join(format!("translator_rpa_mod_{}", uuid::Uuid::new_v4()));
        let source = temp_dir.join("game").join("tl").join("russian");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("story.rpy"), "translate russian strings:\n").unwrap();
        let archive_path = temp_dir.join("mod.rpa");

        assert_eq!(RpaArchive::pack_mod_from_folder(&source, &archive_path).unwrap(), 1);
        let archive = RpaArchive::open(&archive_path).unwrap();
        assert!(archive.entries.contains_key("tl/russian/story.rpy"));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_archive_change_refreshes_scripts_without_deleting_glossary() {
        let temp_dir = std::env::temp_dir().join(format!("translator_rpa_cache_{}", uuid::Uuid::new_v4()));
        let game_dir = temp_dir.join("game");
        let cache_dir = temp_dir.join("cache");
        let scripts_dir = cache_dir.join("scripts");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("glossary.json"), "saved").unwrap();
        let archive_path = game_dir.join("archive.rpa");
        RpaArchive::create_archive(&archive_path, &[("story.rpy".into(), b"first".to_vec())]).unwrap();
        RpaArchive::extract_scripts_from_game_archives(&game_dir, &scripts_dir).unwrap();
        assert_eq!(fs::read(scripts_dir.join("story.rpy")).unwrap(), b"first");

        RpaArchive::create_archive(&archive_path, &[("story.rpy".into(), b"second version".to_vec())]).unwrap();
        RpaArchive::extract_scripts_from_game_archives(&game_dir, &scripts_dir).unwrap();
        assert_eq!(fs::read(scripts_dir.join("story.rpy")).unwrap(), b"second version");
        assert_eq!(fs::read_to_string(cache_dir.join("glossary.json")).unwrap(), "saved");
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_read_real_game_of_whores_archive() {
        let path = Path::new(r"F:\Games\Game of Whores\Game of Whores (0.34)\game\archive.rpa");
        if !path.exists() {
            return;
        }

        let start = std::time::Instant::now();
        let archive = RpaArchive::open(path).expect("Ошибка открытия archive.rpa в Game of Whores");
        println!("archive.rpa открыт за {:?}! Всего файлов в индексе: {}", start.elapsed(), archive.entries.len());

        let mut rpy_count = 0;
        let mut rpyc_count = 0;
        let mut script_names = Vec::new();

        for name in archive.entries.keys() {
            if name.ends_with(".rpy") {
                rpy_count += 1;
                script_names.push(name.clone());
            } else if name.ends_with(".rpyc") {
                rpyc_count += 1;
                script_names.push(name.clone());
            }
        }

        println!("Найдено .rpy: {}, .rpyc: {}", rpy_count, rpyc_count);
        for s in script_names.iter().take(15) {
            println!("  [Скрипт] {}", s);
        }

        // Тестируем извлечение и декомпиляцию brothel_keyhole.rpyc прямо из архива!
        if let Some(entry) = archive.entries.get("brothel_keyhole.rpyc") {
            let mut file = File::open(path).unwrap();
            use std::io::Seek;
            file.seek(std::io::SeekFrom::Start(entry.offset)).unwrap();
            let mut payload = vec![0u8; entry.length as usize];
            file.read_exact(&mut payload).unwrap();

            let slots = crate::rpyc::RpycParser::read_slots(&payload).unwrap();
            let slot1 = slots.iter().find(|s| s.slot == 1).unwrap();
            let decomp_ast = crate::rpyc::RpycParser::decompress_slot(&payload, slot1).unwrap();
            let mut dec = crate::pickle::PickleDecoder::new(&decomp_ast);
            let _val = dec.decode().unwrap();

            let temp_rpyc = std::env::temp_dir().join("test_extract.rpyc");
            fs::write(&temp_rpyc, &payload).unwrap();
            let stmts = crate::rpyc::RpycParser::parse_rpyc_file(&temp_rpyc).unwrap();
            let rpy_code = crate::rpyc::RpycParser::decompile_to_rpy_text(&stmts);
            println!("Декомпилировано brothel_keyhole.rpyc из архива:\n  Инструкций: {}, Строк: {}", stmts.len(), rpy_code.lines().count());
            let _ = fs::remove_file(&temp_rpyc);
        }

        // БЕНЧМАРК: извлечение и декомпиляция ВСЕХ 204 скриптов во временную папку
        let bench_dir = std::env::temp_dir().join("gow_scripts_bench");
        let bench_start = std::time::Instant::now();
        let decompiled_count = RpaArchive::extract_and_decompile_scripts(path, &bench_dir).expect("Ошибка пакетной декомпиляции");
        println!(
            "БЕНЧМАРК: извлечено и декомпилировано {} скриптов за {:?} в {:?}",
            decompiled_count, bench_start.elapsed(), bench_dir
        );
        let _ = fs::remove_dir_all(&bench_dir);
    }
}
