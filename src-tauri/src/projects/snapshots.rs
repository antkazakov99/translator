//! Именованные снимки проекта. Каждый снимок можно восстановить несколько раз.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{ProjectId, ProjectInfoRepo, Title};
use crate::common::{AppError, Timestamp, uuid_id};
use crate::files::FileStore;
use crate::llm::LlmRunRepo;
use crate::scan::{ScanRepo, ScanService};
use crate::state::AppState;
use crate::storage::ProjectDb;

uuid_id!(
    /// Имя папки снимка.
    SnapshotId
);

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub title: String,
    pub created_at: i64,
    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, serde_json::Value>,
}

pub struct SnapshotService<'a> {
    pub state: &'a AppState,
}

impl SnapshotService<'_> {
    pub fn save(&self, id: ProjectId, title: Title) -> Result<(), AppError> {
        if self.state.llm_run.is_active() {
            return Err(AppError::RunActive);
        }
        let open = self.state.open_project(id)?;
        let _lock = open.snapshot_lock.lock();

        let root = self.root(id);
        fs::create_dir_all(&root)?;
        let stage = tempfile::tempdir_in(&root)?;
        open.project.save_snapshot(stage.path())?;
        let db = ProjectDb::open(stage.path())?;
        copy_files(&db, &open.files, &FileStore::create(stage.path())?)?;
        drop(db);
        let snapshot = Snapshot {
            title: title.as_str().to_owned(),
            created_at: Timestamp::now().secs(),
            extra: Default::default(),
        };
        let metadata = serde_json::to_vec(&snapshot).map_err(io::Error::other)?;
        fs::write(stage.path().join("snapshot.json"), metadata)?;
        fs::rename(stage.path(), root.join(SnapshotId::generate().to_string()))?;
        Ok(())
    }

    pub fn list(&self, project: ProjectId) -> Result<Vec<(SnapshotId, Snapshot)>, AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let mut snapshots = Vec::new();
        for (id, path) in snapshot_dirs(&self.root(project))? {
            snapshots.push((id, read_snapshot(&path)?));
        }
        for (id, path) in snapshot_dirs(&self.legacy_root())? {
            if ProjectDb::snapshot_owner(&path).is_ok_and(|owner| owner == project) {
                snapshots.push((id, read_snapshot(&path)?));
            }
        }
        snapshots.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        Ok(snapshots)
    }

    pub fn rename(&self, project: ProjectId, id: SnapshotId, title: Title) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let path = self.snapshot_path(project, id)?;
        let mut snapshot = read_snapshot(&path)?;
        snapshot.title = title.as_str().to_owned();

        let mut metadata = tempfile::NamedTempFile::new_in(&path)?;
        serde_json::to_writer(&mut metadata, &snapshot).map_err(io::Error::other)?;
        metadata.flush()?;
        metadata.as_file().sync_all()?;
        metadata.persist(path.join("snapshot.json")).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn delete(&self, project: ProjectId, id: SnapshotId) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let path = self.snapshot_path(project, id)?;
        let parent = path.parent().ok_or(AppError::SnapshotNotFound)?;
        // Сначала убираем снимок из списка, чтобы ошибка очистки не оставила видимый обрывок.
        let removed = tempfile::tempdir_in(parent)?;
        fs::rename(&path, removed.path().join("snapshot"))?;
        if let Err(error) = removed.close() {
            tracing::warn!("Не удалось полностью очистить удалённый снимок {id}: {error}");
        }
        Ok(())
    }

    pub fn restore(&self, project: ProjectId, id: SnapshotId) -> Result<(), AppError> {
        if self.state.llm_run.is_active() {
            return Err(AppError::RunActive);
        }
        // Держим реестр, пока база и файлы приводятся к одному состоянию.
        let projects = self.state.open_projects.lock();
        let open = projects.get(&project).ok_or(AppError::ProjectNotOpen)?;
        if Arc::strong_count(open) != 1 {
            return Err(AppError::ProjectBusy);
        }

        let source = self.snapshot_path(project, id)?;
        let stage = tempfile::tempdir_in(&open.dir)?;
        let db = ProjectDb::copy_snapshot(&source, stage.path())?;
        if db.read(|tx| (ProjectInfoRepo { conn: tx }).get())?.id != project {
            return Err(AppError::SnapshotNotFound);
        }
        db.write(|tx| (LlmRunRepo { conn: tx }).fail_stale(Timestamp::now()))?;
        copy_files(&db, &FileStore::open(&source)?, &open.files)?;
        // Идентификатор считывания и ревизии поиска может совпасть у двух разных состояний.
        open.cache.write(|tx| Ok(tx.execute("DELETE FROM meta", [])?))?;
        open.project.restore_snapshot(&db)?;
        ScanService::clean_files(open);
        Ok(())
    }

    fn snapshot_path(&self, project: ProjectId, id: SnapshotId) -> Result<PathBuf, AppError> {
        let current_root = self.root(project);
        let legacy_root = self.legacy_root();
        let folder = id.to_string();
        for root in [&current_root, &legacy_root] {
            let path = root.join(&folder);
            if !path.is_dir() || path.canonicalize()? != root.canonicalize()?.join(&folder) {
                continue;
            }
            if root == &current_root || ProjectDb::snapshot_owner(&path).is_ok_and(|owner| owner == project) {
                return Ok(path);
            }
        }
        Err(AppError::SnapshotNotFound)
    }

    fn root(&self, project: ProjectId) -> PathBuf {
        self.state.project_dir(project).join("snapshots")
    }

    fn legacy_root(&self) -> PathBuf {
        self.state.data_dir.join("snapshots")
    }
}

fn snapshot_dirs(root: &Path) -> Result<Vec<(SnapshotId, PathBuf)>, io::Error> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Ok(id) = SnapshotId::parse(&entry.file_name().to_string_lossy()) {
            dirs.push((id, entry.path()));
        }
    }
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    #[test]
    fn rename_and_delete_apply_only_to_the_selected_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().to_owned(), memory_secrets()).unwrap();
        let projects = ProjectService { state: &state };
        let snapshots = SnapshotService { state: &state };
        let owner = crate::state::tests::ready_project(&state, "owner").id();
        let other = crate::state::tests::ready_project(&state, "other").id();
        projects.open(owner).unwrap();
        projects.open(other).unwrap();
        snapshots.save(owner, Title::new("Снимок").unwrap()).unwrap();
        snapshots.save(other, Title::new("Другой снимок").unwrap()).unwrap();
        let (id, original) = snapshots.list(owner).unwrap().pop().unwrap();
        assert!(matches!(snapshots.delete(other, id), Err(AppError::SnapshotNotFound)));

        let current = snapshots.root(owner).join(id.to_string());
        let renamed = Title::new("Новое название").unwrap();
        snapshots.rename(owner, id, renamed.clone()).unwrap();
        assert_eq!(snapshots.list(owner).unwrap()[0].1.title, renamed.as_str());

        fs::create_dir_all(snapshots.legacy_root()).unwrap();
        let legacy = snapshots.legacy_root().join(id.to_string());
        fs::rename(&current, &legacy).unwrap();
        assert!(matches!(
            snapshots.rename(other, id, renamed),
            Err(AppError::SnapshotNotFound)
        ));
        assert!(matches!(snapshots.delete(other, id), Err(AppError::SnapshotNotFound)));
        snapshots.rename(owner, id, Title::new("Прежний формат").unwrap()).unwrap();
        let (renamed_id, renamed_snapshot) = snapshots.list(owner).unwrap().pop().unwrap();
        assert_eq!(renamed_id, id);
        assert_eq!(renamed_snapshot.created_at, original.created_at);
        assert_eq!(renamed_snapshot.title, "Прежний формат");
        snapshots.restore(owner, id).unwrap();

        snapshots.delete(owner, id).unwrap();
        assert!(!legacy.exists());
        assert!(snapshots.list(owner).unwrap().is_empty());
        assert!(matches!(snapshots.restore(owner, id), Err(AppError::SnapshotNotFound)));
        assert_eq!(snapshots.list(other).unwrap()[0].1.title, "Другой снимок");
    }

    #[test]
    fn snapshots_restore_in_place_and_only_belong_to_their_project() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let state = AppState::start(data.clone(), memory_secrets()).unwrap();
        let projects = ProjectService { state: &state };
        let snapshots = SnapshotService { state: &state };
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.path().join("game"));
        let original = projects.create(root.clone()).unwrap();
        projects.open(original.id()).unwrap();
        let open = state.open_project(original.id()).unwrap();
        let hash = open
            .project
            .write(|tx| {
                tx.execute(
                    "INSERT INTO translations (segment_id, text, source_hash, updated_at)
                 SELECT id, 'Сохранённый перевод', source_hash, 1 FROM segments ORDER BY id LIMIT 1",
                    [],
                )?;
                tx.query_row("SELECT hash FROM files ORDER BY id LIMIT 1", [], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(Into::into)
            })
            .unwrap();
        let hash = crate::files::ContentHash::parse(&hash).unwrap();
        let file = open.files.read(&hash).unwrap();
        snapshots
            .save(original.id(), Title::new("До запуска модели").unwrap())
            .unwrap();
        let baseline = snapshots.list(original.id()).unwrap()[0].0;
        open.project
            .write(|tx| Ok(tx.execute("UPDATE translations SET text = 'Результат модели'", [])?))
            .unwrap();
        snapshots.save(original.id(), Title::new("Модель A").unwrap()).unwrap();
        let result = snapshots.list(original.id()).unwrap()[0].0;
        assert!(matches!(
            snapshots.restore(original.id(), baseline),
            Err(AppError::ProjectBusy)
        ));
        fs::remove_file(open.files.path_of(&hash)).unwrap();
        drop(open);

        // Снимок, созданный прежним интерфейсом, остаётся видимым только в исходном проекте.
        fs::create_dir_all(snapshots.legacy_root()).unwrap();
        fs::rename(
            snapshots.root(original.id()).join(baseline.to_string()),
            snapshots.legacy_root().join(baseline.to_string()),
        )
        .unwrap();
        let other = crate::state::tests::ready_project(&state, "other");
        projects.open(other.id()).unwrap();
        assert!(snapshots.list(other.id()).unwrap().is_empty());
        for snapshot in [baseline, result] {
            assert!(matches!(
                snapshots.restore(other.id(), snapshot),
                Err(AppError::SnapshotNotFound)
            ));
        }
        drop(state);

        let state = AppState::start(data, memory_secrets()).unwrap();
        let projects = ProjectService { state: &state };
        projects.open(original.id()).unwrap();
        let snapshots = SnapshotService { state: &state };
        let saved = snapshots.list(original.id()).unwrap();
        assert_eq!(
            saved.iter().map(|(_, snapshot)| snapshot.title.as_str()).collect::<Vec<_>>(),
            ["Модель A", "До запуска модели"]
        );
        for (snapshot, expected) in [
            (baseline, "Сохранённый перевод"),
            (result, "Результат модели"),
            (baseline, "Сохранённый перевод"),
        ] {
            {
                let open = state.open_project(original.id()).unwrap();
                open.cache
                    .write(|tx| Ok(tx.execute("INSERT INTO meta VALUES ('scan_id', '1')", [])?))
                    .unwrap();
            }
            snapshots.restore(original.id(), snapshot).unwrap();
            let open = state.open_project(original.id()).unwrap();
            assert_eq!(
                open.project.read(|tx| ProjectInfoRepo { conn: tx }.get()).unwrap().id,
                original.id()
            );
            let text: String = open
                .project
                .read(|tx| Ok(tx.query_row("SELECT text FROM translations", [], |row| row.get(0))?))
                .unwrap();
            assert_eq!(text, expected);
            assert_eq!(open.files.read(&hash).unwrap(), file);
            assert!(
                !open
                    .cache
                    .read(|tx| Ok(tx.prepare("SELECT 1 FROM meta")?.exists([])?))
                    .unwrap()
            );
        }
        let listed = projects.list().unwrap();
        assert_eq!(listed.len(), 2);
        let current = listed.iter().find(|project| project.id() == original.id()).unwrap();
        assert_eq!(current.game_root(), Some(&root));
        assert_eq!(current.title(), original.title());
    }
}

fn read_snapshot(dir: &Path) -> Result<Snapshot, io::Error> {
    serde_json::from_slice(&fs::read(dir.join("snapshot.json"))?).map_err(io::Error::other)
}

fn copy_files(db: &ProjectDb, source: &FileStore, destination: &FileStore) -> Result<(), AppError> {
    let hashes = db.read(|tx| ScanRepo { conn: tx }.referenced_hashes())?;
    for hash in hashes {
        if !destination.contains(&hash) && destination.put(source.open_file(&hash)?)?.hash != hash {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Файл {} повреждён", hash.as_str())).into());
        }
    }
    Ok(())
}
