//! Разбор и проверка ответа модели.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use super::text::{letters, usable_translation};
use super::{Batch, Draft, LinesMode, Scene, SceneLine};
use crate::scan::SegmentId;

/// С какой длины одинаковые переводы разных реплик считаются сдвигом: короткие вроде «Да.» совпадают законно.
const MIN_SHIFT_LETTERS: usize = 20;

/// Номер и перевод реплики в сломанном JSON, даже без кавычки у поля.
static LINE_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""id"\s*:\s*"?(\d+)"?\s*,\s*(?:"\w+"\s*:\s*"[^"]*"\s*,\s*)*"?(?:text|translation)"?\s*:\s*"((?:[^"\\]|\\.)*)""#)
        .expect("регулярное выражение реплики")
});

/// Ответ модели: номера реплик с текстом.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinesAnswer {
    /// Номер реплики в пакете (с единицы) и текст.
    pub lines: Vec<(usize, String)>,
}

/// Элемент ответа: перевод реплики. Элемент без номера (например, термин по старой привычке модели) пропускается.
#[derive(Debug, Default, Deserialize)]
struct Item {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    translation: Option<String>,
}

impl Item {
    /// Модели возвращают номер и строкой, и числом.
    fn number(&self) -> Option<usize> {
        match self.id.as_ref()? {
            serde_json::Value::String(id) => id.trim().parse().ok(),
            serde_json::Value::Number(id) => id.as_u64().and_then(|id| usize::try_from(id).ok()),
            _ => None,
        }
    }

    /// Перевод реплики. Локальные модели пишут его и в поле `translation`, как у реплик контекста.
    fn line_text(&self) -> Option<&str> {
        self.text.as_deref().or(self.translation.as_deref())
    }
}

/// Первый массив объектов в тексте: модели окружают JSON пояснениями и блоком кода.
pub(crate) fn json_array(raw: &str) -> &str {
    let trimmed = raw.trim();
    for (start, _) in trimmed.match_indices('[') {
        let mut values = serde_json::Deserializer::from_str(&trimmed[start..]).into_iter::<serde_json::Value>();
        if let Some(Ok(serde_json::Value::Array(items))) = values.next()
            && items.iter().all(serde_json::Value::is_object)
        {
            return &trimmed[start..start + values.byte_offset()];
        }
    }
    trimmed
}

impl LinesAnswer {
    /// JSON ответа, а сломанный (комментарии, пояснения вне строк, пропущенные кавычки) запасным разбором
    /// пар номера и перевода (`line_answer`). Перевод принимается и из поля `translation`.
    /// `None`: в ответе нет ни одной реплики.
    pub fn parse(text: &str) -> Option<Self> {
        if let Ok(items) = serde_json::from_str::<Vec<Item>>(json_array(text)) {
            let mut answer = Self::default();
            for item in items {
                if let (Some(number), Some(line)) = (item.number(), item.line_text()) {
                    answer.lines.push((number, line.trim().to_string()));
                }
            }
            return Some(answer);
        }
        let lines: Vec<(usize, String)> = LINE_ITEM
            .captures_iter(text)
            .filter_map(|caps| {
                let number = caps[1].parse().ok()?;
                let line: String = serde_json::from_str(&format!("\"{}\"", &caps[2])).ok()?;
                Some((number, line.trim().to_string()))
            })
            .collect();
        (!lines.is_empty()).then_some(Self { lines })
    }

    /// Отбрасывает перевод без токенов и тегов оригинала (`usable_translation`), правку редактуры без изменений
    /// (`review_fixes`) и длинный перевод, совпавший с переводом реплики с другим оригиналом (`accepted_lines`):
    /// модель пропустила реплику и подписала её номером перевод соседней.
    pub fn accept(&self, mode: LinesMode, scene: &Scene, batch: &Batch) -> Vec<Draft> {
        let by_id: HashMap<SegmentId, &SceneLine> = scene.lines.iter().map(|line| (line.id, line)).collect();
        let mut seen = HashSet::new();
        let candidates: Vec<(&SceneLine, &str)> = self
            .lines
            .iter()
            .filter_map(|(number, text)| {
                let line = *by_id.get(batch.lines.get(number.checked_sub(1)?)?)?;
                let fits = mode.wants(line)
                    && usable_translation(&line.source_text, text)
                    && (mode == LinesMode::Translate || line.text.trim() != text.trim())
                    && seen.insert(line.id);
                fits.then_some((line, text.as_str()))
            })
            .collect();

        let shifted = match mode {
            LinesMode::Translate => shifted_lines(&candidates, scene),
            LinesMode::Review => HashSet::new(),
        };
        candidates
            .into_iter()
            .filter(|(line, _)| !shifted.contains(&line.id))
            .map(|(line, text)| Draft { line: line.id, text: text.to_string(), sent: line.sent() })
            .collect()
    }

}

/// Реплики с одинаковым длинным переводом при разных оригиналах: и новые, и уже переведённые в сцене.
fn shifted_lines(candidates: &[(&SceneLine, &str)], scene: &Scene) -> HashSet<SegmentId> {
    let fresh: HashSet<SegmentId> = candidates.iter().map(|(line, _)| line.id).collect();
    let translations: Vec<(&SceneLine, String)> = candidates
        .iter()
        .map(|(line, text)| (*line, letters(text)))
        .chain(scene.lines.iter().filter(|line| !line.is_blank() && !fresh.contains(&line.id)).map(|line| (line, letters(&line.text))))
        .filter(|(_, text)| text.chars().count() >= MIN_SHIFT_LETTERS)
        .collect();
    let mut shifted = HashSet::new();
    for (index, (line, text)) in translations.iter().enumerate() {
        for (other, same) in &translations[index + 1..] {
            if same == text && letters(&other.source_text) != letters(&line.source_text) {
                shifted.extend([line.id, other.id].into_iter().filter(|id| fresh.contains(id)));
            }
        }
    }
    if !shifted.is_empty() {
        tracing::warn!("Модель повторила перевод на разных репликах, они переводятся заново: {shifted:?}");
    }
    shifted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::batcher::tests::{line, scene};

    fn batch(lines: &[i64]) -> Batch {
        Batch { lines: lines.iter().map(|id| SegmentId::from_row(*id)).collect(), context: Vec::new() }
    }

    #[test]
    fn json_inside_text_is_read_and_items_without_id_skipped() {
        let raw = "Вот перевод:\n```json\n[{\"id\": \"1\", \"text\": \"Привет.\"}, {\"id\": 2, \"translation\": \"Пока.\"},\
                   {\"term\": \"Silver Key\", \"translation\": \"Серебряный ключ\", \"category\": \"item\"}]\n```";
        let answer = LinesAnswer::parse(raw).unwrap();
        assert_eq!(answer.lines, [(1, "Привет.".to_string()), (2, "Пока.".to_string())]);
    }

    #[test]
    fn broken_json_still_gives_lines() {
        let raw = "[{\"id\": \"1\", \"speaker\": \"Max\", text: \"Привет, \\\"друг\\\".\"} // комментарий\n{\"id\": 2, \"text\": \"Пока.\"}]";
        let answer = LinesAnswer::parse(raw).unwrap();
        assert_eq!(answer.lines, [(1, "Привет, \"друг\".".to_string()), (2, "Пока.".to_string())]);
        assert_eq!(LinesAnswer::parse("Не могу помочь."), None);
    }

    #[test]
    fn accept_checks_markers_numbers_and_shifts() {
        let long = "This is a fairly long sentence number";
        let scene = scene(vec![
            line(1, "Hi, [mc].", ""),
            line(2, "Bye.", ""),
            line(3, &format!("{long} one."), ""),
            line(4, &format!("{long} two."), ""),
            line(5, "Ready.", "Готово."),
        ]);
        let shifted = "Это довольно длинное предложение номер один.";
        let answer = LinesAnswer {
            lines: vec![
                (1, "Привет.".to_string()),
                (2, "Пока.".to_string()),
                (2, "Ещё раз пока.".to_string()),
                (3, shifted.to_string()),
                (4, shifted.to_string()),
                (9, "Лишний номер.".to_string()),
            ],
        };
        let drafts = answer.accept(LinesMode::Translate, &scene, &batch(&[1, 2, 3, 4]));
        assert_eq!(drafts.len(), 1, "токен потерян, сдвиг и лишний номер не проходят");
        assert_eq!((drafts[0].line.get(), drafts[0].text.as_str()), (2, "Пока."));
        assert_eq!(drafts[0].sent.source_hash, "hash2");
        assert_eq!(drafts[0].sent.text, "");
    }

    #[test]
    fn review_keeps_only_real_changes() {
        let scene = scene(vec![line(1, "Sleep well, honey.", "Спи хорошо, медок."), line(2, "Yes.", "Да.")]);
        let answer = LinesAnswer { lines: vec![(1, "Спокойной ночи, милый.".to_string()), (2, "Да.".to_string())] };
        let drafts = answer.accept(LinesMode::Review, &scene, &batch(&[1, 2]));
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].sent.text, "Спи хорошо, медок.");
    }
}
