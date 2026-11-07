//! Сопоставление реплик после считывания. Код ядра, одинаковый для всех движков.
//!
//! Каждый шаг берёт только то, что не сопоставили предыдущие:
//! 1. тот же ключ, если `source_hash` встречается в контейнере один раз и в старой, и в новой версии;
//! 2. `diff` сцены по `source_hash`, повтор с несовпавшими соседями переносится черновиком;
//! 3. тот же `source_hash`, единственный среди старых и новых реплик без пары во всей игре;
//! 4. похожий текст в замене из `diff`, только черновиком.

use super::SegmentId;
use crate::engines::{ContainerKey, SegmentKey};

/// Порог сходства текста для шага 4.
pub const SIMILARITY_THRESHOLD: f64 = 0.8;

/// Реплика для сопоставления: старая из базы или новая из снимка.
#[derive(Debug, Clone)]
pub struct MatchSegment<Ref> {
    /// Строка базы у старой реплики, номер в снимке у новой.
    pub id: Ref,
    pub key: SegmentKey,
    pub container: Option<ContainerKey>,
    pub source_hash: String,
    pub source_text: String,
    pub story_order: Option<i64>,
}

/// Каким шагом найдена пара.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    SameKey,
    Aligned,
    UniqueHash,
    Similar,
}

/// Пара старой и новой реплики.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedPair {
    pub old: SegmentId,
    /// Номер реплики в новом снимке.
    pub new: usize,
    pub kind: MatchKind,
    /// Перевод переносится черновиком: шаг 4, сомнительный повтор или ключ совпал при другом тексте.
    pub draft: bool,
}

/// Итог сопоставления.
#[derive(Debug, Clone, Default)]
pub struct MatchPlan {
    pub pairs: Vec<MatchedPair>,
    /// Старые реплики без пары: уходят в «Пропавшие» или удаляются, если перевода и истории нет.
    pub missing: Vec<SegmentId>,
    /// Старый ключ контейнера и новый: описание сцены идёт за репликами, если все они попали в один контейнер.
    pub moved_containers: Vec<(ContainerKey, ContainerKey)>,
}

pub struct Matcher;

impl Matcher {
    pub fn plan(old: &[MatchSegment<SegmentId>], new: &[MatchSegment<usize>]) -> MatchPlan {
        todo!()
    }

    /// Сходство двух текстов от 0 до 1.
    pub fn similarity(a: &str, b: &str) -> f64 {
        todo!()
    }
}
