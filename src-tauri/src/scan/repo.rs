//! Снимок игры в project.db: `scans`, `files`, `speakers`, `containers`, `segments`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{FileId, MatchPlan, MatchSegment, MissingSegment, Scan, ScanId, ScanReport, SegmentId};
use crate::common::Timestamp;
use crate::engines::{ContainerKey, FileKind, GameInfo, GameSnapshot, SegmentKey, SnapshotEdge, SnapshotFile, SnapshotSegment};
use crate::files::ContentHash;
use crate::glossary::NameKey;
use crate::projects::GameRoot;
use crate::storage::DbError;

pub struct ScanRepo<'c> {
    pub conn: &'c Connection,
}

/// Как пара из сопоставления меняет перевод новой строки.
struct Transfer {
    old: SegmentId,
    draft: bool,
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
        let scan = self.insert_scan(snapshot, root, parser_version)?;
        let file_ids = self.write_files(snapshot, scan)?;
        self.write_speakers(snapshot, scan)?;
        let container_ids = self.write_containers(snapshot, scan, &file_ids)?;
        for segment in &snapshot.segments {
            self.insert_segment(segment, scan, &file_ids, &container_ids)?;
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

    /// Путь файла, если строка из текущего снимка.
    pub fn current_path(&self, file: FileId) -> Result<Option<String>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT path FROM files WHERE id = ?1 AND last_seen_scan = (SELECT max(id) FROM scans)",
                [file.get()],
                |row| row.get(0),
            )
            .optional()?)
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

    /// Текущие реплики для сопоставления, по сюжету.
    pub fn current_segments(&self) -> Result<Vec<MatchSegment<SegmentId>>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT s.id, s.key, c.key, f.path, s.source_hash, s.source_text, s.story_order
             FROM segments s
             JOIN files f ON f.id = s.file_id
             LEFT JOIN containers c ON c.id = s.container_id
             WHERE s.missing = 0
             ORDER BY s.story_order IS NULL, s.story_order, s.id",
        )?;
        type Row = (i64, String, Option<String>, String, String, String, Option<i64>);
        let rows = statement
            .query_map([], |row| -> rusqlite::Result<Row> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, key, container, file, source_hash, source_text, story_order)| {
                Ok(MatchSegment {
                    id: SegmentId::from_row(id),
                    key: SegmentKey::new(key).map_err(|error| DbError::bad_value("segments", "key", error))?,
                    container: container
                        .map(ContainerKey::new)
                        .transpose()
                        .map_err(|error| DbError::bad_value("containers", "key", error))?,
                    file,
                    source_hash,
                    source_text,
                    story_order,
                })
            })
            .collect()
    }

    /// Повторное считывание: новые строки, перенос `translations` и `revisions` по плану, строки `transfer`,
    /// `missing = 1` у старых, перенос ключей описаний сцен, удаление лишнего. Поднимает `project.search_rev`.
    ///
    /// Номера новых реплик в плане это места в `snapshot.segments`.
    pub fn apply(
        &self,
        snapshot: &GameSnapshot,
        plan: &MatchPlan,
        root: Option<&GameRoot>,
        parser_version: u32,
        edges_version: u32,
    ) -> Result<ScanReport, DbError> {
        let now = Timestamp::now().secs();
        let scan = self.insert_scan(snapshot, root, parser_version)?;
        let file_ids = self.write_files(snapshot, scan)?;
        self.write_speakers(snapshot, scan)?;
        let container_ids = self.write_containers(snapshot, scan, &file_ids)?;

        // Сначала все прежние реплики пропали: строку возвращает только пара с тем же ключом,
        // поэтому ключ в индексе текущих реплик не встретится дважды.
        self.conn.execute("UPDATE segments SET missing = 1 WHERE missing = 0", [])?;

        let mut same_rows: HashMap<usize, Transfer> = HashMap::new();
        let mut moves: HashMap<usize, Transfer> = HashMap::new();
        for pair in &plan.pairs {
            let transfer = Transfer { old: pair.old, draft: pair.draft };
            if pair.same_key {
                same_rows.insert(pair.new, transfer);
            } else {
                moves.insert(pair.new, transfer);
            }
        }

        let mut report = ScanReport { scan, lines: snapshot.segments.len(), new_lines: 0, transferred: 0, drafts: 0, missing: 0 };
        for (index, segment) in snapshot.segments.iter().enumerate() {
            if let Some(transfer) = same_rows.get(&index) {
                self.update_segment(transfer.old, segment, scan, &file_ids, &container_ids)?;
                if transfer.draft && self.carry_translation(transfer.old, transfer.old.get(), true, now)?.made_draft {
                    report.drafts += 1;
                }
                continue;
            }
            let id = self.insert_segment(segment, scan, &file_ids, &container_ids)?;
            match moves.get(&index) {
                Some(transfer) => {
                    let carried = self.carry_translation(transfer.old, id, transfer.draft, now)?;
                    report.transferred += usize::from(carried.moved);
                    report.drafts += usize::from(carried.made_draft);
                }
                None => report.new_lines += 1,
            }
        }

        for (from, to) in &plan.moved_containers {
            self.move_container_notes(from, to)?;
        }

        // Пропавшие без перевода и истории не нужны, как и старые записи снимка без ссылок.
        self.conn.execute(
            "DELETE FROM segments WHERE missing = 1
               AND id NOT IN (SELECT segment_id FROM translations)
               AND id NOT IN (SELECT segment_id FROM revisions)",
            [],
        )?;
        self.conn.execute(
            "DELETE FROM containers WHERE last_seen_scan <> ?1
               AND id NOT IN (SELECT container_id FROM segments WHERE container_id IS NOT NULL)",
            [scan.get()],
        )?;
        self.conn.execute(
            "DELETE FROM speakers WHERE last_seen_scan <> ?1
               AND key NOT IN (SELECT speaker_key FROM segments WHERE speaker_key IS NOT NULL)",
            [scan.get()],
        )?;
        self.conn.execute(
            "DELETE FROM files WHERE last_seen_scan <> ?1 AND id NOT IN (SELECT file_id FROM segments)",
            [scan.get()],
        )?;
        report.missing = self.conn.query_row("SELECT count(*) FROM segments WHERE missing = 1", [], |row| row.get::<_, i64>(0))? as usize;

        EdgeRepo { conn: self.conn }.replace(&snapshot.edges, edges_version, scan)?;
        self.conn.execute("UPDATE project SET search_rev = search_rev + 1", [])?;
        Ok(report)
    }

    /// «Пропавшие»: реплики прошлых версий игры с переводом или историей, по файлам.
    pub fn missing(&self) -> Result<Vec<MissingSegment>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT s.id, s.key, c.key, s.source_text, t.text, f.path, s.location
             FROM segments s
             JOIN files f ON f.id = s.file_id
             LEFT JOIN containers c ON c.id = s.container_id
             LEFT JOIN translations t ON t.segment_id = s.id
             WHERE s.missing = 1
             ORDER BY f.path, s.story_order IS NULL, s.story_order, s.id",
        )?;
        type Row = (i64, String, Option<String>, String, Option<String>, String, Option<String>);
        let rows = statement
            .query_map([], |row| -> rusqlite::Result<Row> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(id, key, container, source_text, translation, file, location)| {
                Ok(MissingSegment {
                    id: SegmentId::from_row(id),
                    key: SegmentKey::new(key).map_err(|error| DbError::bad_value("segments", "key", error))?,
                    container: container
                        .map(ContainerKey::new)
                        .transpose()
                        .map_err(|error| DbError::bad_value("containers", "key", error))?,
                    source_text,
                    translation,
                    file,
                    location,
                })
            })
            .collect()
    }

    /// Переносит перевод и историю пропавшей реплики на текущую. Перевод текущей реплики заменяется,
    /// прежнее состояние остаётся в строке `transfer`. Пропавшая строка после переноса удаляется.
    /// `NotFound`: пропавшей реплики с переводом или текущей реплики нет.
    pub fn attach_missing(&self, missing: SegmentId, target: SegmentId) -> Result<(), DbError> {
        let found: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM segments s JOIN translations t ON t.segment_id = s.id
                            WHERE s.id = ?1 AND s.missing = 1)
                AND EXISTS (SELECT 1 FROM segments WHERE id = ?2 AND missing = 0)",
            params![missing.get(), target.get()],
            |row| row.get(0),
        )?;
        if !found {
            return Err(DbError::NotFound);
        }
        let before = self.translation(target)?;
        let (text, _, _) = self.translation(missing)?.ok_or(DbError::NotFound)?;
        self.conn.execute("DELETE FROM translations WHERE segment_id = ?1", [target.get()])?;
        self.conn.execute("UPDATE revisions SET segment_id = ?2 WHERE segment_id = ?1", params![missing.get(), target.get()])?;
        self.conn.execute("UPDATE translations SET segment_id = ?2 WHERE segment_id = ?1", params![missing.get(), target.get()])?;
        let now = Timestamp::now().secs();
        let (had_row, text_before, draft_before, hash_before) = match before {
            Some((text, draft, hash)) => (true, Some(text), Some(draft), Some(hash)),
            None => (false, None, None, None),
        };
        self.conn.execute(
            "INSERT INTO revisions (segment_id, kind, had_row, text_before, draft_before, source_hash_before, text_after, created_at)
             VALUES (?1, 'transfer', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![target.get(), had_row, text_before, draft_before, hash_before, text, now],
        )?;
        self.conn.execute("DELETE FROM segments WHERE id = ?1", [missing.get()])?;
        self.conn.execute("UPDATE project SET search_rev = search_rev + 1", [])?;
        Ok(())
    }

    /// Ключи реплик текущего снимка: для проверки «та ли это игра» при смене папки.
    pub fn segment_keys(&self) -> Result<Vec<String>, DbError> {
        let mut statement = self.conn.prepare("SELECT key FROM segments WHERE missing = 0")?;
        let keys = statement.query_map([], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(keys)
    }

    /// Хэши, на которые ссылается проект: всё остальное в `files/` уборка удаляет.
    /// Замены шрифтов тоже лежат в `files/`, поэтому их хэши входят сюда.
    pub fn referenced_hashes(&self) -> Result<HashSet<ContentHash>, DbError> {
        let mut statement = self.conn.prepare("SELECT hash FROM files UNION SELECT hash FROM font_replacements")?;
        let hashes = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        hashes
            .into_iter()
            .map(|hash| ContentHash::parse(&hash).map_err(|error| DbError::bad_value("files", "hash", error)))
            .collect()
    }

    fn insert_scan(&self, snapshot: &GameSnapshot, root: Option<&GameRoot>, parser_version: u32) -> Result<ScanId, DbError> {
        let info = serde_json::to_string(&snapshot.info).map_err(|error| DbError::Corrupt(error.to_string()))?;
        let root = root.map(|root| root.as_path().to_string_lossy().into_owned());
        self.conn.execute(
            "INSERT INTO scans (read_at, game_root, parser_version, game_info) VALUES (?1, ?2, ?3, ?4)",
            params![Timestamp::now().secs(), root, parser_version, info],
        )?;
        Ok(ScanId::from_row(self.conn.last_insert_rowid()))
    }

    /// Строки файлов снимка: неизменившийся файл сохраняет строку, изменённый получает новую.
    fn write_files<'s>(&self, snapshot: &'s GameSnapshot, scan: ScanId) -> Result<HashMap<&'s str, i64>, DbError> {
        let mut file_ids = HashMap::new();
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
        Ok(file_ids)
    }

    fn write_speakers(&self, snapshot: &GameSnapshot, scan: ScanId) -> Result<(), DbError> {
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
        Ok(())
    }

    /// Контейнеры по ключу: строка того же ключа сохраняется, пропавшие реплики остаются в своей сцене.
    fn write_containers<'s>(
        &self,
        snapshot: &'s GameSnapshot,
        scan: ScanId,
        file_ids: &HashMap<&str, i64>,
    ) -> Result<HashMap<&'s str, i64>, DbError> {
        let mut container_ids = HashMap::new();
        let mut insert = self.conn.prepare(
            "INSERT INTO containers (key, kind, file_id, story_order, last_seen_scan) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key) DO UPDATE SET parent_id = NULL, kind = excluded.kind, file_id = excluded.file_id,
               story_order = excluded.story_order, last_seen_scan = excluded.last_seen_scan
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
        Ok(container_ids)
    }

    fn segment_refs(
        segment: &SnapshotSegment,
        file_ids: &HashMap<&str, i64>,
        container_ids: &HashMap<&str, i64>,
    ) -> Result<(i64, Option<i64>, Option<String>), DbError> {
        let file = *file_ids
            .get(segment.file.as_str())
            .ok_or_else(|| DbError::Corrupt(format!("реплика из файла не из снимка: {}", segment.file)))?;
        let container = segment.container.as_ref().and_then(|key| container_ids.get(key.as_str())).copied();
        let meta = segment.engine_meta.as_ref().map(serde_json::Value::to_string);
        Ok((file, container, meta))
    }

    fn insert_segment(
        &self,
        segment: &SnapshotSegment,
        scan: ScanId,
        file_ids: &HashMap<&str, i64>,
        container_ids: &HashMap<&str, i64>,
    ) -> Result<i64, DbError> {
        let (file, container, meta) = Self::segment_refs(segment, file_ids, container_ids)?;
        let mut insert = self.conn.prepare_cached(
            "INSERT INTO segments (key, container_id, file_id, speaker_key, source_text, source_hash, game_text,
               location, story_order, engine_meta, last_seen_scan)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
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
        Ok(self.conn.last_insert_rowid())
    }

    /// Пара по ключу: та же строка с данными новой версии.
    fn update_segment(
        &self,
        id: SegmentId,
        segment: &SnapshotSegment,
        scan: ScanId,
        file_ids: &HashMap<&str, i64>,
        container_ids: &HashMap<&str, i64>,
    ) -> Result<(), DbError> {
        let (file, container, meta) = Self::segment_refs(segment, file_ids, container_ids)?;
        let mut update = self.conn.prepare_cached(
            "UPDATE segments SET missing = 0, container_id = ?2, file_id = ?3, speaker_key = ?4, source_text = ?5,
               source_hash = ?6, game_text = ?7, location = ?8, story_order = ?9, engine_meta = ?10, last_seen_scan = ?11
             WHERE id = ?1",
        )?;
        update.execute(params![
            id.get(),
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
        Ok(())
    }

    /// Текст, черновик и отпечаток оригинала перевода реплики.
    fn translation(&self, segment: SegmentId) -> Result<Option<(String, bool, String)>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT text, draft, source_hash FROM translations WHERE segment_id = ?1",
                [segment.get()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }

    /// Ведёт перевод и историю старой строки на новую (`old == new`: строка та же). `draft` делает
    /// непустой перевод черновиком. Перенос или смена черновика пишет строку `transfer`.
    /// `source_hash` перевода не меняется: окно покажет, что оригинал изменился.
    fn carry_translation(&self, old: SegmentId, new: i64, draft: bool, now: i64) -> Result<Carried, DbError> {
        let moved = old.get() != new;
        if moved {
            self.conn.execute("UPDATE revisions SET segment_id = ?2 WHERE segment_id = ?1", params![old.get(), new])?;
        }
        let Some((text, draft_before, hash_before)) = self.translation(old)? else {
            return Ok(Carried { moved: false, made_draft: false });
        };
        let made_draft = draft && !draft_before && !text.trim().is_empty();
        if !moved && !made_draft {
            return Ok(Carried { moved: false, made_draft: false });
        }
        self.conn.execute(
            "UPDATE translations SET segment_id = ?2, draft = draft OR ?3 WHERE segment_id = ?1",
            params![old.get(), new, made_draft],
        )?;
        self.conn.execute(
            "INSERT INTO revisions (segment_id, kind, had_row, text_before, draft_before, source_hash_before, text_after, created_at)
             VALUES (?1, 'transfer', 1, ?2, ?3, ?4, ?2, ?5)",
            params![new, text, draft_before, hash_before, now],
        )?;
        Ok(Carried { moved, made_draft })
    }

    /// Описание сцены, исключения и заметки уходят на новый ключ контейнера, если у него ещё нет описания.
    fn move_container_notes(&self, from: &ContainerKey, to: &ContainerKey) -> Result<(), DbError> {
        let described: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM scene_descriptions WHERE container_key = ?1)",
            [to.as_str()],
            |row| row.get(0),
        )?;
        if described {
            return Ok(());
        }
        let keys = params![from.as_str(), to.as_str()];
        self.conn.execute("UPDATE scene_descriptions SET container_key = ?2 WHERE container_key = ?1", keys)?;
        for table in ["character_excluded_containers", "term_excluded_containers"] {
            self.conn.execute(&format!("UPDATE OR IGNORE {table} SET container_key = ?2 WHERE container_key = ?1"), keys)?;
            self.conn.execute(&format!("DELETE FROM {table} WHERE container_key = ?1"), [from.as_str()])?;
        }
        self.conn.execute(
            "UPDATE OR IGNORE notes SET owner_ref = ?2 WHERE owner_kind = 'container' AND owner_ref = ?1",
            keys,
        )?;
        self.conn.execute("DELETE FROM notes WHERE owner_kind = 'container' AND owner_ref = ?1", [from.as_str()])?;
        Ok(())
    }
}

/// Что сделал перенос перевода одной пары.
struct Carried {
    moved: bool,
    made_draft: bool,
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
