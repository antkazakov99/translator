//! `translations` и `revisions` в project.db. Каждая запись пишет историю и поднимает `project.search_rev`.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Draft, FileProgress, LineState, RevisionKind, Scene, SceneLine};
use crate::common::Timestamp;
use crate::engines::{ContainerKey, SpeakerKey};
use crate::llm::RunId;
use crate::scan::{FileId, SegmentId};
use crate::storage::DbError;

/// Реплика текущего снимка со сценой, подписью говорящего и действующим текстом.
/// Подпись пока из снимка: имена из глоссария подставятся вместе с ним.
const LINE_SELECT: &str = "
SELECT s.id, c.key, d.title, d.description,
       CASE WHEN sp.label IS NOT NULL THEN sp.label
            WHEN s.speaker_key IS NOT NULL THEN substr(s.speaker_key, instr(s.speaker_key, ':') + 1) END,
       s.source_text, s.source_hash, s.game_text, s.location,
       t.text, t.draft, t.source_hash, s.speaker_key
FROM segments s
LEFT JOIN containers c ON c.id = s.container_id
LEFT JOIN scene_descriptions d ON d.container_key = c.key
LEFT JOIN speakers sp ON sp.key = s.speaker_key
LEFT JOIN translations t ON t.segment_id = s.id
WHERE s.missing = 0";

/// Строка `LINE_SELECT`: сцена (ключ, название, описание) и реплика.
struct LineRow {
    scene: Option<String>,
    title: Option<String>,
    description: Option<String>,
    line: SceneLine,
}

fn read_line(row: &Row) -> rusqlite::Result<LineRow> {
    let source_hash: String = row.get(6)?;
    let game_text: Option<String> = row.get(7)?;
    let text: Option<String> = row.get(9)?;
    let draft: Option<bool> = row.get(10)?;
    let sent_hash: Option<String> = row.get(11)?;
    let speaker_key: Option<String> = row.get(12)?;
    let state = LineState::of(text.as_deref().map(|text| (text, draft.unwrap_or(false))), game_text.as_deref());
    let source_changed = sent_hash.as_deref().is_some_and(|hash| hash != source_hash);
    Ok(LineRow {
        scene: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        line: SceneLine {
            id: SegmentId::from_row(row.get(0)?),
            speaker: row.get(4)?,
            speaker_gender: None,
            speaker_key: speaker_key.map(SpeakerKey::new).transpose().ok().flatten(),
            source_text: row.get(5)?,
            source_hash,
            text: text.or(game_text).unwrap_or_default(),
            state,
            location: row.get(8)?,
            source_changed,
        },
    })
}

/// Подряд идущие реплики одной сцены собираются в сцену. Реплики вне сцен идут своей группой.
fn group_scenes(rows: Vec<LineRow>) -> Result<Vec<Scene>, DbError> {
    let mut scenes: Vec<Scene> = Vec::new();
    let mut current: Option<Option<String>> = None;
    for row in rows {
        if current.as_ref() != Some(&row.scene) {
            let key = row
                .scene
                .clone()
                .map(ContainerKey::new)
                .transpose()
                .map_err(|error| DbError::bad_value("containers", "key", error))?;
            scenes.push(Scene { key, title: row.title, description: row.description, lines: Vec::new() });
            current = Some(row.scene);
        }
        if let Some(scene) = scenes.last_mut() {
            scene.lines.push(row.line);
        }
    }
    Ok(scenes)
}

pub struct TranslationRepo<'c> {
    pub conn: &'c Connection,
}

impl TranslationRepo<'_> {
    /// Файлы текущего снимка с репликами и прогресс по ним.
    pub fn files(&self) -> Result<Vec<FileProgress>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT f.id, f.path, count(*),
                    sum(trim(coalesce(t.text, s.game_text, '')) <> ''),
                    sum(coalesce(t.draft, 0) = 1 AND trim(t.text) <> '')
             FROM segments s
             JOIN files f ON f.id = s.file_id
             LEFT JOIN translations t ON t.segment_id = s.id
             WHERE s.missing = 0
             GROUP BY f.id
             ORDER BY f.path",
        )?;
        let files = statement
            .query_map([], |row| {
                Ok(FileProgress {
                    id: FileId::from_row(row.get(0)?),
                    path: row.get(1)?,
                    total: row.get::<_, i64>(2)? as usize,
                    translated: row.get::<_, i64>(3)? as usize,
                    drafts: row.get::<_, i64>(4)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(files)
    }

    /// Реплики сцены текущего снимка в порядке сюжета с подписями из глоссария.
    pub fn scene(&self, key: &ContainerKey) -> Result<Scene, DbError> {
        let sql = format!("{LINE_SELECT} AND c.key = ?1 ORDER BY s.story_order, s.id");
        let rows = self.conn.prepare(&sql)?.query_map([key.as_str()], read_line)?.collect::<Result<Vec<_>, _>>()?;
        group_scenes(rows)?.into_iter().next().ok_or(DbError::NotFound)
    }

    /// Реплики файла в порядке файла, разбитые на сцены.
    pub fn file_scenes(&self, file: FileId) -> Result<Vec<Scene>, DbError> {
        let sql = format!("{LINE_SELECT} AND s.file_id = ?1 ORDER BY s.id");
        let rows = self.conn.prepare(&sql)?.query_map([file.get()], read_line)?.collect::<Result<Vec<_>, _>>()?;
        group_scenes(rows)
    }

    /// Одна реплика: окно перерисовывает строку после правки.
    pub fn line(&self, line: SegmentId) -> Result<SceneLine, DbError> {
        let sql = format!("{LINE_SELECT} AND s.id = ?1");
        let row = self.conn.query_row(&sql, [line.get()], read_line).optional()?;
        Ok(row.ok_or(DbError::NotFound)?.line)
    }

    /// Сцена одной реплики: её соседи нужны как контекст.
    /// Реплика вне сцен получает соседей по файлу.
    pub fn scene_of(&self, line: SegmentId) -> Result<Scene, DbError> {
        let (key, file): (Option<String>, i64) = self
            .conn
            .query_row(
                "SELECT c.key, s.file_id FROM segments s LEFT JOIN containers c ON c.id = s.container_id
                 WHERE s.id = ?1 AND s.missing = 0",
                [line.get()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(DbError::NotFound)?;
        if let Some(key) = key {
            let key = ContainerKey::new(key).map_err(|error| DbError::bad_value("containers", "key", error))?;
            return self.scene(&key);
        }
        self.file_scenes(FileId::from_row(file))?
            .into_iter()
            .find(|scene| scene.key.is_none() && scene.lines.iter().any(|item| item.id == line))
            .ok_or(DbError::NotFound)
    }

    /// Черновики перевода только в пустые реплики с тем же `source_hash`, что ушёл модели. Возвращает число записанных.
    pub fn save_drafts(&self, drafts: &[Draft], run: RunId, now: Timestamp) -> Result<usize, DbError> {
        self.write_drafts(drafts, run, now, true)
    }

    /// Исправления редактуры: заменяют и заполненные реплики, если текст и `source_hash` те же, что ушли модели.
    pub fn replace_with_drafts(&self, drafts: &[Draft], run: RunId, now: Timestamp) -> Result<usize, DbError> {
        self.write_drafts(drafts, run, now, false)
    }

    /// Пишет черновик, если реплика осталась такой, какой ушла модели: пользователь мог поправить её
    /// или считывание могло сменить оригинал, пока модель отвечала.
    fn write_drafts(&self, drafts: &[Draft], run: RunId, now: Timestamp, blank_only: bool) -> Result<usize, DbError> {
        let mut written = 0;
        for draft in drafts {
            let Some((source_hash, game_text)) = self
                .conn
                .query_row("SELECT source_hash, game_text FROM segments WHERE id = ?1 AND missing = 0", [draft.line.get()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .optional()?
            else {
                continue;
            };
            let before = self.translation(draft.line)?;
            let current = match &before {
                Some(before) => before.text.as_str(),
                None => game_text.as_deref().unwrap_or_default(),
            };
            let unchanged = source_hash == draft.sent.source_hash && current == draft.sent.text;
            if !unchanged || (blank_only && !current.trim().is_empty()) || draft.text.trim().is_empty() {
                continue;
            }
            self.conn.execute(
                "INSERT INTO translations (segment_id, text, draft, run_id, source_hash, updated_at)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5)
                 ON CONFLICT (segment_id) DO UPDATE SET
                   text = excluded.text, draft = 1, run_id = excluded.run_id,
                   source_hash = excluded.source_hash, updated_at = excluded.updated_at",
                params![draft.line.get(), draft.text, run.get(), source_hash, now.secs()],
            )?;
            self.revision(draft.line, RevisionKind::Llm, before.as_ref(), Some(&draft.text), Some(run), now)?;
            written += 1;
        }
        if written > 0 {
            self.bump_search()?;
        }
        Ok(written)
    }

    /// Правка пользователя: снимает `draft`. Пустой текст значит «очищено», запуск переведёт заново.
    /// Текст, совпавший с действующим, ничего не пишет. Возвращает, была ли запись.
    pub fn save_user_text(&self, line: SegmentId, text: &str, now: Timestamp) -> Result<bool, DbError> {
        let text = if text.trim().is_empty() { "" } else { text };
        let (source_hash, game_text): (String, Option<String>) = self
            .conn
            .query_row("SELECT source_hash, game_text FROM segments WHERE id = ?1 AND missing = 0", [line.get()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?
            .ok_or(DbError::NotFound)?;
        let before = self.translation(line)?;
        let unchanged = match &before {
            Some(before) => before.text == text && !before.draft,
            None => game_text.as_deref().unwrap_or_default() == text,
        };
        if unchanged {
            return Ok(false);
        }

        self.conn.execute(
            "INSERT INTO translations (segment_id, text, draft, run_id, source_hash, updated_at)
             VALUES (?1, ?2, 0, NULL, ?3, ?4)
             ON CONFLICT (segment_id) DO UPDATE SET
               text = excluded.text, draft = 0, run_id = NULL,
               source_hash = excluded.source_hash, updated_at = excluded.updated_at",
            params![line.get(), text, source_hash, now.secs()],
        )?;
        self.revision(line, RevisionKind::User, before.as_ref(), Some(text), None, now)?;
        self.bump_search()?;
        Ok(true)
    }

    /// Подтверждение черновика: перевод модели становится переводом пользователя. Возвращает число подтверждённых.
    pub fn confirm(&self, lines: &[SegmentId], now: Timestamp) -> Result<usize, DbError> {
        let mut confirmed = 0;
        for &line in lines {
            let Some(before) = self.translation(line)?.filter(|before| before.draft && !before.text.trim().is_empty())
            else {
                continue;
            };
            self.conn.execute(
                "UPDATE translations SET draft = 0, updated_at = ?2 WHERE segment_id = ?1",
                params![line.get(), now.secs()],
            )?;
            self.revision(line, RevisionKind::User, Some(&before), Some(&before.text), None, now)?;
            confirmed += 1;
        }
        Ok(confirmed)
    }

    /// Откат запуска: только реплики, где последняя правка без переносов принадлежит запуску.
    /// Возвращает число откаченных и число оставленных.
    pub fn rollback_run(&self, run: RunId, now: Timestamp) -> Result<(usize, usize), DbError> {
        todo!()
    }

    fn translation(&self, line: SegmentId) -> Result<Option<Translation>, DbError> {
        Ok(self
            .conn
            .query_row(
                "SELECT text, draft, source_hash FROM translations WHERE segment_id = ?1",
                [line.get()],
                |row| Ok(Translation { text: row.get(0)?, draft: row.get(1)?, source_hash: row.get(2)? }),
            )
            .optional()?)
    }

    /// Строка истории с полным состоянием до правки: была ли строка, текст, `draft` и `source_hash`.
    fn revision(
        &self,
        line: SegmentId,
        kind: RevisionKind,
        before: Option<&Translation>,
        text_after: Option<&str>,
        run: Option<RunId>,
        now: Timestamp,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO revisions (segment_id, kind, had_row, text_before, draft_before, source_hash_before,
               text_after, run_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                line.get(),
                kind.as_str(),
                before.is_some(),
                before.map(|b| b.text.as_str()),
                before.map(|b| b.draft),
                before.map(|b| b.source_hash.as_str()),
                text_after,
                run.map(RunId::get),
                now.secs()
            ],
        )?;
        Ok(())
    }

    fn bump_search(&self) -> Result<(), DbError> {
        self.conn.execute("UPDATE project SET search_rev = search_rev + 1", [])?;
        Ok(())
    }
}

/// Строка `translations`.
struct Translation {
    text: String,
    draft: bool,
    source_hash: String,
}
