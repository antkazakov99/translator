//! Папка `files/` проекта: содержимое файлов игры и загруженных шрифтов по хэшу.

use std::collections::HashSet;
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::common::ValidationError;

/// SHA-256 содержимого: 64 строчные шестнадцатеричные цифры. Он же имя файла в `files/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        let valid = text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid {
            return Err(ValidationError::UnknownValue { field: "hash", value: text.to_string() });
        }
        Ok(Self(text.to_string()))
    }

    /// Хэш байтов в памяти.
    pub fn of(bytes: &[u8]) -> Self {
        Self::from_digest(Sha256::digest(bytes).as_slice())
    }

    fn from_digest(digest: &[u8]) -> Self {
        Self(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Записанный файл: по хэшу его находят, размер идёт в `files.size`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub hash: ContentHash,
    pub size: u64,
}

/// Начало имени временного файла. Уборка удаляет такие файлы: их оставила прерванная запись.
const TEMP_PREFIX: &str = ".tmp-";

/// Размер куска при потоковой записи.
const CHUNK: usize = 64 * 1024;

/// Файлы по хэшу содержимого. Чьи это файлы, хранилище не знает.
///
/// - Файлы не меняются, только добавляются и удаляются.
/// - Запись идёт во временный файл и переименование: сбой оставит лишний файл, но не битый.
/// - Сначала файл, потом строка в базе. Уборка идёт под `snapshot_lock` после фиксации транзакции.
pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    const DIR: &str = "files";

    /// Создаёт папку `files/` в папке проекта.
    pub fn create(project_dir: &Path) -> std::io::Result<Self> {
        let dir = project_dir.join(Self::DIR);
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Папка `files/` готового проекта. Её отсутствие это ошибка: файлы снимка пропали.
    pub fn open(project_dir: &Path) -> std::io::Result<Self> {
        let dir = project_dir.join(Self::DIR);
        if !dir.is_dir() {
            return Err(std::io::Error::new(ErrorKind::NotFound, format!("нет папки {}", dir.display())));
        }
        Ok(Self { dir })
    }

    /// Записывает поток, считая хэш на ходу: архивы игр весят гигабайты, целиком в память они не читаются.
    /// Одинаковое содержимое уже лежит, тогда временный файл просто удаляется.
    pub fn put(&self, mut content: impl Read) -> std::io::Result<StoredBlob> {
        let temp = self.dir.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4()));
        let written = (|| {
            let mut out = BufWriter::new(std::fs::File::create_new(&temp)?);
            let mut hasher = Sha256::new();
            let mut buffer = vec![0; CHUNK];
            let mut size = 0u64;
            loop {
                let read = match content.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                };
                hasher.update(&buffer[..read]);
                out.write_all(&buffer[..read])?;
                size += read as u64;
            }
            let file = out.into_inner().map_err(|error| error.into_error())?;
            // Строка в базе появится после файла, поэтому файл должен дожить до сбоя питания.
            file.sync_all()?;
            Ok(StoredBlob { hash: ContentHash::from_digest(hasher.finalize().as_slice()), size })
        })();

        let blob = match written {
            Ok(blob) => blob,
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(error);
            }
        };
        let target = self.path_of(&blob.hash);
        if target.exists() {
            std::fs::remove_file(&temp)?;
        } else if let Err(error) = std::fs::rename(&temp, &target) {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        Ok(blob)
    }

    pub fn open_file(&self, hash: &ContentHash) -> std::io::Result<std::fs::File> {
        std::fs::File::open(self.path_of(hash))
    }

    pub fn read(&self, hash: &ContentHash) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path_of(hash))
    }

    pub fn contains(&self, hash: &ContentHash) -> bool {
        self.path_of(hash).is_file()
    }

    pub fn path_of(&self, hash: &ContentHash) -> PathBuf {
        self.dir.join(hash.as_str())
    }

    /// Уборка: удаляет файлы, которых нет в `keep` (ссылки из `files` и `font_replacements`), и временные.
    /// Чужие имена не трогает. Возвращает число удалённых.
    pub fn retain(&self, keep: &HashSet<ContentHash>) -> std::io::Result<usize> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let unused = match ContentHash::parse(&name) {
                Ok(hash) => !keep.contains(&hash),
                Err(_) => name.starts_with(TEMP_PREFIX),
            };
            if unused && entry.file_type()?.is_file() {
                std::fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_is_stored_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let first = store.put(&b"label start:"[..]).unwrap();
        let second = store.put(&b"label start:"[..]).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.size, 12);
        assert_eq!(first.hash, ContentHash::of(b"label start:"));
        assert_eq!(store.read(&first.hash).unwrap(), b"label start:");
        assert_eq!(std::fs::read_dir(dir.path().join("files")).unwrap().count(), 1);
    }

    #[test]
    fn large_content_is_streamed() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let data: Vec<u8> = (0..CHUNK * 3 + 17).map(|i| (i % 251) as u8).collect();
        let blob = store.put(data.as_slice()).unwrap();
        assert_eq!(blob.hash, ContentHash::of(&data));
        assert_eq!(blob.size, data.len() as u64);
    }

    #[test]
    fn failed_read_leaves_no_temp_file() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("обрыв"))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        assert!(store.put(Broken).is_err());
        assert_eq!(std::fs::read_dir(dir.path().join("files")).unwrap().count(), 0);
    }

    #[test]
    fn retain_removes_unused_and_temp_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::create(dir.path()).unwrap();
        let kept = store.put(&b"kept"[..]).unwrap().hash;
        let dropped = store.put(&b"dropped"[..]).unwrap().hash;
        std::fs::write(dir.path().join("files").join(".tmp-left"), b"x").unwrap();
        std::fs::write(dir.path().join("files").join("notes.txt"), b"x").unwrap();

        let removed = store.retain(&HashSet::from([kept.clone()])).unwrap();
        assert_eq!(removed, 2);
        assert!(store.contains(&kept));
        assert!(!store.contains(&dropped));
        assert!(dir.path().join("files").join("notes.txt").exists());
    }

    #[test]
    fn open_needs_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FileStore::open(dir.path()).is_err());
        FileStore::create(dir.path()).unwrap();
        assert!(FileStore::open(dir.path()).is_ok());
    }
}
