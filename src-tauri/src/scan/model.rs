//! Считывания и строки снимка.

use crate::common::{Timestamp, row_id};
use crate::engines::{ContainerKey, GameInfo, SegmentKey};
use crate::projects::GameRoot;

row_id!(
    /// Строка `scans`.
    ScanId
);
row_id!(
    /// Строка `files`: версия файла, а не путь.
    FileId
);
row_id!(
    /// Строка `segments`: реплика конкретной версии игры, а не ключ.
    SegmentId
);

/// Одно считывание. Действующий `GameInfo` у последнего.
#[derive(Debug, Clone)]
pub struct Scan {
    pub id: ScanId,
    pub read_at: Timestamp,
    /// Откуда считали. `None`: пересборка из `files/`.
    pub game_root: Option<GameRoot>,
    pub parser_version: u32,
    pub game_info: GameInfo,
}

/// Реплика, пропавшая из игры, с переводом или историей. Её перевод можно привязать к новой реплике руками.
#[derive(Debug, Clone)]
pub struct MissingSegment {
    pub id: SegmentId,
    pub key: SegmentKey,
    pub container: Option<ContainerKey>,
    pub source_text: String,
    pub translation: Option<String>,
    /// Путь файла той версии, из которой реплика взята.
    pub file: String,
    pub location: Option<String>,
}

/// Итог считывания для окна.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub scan: ScanId,
    /// Реплик в новом снимке.
    pub lines: usize,
    /// Новых реплик без пары в прошлом снимке.
    pub new_lines: usize,
    /// Переводов, перенесённых на другую строку.
    pub transferred: usize,
    /// Переводов, которые считывание сделало черновиками: текст изменился или пара сомнительная.
    pub drafts: usize,
    /// Реплик в «Пропавших» после считывания.
    pub missing: usize,
}
