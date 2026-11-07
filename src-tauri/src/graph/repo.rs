//! Сцены графа: `containers` текущего снимка с названием, прогрессом и первой репликой.

use rusqlite::Connection;

use super::GraphScene;
use crate::engines::ContainerKey;
use crate::scan::FileId;
use crate::storage::DbError;

pub struct GraphRepo<'c> {
    pub conn: &'c Connection,
}

impl GraphRepo<'_> {
    /// Сцены текущего снимка в порядке сюжета. Сцены без места в сюжете идут в конце по ключу.
    /// Прогресс считается так же, как у файлов таблицы перевода.
    pub fn scenes(&self) -> Result<Vec<GraphScene>, DbError> {
        let mut statement = self.conn.prepare(
            "SELECT c.key, d.title, min(s.file_id), count(s.id),
                    coalesce(sum(trim(coalesce(t.text, s.game_text, '')) <> ''), 0),
                    coalesce(sum(coalesce(t.draft, 0) = 1 AND trim(t.text) <> ''), 0),
                    (SELECT f.source_text FROM segments f
                     WHERE f.container_id = c.id AND f.missing = 0
                     ORDER BY f.story_order, f.id LIMIT 1)
             FROM containers c
             LEFT JOIN scene_descriptions d ON d.container_key = c.key
             LEFT JOIN segments s ON s.container_id = c.id AND s.missing = 0
             LEFT JOIN translations t ON t.segment_id = s.id
             WHERE c.kind = 'scene' AND c.last_seen_scan = (SELECT max(id) FROM scans)
             GROUP BY c.id
             ORDER BY c.story_order IS NULL, c.story_order, c.key",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(key, title, file, total, translated, drafts, first_line)| {
                Ok(GraphScene {
                    key: ContainerKey::new(key).map_err(|error| DbError::bad_value("containers", "key", error))?,
                    title,
                    file: file.map(FileId::from_row),
                    total: total as usize,
                    translated: translated as usize,
                    drafts: drafts as usize,
                    first_line,
                })
            })
            .collect()
    }
}
