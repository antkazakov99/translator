//! Сборка и установка мода. Подписи говорящих переводит сервис, движок только кладёт пары в мод.

pub mod commands;

use std::collections::{BTreeMap, HashSet};

use rusqlite::Connection;

use crate::common::AppError;
use crate::engines::{
    self, ContainerKey, GameEngine, GameSnapshot, ModArchive, ModInput, SegmentKey, SegmentTranslation, SnapshotSegment, SnapshotSpeaker,
    SpeakerKey,
};
use crate::glossary::{Character, GlossaryData, SpeakerEntry, SpeakerRepo, Term};
use crate::projects::{ProjectId, ProjectInfoRepo, ProjectRepo};
use crate::scan::ScanRepo;
use crate::state::AppState;
use crate::storage::DbError;

/// Подпись говорящего текущего снимка и что о ней известно из глоссария.
#[derive(Debug, Clone)]
pub struct SpeakerLabelSource<'a> {
    pub label: &'a str,
    pub entry: Option<&'a SpeakerEntry>,
    /// Связанные персонажи.
    pub linked: Vec<&'a Character>,
}

/// Перевод подписей говорящих (`character_name_strings` в 1.0). Первый найденный побеждает:
/// 1. перевод подписи в записи говорящего;
/// 2. имя связанного персонажа, подпись на нескольких переводится по частям;
/// 3. совпавшее имя любого персонажа;
/// 4. подтверждённый термин.
///
/// Подпись из одной переменной не переводится, несколько разных переводов на одном шаге пропускают подпись.
pub struct SpeakerLabels;

impl SpeakerLabels {
    pub fn translate(labels: &[SpeakerLabelSource<'_>], characters: &[Character], terms: &[Term]) -> Vec<(String, String)> {
        let mut variants: BTreeMap<&str, HashSet<String>> = BTreeMap::new();
        for source in labels {
            let label = source.label.trim();
            if label.is_empty() || label.contains('{') || is_whole_token(label) {
                continue;
            }
            let found = Self::variants(label, source.entry, &source.linked, characters, terms);
            variants.entry(source.label).or_default().extend(found);
        }
        variants
            .into_iter()
            .filter(|(_, found)| found.len() == 1)
            .filter_map(|(label, found)| found.into_iter().next().filter(|text| text != label).map(|text| (label.to_string(), text)))
            .collect()
    }

    fn variants(label: &str, entry: Option<&SpeakerEntry>, linked: &[&Character], characters: &[Character], terms: &[Term]) -> HashSet<String> {
        if let Some(text) = entry.and_then(SpeakerEntry::translation).map(str::trim).filter(|text| !text.is_empty()) {
            return HashSet::from([text.to_string()]);
        }
        let found: HashSet<String> = linked.iter().filter_map(|character| translation_of(character, label)).collect();
        if !found.is_empty() {
            return found;
        }
        let found: HashSet<String> = characters.iter().filter_map(|character| translation_of(character, label)).collect();
        if !found.is_empty() {
            return found;
        }
        let found: HashSet<String> = terms.iter().filter(|term| !term.is_guess()).filter_map(|term| term_translation(term, label)).collect();
        if !found.is_empty() {
            return found;
        }
        for (separator, joined) in [(" & ", " & "), (" and ", " и ")] {
            if let Some((first, second)) = label.split_once(separator) {
                let first = Self::variants(first.trim(), None, &[], characters, terms);
                let second = Self::variants(second.trim(), None, &[], characters, terms);
                return first.iter().flat_map(|first| second.iter().map(move |second| format!("{first}{joined}{second}"))).collect();
            }
        }
        HashSet::new()
    }
}

/// Подпись целиком из одной переменной вроде `[player]`: её значение задаёт игра.
fn is_whole_token(label: &str) -> bool {
    label.starts_with('[') && label.ends_with(']') && !label[1..label.len() - 1].contains(['[', ']'])
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Перевод подписи по имени персонажа: полное имя, имя, фамилия или форма.
fn translation_of(character: &Character, label: &str) -> Option<String> {
    let pairs = [
        (character.full_name(), character.translated_full_name()),
        (character.first_name().map(str::to_string), character.translated_first_name().map(str::to_string)),
        (character.last_name().map(str::to_string), character.translated_last_name().map(str::to_string)),
    ];
    let forms = character.forms().iter().map(|form| (Some(form.text().to_string()), form.translation().map(str::to_string)));
    pairs
        .into_iter()
        .chain(forms)
        .find(|(name, _)| name.as_deref().is_some_and(|name| same(name, label)))
        .and_then(|(_, translated)| translated)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn term_translation(term: &Term, label: &str) -> Option<String> {
    let text = if same(term.name(), label) {
        term.translation()?
    } else {
        let form = term.forms().iter().find(|form| same(form.text(), label))?;
        form.translation().or(term.translation())?
    };
    Some(text.trim().to_string()).filter(|text| !text.is_empty())
}

/// Итог сборки или установки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModResult {
    pub archive: ModArchive,
    /// В папке игры на диске есть свой перевод: он важнее архива, часть правок игра не покажет.
    pub tl_on_disk: bool,
}

pub struct ExportService<'a> {
    pub state: &'a AppState,
}

impl ExportService<'_> {
    /// Всё для мода из project.db: текущий снимок, действующие переводы, подписи говорящих, шрифты.
    pub fn mod_input(&self, project: ProjectId) -> Result<ModInput, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let scan = ScanRepo { conn: tx }.latest()?.ok_or(DbError::NotFound)?;
            let files = ScanRepo { conn: tx }.current_files()?;
            let (segments, translations) = segments(tx)?;
            let glossary = GlossaryData::load(tx)?;
            let current = SpeakerRepo { conn: tx }.current()?;
            let sources: Vec<SpeakerLabelSource> = current
                .iter()
                .filter_map(|item| {
                    let label = item.speaker.label.as_deref()?;
                    let entry = glossary.entries.iter().find(|entry| entry.key() == &item.speaker.key);
                    let linked = glossary
                        .links
                        .iter()
                        .filter(|link| link.speaker == item.speaker.key)
                        .filter_map(|link| glossary.characters.iter().find(|character| character.id() == Some(link.character)))
                        .collect();
                    Some(SpeakerLabelSource { label, entry, linked })
                })
                .collect();
            let speaker_labels = SpeakerLabels::translate(&sources, &glossary.characters, &glossary.terms);
            let speakers = current
                .iter()
                .map(|item| SnapshotSpeaker { key: item.speaker.key.clone(), kind: item.speaker.kind.clone(), label: item.speaker.label.clone() })
                .collect();
            let target_language = ProjectInfoRepo { conn: tx }.target_language()?.to_string();
            Ok(ModInput {
                snapshot: GameSnapshot { info: scan.game_info, files, containers: Vec::new(), segments, speakers, edges: Vec::new() },
                translations,
                speaker_labels,
                fonts: crate::fonts::FontService::swaps(tx)?,
                target_language,
            })
        })?)
    }

    /// Собирает мод в `exports/`. Папка игры не нужна.
    pub fn build(&self, project: ProjectId) -> Result<ModArchive, AppError> {
        let engine = self.engine(project)?;
        let input = self.mod_input(project)?;
        let open = self.state.open_project(project)?;
        let out_dir = open.dir.join("exports");
        std::fs::create_dir_all(&out_dir)?;
        Ok(engine.build_mod(&input, &open.files, &out_dir)?)
    }

    /// Собирает мод заново и ставит в привязанную папку: нужна привязка (`is_bound`) и папка на диске.
    pub fn install(&self, project: ProjectId) -> Result<ModResult, AppError> {
        let root = self.root(project)?;
        let archive = self.build(project)?;
        self.engine(project)?.install(&root, &archive)?;
        let tl_on_disk = root.as_path().join("game").join("tl").join("russian").is_dir();
        Ok(ModResult { archive, tl_on_disk })
    }

    pub fn uninstall(&self, project: ProjectId) -> Result<(), AppError> {
        let root = self.root(project)?;
        Ok(self.engine(project)?.uninstall(&root)?)
    }

    fn engine(&self, project: ProjectId) -> Result<&'static dyn GameEngine, AppError> {
        let stored = self.state.app.read(|tx| ProjectRepo { conn: tx }.get(project))?;
        engines::by_id(stored.engine()).ok_or(AppError::UnsupportedEngine(stored.engine()))
    }

    fn root(&self, project: ProjectId) -> Result<crate::projects::GameRoot, AppError> {
        let stored = self.state.app.read(|tx| ProjectRepo { conn: tx }.get(project))?;
        let root = stored.game_root().cloned().ok_or(AppError::Unbound)?;
        if !root.as_path().is_dir() {
            return Err(AppError::GameMissing);
        }
        Ok(root)
    }
}

/// Реплики текущего снимка по сюжету и их действующие переводы.
fn segments(tx: &Connection) -> Result<(Vec<SnapshotSegment>, Vec<SegmentTranslation>), DbError> {
    let mut statement = tx.prepare(
        "SELECT s.key, c.key, f.path, s.speaker_key, s.source_text, s.source_hash, s.game_text, s.location, s.story_order,
                s.engine_meta, t.text
         FROM segments s
         JOIN files f ON f.id = s.file_id
         LEFT JOIN containers c ON c.id = s.container_id
         LEFT JOIN translations t ON t.segment_id = s.id
         WHERE s.missing = 0
         ORDER BY c.story_order IS NULL, c.story_order, s.story_order, s.id",
    )?;
    type Row = (String, Option<String>, String, Option<String>, String, String, Option<String>, Option<String>, Option<i64>, Option<String>, Option<String>);
    let rows = statement
        .query_map([], |row| -> rusqlite::Result<Row> {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut segments = Vec::with_capacity(rows.len());
    let mut translations = Vec::new();
    for (key, container, file, speaker, source_text, source_hash, game_text, location, story_order, meta, text) in rows {
        let key = SegmentKey::new(key).map_err(|error| DbError::bad_value("segments", "key", error))?;
        let container = container.map(ContainerKey::new).transpose().map_err(|error| DbError::bad_value("containers", "key", error))?;
        let speaker = speaker.map(SpeakerKey::new).transpose().map_err(|error| DbError::bad_value("segments", "speaker_key", error))?;
        let engine_meta = meta
            .map(|meta| serde_json::from_str(&meta))
            .transpose()
            .map_err(|error| DbError::Corrupt(format!("segments.engine_meta: {error}")))?;
        if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
            translations.push(SegmentTranslation { key: key.clone(), text });
        }
        segments.push(SnapshotSegment { key, container, file, speaker, source_text, source_hash, game_text, location, story_order, engine_meta });
    }
    Ok((segments, translations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Timestamp;
    use crate::glossary::{CharacterId, Form, GlossaryService, TermCategory};
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;
    use crate::translation::TranslationService;

    fn person(id: i64, first: &str, last: Option<&str>, translated: (&str, Option<&str>), forms: Vec<Form>) -> Character {
        Character::restore(
            CharacterId::from_row(id),
            Some(first.to_string()),
            last.map(str::to_string),
            Some(translated.0.to_string()),
            translated.1.map(str::to_string),
            None,
            None,
            None,
            forms,
        )
    }

    fn source<'a>(label: &'a str, entry: Option<&'a SpeakerEntry>, linked: Vec<&'a Character>) -> SpeakerLabelSource<'a> {
        SpeakerLabelSource { label, entry, linked }
    }

    #[test]
    fn labels_are_translated_by_entry_character_term_and_parts() {
        let lee = person(1, "Celia", Some("Lee"), ("Селия", Some("Ли")), vec![Form::new("Miss Lee", Some("Мисс Ли")).unwrap()]);
        let katy = person(2, "Katy", None, ("Кэти", None), Vec::new());
        let other = person(3, "Celia", None, ("Целия", None), Vec::new());
        let mut guard = Term::new("Guard", TermCategory::Title).unwrap();
        guard.set_translation("Стражник");
        let guess = Term::guess("Ghost", "Призрак", TermCategory::General).unwrap();
        let mut entry = SpeakerEntry::new(SpeakerKey::new("label:Radio").unwrap(), Timestamp::from_secs(0));
        entry.set_translation("Радио");
        let characters = vec![lee.clone(), katy.clone()];
        let terms = vec![guard, guess];
        let labels = [
            source("Radio", Some(&entry), Vec::new()),
            source("Miss Lee", None, vec![&lee]),
            source("Katy", None, Vec::new()),
            source("guard", None, Vec::new()),
            source("Ghost", None, Vec::new()),
            source("Celia & Katy", None, Vec::new()),
            source("[player]", None, Vec::new()),
            source("{b}Katy{/b}", None, Vec::new()),
        ];
        let translated = SpeakerLabels::translate(&labels, &characters, &terms);
        assert_eq!(
            translated,
            [
                ("Celia & Katy".to_string(), "Селия & Кэти".to_string()),
                ("Katy".to_string(), "Кэти".to_string()),
                ("Miss Lee".to_string(), "Мисс Ли".to_string()),
                ("Radio".to_string(), "Радио".to_string()),
                ("guard".to_string(), "Стражник".to_string()),
            ],
            "предположение модели и переменная не переводятся"
        );

        let ambiguous = SpeakerLabels::translate(&[source("Celia", None, Vec::new())], &[lee.clone(), other], &[]);
        assert!(ambiguous.is_empty(), "два персонажа с одним именем переводят подпись по-разному");
        let linked = SpeakerLabels::translate(&[source("Celia", None, vec![&lee])], &[lee.clone(), person(3, "Celia", None, ("Целия", None), Vec::new())], &[]);
        assert_eq!(linked, [("Celia".to_string(), "Селия".to_string())], "связанный персонаж точнее");
    }

    #[test]
    fn project_builds_mod_without_game_folder_and_needs_it_to_install() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::start(dir.path().join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.path().join("game"));
        let projects = ProjectService { state: &state };
        let id = projects.create(root.clone()).unwrap().id();
        projects.open(id).unwrap();
        let translation = TranslationService { state: &state };
        let file = translation.files(id).unwrap()[0].id;
        let lines: Vec<_> = translation.file_scenes(id, file).unwrap().into_iter().flat_map(|scene| scene.lines).collect();
        translation.save_text(id, lines[0].id, "Привет.").unwrap();
        GlossaryService { state: &state }.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let open = state.open_project(id).unwrap();
        let mut eileen = open.project.read(|tx| crate::glossary::CharacterRepo { conn: tx }.list()).unwrap().remove(0);
        eileen.set_translated_names("Эйлин", "");
        GlossaryService { state: &state }.save_character(id, &eileen).unwrap();

        let service = ExportService { state: &state };
        let input = service.mod_input(id).unwrap();
        assert_eq!(input.speaker_labels, [("Eileen".to_string(), "Эйлин".to_string())]);
        assert_eq!(input.translations.len(), 1);
        let built = service.build(id).unwrap();
        assert!(built.path.starts_with(&open.dir), "мод собирается в папке проекта");
        assert_eq!(built.written, 1);

        let installed = service.install(id).unwrap();
        assert!(!installed.tl_on_disk);
        assert!(root.as_path().join("game").join("ru_lang.rpa").is_file());
        service.uninstall(id).unwrap();
        assert!(!root.as_path().join("game").join("ru_lang.rpa").exists());
    }
}
