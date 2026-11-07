//! Деление сцены на пакеты (`line_batches`, `blank_runs` в 1.0).

use super::Scene;
use crate::llm::BatchSize;
use crate::scan::SegmentId;

/// Пакет запроса. Готовые реплики внутри остаются контекстом, реплики перед пакетом идут блоком `Earlier lines`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// Реплики, которые нужно перевести или вычитать.
    pub lines: Vec<SegmentId>,
    /// Готовые реплики пакета и реплики перед ним.
    pub context: Vec<SegmentId>,
}

pub struct Batcher;

impl Batcher {
    /// Пакеты по одной сцене.
    pub fn split(scene: &Scene, size: BatchSize) -> Vec<Batch> {
        todo!()
    }

    /// Повтор: каждая группа пустых реплик подряд отдельным пакетом. Слабые модели бросают конец пакета
    /// и переводят готовые реплики заново вместо пустых.
    pub fn blank_runs(scene: &Scene, batch: &Batch) -> Vec<Batch> {
        todo!()
    }
}
