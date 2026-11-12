//! Реплики и сцены персонажей и терминов, текст для их описаний: части из целых сцен.

use std::collections::HashSet;

use super::parts::{ScenesPart, pack_scenes, script_line};
use super::repo::StoryLine;
use super::{Character, CharacterId, GlossaryData, Term, TermId};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::translation::PlacedLine;
use crate::translation::hints::GlossaryContext;
use crate::translation::text::token_names;
use super::mentions::{Checks, mentioned};

/// Реплика, где встречается персонаж или термин.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
    pub placed: PlacedLine,
    /// Говорит сам персонаж. `false`: его упоминают, у термина всегда.
    pub spoken: bool,
}

/// Как персонаж встречается в реплике: говорит сам или его упоминают. `None`: не встречается.
/// Упоминание решает `mentions::mentioned`: «Rose» не находит «rose», пока модель не сказала иначе.
fn character_use(speaker: Option<&SpeakerKey>, text: &str, hash: &str, speakers: &HashSet<SpeakerKey>, names: &[String], checks: Option<&Checks>) -> Option<bool> {
    if speaker.is_some_and(|key| speakers.contains(key)) {
        Some(true)
    } else if mentioned(text, hash, names, checks) {
        Some(false)
    } else {
        None
    }
}

/// Реплики персонажа по сюжету: где говорит связанный с ним говорящий или упомянуто его имя.
pub fn character_lines(lines: Vec<PlacedLine>, speakers: &HashSet<SpeakerKey>, names: &[String], checks: Option<&Checks>) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter_map(|placed| {
            let line = &placed.line;
            let spoken = character_use(line.speaker_key.as_ref(), &line.source_text, &line.source_hash, speakers, names, checks)?;
            Some(EntryLine { placed, spoken })
        })
        .collect()
}

/// Реплики говорящего по сюжету: все, где говорит он сам.
pub fn speaker_lines(lines: Vec<PlacedLine>, speaker: &SpeakerKey) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter(|placed| placed.line.speaker_key.as_ref() == Some(speaker))
        .map(|placed| EntryLine { placed, spoken: true })
        .collect()
}

/// Реплики токена по сюжету: где он стоит в тексте, `[mom]` или `[mom!c]`.
pub fn token_lines(lines: Vec<PlacedLine>, name: &str) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter(|placed| token_names(&placed.line.source_text).any(|found| found == name))
        .map(|placed| EntryLine { placed, spoken: false })
        .collect()
}

/// Реплики термина по сюжету: где есть его название или форма.
pub fn term_lines(lines: Vec<PlacedLine>, names: &[String], checks: Option<&Checks>) -> Vec<EntryLine> {
    lines
        .into_iter()
        .filter(|placed| mentioned(&placed.line.source_text, &placed.line.source_hash, names, checks))
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

/// Название и формы термина, как они написаны: регистр решает `mentions::mentioned`.
pub fn term_names(term: &Term) -> Vec<String> {
    std::iter::once(term.name())
        .chain(term.forms().iter().map(|form| form.text()))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Сцены персонажа: где говорит связанный с ним говорящий или упомянуто его имя.
pub fn character_scenes(
    lines: &[StoryLine],
    speakers: &HashSet<SpeakerKey>,
    names: &[String],
    checks: Option<&Checks>,
    excluded: &[ContainerKey],
) -> Vec<ContainerKey> {
    scenes_where(lines, excluded, |line| character_use(line.speaker_key.as_ref(), &line.text, &line.source_hash, speakers, names, checks).is_some())
}

/// Сцены термина: где есть название или форма.
pub fn term_scenes(lines: &[StoryLine], names: &[String], checks: Option<&Checks>, excluded: &[ContainerKey]) -> Vec<ContainerKey> {
    scenes_where(lines, excluded, |line| mentioned(&line.text, &line.source_hash, names, checks))
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

/// Персонажи и термины сцены по её репликам: персонаж говорит или упомянут, термин назван. Порядок глоссария.
pub fn scene_entries(glossary: &GlossaryData, lines: &[StoryLine]) -> (Vec<CharacterId>, Vec<TermId>) {
    let characters = glossary
        .characters
        .iter()
        .filter_map(|character| {
            let id = character.id()?;
            let speakers: HashSet<SpeakerKey> = glossary.links.iter().filter(|link| link.character == id).map(|link| link.speaker.clone()).collect();
            let names = character_names(character);
            let checks = glossary.character_checks.get(&id);
            lines
                .iter()
                .any(|line| character_use(line.speaker_key.as_ref(), &line.text, &line.source_hash, &speakers, &names, checks).is_some())
                .then_some(id)
        })
        .collect();
    let terms = glossary
        .terms
        .iter()
        .filter_map(|term| {
            let id = term.id()?;
            let names = term_names(term);
            let checks = glossary.term_checks.get(&id);
            lines.iter().any(|line| mentioned(&line.text, &line.source_hash, &names, checks)).then_some(id)
        })
        .collect();
    (characters, terms)
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
            source_hash: format!("hash:{text}"),
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
        let found = character_scenes(&lines, &speakers, &names, None, &[ContainerKey::new("label:e").unwrap()]);
        let keys: Vec<&str> = found.iter().map(ContainerKey::as_str).collect();
        assert_eq!(keys, ["label:b", "label:c"], "строчная «rose» не упоминание, исключённая сцена пропущена");
    }

    #[test]
    fn term_is_found_by_spelling_and_answers() {
        let lines = [line("label:a", None, "The OLD KEY opens it."), line("label:b", None, "Keystone."), line("label:c", None, "Never mind.")];
        assert_eq!(term_scenes(&lines, &["old key".to_string()], None, &[]).len(), 1, "целиком заглавными засчитывается");
        assert!(term_scenes(&lines, &["key".to_string()], None, &[]).iter().all(|key| key.as_str() == "label:a"));
        assert!(term_scenes(&lines, &["MInD".to_string()], None, &[]).is_empty(), "обычное слово не термин");
        let checks: Checks = [("hash:Never mind.".to_string(), true)].into();
        assert_eq!(term_scenes(&lines, &["MInD".to_string()], Some(&checks), &[]).len(), 1, "ответ модели важнее написания");
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
        let found = character_lines(lines, &speakers, &["Rose".to_string(), "Rosie".to_string()], None);
        let seen: Vec<(i64, bool)> = found.iter().map(|line| (line.placed.line.id.get(), line.spoken)).collect();
        assert_eq!(seen, [(1, true), (2, false), (4, false)], "своя реплика с именем это речь, строчная «rose» не упоминание");
    }

    #[test]
    fn speaker_lines_are_only_his_own() {
        let lines = vec![placed(1, Some("code:r"), "Rose here."), placed(2, None, "Where is Rose?"), placed(3, Some("code:x"), "Hi.")];
        let found = speaker_lines(lines, &SpeakerKey::new("code:r").unwrap());
        let seen: Vec<(i64, bool)> = found.iter().map(|line| (line.placed.line.id.get(), line.spoken)).collect();
        assert_eq!(seen, [(1, true)]);
    }

    #[test]
    fn token_lines_find_the_token_with_modifiers() {
        let lines = vec![placed(1, None, "Hi, [mom]."), placed(2, None, "[Mom!c] said."), placed(3, None, "[mom!c] said."), placed(4, None, "mom")];
        let ids: Vec<i64> = token_lines(lines, "mom").iter().map(|line| line.placed.line.id.get()).collect();
        assert_eq!(ids, [1, 3]);
    }

    #[test]
    fn term_lines_follow_spelling_and_skip_word_parts() {
        let lines = vec![placed(1, None, "The OLD KEY opens it."), placed(2, None, "Keystone."), placed(3, None, "A key."), placed(4, None, "Key Lime.")];
        let found = term_lines(lines, &["key".to_string()], None);
        let ids: Vec<i64> = found.iter().map(|line| line.placed.line.id.get()).collect();
        assert_eq!(ids, [1, 3, 4]);
        assert!(found.iter().all(|line| !line.spoken));
    }
}
