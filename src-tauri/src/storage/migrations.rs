//! Миграции app.db и project.db. История применённых шагов лежит в таблице `schema_migrations`.
//!
//! - Шаг это SQL-файл `schema/<база>/NNNN_<имя>.sql`, список шагов явный. Номера идут подряд с 1.
//! - Применённые шаги совпадают с началом списка. Шаг новее известных: база от более новой версии.
//! - Все недостающие шаги идут одной транзакцией с `foreign_keys = OFF` и `foreign_key_check` перед фиксацией.
//! - Если в базе уже были шаги, перед миграцией делается копия `<файл>.v<номер>.bak`, хранится одна последняя.
//! - После выпуска шаги не меняются: исправление идёт новым шагом.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::DbError;
use crate::common::Timestamp;

/// Шаг миграции.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

impl Migration {
    /// SHA-256 текста шага. Переводы строк приводятся к `\n`: git на Windows отдаёт файл с `\r\n`.
    pub fn checksum(&self) -> String {
        let text = self.sql.replace("\r\n", "\n");
        Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Шаги app.db.
pub const APP: &[Migration] = &[
    Migration { version: 1, name: "init", sql: include_str!("schema/app/0001_init.sql") },
    Migration { version: 2, name: "analysis_action", sql: include_str!("schema/app/0002_analysis_action.sql") },
    Migration { version: 3, name: "reading_depth", sql: include_str!("schema/app/0003_reading_depth.sql") },
];

/// Шаги project.db.
pub const PROJECT: &[Migration] = &[
    Migration { version: 1, name: "init", sql: include_str!("schema/project/0001_init.sql") },
    Migration { version: 2, name: "reading_depth", sql: include_str!("schema/project/0002_reading_depth.sql") },
    Migration { version: 3, name: "scene_analysis", sql: include_str!("schema/project/0003_scene_analysis.sql") },
    Migration { version: 4, name: "app_reading_depth", sql: include_str!("schema/project/0004_app_reading_depth.sql") },
    Migration { version: 5, name: "game_run", sql: include_str!("schema/project/0005_game_run.sql") },
    Migration { version: 6, name: "discarded_term_payload", sql: include_str!("schema/project/0006_discarded_term_payload.sql") },
    Migration { version: 7, name: "character_pair_checks", sql: include_str!("schema/project/0007_character_pair_checks.sql") },
    Migration { version: 8, name: "mention_checks", sql: include_str!("schema/project/0008_mention_checks.sql") },
    Migration { version: 9, name: "entry_pair_checks", sql: include_str!("schema/project/0009_entry_pair_checks.sql") },
    Migration { version: 10, name: "glossary_asks", sql: include_str!("schema/project/0010_glossary_asks.sql") },
    Migration { version: 11, name: "form_kind", sql: include_str!("schema/project/0011_form_kind.sql") },
    Migration { version: 12, name: "description_asks", sql: include_str!("schema/project/0012_description_asks.sql") },
    Migration { version: 13, name: "drop_note_exclusion", sql: include_str!("schema/project/0013_drop_note_exclusion.sql") },
];

/// История шагов. Её создаёт механизм миграций, в самих шагах её нет.
const HISTORY: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations (
  version     INTEGER NOT NULL PRIMARY KEY,
  name        TEXT NOT NULL,
  checksum    TEXT NOT NULL,
  app_version TEXT NOT NULL,
  applied_at  INTEGER NOT NULL
) STRICT;
";

/// Строка истории.
struct Applied {
    version: u32,
    name: String,
    checksum: String,
}

/// Применяет недостающие шаги. Возвращает число применённых.
pub(super) fn migrate(conn: &mut Connection, path: &Path, steps: &[Migration]) -> Result<usize, DbError> {
    conn.execute_batch(HISTORY)?;
    let applied = applied(conn)?;
    check_history(&applied, steps)?;

    let pending = &steps[applied.len()..];
    if pending.is_empty() {
        return Ok(0);
    }
    if let Some(last) = applied.last() {
        backup(conn, path, last.version)?;
    }

    // Внутри транзакции PRAGMA foreign_keys не действует, поэтому её выключают до начала,
    // а после возвращают прежнее значение и при сбое.
    let foreign_keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = apply(conn, pending);
    conn.pragma_update(None, "foreign_keys", foreign_keys)?;
    result.map(|()| pending.len())
}

fn applied(conn: &Connection) -> Result<Vec<Applied>, DbError> {
    let mut statement = conn.prepare("SELECT version, name, checksum FROM schema_migrations ORDER BY version")?;
    let rows = statement.query_map([], |row| {
        Ok(Applied { version: row.get(0)?, name: row.get(1)?, checksum: row.get(2)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// История должна быть началом списка шагов.
fn check_history(applied: &[Applied], steps: &[Migration]) -> Result<(), DbError> {
    let known = steps.len() as u32;
    if let Some(newest) = applied.iter().map(|row| row.version).max().filter(|&newest| newest > known) {
        return Err(DbError::TooNew { found: newest, known });
    }
    for (row, step) in applied.iter().zip(steps) {
        if row.version != step.version || row.name != step.name {
            return Err(DbError::Corrupt(format!(
                "в истории миграций шаг {} «{}» там, где ждали {} «{}»",
                row.version, row.name, step.version, step.name
            )));
        }
        if row.checksum != step.checksum() {
            tracing::warn!("Шаг миграции {} «{}» изменился после применения", step.version, step.name);
        }
    }
    Ok(())
}

/// Все шаги одной транзакцией: сбой откатывает всё, база остаётся на прежнем шаге.
fn apply(conn: &mut Connection, steps: &[Migration]) -> Result<(), DbError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = Timestamp::now().secs();
    for step in steps {
        let failed = |reason: String| DbError::Migration { version: step.version, name: step.name, reason };
        tx.execute_batch(step.sql).map_err(|error| failed(error.to_string()))?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, checksum, app_version, applied_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![step.version, step.name, step.checksum(), env!("CARGO_PKG_VERSION"), now],
        )?;
        if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
            return Err(failed("нарушены внешние ключи".to_string()));
        }
    }
    tx.commit()?;
    Ok(())
}

/// Копия базы перед миграцией. Хранится одна последняя: старые копии удаляются после новой.
fn backup(conn: &Connection, path: &Path, version: u32) -> Result<PathBuf, DbError> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| DbError::Corrupt(format!("у базы нет имени файла: {}", path.display())))?;
    let target = path.with_file_name(format!("{file_name}.v{version}.bak"));
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;

    let prefix = format!("{file_name}.v");
    if let Some(dir) = path.parent() {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".bak") && entry.path() != target {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: &[Migration] = &[
        Migration { version: 1, name: "parents", sql: "CREATE TABLE parents (id INTEGER PRIMARY KEY) STRICT;" },
        Migration {
            version: 2,
            name: "children",
            sql: "CREATE TABLE children (parent_id INTEGER NOT NULL REFERENCES parents (id)) STRICT;",
        },
        Migration { version: 3, name: "seed", sql: "INSERT INTO parents (id) VALUES (1);" },
    ];

    fn open(dir: &Path) -> (Connection, PathBuf) {
        let path = dir.join("test.db");
        (Connection::open(&path).unwrap(), path)
    }

    fn versions(conn: &Connection) -> Vec<u32> {
        applied(conn).unwrap().iter().map(|row| row.version).collect()
    }

    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1").unwrap().exists([name]).unwrap()
    }

    #[test]
    fn fresh_database_gets_all_steps_once() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        assert_eq!(migrate(&mut conn, &path, STEPS).unwrap(), 3);
        assert_eq!(migrate(&mut conn, &path, STEPS).unwrap(), 0);
        assert_eq!(versions(&conn), [1, 2, 3]);
        let (checksum, app_version): (String, String) = conn
            .query_row("SELECT checksum, app_version FROM schema_migrations WHERE version = 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(checksum, STEPS[0].checksum());
        assert_eq!(app_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn only_missing_steps_are_applied() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        migrate(&mut conn, &path, &STEPS[..1]).unwrap();
        assert_eq!(migrate(&mut conn, &path, STEPS).unwrap(), 2);
        assert_eq!(versions(&conn), [1, 2, 3]);
    }

    #[test]
    fn newer_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        migrate(&mut conn, &path, STEPS).unwrap();
        let result = migrate(&mut conn, &path, &STEPS[..2]);
        assert!(matches!(result, Err(DbError::TooNew { found: 3, known: 2 })));
    }

    #[test]
    fn foreign_history_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        migrate(&mut conn, &path, &STEPS[..1]).unwrap();
        let other = [Migration { name: "other", ..STEPS[0] }, STEPS[1]];
        assert!(matches!(migrate(&mut conn, &path, &other), Err(DbError::Corrupt(_))));
    }

    #[test]
    fn failed_step_rolls_back_the_whole_run() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        let broken = [STEPS[0], STEPS[1], Migration { version: 3, name: "broken", sql: "INSERT INTO missing VALUES (1);" }];
        let result = migrate(&mut conn, &path, &broken);
        assert!(matches!(result, Err(DbError::Migration { version: 3, .. })));
        assert!(versions(&conn).is_empty());
        assert!(!has_table(&conn, "parents"));
        let keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0)).unwrap();
        assert!(!keys, "после сбоя внешние ключи возвращаются в прежнее состояние");
    }

    #[test]
    fn foreign_key_violation_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let orphan = [STEPS[0], STEPS[1], Migration { version: 3, name: "orphan", sql: "INSERT INTO children VALUES (42);" }];
        let result = migrate(&mut conn, &path, &orphan);
        assert!(matches!(result, Err(DbError::Migration { version: 3, .. })));
        assert!(versions(&conn).is_empty());
        let keys: bool = conn.pragma_query_value(None, "foreign_keys", |row| row.get(0)).unwrap();
        assert!(keys, "внешние ключи включаются обратно и после сбоя");
    }

    #[test]
    fn backup_keeps_only_the_last_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, path) = open(dir.path());
        migrate(&mut conn, &path, &STEPS[..1]).unwrap();
        assert!(!dir.path().join("test.db.v0.bak").exists(), "у новой базы копии нет");

        migrate(&mut conn, &path, &STEPS[..2]).unwrap();
        assert!(dir.path().join("test.db.v1.bak").exists());

        migrate(&mut conn, &path, STEPS).unwrap();
        assert!(dir.path().join("test.db.v2.bak").exists());
        assert!(!dir.path().join("test.db.v1.bak").exists());

        let copy = Connection::open(dir.path().join("test.db.v2.bak")).unwrap();
        assert_eq!(versions(&copy), [1, 2]);
    }

    #[test]
    fn checksum_ignores_line_endings() {
        let unix = Migration { version: 1, name: "a", sql: "CREATE TABLE a (x INTEGER);\n" };
        let windows = Migration { sql: "CREATE TABLE a (x INTEGER);\r\n", ..unix };
        assert_eq!(unix.checksum(), windows.checksum());
        assert_eq!(unix.checksum().len(), 64);
    }

    #[test]
    fn step_lists_are_numbered_in_order() {
        for steps in [APP, PROJECT] {
            for (index, step) in steps.iter().enumerate() {
                assert_eq!(step.version as usize, index + 1, "шаг «{}»", step.name);
            }
        }
    }

    #[test]
    fn real_schemas_apply_with_foreign_keys_on() {
        let dir = tempfile::tempdir().unwrap();
        for (steps, file) in [(APP, "app.db"), (PROJECT, "project.db")] {
            let path = dir.path().join(file);
            let mut conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            assert_eq!(migrate(&mut conn, &path, steps).unwrap(), steps.len());
        }
    }
}
