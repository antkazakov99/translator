//! Правки глоссария без модели: разбор «Новых», связи, передача термина персонажу, удаление.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::candidates::{TERM_CANDIDATES_VERSION, find_candidates, name_words, without_names};
use super::{
    Character, CharacterId, CharacterRepo, DiscardReason, DiscardedTerm, Form, LineSampleRepo, NameKey, NoteOwner, NoteRepo,
    SceneDescription, SceneDescriptionRepo, Speaker, SpeakerEntry, SpeakerLink, SpeakerRepo, SpeakerUse, Term, TermCandidate,
    TermCategory, TermId, TermRepo, Token, TokenRepo,
};
use crate::common::{AppError, Timestamp, ValidationError};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::projects::ProjectId;
use crate::scan::ScanRepo;
use crate::state::AppState;
use crate::storage::DbError;

/// Глоссарий проекта целиком: для окна и для подсказок модели.
#[derive(Debug, Clone, Default)]
pub struct GlossaryData {
    pub characters: Vec<Character>,
    pub links: Vec<SpeakerLink>,
    pub entries: Vec<SpeakerEntry>,
    pub terms: Vec<Term>,
    pub tokens: Vec<Token>,
}

impl GlossaryData {
    /// Одним чтением, чтобы части глоссария не разошлись.
    pub fn load(conn: &Connection) -> Result<Self, DbError> {
        let speakers = SpeakerRepo { conn };
        Ok(Self {
            characters: CharacterRepo { conn }.list()?,
            links: speakers.links()?,
            entries: speakers.entries()?,
            terms: TermRepo { conn }.list()?,
            tokens: TokenRepo { conn }.list()?,
        })
    }
}

/// Раздел «Говорящие» окна: говорящие снимка, корзина и корзина названий.
#[derive(Debug, Clone, Default)]
pub struct GlossaryOverview {
    pub data: GlossaryData,
    pub speakers: Vec<SpeakerUse>,
    pub discarded_speakers: Vec<Speaker>,
    pub discarded_terms: Vec<DiscardedTerm>,
    /// Токены из текста игры и число реплик с ними.
    pub game_tokens: BTreeMap<String, usize>,
    pub scenes: Vec<SceneDescription>,
}

pub struct GlossaryService<'a> {
    pub state: &'a AppState,
}

impl GlossaryService<'_> {
    /// Всё для окна глоссария одним чтением.
    pub fn overview(&self, project: ProjectId) -> Result<GlossaryOverview, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let speakers = SpeakerRepo { conn: tx };
            Ok(GlossaryOverview {
                data: GlossaryData::load(tx)?,
                speakers: speakers.current()?,
                discarded_speakers: speakers.discarded()?,
                discarded_terms: TermRepo { conn: tx }.discarded()?,
                game_tokens: game_tokens(tx)?,
                scenes: SceneDescriptionRepo { conn: tx }.list()?,
            })
        })?)
    }

    /// «Новые термины»: кандидаты из текста игры без известного глоссарию и без названий, все слова которых
    /// это слова имён персонажей и подписей говорящих. Отставшую таблицу кандидатов сначала пересобирает.
    pub fn new_terms(&self, project: ProjectId) -> Result<Vec<TermCandidate>, AppError> {
        let open = self.state.open_project(project)?;
        let (state, scan, texts) = open.project.read(|tx| {
            let state = TermRepo { conn: tx }.candidates_state()?;
            let scan = ScanRepo { conn: tx }.latest()?.map(|scan| scan.id);
            let fresh = scan.is_none_or(|scan| state == Some((TERM_CANDIDATES_VERSION, scan)));
            Ok((state, scan, if fresh { Vec::new() } else { LineSampleRepo { conn: tx }.all_texts()? }))
        })?;
        if let Some(scan) = scan
            && state != Some((TERM_CANDIDATES_VERSION, scan))
        {
            let found = find_candidates(&texts);
            open.project.write(|tx| TermRepo { conn: tx }.replace_candidates(&found, TERM_CANDIDATES_VERSION, scan))?;
        }
        let (candidates, data, labels) = open.project.read(|tx| {
            Ok((TermRepo { conn: tx }.new_candidates()?, GlossaryData::load(tx)?, LineSampleRepo { conn: tx }.speaker_labels()?))
        })?;
        let names = data.characters.iter().flat_map(|character| {
            [character.first_name(), character.last_name()].into_iter().flatten().chain(character.forms().iter().map(|form| form.text()))
        });
        Ok(without_names(candidates, &name_words(names.chain(labels.iter().map(String::as_str)))))
    }

    /// «Персонаж»: создаёт персонажа с именем из подписи и связывает с ним говорящего основным кодом.
    /// Подпись, не совпавшая с именем и фамилией, становится формой.
    pub fn speaker_to_character(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<CharacterId, AppError> {
        self.write(project, |tx| character_from_speaker(tx, speaker))
    }

    /// «Взять всех» в «Новых говорящих».
    pub fn all_speakers_to_characters(&self, project: ProjectId) -> Result<Vec<CharacterId>, AppError> {
        self.write(project, |tx| {
            let fresh = SpeakerRepo { conn: tx }.new_speakers()?;
            fresh.iter().map(|speaker| character_from_speaker(tx, &speaker.key)).collect()
        })
    }

    /// «Привязать» к одному или нескольким существующим персонажам. Персонаж без основного кода получает этот.
    pub fn link_speaker(&self, project: ProjectId, speaker: &SpeakerKey, characters: &[CharacterId]) -> Result<(), AppError> {
        self.write(project, |tx| {
            let repo = SpeakerRepo { conn: tx };
            for &character in characters {
                CharacterRepo { conn: tx }.get(character)?;
                let has_primary = repo.links_of(character)?.iter().any(|link| link.is_primary && &link.speaker != speaker);
                repo.link(&SpeakerLink { speaker: speaker.clone(), character, is_primary: !has_primary })?;
            }
            repo.restore_discarded(speaker)
        })
    }

    pub fn unlink_speaker(&self, project: ProjectId, speaker: &SpeakerKey, character: CharacterId) -> Result<(), AppError> {
        self.write(project, |tx| SpeakerRepo { conn: tx }.unlink(speaker, character))
    }

    /// «Оставить говорящим»: запись со своим переводом подписи и родом.
    pub fn keep_as_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        self.write(project, |tx| {
            let repo = SpeakerRepo { conn: tx };
            repo.save_entry(&SpeakerEntry::new(speaker.clone(), Timestamp::now()))?;
            repo.restore_discarded(speaker)
        })
    }

    /// Перевод подписи и род записи говорящего.
    pub fn save_speaker_entry(&self, project: ProjectId, entry: &SpeakerEntry) -> Result<(), AppError> {
        self.write(project, |tx| SpeakerRepo { conn: tx }.save_entry(entry))
    }

    /// Убирает запись говорящего. Говорящий без связей вернётся в «Новые».
    pub fn delete_speaker_entry(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        self.write(project, |tx| SpeakerRepo { conn: tx }.delete_entry(speaker))
    }

    pub fn discard_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        self.write(project, |tx| SpeakerRepo { conn: tx }.discard(speaker, Timestamp::now()))
    }

    pub fn restore_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        self.write(project, |tx| SpeakerRepo { conn: tx }.restore_discarded(speaker))
    }

    /// Персонаж без говорящих: его только упоминают.
    pub fn create_character(&self, project: ProjectId, first_name: &str) -> Result<CharacterId, AppError> {
        if first_name.trim().is_empty() {
            return Err(ValidationError::Empty("first_name").into());
        }
        self.write(project, |tx| CharacterRepo { conn: tx }.save(&Character::new(first_name)))
    }

    pub fn save_character(&self, project: ProjectId, character: &Character) -> Result<(), AppError> {
        if character.id().is_none() {
            return Err(AppError::Db(DbError::NotFound));
        }
        self.write(project, |tx| CharacterRepo { conn: tx }.save(character).map(|_| ()))
    }

    /// Удаляет персонажа, его связи и заметки. Говорящие без связей и записи уходят в корзину.
    pub fn delete_character(&self, project: ProjectId, id: CharacterId) -> Result<(), AppError> {
        self.write(project, |tx| {
            let speakers = SpeakerRepo { conn: tx };
            let linked: Vec<SpeakerKey> = speakers.links_of(id)?.into_iter().map(|link| link.speaker).collect();
            CharacterRepo { conn: tx }.delete(id)?;
            NoteRepo { conn: tx }.delete_owner(&NoteOwner::Character(id))?;
            let now = Timestamp::now();
            for key in speakers.orphans(&linked)? {
                speakers.discard(&key, now)?;
            }
            Ok(())
        })
    }

    pub fn take_term(&self, project: ProjectId, name: &str, category: TermCategory) -> Result<TermId, AppError> {
        let term = Term::new(name, category)?;
        self.write(project, |tx| {
            TermRepo { conn: tx }.restore_discarded(term.name_key())?;
            TermRepo { conn: tx }.save(&term)
        })
    }

    pub fn save_term(&self, project: ProjectId, term: &Term) -> Result<(), AppError> {
        if term.id().is_none() {
            return Err(AppError::Db(DbError::NotFound));
        }
        self.write(project, |tx| TermRepo { conn: tx }.save(term).map(|_| ()))
    }

    /// Удаляет термин и его заметки: название вернётся в «Новые».
    pub fn delete_term(&self, project: ProjectId, id: TermId) -> Result<(), AppError> {
        self.write(project, |tx| {
            TermRepo { conn: tx }.delete(id)?;
            NoteRepo { conn: tx }.delete_owner(&NoteOwner::Term(id))?;
            Ok(())
        })
    }

    /// Одной транзакцией: термин удаляется, название уходит в корзину `given_to_character`,
    /// у персонажа появляется форма с переводом термина.
    pub fn give_term_to_character(&self, project: ProjectId, term: TermId, character: CharacterId) -> Result<(), AppError> {
        self.write(project, |tx| {
            let terms = TermRepo { conn: tx };
            let term = terms.get(term)?;
            add_form(tx, character, term.name(), term.translation())?;
            terms.delete(term.id().ok_or(DbError::NotFound)?)?;
            NoteRepo { conn: tx }.delete_owner(&NoteOwner::Term(term.id().ok_or(DbError::NotFound)?))?;
            terms.discard(&DiscardedTerm {
                text: term.name().to_string(),
                key: term.name_key().clone(),
                reason: DiscardReason::GivenToCharacter,
                discarded_at: Timestamp::now(),
            })
        })
    }

    /// Название из корзины ещё одному персонажу: общая фамилия достаётся всем.
    pub fn give_discarded_to_character(&self, project: ProjectId, name: &NameKey, character: CharacterId) -> Result<(), AppError> {
        self.write(project, |tx| {
            let discarded = TermRepo { conn: tx }.discarded()?.into_iter().find(|term| &term.key == name).ok_or(DbError::NotFound)?;
            add_form(tx, character, &discarded.text, None)
        })
    }

    pub fn discard_term_name(&self, project: ProjectId, name: &str) -> Result<(), AppError> {
        let key = NameKey::new(name).ok_or(ValidationError::Empty("name"))?;
        self.write(project, |tx| {
            TermRepo { conn: tx }.discard(&DiscardedTerm {
                text: name.trim().to_string(),
                key: key.clone(),
                reason: DiscardReason::Junk,
                discarded_at: Timestamp::now(),
            })
        })
    }

    pub fn restore_term_name(&self, project: ProjectId, name: &NameKey) -> Result<(), AppError> {
        self.write(project, |tx| TermRepo { conn: tx }.restore_discarded(name))
    }

    /// Пустой токен удаляется.
    pub fn save_token(&self, project: ProjectId, token: &Token) -> Result<(), AppError> {
        self.write(project, |tx| TokenRepo { conn: tx }.save(token))
    }

    /// Пустое описание удаляет строку.
    pub fn save_scene(&self, project: ProjectId, scene: &SceneDescription) -> Result<(), AppError> {
        self.write(project, |tx| SceneDescriptionRepo { conn: tx }.save(scene))
    }

    /// Сцена не передаётся модели при описании персонажа или термина.
    pub fn set_excluded_scene(&self, project: ProjectId, owner: ExcludeOwner, container: &ContainerKey, excluded: bool) -> Result<(), AppError> {
        todo!()
    }

    fn write<T>(&self, project: ProjectId, work: impl FnOnce(&rusqlite::Transaction) -> Result<T, DbError>) -> Result<T, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(work)?)
    }
}

/// Персонаж по подписи говорящего: имя это подпись, у говорящего без подписи хвост ключа.
fn character_from_speaker(conn: &Connection, key: &SpeakerKey) -> Result<CharacterId, DbError> {
    let speakers = SpeakerRepo { conn };
    let speaker = speakers.get(key)?;
    let name = speaker.label.clone().unwrap_or_else(|| key.as_str().split_once(':').map_or(key.as_str(), |(_, tail)| tail).to_string());
    let id = CharacterRepo { conn }.save(&Character::new(&name))?;
    speakers.link(&SpeakerLink { speaker: key.clone(), character: id, is_primary: true })?;
    speakers.restore_discarded(key)?;
    Ok(id)
}

/// Форма персонажа. Совпадение с именем, фамилией или другой формой не ошибка: перевод заполнит пустое поле.
fn add_form(conn: &Connection, character: CharacterId, text: &str, translation: Option<&str>) -> Result<(), DbError> {
    let repo = CharacterRepo { conn };
    let mut person = repo.get(character)?;
    let form = Form::new(text, translation).map_err(|error| DbError::bad_value("character_forms", "text", error))?;
    match person.add_form(form.clone()) {
        Ok(()) => {}
        Err(ValidationError::DuplicateForm(_)) => {
            // Повтор: перевод уходит в пустое поле совпавшей записи.
            let mut forms = person.forms().to_vec();
            forms.push(form);
            person = Character::restore(
                character,
                person.first_name().map(str::to_string),
                person.last_name().map(str::to_string),
                person.translated_first_name().map(str::to_string),
                person.translated_last_name().map(str::to_string),
                person.gender(),
                person.speech().map(str::to_string),
                person.description().map(str::to_string),
                forms,
            );
        }
        Err(error) => return Err(DbError::bad_value("character_forms", "text", error)),
    }
    repo.save(&person)?;
    Ok(())
}

/// Токены в репликах текущего снимка и число реплик с каждым.
fn game_tokens(conn: &Connection) -> Result<BTreeMap<String, usize>, DbError> {
    let mut statement = conn.prepare("SELECT source_text FROM segments WHERE missing = 0 AND instr(source_text, '[') > 0")?;
    let texts = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let mut tokens: BTreeMap<String, usize> = BTreeMap::new();
    for text in texts {
        let mut seen: Vec<&str> = crate::translation::text::token_names(&text).collect();
        seen.sort_unstable();
        seen.dedup();
        for name in seen {
            *tokens.entry(name.to_string()).or_default() += 1;
        }
    }
    Ok(tokens)
}

/// Чья сцена исключается.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludeOwner {
    Character(CharacterId),
    Term(TermId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Gender;
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    /// Открытый проект по синтетической игре: говорящие `code:e` (Eileen) и `label:Radio Chatter`.
    fn project(dir: &std::path::Path) -> (AppState, ProjectId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        (state, id)
    }

    fn new_keys(service: &GlossaryService, project: ProjectId) -> Vec<String> {
        let open = service.state.open_project(project).unwrap();
        let fresh = open.project.read(|tx| SpeakerRepo { conn: tx }.new_speakers()).unwrap();
        fresh.into_iter().map(|speaker| speaker.key.as_str().to_string()).collect()
    }

    #[test]
    fn speakers_become_characters_and_return_to_trash_with_them() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = GlossaryService { state: &state };
        let fresh = new_keys(&service, id);
        assert!(fresh.iter().any(|key| key.starts_with("code:")), "{fresh:?}");

        let eileen = SpeakerKey::new("code:e").unwrap();
        assert!(fresh.contains(&eileen.as_str().to_string()), "{fresh:?}");
        let character = service.speaker_to_character(id, &eileen).unwrap();
        assert!(!new_keys(&service, id).contains(&eileen.as_str().to_string()));

        let overview = service.overview(id).unwrap();
        let person = overview.data.characters.iter().find(|person| person.id() == Some(character)).unwrap();
        assert_eq!(person.first_name(), Some("Eileen"));
        assert!(overview.data.links.iter().any(|link| link.speaker == eileen && link.is_primary));
        assert!(overview.speakers.iter().any(|item| item.speaker.key == eileen && item.lines > 0));

        let radio = SpeakerKey::new("label:Radio Chatter").unwrap();
        service.keep_as_speaker(id, &radio).unwrap();
        let mut entry = SpeakerEntry::new(radio.clone(), Timestamp::now());
        entry.set_translation("Радио");
        entry.set_gender(Some(Gender::Male));
        service.save_speaker_entry(id, &entry).unwrap();
        assert_eq!(service.overview(id).unwrap().data.entries[0].translation(), Some("Радио"));

        service.delete_character(id, character).unwrap();
        let overview = service.overview(id).unwrap();
        assert!(overview.data.characters.is_empty());
        assert!(overview.discarded_speakers.iter().any(|speaker| speaker.key == eileen), "говорящий ушёл в корзину");
        service.restore_speaker(id, &eileen).unwrap();
        assert!(new_keys(&service, id).contains(&eileen.as_str().to_string()));
    }

    #[test]
    fn character_edits_forms_and_terms_given_to_characters() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = GlossaryService { state: &state };
        let character = service.create_character(id, "Anna").unwrap();
        let open = state.open_project(id).unwrap();
        let mut anna = open.project.read(|tx| CharacterRepo { conn: tx }.get(character)).unwrap();
        anna.set_last_name("Lee");
        anna.set_translated_names("Анна", "Ли");
        anna.add_form(Form::new("Annie", Some("Энни")).unwrap()).unwrap();
        service.save_character(id, &anna).unwrap();

        let term = service.take_term(id, "Lee Manor", TermCategory::Location).unwrap();
        let mut manor = open.project.read(|tx| TermRepo { conn: tx }.get(term)).unwrap();
        manor.set_translation("Поместье Ли");
        service.save_term(id, &manor).unwrap();
        assert!(service.take_term(id, "lee manor", TermCategory::General).is_err(), "название уже занято");

        let key = |text: &str| NameKey::new(text).unwrap();
        let known = |text: &str| open.project.read(|tx| TermRepo { conn: tx }.is_known(&key(text))).unwrap();
        assert!(known("annie") && known("Anna Lee") && known("LEE MANOR"));
        assert!(!known("Silver Key"));

        service.give_term_to_character(id, term, character).unwrap();
        let overview = service.overview(id).unwrap();
        assert!(overview.data.terms.is_empty());
        let forms = overview.data.characters[0].forms();
        assert!(forms.iter().any(|form| form.text() == "Lee Manor" && form.translation() == Some("Поместье Ли")));
        assert_eq!(overview.discarded_terms[0].reason, DiscardReason::GivenToCharacter);

        service.discard_term_name(id, "Silver Key").unwrap();
        assert!(known("silver key"));
        service.restore_term_name(id, &key("Silver Key")).unwrap();
        assert!(!known("silver key"));
    }

    #[test]
    fn tokens_and_scene_descriptions_keep_only_filled_rows() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = GlossaryService { state: &state };
        let mut token = Token::new("[mom]").unwrap();
        token.set_value("mother");
        service.save_token(id, &token).unwrap();
        assert_eq!(service.overview(id).unwrap().data.tokens.len(), 1);
        token.set_value("");
        service.save_token(id, &token).unwrap();
        assert!(service.overview(id).unwrap().data.tokens.is_empty());

        let key = ContainerKey::new("label:start").unwrap();
        let mut scene = SceneDescription::new(key.clone(), Timestamp::now());
        scene.edit("Утро", "Эйлин здоровается.", Timestamp::now());
        service.save_scene(id, &scene).unwrap();
        assert_eq!(service.overview(id).unwrap().scenes[0].title(), Some("Утро"));
        scene.edit("", "", Timestamp::now());
        service.save_scene(id, &scene).unwrap();
        assert!(service.overview(id).unwrap().scenes.is_empty());
    }

    #[test]
    fn new_terms_are_built_once_per_scan_and_skip_what_the_glossary_knows() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = GlossaryService { state: &state };
        let open = state.open_project(id).unwrap();
        let set_texts = |texts: &[&str]| {
            open.project
                .write(|tx| {
                    let ids: Vec<i64> =
                        tx.prepare("SELECT id FROM segments ORDER BY id")?.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
                    for (id, text) in ids.iter().zip(texts) {
                        tx.execute("UPDATE segments SET source_text = ?1 WHERE id = ?2", rusqlite::params![text, id])?;
                    }
                    Ok(())
                })
                .unwrap();
        };
        set_texts(&[
            "We met at the Ninja Academy.",
            "Is the Ninja Academy far, Eileen?",
            "Ask the PPF and the Silver Order.",
            "Grab the Old Lantern.",
            "so grab it, is it far?",
        ]);
        let texts = |found: Vec<TermCandidate>| found.into_iter().map(|item| (item.text, item.occurrences)).collect::<Vec<_>>();
        assert_eq!(
            texts(service.new_terms(id).unwrap()),
            [("Ninja Academy".to_string(), 2), ("Old Lantern".to_string(), 1), ("PPF".to_string(), 1), ("Silver Order".to_string(), 1)],
            "Eileen это подпись говорящего"
        );

        service.take_term(id, "Ninja Academy", TermCategory::Location).unwrap();
        service.discard_term_name(id, "PPF").unwrap();
        let mut character = Character::new("Lantern");
        character.set_last_name("Old");
        open.project.write(|tx| CharacterRepo { conn: tx }.save(&character).map(|_| ())).unwrap();
        set_texts(&["Nothing here."]);
        assert_eq!(
            texts(service.new_terms(id).unwrap()),
            [("Silver Order".to_string(), 1)],
            "термин, корзина и слова имён вычитаются, а кандидаты собраны по прежнему считыванию"
        );
    }
}
