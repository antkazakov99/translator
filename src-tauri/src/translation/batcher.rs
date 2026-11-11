//! Деление сцены на пакеты (`line_batches`, `blank_runs` в 1.0).

use std::ops::Range;

use super::{LinesMode, Scene};
use crate::llm::BatchSize;
use crate::scan::SegmentId;

/// Предел текста пакета в символах: русский ответ длиннее оригинала и должен уместиться целиком.
pub const MAX_BATCH_TEXT: usize = 3000;
/// Сколько реплик перед пакетом модель видит ради связности текста.
pub const LINE_CONTEXT: usize = 8;

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
    /// Пакеты по одной сцене. Реплики, которые не нужны режиму, между нужными остаются в пакете контекстом
    /// и считаются в его тексте, а одна длинная реплика может занять пакет целиком.
    pub fn split(scene: &Scene, mode: LinesMode, size: BatchSize) -> Vec<Batch> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut open: Option<Range<usize>> = None;
        let (mut count, mut text) = (0, 0);
        for (index, line) in scene.lines.iter().enumerate() {
            let chars = line.source_text.chars().count();
            if !mode.wants(line) {
                text += chars;
                continue;
            }
            if open.is_some() && (count == size.get() || text + chars > MAX_BATCH_TEXT) {
                ranges.extend(open.take());
            }
            let start = match &open {
                Some(range) => range.start,
                None => {
                    (count, text) = (0, 0);
                    index
                }
            };
            open = Some(start..index + 1);
            count += 1;
            text += chars;
        }
        ranges.extend(open);
        ranges.into_iter().map(|range| Self::batch(scene, mode, range)).collect()
    }

    /// Повтор: каждая группа пустых реплик подряд отдельным пакетом. Слабые модели бросают конец пакета
    /// и переводят готовые реплики заново вместо пустых.
    pub fn blank_runs(scene: &Scene, batch: &Batch) -> Vec<Batch> {
        let mut runs: Vec<Range<usize>> = Vec::new();
        let positions = scene
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| batch.lines.contains(&line.id) && LinesMode::Translate.wants(line))
            .map(|(index, _)| index);
        for index in positions {
            match runs.last_mut() {
                Some(run) if run.end == index => run.end += 1,
                _ => runs.push(index..index + 1),
            }
        }
        runs.into_iter().map(|range| Self::batch(scene, LinesMode::Translate, range)).collect()
    }

    fn batch(scene: &Scene, mode: LinesMode, range: Range<usize>) -> Batch {
        let shown = range.start.saturating_sub(LINE_CONTEXT)..range.end;
        let (lines, context) = scene.lines[shown.clone()]
            .iter()
            .zip(shown)
            .partition::<Vec<_>, _>(|(line, index)| range.contains(index) && mode.wants(line));
        Batch {
            lines: lines.into_iter().map(|(line, _)| line.id).collect(),
            context: context.into_iter().map(|(line, _)| line.id).collect(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::translation::{LineState, SceneLine};

    pub fn line(id: i64, source: &str, text: &str) -> SceneLine {
        SceneLine {
            id: SegmentId::from_row(id),
            speaker: None,
            speaker_gender: None,
            speaker_key: None,
            source_text: source.to_string(),
            source_hash: format!("hash{id}"),
            text: text.to_string(),
            state: if text.is_empty() { LineState::Empty } else { LineState::Done },
            location: None,
            source_changed: false,
            reviewed: false,
        }
    }

    pub fn scene(lines: Vec<SceneLine>) -> Scene {
        Scene { key: None, title: None, description: None, lines }
    }

    fn ids(ids: &[SegmentId]) -> Vec<i64> {
        ids.iter().map(|id| id.get()).collect()
    }

    #[test]
    fn ready_lines_between_blanks_stay_as_context() {
        let scene = scene(vec![
            line(1, "One.", "Один."),
            line(2, "Two.", ""),
            line(3, "Three.", "Три."),
            line(4, "Four.", ""),
            line(5, "Five.", ""),
            line(6, "...", ""),
        ]);
        let batches = Batcher::split(&scene, LinesMode::Translate, BatchSize::new(2).unwrap());
        assert_eq!(batches.len(), 2);
        assert_eq!(ids(&batches[0].lines), [2, 4]);
        assert_eq!(ids(&batches[0].context), [1, 3]);
        assert_eq!(ids(&batches[1].lines), [5]);
        assert_eq!(ids(&batches[1].context), [1, 2, 3, 4], "реплики перед пакетом идут контекстом");

        let review = Batcher::split(&scene, LinesMode::Review, BatchSize::new(30).unwrap());
        assert_eq!(review.len(), 1);
        assert_eq!(ids(&review[0].lines), [1, 3]);
    }

    #[test]
    fn long_text_starts_a_new_batch_and_retry_splits_blank_runs() {
        let long = "a".repeat(MAX_BATCH_TEXT - 10);
        let mut scene = scene(vec![line(1, &long, ""), line(2, "Hello there.", ""), line(3, "Ready.", "Готово."), line(4, "Bye.", "")]);
        let batches = Batcher::split(&scene, LinesMode::Translate, BatchSize::new(30).unwrap());
        assert_eq!(batches.iter().map(|batch| ids(&batch.lines)).collect::<Vec<_>>(), [vec![1], vec![2, 4]]);

        scene.lines[1].text = "Привет.".to_string();
        let all = Batch { lines: scene.lines.iter().map(|line| line.id).collect(), context: Vec::new() };
        let runs = Batcher::blank_runs(&scene, &all);
        assert_eq!(runs.iter().map(|batch| ids(&batch.lines)).collect::<Vec<_>>(), [vec![1], vec![4]]);
        assert_eq!(ids(&runs[1].context), [1, 2, 3]);
    }
}
