//! Реплики и сцены персонажей и терминов, текст для их описаний: части из целых сцен.

use std::collections::HashSet;

use super::parts::{ScenesPart, pack_scenes, script_line};
use super::repo::StoryLine;
use super::{Character, Term};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::translation::PlacedLine;
use crate::translation::hints::GlossaryContext;
use crate::translation::text::mentions;

/// Реплика, где встречается персонаж или термин.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub placed: PlacedLine,
    /// Говорит сам персонаж. `false`: его упоминают, у термина всегда.
    pub spoken: bool,
}

/// Как персонаж встречается в реплике: говорит сам или его упоминают. `None`: не встречается.
/// Упоминание это имя целым словом с учётом регистра, чтобы «Rose» не находила «rose».
fn character_use(speaker: Option<&SpeakerKey>, text: &str, speakers: &HashSet<SpeakerKey>, names: &[String]) -> Option<bool> {
    if speaker.is_some_and(|key| speakers.contains(key)) {
        Some(true)
    } else if names.iter().any(|name| mentions(text, name)) {
        Some(false)
    } else {
        None
    }
}

/// Есть ли в реплике название или форма термина целым словом без учёта регистра.
fn term_used(text: &str, names: &[String]) -> bool {
    let lower = text.to_lowercase();
    names.iter().any(|name| mentions(&lower, name))
}

/// Реплики персонажа по сюжету: где говорит связанный с ним говорящий или упомянуто его имя.
pub fn character_lines(lines: Vec<PlacedLine>, speakers: &HashSet<SpeakerKey>, names: &[String]) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter_map(|placed| {
            let spoken = character_use(placed.line.speaker_key.as_ref(), &placed.line.source_text, speakers, names)?;
            Some(EntryLine { placed, spoken })
        })
        .collect()
}

/// Реплики термина по сюжету: где есть его название или форма.
pub fn term_lines(lines: Vec<PlacedLine>, names: &[String]) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter(|placed| term_used(&placed.line.source_text, names))
        .map(|placed| EntryLine { placed, spoken: false })
        .collect()
}

/// Слова имени короче этого упоминанием не считаются: «Al» найдётся в чём угодно.
const MIN_NAME_CHARS: usize = 3;

/// Имена, по которым персонаж находится в репликах: полное имя, имя, фамилия и формы. Имена с токенами
/// и слишком короткие пропускаются.
pub fn character_names(character: &Character) -> Vec<String> {
    let mut names: Vec<String> = character
        .full_name()
        .into_iter()
        .chain(character.first_name().map(str::to_string))
        .chain(character.last_name().map(str::to_string))
        .chain(character.forms().iter().map(|form| form.text().to_string()))
        .map(|name| name.trim().to_string())
        .filter(|name| name.chars().count() >= MIN_NAME_CHARS && !name.contains('['))
        .collect();
    names.dedup();
    names
}

/// Название и формы термина в нижнем регистре: термин ищется без учёта регистра.
pub fn term_names(term: &Term) -> Vec<String> {
    std::iter::once(term.name())
        .chain(term.forms().iter().map(|form| form.text()))
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Сцены персонажа: где говорит связанный с ним говорящий или упомянуто его имя.
pub fn character_scenes(lines: &[StoryLine], speakers: &HashSet<SpeakerKey>, names: &[String], excluded: &[ContainerKey]) -> Vec<ContainerKey> {
    scenes_where(lines, excluded, |line| character_use(line.speaker_key.as_ref(), &line.text, speakers, names).is_some())
}

/// Сцены термина: где есть название или форма.
pub fn term_scenes(lines: &[StoryLine], names: &[String], excluded: &[ContainerKey]) -> Vec<ContainerKey> {
    scenes_where(lines, excluded, |line| term_used(&line.text, names))
}

fn scenes_where(lines: &[StoryLine], excluded: &[ContainerKey], wanted: impl Fn(&StoryLine) -> bool) -> Vec<ContainerKey> {
    let excluded: HashSet<&ContainerKey> = excluded.iter().collect();
    let mut seen: HashSet<&ContainerKey> = HashSet::new();
    let mut found: Vec<ContainerKey> = Vec::new();
    for line in lines {
        if seen.contains(&line.scene) || excluded.contains(&line.scene) {
            continue;
        }
        if wanted(line) {
            seen.insert(&line.scene);
            found.push(line.scene.clone());
        }
    }
    found
}

/// Части из выбранных сцен: реплики строками «Говорящий: текст» с именами из глоссария и значениями токенов.
pub fn entry_parts(lines: &[StoryLine], scenes: &[ContainerKey], context: &GlossaryContext) -> Vec<ScenesPart> {
    let chosen: HashSet<&ContainerKey> = scenes.iter().collect();
    let mut transcripts: Vec<(ContainerKey, Vec<String>)> = Vec::new();
    for line in lines.iter().filter(|line| chosen.contains(&line.scene)) {
        let speaker = line
            .speaker_key
            .as_ref()
            .and_then(|key| context.speaker_name(key).map(str::to_string))
            .or_else(|| line.label.clone())
            .or_else(|| line.speaker_key.as_ref().map(|key| key.as_str().split_once(':').map_or(key.as_str(), |(_, tail)| tail).to_string()));
        let text = script_line(speaker.as_deref(), &context.fill(&line.text));
        match transcripts.last_mut() {
            Some((key, texts)) if *key == line.scene => texts.push(text),
            _ => transcripts.push((line.scene.clone(), vec![text])),
        }
    }
    pack_scenes(&transcripts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(scene: &str, speaker: Option<&str>, text: &str) -> StoryLine {
        StoryLine {
            scene: ContainerKey::new(scene).unwrap(),
            speaker_key: speaker.map(|key| SpeakerKey::new(key).unwrap()),
            label: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn character_is_found_by_speaker_and_names_with_case() {
        let lines = [
            line("label:a", None, "A rose in bloom."),
            line("label:b", None, "Rose is late."),
            line("label:c", Some("code:r"), "Hi."),
            line("label:d", None, "Nobody here."),
            line("label:e", None, "Rose again."),
        ];
        let speakers = HashSet::from([SpeakerKey::new("code:r").unwrap()]);
        let names = vec!["Rose".to_string()];
        let found = character_scenes(&lines, &speakers, &names, &[ContainerKey::new("label:e").unwrap()]);
        let keys: Vec<&str> = found.iter().map(ContainerKey::as_str).collect();
        assert_eq!(keys, ["label:b", "label:c"], "строчная «rose» не упоминание, исключённая сцена пропущена");
    }

    #[test]
    fn term_is_found_in_any_case() {
        let lines = [line("label:a", None, "The OLD KEY opens it."), line("label:b", None, "Keystone.")];
        assert_eq!(term_scenes(&lines, &["old key".to_string()], &[]).len(), 1);
        assert!(term_scenes(&lines, &["key".to_string()], &[]).iter().all(|key| key.as_str() == "label:a"));
    }

    #[test]
    fn parts_name_their_scenes() {
        let lines = [line("label:a", None, "One."), line("label:a", None, "Two."), line("label:b", None, "Three.")];
        let scenes = [ContainerKey::new("label:a").unwrap(), ContainerKey::new("label:b").unwrap()];
        let parts = entry_parts(&lines, &scenes, &GlossaryContext::default());
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].part.text, "One.\nTwo.\n---\nThree.\n");
        assert_eq!(parts[0].scenes, scenes);
    }

    fn placed(id: i64, speaker: Option<&str>, text: &str) -> PlacedLine {
        let mut line = crate::translation::batcher::tests::line(id, text, "");
        line.speaker_key = speaker.map(|key| SpeakerKey::new(key).unwrap());
        PlacedLine { scene: None, scene_title: None, file: crate::scan::FileId::from_row(1), line }
    }

    #[test]
    fn character_lines_tell_speech_from_mentions() {
        let lines = vec![
            placed(1, Some("code:r"), "Rose here."),
            placed(2, None, "Where is Rose?"),
            placed(3, None, "A rose in bloom."),
            placed(4, Some("code:x"), "Hi, Rosie."),
        ];
        let speakers = HashSet::from([SpeakerKey::new("code:r").unwrap()]);
        let found = character_lines(lines, &speakers, &["Rose".to_string(), "Rosie".to_string()]);
        let seen: Vec<(i64, bool)> = found.iter().map(|line| (line.placed.line.id.get(), line.spoken)).collect();
        assert_eq!(seen, [(1, true), (2, false), (4, false)], "своя реплика с именем это речь, строчная «rose» не упоминание");
    }

    #[test]
    fn term_lines_ignore_case_and_word_parts() {
        let lines = vec![placed(1, None, "The OLD KEY opens it."), placed(2, None, "Keystone."), placed(3, None, "A key.")];
        let found = term_lines(lines, &["key".to_string()]);
        let ids: Vec<i64> = found.iter().map(|line| line.placed.line.id.get()).collect();
        assert_eq!(ids, [1, 3]);
        assert!(found.iter().all(|line| !line.spoken));
    }
}
