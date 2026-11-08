//! Части текста для описаний моделью: сцена режется по репликам на куски, которые помещаются в запрос.

use sha2::{Digest, Sha256};

/// Сколько символов текста уходит модели в одном запросе части.
pub const MAX_PART_TEXT: usize = 12_000;
/// Меньше этого в конце длинной сцены модели не хватает контекста: хвост начинается с конца предыдущей части.
const MIN_PART_TAIL: usize = 3_000;
/// Сколько символов начала части показывает окно, чтобы её можно было узнать.
const PART_START: usize = 140;

/// Часть сцены: текст для модели и его отпечаток, по которому лежит заметка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub hash: String,
}

impl Part {
    fn new(text: String) -> Self {
        let hash = hash_of(text.as_bytes());
        Self { text, hash }
    }

    /// Первая строка части, укороченная для окна.
    pub fn start(&self) -> String {
        let line = self.text.lines().next().unwrap_or_default();
        let mut start: String = line.chars().take(PART_START).collect();
        if line.chars().count() > PART_START {
            start.push('…');
        }
        start
    }
}

/// Строка реплики для модели: «Говорящий: текст», текст в одну строку.
pub fn script_line(speaker: Option<&str>, text: &str) -> String {
    let text = text.replace('\n', " ");
    match speaker.map(str::trim).filter(|speaker| !speaker.is_empty()) {
        Some(speaker) => format!("{speaker}: {text}"),
        None => text,
    }
}

/// Режет реплики на части не длиннее `MAX_PART_TEXT`, только между репликами. Реплика длиннее предела
/// идёт отдельной частью целиком. Короткий последний кусок начинается с конца предыдущего.
pub fn split_lines(lines: &[String]) -> Vec<Part> {
    let length = |line: &String| line.chars().count() + 1;
    let mut pieces: Vec<Vec<&String>> = vec![Vec::new()];
    let mut size = 0;
    for line in lines {
        if size > 0 && size + length(line) > MAX_PART_TEXT {
            pieces.push(Vec::new());
            size = 0;
        }
        if let Some(piece) = pieces.last_mut() {
            piece.push(line);
        }
        size += length(line);
    }
    if let [.., previous, tail] = pieces.as_mut_slice() {
        let mut start = previous.len();
        while start > 0 && size < MIN_PART_TAIL {
            start -= 1;
            size += length(previous[start]);
        }
        tail.splice(0..0, previous[start..].iter().copied());
    }
    pieces
        .into_iter()
        .filter(|piece| !piece.is_empty())
        .map(|piece| Part::new(piece.iter().map(|line| format!("{line}\n")).collect()))
        .collect()
}

/// Отпечаток текста сцены по отпечаткам оригиналов её реплик: другой текст игры даёт другой отпечаток.
pub fn scene_hash<'a>(source_hashes: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    for hash in source_hashes {
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    hex(hasher.finalize().as_slice())
}

fn hash_of(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).as_slice())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Номера частей для чтения: `count` штук, но не меньше прочитанных, чтобы готовое не пропадало.
/// Остальные берутся равномерно по сцене, и меньший набор всегда входит в больший: при другой глубине
/// готовые заметки подходят снова.
pub fn pick_parts(ready: &[bool], count: usize) -> Vec<usize> {
    let ready_count = ready.iter().filter(|ready| **ready).count();
    let mut picked: Vec<usize> = (0..ready.len()).collect();
    picked.sort_by_key(|index| (!ready[*index], index.reverse_bits()));
    picked.truncate(count.max(ready_count).max(1));
    picked.sort_unstable();
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(count: usize, width: usize) -> Vec<String> {
        (0..count).map(|index| format!("{index:04}{}", "x".repeat(width - 4))).collect()
    }

    #[test]
    fn short_scene_is_one_part() {
        let parts = split_lines(&["Anna: Hi.".to_string(), "Bob: Hello.".to_string()]);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "Anna: Hi.\nBob: Hello.\n");
        assert_eq!(parts[0].hash.len(), 64);
        assert!(split_lines(&[]).is_empty());
    }

    #[test]
    fn long_scene_is_cut_between_lines_and_short_tail_takes_context() {
        // 25 реплик по 1000 символов с переводом строки: 12 + 12 + 1, хвост добирает две реплики второй части.
        let parts = split_lines(&lines(25, 999));
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|part| part.text.chars().count() <= MAX_PART_TEXT));
        assert!(parts[0].text.starts_with("0000"));
        assert!(parts[1].text.starts_with("0012"));
        assert!(parts[2].text.starts_with("0022"), "хвост начинается с конца второй части: {}", &parts[2].text[..4]);
        assert!(parts[2].text.contains("0024"));
    }

    #[test]
    fn script_line_keeps_speaker_and_one_line() {
        assert_eq!(script_line(Some("Anna"), "Hi\nthere"), "Anna: Hi there");
        assert_eq!(script_line(Some(" "), "Rain."), "Rain.");
        assert_eq!(script_line(None, "Rain."), "Rain.");
    }

    #[test]
    fn scene_hash_follows_line_hashes() {
        assert_eq!(scene_hash(["a", "b"]), scene_hash(["a", "b"]));
        assert_ne!(scene_hash(["a", "b"]), scene_hash(["a", "c"]));
        assert_ne!(scene_hash(["ab"]), scene_hash(["a", "b"]));
    }

    #[test]
    fn picked_parts_keep_ready_ones_and_grow_as_supersets() {
        let none = [false; 8];
        let three = pick_parts(&none, 3);
        let five = pick_parts(&none, 5);
        assert_eq!(three.len(), 3);
        assert!(three.iter().all(|index| five.contains(index)));
        assert_eq!(pick_parts(&[false, true, true, false], 1), [1, 2], "прочитанное не пропадает");
        assert_eq!(pick_parts(&[false; 3], 0), [0]);
        assert!(pick_parts(&[], 5).is_empty());
    }
}
