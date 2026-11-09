//! Сопоставление реплик после считывания. Код ядра, одинаковый для всех движков.
//!
//! Каждый шаг берёт только то, что не сопоставили предыдущие:
//! 1. тот же ключ, если `source_hash` встречается в контейнере один раз и в старой, и в новой версии;
//! 2. `diff` сцены по `source_hash`, повтор с несовпавшими соседями переносится черновиком;
//! 3. тот же `source_hash`, единственный среди старых и новых реплик без пары во всей игре;
//! 4. похожий текст в замене из `diff`, только черновиком.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::SegmentId;
use crate::engines::{ContainerKey, SegmentKey};

/// Порог сходства текста для шага 4.
pub const SIMILARITY_THRESHOLD: f64 = 0.8;

/// Предел правок в `diff` одного контейнера. Больше: сцену переписали целиком, выравнивать нечего,
/// и весь контейнер идёт в шаги 3 и 4 как одна замена.
const MAX_EDITS: usize = 2000;

/// На шаге 4 старая реплика сравнивается с новыми не дальше этого от своего места в замене.
const SIMILAR_WINDOW: usize = 10;

/// Реплика для сопоставления: старая из базы или новая из снимка. Реплики одного контейнера
/// передаются в порядке сюжета: по нему идёт `diff`.
#[derive(Debug, Clone)]
pub struct MatchSegment<Ref> {
    /// Строка базы у старой реплики, номер в снимке у новой.
    pub id: Ref,
    pub key: SegmentKey,
    pub container: Option<ContainerKey>,
    /// Путь файла: реплики вне контейнеров выравниваются по файлу.
    pub file: String,
    pub source_hash: String,
    pub source_text: String,
    pub story_order: Option<i64>,
}

impl<Ref> MatchSegment<Ref> {
    fn group(&self) -> Group<'_> {
        match &self.container {
            Some(container) => Group::Container(container.as_str()),
            None => Group::File(&self.file),
        }
    }
}

/// Где реплики выравниваются друг с другом: контейнер, а вне контейнеров файл.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group<'a> {
    Container(&'a str),
    File(&'a str),
}

/// Каким шагом найдена пара.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
    /// Ключ тот же: строка базы остаётся, меняются только её данные.
    pub same_key: bool,
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
        let mut state = State { old, new, old_pair: vec![None; old.len()], new_taken: vec![false; new.len()], pairs: Vec::new() };

        let old_counts = hash_counts(old);
        let new_counts = hash_counts(new);
        let count = |counts: &HashMap<(Group, &str), usize>, group, hash| counts.get(&(group, hash)).copied().unwrap_or(0);

        // 1. Тот же ключ, текст не повторяется в контейнере.
        let new_by_key: HashMap<&SegmentKey, usize> = new.iter().enumerate().map(|(index, segment)| (&segment.key, index)).collect();
        for (o, segment) in old.iter().enumerate() {
            let Some(&n) = new_by_key.get(&segment.key) else { continue };
            let fresh = &new[n];
            if count(&old_counts, segment.group(), &segment.source_hash) == 1
                && count(&new_counts, fresh.group(), &fresh.source_hash) == 1
            {
                state.pair(o, n, MatchKind::SameKey, segment.source_hash != fresh.source_hash);
            }
        }

        // 2. Выравнивание внутри контейнера.
        let old_groups = groups(old);
        let new_groups = groups(new);
        let mut hunks: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
        for (group, old_indices) in &old_groups {
            let Some(new_indices) = new_groups.get(group) else { continue };
            let a: Vec<&str> = old_indices.iter().map(|&o| old[o].source_hash.as_str()).collect();
            let b: Vec<&str> = new_indices.iter().map(|&n| new[n].source_hash.as_str()).collect();
            let Some(ops) = diff(&a, &b) else {
                hunks.push((old_indices.clone(), new_indices.clone()));
                continue;
            };
            let (mut deleted, mut inserted) = (Vec::new(), Vec::new());
            for (step, op) in ops.iter().enumerate() {
                match *op {
                    Op::Equal(i, j) => {
                        if !deleted.is_empty() && !inserted.is_empty() {
                            hunks.push((std::mem::take(&mut deleted), std::mem::take(&mut inserted)));
                        }
                        deleted.clear();
                        inserted.clear();
                        let (o, n) = (old_indices[i], new_indices[j]);
                        if state.old_pair[o].is_some() || state.new_taken[n] {
                            continue;
                        }
                        let repeat = count(&old_counts, *group, a[i]) > 1 || count(&new_counts, *group, b[j]) > 1;
                        let before = step == 0 || matches!(ops[step - 1], Op::Equal(..));
                        let after = step + 1 == ops.len() || matches!(ops[step + 1], Op::Equal(..));
                        state.pair(o, n, MatchKind::Aligned, repeat && !(before && after));
                    }
                    Op::Delete(i) => deleted.push(old_indices[i]),
                    Op::Insert(j) => inserted.push(new_indices[j]),
                }
            }
            if !deleted.is_empty() && !inserted.is_empty() {
                hunks.push((deleted, inserted));
            }
        }

        // 3. Текст, единственный среди реплик без пары во всей игре.
        let mut old_free: HashMap<&str, Vec<usize>> = HashMap::new();
        for (o, segment) in old.iter().enumerate().filter(|(o, _)| state.old_pair[*o].is_none()) {
            old_free.entry(segment.source_hash.as_str()).or_default().push(o);
        }
        let mut new_free: HashMap<&str, Vec<usize>> = HashMap::new();
        for (n, segment) in new.iter().enumerate().filter(|(n, _)| !state.new_taken[*n]) {
            new_free.entry(segment.source_hash.as_str()).or_default().push(n);
        }
        for (o, segment) in old.iter().enumerate() {
            if state.old_pair[o].is_some() {
                continue;
            }
            let hash = segment.source_hash.as_str();
            if let (Some([_]), Some([n])) = (old_free.get(hash).map(Vec::as_slice), new_free.get(hash).map(Vec::as_slice)) {
                state.pair(o, *n, MatchKind::UniqueHash, false);
            }
        }

        // 4. Похожий текст в заменах.
        for (deleted, inserted) in &hunks {
            let olds: Vec<usize> = deleted.iter().copied().filter(|&o| state.old_pair[o].is_none()).collect();
            let news: Vec<usize> = inserted.iter().copied().filter(|&n| !state.new_taken[n]).collect();
            if olds.is_empty() || news.is_empty() {
                continue;
            }
            let mut candidates = Vec::new();
            for (a, &o) in olds.iter().enumerate() {
                let expected = a * news.len() / olds.len();
                let from = expected.saturating_sub(SIMILAR_WINDOW);
                let to = (expected + SIMILAR_WINDOW + 1).min(news.len());
                for (b, &n) in news.iter().enumerate().take(to).skip(from) {
                    let similarity = Self::similarity(&old[o].source_text, &new[n].source_text);
                    if similarity >= SIMILARITY_THRESHOLD {
                        candidates.push((similarity, a.abs_diff(expected).max(b.abs_diff(expected)), o, n));
                    }
                }
            }
            candidates.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
            for (_, _, o, n) in candidates {
                if state.old_pair[o].is_none() && !state.new_taken[n] {
                    state.pair(o, n, MatchKind::Similar, true);
                }
            }
        }

        let missing = old.iter().enumerate().filter(|(o, _)| state.old_pair[*o].is_none()).map(|(_, segment)| segment.id).collect();
        let moved_containers = moved_containers(old, new, &state.old_pair);
        let mut pairs = state.pairs;
        pairs.sort_by_key(|pair| pair.new);
        MatchPlan { pairs, missing, moved_containers }
    }

    /// Сходство двух текстов от 0 до 1: доля символов, которые не пришлось менять.
    pub fn similarity(a: &str, b: &str) -> f64 {
        if a == b {
            return 1.0;
        }
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let longest = a.len().max(b.len());
        if longest == 0 {
            return 1.0;
        }
        // Правок не меньше разницы длин: заведомо далёкие тексты не сравниваются целиком.
        if 1.0 - (a.len().abs_diff(b.len()) as f64) / (longest as f64) < SIMILARITY_THRESHOLD {
            return 0.0;
        }
        1.0 - levenshtein(&a, &b) as f64 / longest as f64
    }
}

/// Пары по ходу шагов.
struct State<'a> {
    old: &'a [MatchSegment<SegmentId>],
    new: &'a [MatchSegment<usize>],
    /// Номер новой реплики в паре у каждой старой.
    old_pair: Vec<Option<usize>>,
    new_taken: Vec<bool>,
    pairs: Vec<MatchedPair>,
}

impl State<'_> {
    fn pair(&mut self, o: usize, n: usize, kind: MatchKind, draft: bool) {
        self.old_pair[o] = Some(n);
        self.new_taken[n] = true;
        let same_key = self.old[o].key == self.new[n].key;
        self.pairs.push(MatchedPair { old: self.old[o].id, new: self.new[n].id, kind, draft, same_key });
    }
}

fn hash_counts<Ref>(segments: &[MatchSegment<Ref>]) -> HashMap<(Group<'_>, &str), usize> {
    let mut counts = HashMap::new();
    for segment in segments {
        *counts.entry((segment.group(), segment.source_hash.as_str())).or_insert(0) += 1;
    }
    counts
}

/// Номера реплик каждой группы по сюжету, без места в сюжете в порядке входа.
fn groups<Ref>(segments: &[MatchSegment<Ref>]) -> BTreeMap<Group<'_>, Vec<usize>> {
    let mut groups: BTreeMap<Group, Vec<usize>> = BTreeMap::new();
    for (index, segment) in segments.iter().enumerate() {
        groups.entry(segment.group()).or_default().push(index);
    }
    for indices in groups.values_mut() {
        indices.sort_by_key(|&index| (segments[index].story_order.is_none(), segments[index].story_order, index));
    }
    groups
}

/// Старый контейнер, все пары которого ушли в один другой новый контейнер, а сам он из игры пропал.
fn moved_containers(
    old: &[MatchSegment<SegmentId>],
    new: &[MatchSegment<usize>],
    old_pair: &[Option<usize>],
) -> Vec<(ContainerKey, ContainerKey)> {
    let current: BTreeSet<&ContainerKey> = new.iter().filter_map(|segment| segment.container.as_ref()).collect();
    let mut targets: BTreeMap<&ContainerKey, BTreeSet<Option<&ContainerKey>>> = BTreeMap::new();
    for (o, segment) in old.iter().enumerate() {
        let (Some(container), Some(n)) = (&segment.container, old_pair[o]) else { continue };
        targets.entry(container).or_default().insert(new[n].container.as_ref());
    }
    targets
        .into_iter()
        .filter(|(from, _)| !current.contains(from))
        .filter_map(|(from, to)| match to.into_iter().collect::<Vec<_>>().as_slice() {
            [Some(to)] => Some((from.clone(), (*to).clone())),
            _ => None,
        })
        .collect()
}

/// Шаг `diff`: номера в старой и новой последовательности.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

/// Кратчайший `diff` двух последовательностей (алгоритм Майерса). Общие начало и конец отрезаются заранее.
/// `None`: правок больше `MAX_EDITS`.
fn diff(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let middle = myers(&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix])?;

    let mut ops: Vec<Op> = (0..prefix).map(|i| Op::Equal(i, i)).collect();
    ops.extend(middle.into_iter().map(|op| match op {
        Op::Equal(i, j) => Op::Equal(i + prefix, j + prefix),
        Op::Delete(i) => Op::Delete(i + prefix),
        Op::Insert(j) => Op::Insert(j + prefix),
    }));
    ops.extend((0..suffix).map(|k| Op::Equal(a.len() - suffix + k, b.len() - suffix + k)));
    Some(ops)
}

fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = n + m;
    let offset = max + 1;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    // После шага d: v[k] для k от -d до d.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'search: for d in 0..=max {
        if d as usize > MAX_EDITS {
            return None;
        }
        for k in (-d..=d).step_by(2) {
            let at = |k: isize| (offset + k) as usize;
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) { v[at(k + 1)] } else { v[at(k - 1)] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                trace.push(v[at(-d)..=at(d)].to_vec());
                break 'search;
            }
        }
        trace.push(v[(offset - d) as usize..=(offset + d) as usize].to_vec());
    }

    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let k = x - y;
        let (prev_x, prev_y) = if d == 0 {
            (0, 0)
        } else {
            let previous = &trace[(d - 1) as usize];
            let get = |k: isize| previous[(k + d - 1) as usize];
            let prev_k = if k == -d || (k != d && get(k - 1) < get(k + 1)) { k + 1 } else { k - 1 };
            let prev_x = get(prev_k);
            (prev_x, prev_x - prev_k)
        };
        while x > prev_x && y > prev_y {
            ops.push(Op::Equal((x - 1) as usize, (y - 1) as usize));
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            ops.push(if x == prev_x { Op::Insert((y - 1) as usize) } else { Op::Delete((x - 1) as usize) });
        }
        (x, y) = (prev_x, prev_y);
    }
    ops.reverse();
    Some(ops)
}

/// Расстояние Левенштейна по символам.
fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y { diagonal } else { 1 + diagonal.min(above).min(row[j]) };
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment<Ref>(id: Ref, key: &str, container: &str, text: &str) -> MatchSegment<Ref> {
        MatchSegment {
            id,
            key: SegmentKey::new(key).unwrap(),
            container: Some(ContainerKey::new(container).unwrap()),
            file: "game/script.rpy".into(),
            source_hash: format!("#{text}"),
            source_text: text.into(),
            story_order: None,
        }
    }

    fn old(items: &[(&str, &str, &str)]) -> Vec<MatchSegment<SegmentId>> {
        items.iter().enumerate().map(|(i, (key, c, text))| segment(SegmentId::from_row(i as i64 + 1), key, c, text)).collect()
    }

    fn new(items: &[(&str, &str, &str)]) -> Vec<MatchSegment<usize>> {
        items.iter().enumerate().map(|(i, (key, c, text))| segment(i, key, c, text)).collect()
    }

    /// Пары как (строка старой, номер новой, шаг, черновик).
    fn pairs(plan: &MatchPlan) -> Vec<(i64, usize, MatchKind, bool)> {
        let mut pairs: Vec<_> = plan.pairs.iter().map(|p| (p.old.get(), p.new, p.kind, p.draft)).collect();
        pairs.sort();
        pairs
    }

    #[test]
    fn same_keys_keep_their_rows() {
        let before = old(&[("a", "label:x", "Hello."), ("b", "label:x", "Bye.")]);
        let after = new(&[("a", "label:x", "Hello."), ("b", "label:x", "Bye!")]);
        let plan = Matcher::plan(&before, &after);
        assert_eq!(pairs(&plan), vec![(1, 0, MatchKind::SameKey, false), (2, 1, MatchKind::SameKey, true)]);
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn repeats_shifted_by_an_insert_follow_their_text() {
        // Перед двумя «Yes.» вставили третье: старые ключи repeat достались другим репликам.
        let before = old(&[("q1", "label:x", "Ready?"), ("y", "label:x", "Yes."), ("y_1", "label:x", "Yes."), ("e", "label:x", "Go.")]);
        let after = new(&[
            ("q0", "label:x", "Sure?"),
            ("y", "label:x", "Yes."),
            ("q1", "label:x", "Ready?"),
            ("y_1", "label:x", "Yes."),
            ("y_2", "label:x", "Yes."),
            ("e", "label:x", "Go."),
        ]);
        let plan = Matcher::plan(&before, &after);
        // «Ready?» по ключу; первый «Yes.» после него с совпавшими соседями, второй перед «Go.» тоже.
        assert_eq!(
            pairs(&plan),
            vec![
                (1, 2, MatchKind::SameKey, false),
                (2, 3, MatchKind::Aligned, false),
                (3, 4, MatchKind::Aligned, false),
                (4, 5, MatchKind::SameKey, false),
            ]
        );
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn repeat_with_changed_neighbours_becomes_a_draft() {
        let before = old(&[("a", "label:x", "One."), ("y", "label:x", "Yes."), ("b", "label:x", "Two."), ("y_1", "label:x", "Yes.")]);
        let after = new(&[("c", "label:x", "Three."), ("y", "label:x", "Yes."), ("d", "label:x", "Four."), ("y_1", "label:x", "Yes.")]);
        let plan = Matcher::plan(&before, &after);
        let aligned: Vec<_> = pairs(&plan).into_iter().filter(|p| p.2 == MatchKind::Aligned).collect();
        assert_eq!(aligned, vec![(2, 1, MatchKind::Aligned, true), (4, 3, MatchKind::Aligned, true)]);
    }

    #[test]
    fn renamed_label_moves_lines_and_the_description() {
        let before = old(&[("x_1", "label:old", "Morning."), ("x_2", "label:old", "Coffee?")]);
        let after = new(&[("y_1", "label:new", "Morning."), ("y_2", "label:new", "Coffee?")]);
        let plan = Matcher::plan(&before, &after);
        assert_eq!(pairs(&plan), vec![(1, 0, MatchKind::UniqueHash, false), (2, 1, MatchKind::UniqueHash, false)]);
        assert_eq!(plan.moved_containers, vec![(ContainerKey::new("label:old").unwrap(), ContainerKey::new("label:new").unwrap())]);
    }

    #[test]
    fn ambiguous_text_across_the_game_is_not_moved() {
        let before = old(&[("x_1", "label:a", "Hm."), ("x_2", "label:b", "Hm.")]);
        let after = new(&[("y_1", "label:c", "Hm.")]);
        let plan = Matcher::plan(&before, &after);
        assert!(plan.pairs.is_empty());
        assert_eq!(plan.missing.len(), 2);
    }

    #[test]
    fn typo_fix_moves_as_a_draft_and_unrelated_text_goes_missing() {
        let before = old(&[("a", "label:x", "Start."), ("k1", "label:x", "I cant believe it."), ("k2", "label:x", "Totally different."), ("z", "label:x", "End.")]);
        let after = new(&[("a", "label:x", "Start."), ("n1", "label:x", "I can't believe it."), ("n2", "label:x", "Nothing alike here!"), ("z", "label:x", "End.")]);
        let plan = Matcher::plan(&before, &after);
        assert!(pairs(&plan).contains(&(2, 1, MatchKind::Similar, true)));
        assert_eq!(plan.missing, vec![SegmentId::from_row(3)]);
        assert!(plan.moved_containers.is_empty());
    }

    #[test]
    fn diff_matches_the_shortest_edit() {
        let a = ["a", "b", "c", "a", "b", "b", "a"];
        let b = ["c", "b", "a", "b", "a", "c"];
        let ops = diff(&a, &b).unwrap();
        let edits = ops.iter().filter(|op| !matches!(op, Op::Equal(..))).count();
        assert_eq!(edits, 5);
        let rebuilt: Vec<&str> = ops
            .iter()
            .filter_map(|op| match *op {
                Op::Equal(_, j) | Op::Insert(j) => Some(b[j]),
                Op::Delete(_) => None,
            })
            .collect();
        assert_eq!(rebuilt, b);
        assert_eq!(diff(&[], &["x"]).unwrap(), vec![Op::Insert(0)]);
        assert_eq!(diff(&["x"], &[]).unwrap(), vec![Op::Delete(0)]);
    }

    #[test]
    fn similarity_counts_changed_characters() {
        assert_eq!(Matcher::similarity("abc", "abc"), 1.0);
        assert!((Matcher::similarity("kitten", "sitting") - (1.0 - 3.0 / 7.0)).abs() < 1e-9);
        assert_eq!(Matcher::similarity("short", "a much longer sentence"), 0.0);
        assert_eq!(Matcher::similarity("", ""), 1.0);
    }
}
