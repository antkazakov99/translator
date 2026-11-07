//! Снимок игры в project.db: `scans`, `files`, `speakers`, `containers`, `segments`.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{MatchPlan, MatchSegment, MissingSegment, Scan, ScanId, SegmentId};
use crate::common::Timestamp;
use crate::engines::{ContainerKey, FileKind, GameInfo, GameSnapshot, SnapshotEdge, SnapshotFile};
use crate::files::ContentHash;
use crate::glossary::NameKey;
use crate::projects::GameRoot;
use crate::storage::DbError;

pub struct ScanRepo<'c> {
    pub conn: &'c Connection,
}

impl ScanRepo<'_> {
    /// Пишет первый снимок: строка `scans`, все записи с её `last_seen_scan` и переходы `edges`. Поднимает `project.search_rev`.
    pub fn save(
        &self,
        snapshot: &GameSnapshot,
        root: Option<&GameRoot>,
        parser_version: u32,
        edges_version: u32,
    ) -> Result<ScanId, DbError> {
        let info = serde_json::to_string(&snapshot.info).map_err(|error| DbError::Corrupt(error.to_string()))?;
        let root = root.map(|root| root.as_path().to_string_lossy().into_owned());
        self.conn.execute(
            "INSERT INTO scans (read_at, game_root, parser_version, game_info) VALUES (?1, ?2, ?3, ?4)",
            params![Timestamp::now().secs(), root, parser_version, info],
        )?;
        let scan = ScanId::from_row(self.conn.last_insert_rowid());

        let mut file_ids = HashMap::new();
        {
            let mut insert = self.conn.prepare(
                "INSERT INTO files (path, kind, hash, size, last_seen_scan) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (path, hash) DO UPDATE SET kind = excluded.kind, last_seen_scan = excluded.last_seen_scan
                 RETURNING id",
            )?;
            for file in &snapshot.files {
                let id: i64 = insert.query_row(
                    params![file.path, file.kind.as_str(), file.hash.as_str(), file.size as i64, scan.get()],
                    |row| row.get(0),
                )?;
                file_ids.insert(file.path.as_str(), id);
            }
        }

        {
            let mut insert = self.conn.prepare(
                "INSERT INTO speakers (key, kind, label, label_key, last_seen_scan) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (key) DO UPDATE SET kind = excluded.kind, label = excluded.label,
                   label_key = excluded.label_key, last_seen_scan = excluded.last_seen_scan",
            )?;
            for speaker in &snapshot.speakers {
                let label_key = speaker.label.as_deref().and_then(NameKey::new);
                insert.execute(params![
                    speaker.key.as_str(),
                    speaker.kind,
                    speaker.label,
                    label_key.as_ref().map(NameKey::as_str),
                    scan.get()
                ])?;
            }
        }

        let mut container_ids = HashMap::new();
        {
            let mut insert = self.conn.prepare(
                "INSERT INTO containers (key, kind, file_id, story_order, last_seen_scan) VALUES (?1, ?2, ?3, ?4, ?5)
                 RETURNING id",
            )?;
            for container in &snapshot.containers {
                let file = container.file.as_deref().and_then(|path| file_ids.get(path)).copied();
                let id: i64 = insert.query_row(
                    params![container.key.as_str(), container.kind, file, container.story_order, scan.get()],
                    |row| row.get(0),
                )?;
                container_ids.insert(container.key.as_str(), id);
            }
            let mut parent = self.conn.prepare("UPDATE containers SET parent_id = ?2 WHERE id = ?1")?;
            for container in &snapshot.containers {
                if let Some(parent_key) = &container.parent {
                    let parent_id = container_ids.get(parent_key.as_str()).copied();
                    parent.execute(params![container_ids[container.key.as_str()], parent_id])?;
                }
            }
        }

        {
            let mut insert = self.conn.prepare(
                "INSERT INTO segments (key, container_id, file_id, speaker_key, source_text, source_hash, game_text,
                   location, story_order, engine_meta, last_seen_scan)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for segment in &snapshot.segments {
                let file = *file_ids
                    .get(segment.file.as_str())
                    .ok_or_else(|| DbError::Corrupt(format!("реплика из файла не из снимка: {}", segment.file)))?;
                let container = segment.container.as_ref().and_then(|key| container_ids.get(key.as_str())).copied();
                let meta = segment.engine_meta.as_ref().map(serde_json::Value::to_string);
                insert.execute(params![
                    segment.key.as_str(),
                    container,
                    file,
                    segment.speaker.as_ref().map(|key| key.as_str()),
                    segment.source_text,
                    segment.source_hash,
                    segment.game_text,
                    segment.location,
                    segment.story_order,
                    meta,
                    scan.get()
                ])?;
            }
        }

        EdgeRepo { conn: self.conn }.replace(&snapshot.edges, edges_version, scan)?;
        self.conn.execute("UPDATE project SET search_rev = search_rev + 1", [])?;
        Ok(scan)
    }

    /// Последнее считывание.
    pub fn latest(&self) -> Result<Option<Scan>, DbError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, read_at, game_root, parser_version, game_info FROM scans ORDER BY id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, read_at, root, parser_version, info)) = row else { return Ok(None) };
        let game_root = root
            .map(|root| GameRoot::new(Path::new(&root)))
            .transpose()
            .map_err(|error| DbError::bad_value("scans", "game_root", error))?;
        let game_info: GameInfo =
            serde_json::from_str(&info).map_err(|error| DbError::Corrupt(format!("scans.game_info: {error}")))?;
        Ok(Some(Scan {
            id: ScanId::from_row(id),
            read_at: Timestamp::from_secs(read_at),
            game_root,
            parser_version,
            game_info,
        }))
    }

    /// Файлы текущего снимка: для пересборки из `files/` и сборки мода.
    pub fn current_files(&self) -> Result<Vec<SnapshotFile>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT path, kind, hash, size FROM files WHERE last_seen_scan = (SELECT max(id) FROM scans) ORDER BY path",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(path, kind, hash, size)| {
                Ok(SnapshotFile {
                    path,
                    kind: FileKind::parse(&kind).map_err(|error| DbError::bad_value("files", "kind", error))?,
                    hash: ContentHash::parse(&hash).map_err(|error| DbError::bad_value("files", "hash", error))?,
                    size: u64::try_from(size).unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Текущие реплики для сопоставления.
    pub fn current_segments(&self) -> Result<Vec<MatchSegment<SegmentId>>, DbError> {
        todo!()
    }

    /// Повторное считывание: новые строки, перенос `translations` и `revisions` по плану, строки `transfer`,
    /// `missing = 1` у старых, перенос ключей описаний сцен, удаление лишнего. Поднимает `project.search_rev`.
    pub fn apply(&self, snapshot: &GameSnapshot, plan: &MatchPlan, root: Option<&GameRoot>, parser_version: u32) -> Result<ScanId, DbError> {
        todo!()
    }

    /// «Пропавшие».
    pub fn missing(&self) -> Result<Vec<MissingSegment>, DbError> {
        todo!()
    }

    /// Ключи реплик текущего снимка: для проверки «та ли это игра» при смене папки.
    pub fn segment_keys(&self) -> Result<Vec<String>, DbError> {
        let mut statement = self.conn.prepare("SELECT key FROM segments WHERE missing = 0")?;
        let keys = statement.query_map([], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(keys)
    }
}

/// Имя таблицы переходов в `derived`.
const EDGES: &str = "edges";

/// `edges` и её строка в `derived`: переходы текущего снимка, пересобираются без игры.
pub struct EdgeRepo<'c> {
    pub conn: &'c Connection,
}

impl EdgeRepo<'_> {
    /// Версия сборки и считывание, по которому собрана таблица. `None`: таблица ещё не собиралась.
    pub fn state(&self) -> Result<Option<(u32, ScanId)>, DbError> {
        let row = self
            .conn
            .query_row("SELECT version, scan_id FROM derived WHERE name = ?1", [EDGES], |row| {
                Ok((row.get::<_, u32>(0)?, ScanId::from_row(row.get(1)?)))
            })
            .optional()?;
        Ok(row)
    }

    /// Заменяет переходы целиком и отмечает, по какому считыванию и версии они собраны.
    pub fn replace(&self, edges: &[SnapshotEdge], version: u32, scan: ScanId) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM edges", [])?;
        let mut insert = self.conn.prepare(
            "INSERT INTO edges (from_container, to_container, kind, condition) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT DO NOTHING",
        )?;
        for edge in edges {
            insert.execute(params![edge.from.as_str(), edge.to.as_str(), edge.kind, edge.condition])?;
        }
        self.conn.execute(
            "INSERT INTO derived (name, version, scan_id) VALUES (?1, ?2, ?3)
             ON CONFLICT (name) DO UPDATE SET version = excluded.version, scan_id = excluded.scan_id",
            params![EDGES, version, scan.get()],
        )?;
        Ok(())
    }

    /// Все переходы.
    pub fn list(&self) -> Result<Vec<SnapshotEdge>, DbError> {
        let mut statement =
            self.conn.prepare("SELECT from_container, to_container, kind, condition FROM edges ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(from, to, kind, condition)| {
                let bad = |column| move |error| DbError::bad_value("edges", column, error);
                Ok(SnapshotEdge {
                    from: ContainerKey::new(from).map_err(bad("from_container"))?,
                    to: ContainerKey::new(to).map_err(bad("to_container"))?,
                    kind,
                    condition,
                })
            })
            .collect()
    }
}
