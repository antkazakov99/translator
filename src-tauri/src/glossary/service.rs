//! Правки глоссария без модели: разбор «Новых», связи, передача термина персонажу, удаление.

use std::collections::BTreeMap;

use rusqlite::Connection;
use super::{NameField, SpeakerSuggestion, TermSuggestion, TokenSuggestion};

use super::entries::{EntryLine, character_lines, character_names, scene_entries, term_lines, term_names};
use super::{
    Character, CharacterId, CharacterRepo, DiscardReason, DiscardedTerm, Form, LineSampleRepo, NameKey, NoteOwner, NoteRepo,
    SceneDescription, SceneDescriptionRepo, Speaker, SpeakerEntry, SpeakerLink, SpeakerRepo, SpeakerUse, Term,
    TermCategory, TermId, TermRepo, Token, TokenRepo,
};
use crate::common::{AppError, Timestamp, ValidationError};
use crate::engines::{ContainerKey, SpeakerKey};
use crate::projects::ProjectId;
use crate::state::AppState;
use crate::storage::DbError;
use crate::translation::TranslationRepo;

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
        self.write(project, |tx| give_term(tx, term, character))
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

    /// Правка названия и описания сцены пользователем. Пустые поля удаляют строку.
    pub fn edit_scene(&self, project: ProjectId, key: &ContainerKey, title: &str, description: &str) -> Result<(), AppError> {
        self.write(project, |tx| {
            let repo = SceneDescriptionRepo { conn: tx };
            let mut scene = repo.get(key)?.unwrap_or_else(|| SceneDescription::new(key.clone(), Timestamp::now()));
            scene.edit(title, description, Timestamp::now());
            repo.save(&scene)
        })
    }

    /// Персонажи и термины сцены: говорят или упомянуты в её репликах, термин назван. Порядок глоссария.
    pub fn scene_entries(&self, project: ProjectId, key: &ContainerKey) -> Result<(Vec<Character>, Vec<Term>), AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let (characters, terms) = scene_entries(&glossary, &LineSampleRepo { conn: tx }.scene_lines(key)?);
            let characters = glossary.characters.iter().filter(|item| item.id().is_some_and(|id| characters.contains(&id))).cloned().collect();
            let terms = glossary.terms.iter().filter(|item| item.id().is_some_and(|id| terms.contains(&id))).cloned().collect();
            Ok((characters, terms))
        })?)
    }

    /// Реплики персонажа или термина по сюжету с переводом и местом в игре. Исключённые из описания
    /// сцены здесь не исключаются: это список всех реплик записи. Удалённой записи соответствует пустой список.
    pub fn entry_lines(&self, project: ProjectId, owner: ExcludeOwner) -> Result<Vec<EntryLine>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            Ok(match owner {
                ExcludeOwner::Character(id) => {
                    let Some(character) = glossary.characters.iter().find(|character| character.id() == Some(id)) else { return Ok(Vec::new()) };
                    let speakers = glossary.links.iter().filter(|link| link.character == id).map(|link| link.speaker.clone()).collect();
                    character_lines(TranslationRepo { conn: tx }.story()?, &speakers, &character_names(character))
                }
                ExcludeOwner::Term(id) => {
                    let Some(term) = glossary.terms.iter().find(|term| term.id() == Some(id)) else { return Ok(Vec::new()) };
                    term_lines(TranslationRepo { conn: tx }.story()?, &term_names(term))
                }
            })
        })?)
    }

    /// Сцена не передаётся модели при описании персонажа или термина.
    pub fn set_excluded_scene(&self, project: ProjectId, owner: ExcludeOwner, container: &ContainerKey, excluded: bool) -> Result<(), AppError> {
        self.write(project, |tx| match owner {
            ExcludeOwner::Character(id) => CharacterRepo { conn: tx }.set_excluded(id, container, excluded),
            ExcludeOwner::Term(id) => TermRepo { conn: tx }.set_excluded(id, container, excluded),
        })
    }

    /// Глубина чтения для описаний моделью: одна на приложение.
    pub fn reading_depth(&self) -> Result<super::ReadingDepth, AppError> {
        super::ReadingDepth::load(self.state)
    }

    pub fn save_reading_depth(&self, depth: &super::ReadingDepth) -> Result<(), AppError> {
        Ok(self.state.app.write(|tx| super::ReadingDepthRepo { conn: tx }.save(depth))?)
    }

    /// Переводы терминов от модели без отбора: только термину без перевода. Категория меняется на названную
    /// моделью, описание пишется в пустое. Возвращает число записанных.
    pub fn accept_terms(&self, project: ProjectId, items: &[TermSuggestion]) -> Result<usize, AppError> {
        self.write(project, |tx| {
            let repo = TermRepo { conn: tx };
            let mut written = 0;
            for item in items {
                let mut term = match repo.get(item.id) {
                    Ok(term) => term,
                    Err(DbError::NotFound) => continue,
                    Err(error) => return Err(error),
                };
                if term.translation().is_some() || item.translation.trim().is_empty() {
                    continue;
                }
                term.set_translation(&item.translation);
                if let Some(category) = item.category {
                    term.set_category(category);
                }
                if let (None, Some(description)) = (term.description(), &item.description) {
                    term.set_description(description);
                }
                repo.save(&term)?;
                written += 1;
            }
            Ok(written)
        })
    }

    /// Переводы подписей от модели без отбора: только записи говорящего без перевода, род в пустое поле.
    pub fn accept_speakers(&self, project: ProjectId, items: &[SpeakerSuggestion]) -> Result<usize, AppError> {
        self.write(project, |tx| {
            let repo = SpeakerRepo { conn: tx };
            let entries = repo.entries()?;
            let mut written = 0;
            for item in items {
                let Some(entry) = entries.iter().find(|entry| entry.key() == &item.key) else { continue };
                if entry.translation().is_some() || item.translation.trim().is_empty() {
                    continue;
                }
                let mut entry = entry.clone();
                entry.set_translation(&item.translation);
                if entry.gender().is_none() {
                    entry.set_gender(item.gender);
                }
                repo.save_entry(&entry)?;
                written += 1;
            }
            Ok(written)
        })
    }

    /// Значения и пояснения токенов от модели без отбора: только в пустые поля.
    pub fn accept_tokens(&self, project: ProjectId, items: &[TokenSuggestion]) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| {
            let repo = TokenRepo { conn: tx };
            let tokens = repo.list()?;
            let mut written = 0;
            for item in items {
                let mut token = match tokens.iter().find(|token| token.name() == item.name) {
                    Some(token) => token.clone(),
                    None => match Token::new(&item.name) {
                        Ok(token) => token,
                        Err(_) => continue,
                    },
                };
                let mut changed = false;
                if let (None, Some(value)) = (token.value(), item.value.as_deref().filter(|value| !value.trim().is_empty())) {
                    token.set_value(value);
                    changed = true;
                }
                if let (None, Some(note)) = (token.note(), item.note.as_deref().filter(|note| !note.trim().is_empty())) {
                    token.set_note(note);
                    changed = true;
                }
                if changed {
                    repo.save(&token)?;
                    written += 1;
                }
            }
            Ok(written)
        })?)
    }

    /// Написания имён в пустые поля персонажей: выбор окна или первый вариант модели. Возвращает число
    /// изменённых персонажей.
    pub fn fill_names(&self, project: ProjectId, picks: &[NamePick]) -> Result<usize, AppError> {
        self.write(project, |tx| {
            let repo = CharacterRepo { conn: tx };
            let mut ids: Vec<CharacterId> = picks.iter().map(|pick| pick.character).collect();
            ids.dedup();
            let mut written = 0;
            for id in ids {
                let mut character = match repo.get(id) {
                    Ok(character) => character,
                    Err(DbError::NotFound) => continue,
                    Err(error) => return Err(error),
                };
                let mut changed = false;
                for pick in picks.iter().filter(|pick| pick.character == id) {
                    changed |= character.fill_spelling(pick.field, &pick.source, &pick.translation);
                }
                if changed {
                    repo.save(&character)?;
                    written += 1;
                }
            }
            Ok(written)
        })
    }

    fn write<T>(&self, project: ProjectId, work: impl FnOnce(&rusqlite::Transaction) -> Result<T, DbError>) -> Result<T, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(work)?)
    }
}

/// Написание одного поля персонажа для записи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePick {
    pub character: CharacterId,
    pub field: NameField,
    /// Написание в игре: у формы по нему находится сама форма.
    pub source: String,
    pub translation: String,
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
/// Термин уходит персонажу формой с переводом термина: термин и его заметки удаляются, название уходит
/// в корзину `given_to_character`.
pub(crate) fn give_term(tx: &Connection, term: TermId, character: CharacterId) -> Result<(), DbError> {
    let terms = TermRepo { conn: tx };
    let term = terms.get(term)?;
    let id = term.id().ok_or(DbError::NotFound)?;
    add_form(tx, character, term.name(), term.translation())?;
    terms.delete(id)?;
    NoteRepo { conn: tx }.delete_owner(&NoteOwner::Term(id))?;
    terms.discard(&DiscardedTerm {
        text: term.name().to_string(),
        key: term.name_key().clone(),
        reason: DiscardReason::GivenToCharacter,
        discarded_at: Timestamp::now(),
    })
}

/// Термин `from` сливается в `into`: название и формы `from` становятся формами `into` со своими переводами,
/// описание пишется в пустое, исключённые сцены объединяются, `from` и его заметки удаляются.
pub(crate) fn merge_terms(tx: &Connection, from: TermId, into: TermId) -> Result<(), DbError> {
    if from == into {
        return Ok(());
    }
    let terms = TermRepo { conn: tx };
    let (source, mut target) = (terms.get(from)?, terms.get(into)?);
    let names = std::iter::once((source.name(), source.translation())).chain(source.forms().iter().map(|form| (form.text(), form.translation())));
    for (text, translation) in names {
        if let Ok(form) = Form::new(text, translation) {
            let _ = target.add_form(form);
        }
    }
    if let (None, Some(description)) = (target.description(), source.description()) {
        target.set_description(description);
    }
    let excluded = terms.excluded_containers(from)?;
    terms.delete(from)?;
    NoteRepo { conn: tx }.delete_owner(&NoteOwner::Term(from))?;
    terms.save(&target)?;
    for container in excluded {
        terms.set_excluded(into, &container, true)?;
    }
    Ok(())
}

/// Персонаж `from` сливается в `into`: полное имя `from` и его формы становятся формами `into` с переводами,
/// связи говорящих переходят без основного кода, род, описание и манера речи пишутся в пустое, исключённые
/// сцены объединяются, `from` и его заметки удаляются.
pub(crate) fn merge_characters(tx: &Connection, from: CharacterId, into: CharacterId) -> Result<(), DbError> {
    if from == into {
        return Ok(());
    }
    let characters = CharacterRepo { conn: tx };
    let speakers = SpeakerRepo { conn: tx };
    let source = characters.get(from)?;
    let links = speakers.links_of(from)?;
    let excluded = characters.excluded_containers(from)?;
    characters.delete(from)?;
    NoteRepo { conn: tx }.delete_owner(&NoteOwner::Character(from))?;
    let names = source
        .full_name()
        .map(|name| (name, source.translated_full_name()))
        .into_iter()
        .chain(source.forms().iter().map(|form| (form.text().to_string(), form.translation().map(str::to_string))));
    for (text, translation) in names {
        add_form(tx, into, &text, translation.as_deref())?;
    }
    let mut target = characters.get(into)?;
    if target.gender().is_none() {
        target.set_gender(source.gender());
    }
    if let (None, Some(description)) = (target.description(), source.description()) {
        target.set_description(description);
    }
    if let (None, Some(speech)) = (target.speech(), source.speech()) {
        target.set_speech(speech);
    }
    characters.save(&target)?;
    for link in links {
        speakers.link(&SpeakerLink { speaker: link.speaker, character: into, is_primary: false })?;
    }
    for container in excluded {
        characters.set_excluded(into, &container, true)?;
    }
    Ok(())
}

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
pub(crate) fn game_tokens(conn: &Connection) -> Result<BTreeMap<String, usize>, DbError> {
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
    fn entry_lines_follow_story_with_translation_and_place() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = GlossaryService { state: &state };
        let eileen = service.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();

        let lines = |owner| service.entry_lines(id, owner).unwrap();
        let found = lines(ExcludeOwner::Character(eileen));
        let texts: Vec<(&str, bool)> = found.iter().map(|line| (line.placed.line.source_text.as_str(), line.spoken)).collect();
        assert_eq!(texts, [("Hello.", true), ("Hello.", true), ("Left side.", true)], "реплики говорящего по сюжету");
        let scenes: Vec<&str> = found.iter().filter_map(|line| line.placed.scene.as_ref().map(ContainerKey::as_str)).collect();
        assert_eq!(scenes, ["label:start", "label:start", "label:left"]);
        assert!(found.iter().all(|line| line.placed.line.location.is_some()), "номер строки ведёт в исходник");

        let first = found[0].placed.line.id;
        crate::translation::TranslationService { state: &state }.save_text(id, first, "Привет.").unwrap();
        let mut static_noise = Character::new("Static");
        static_noise.set_description("Помехи в эфире");
        let open = state.open_project(id).unwrap();
        let noise = open.project.write(|tx| CharacterRepo { conn: tx }.save(&static_noise)).unwrap();
        let mentioned = lines(ExcludeOwner::Character(noise));
        assert_eq!(mentioned.len(), 1);
        assert!(!mentioned[0].spoken, "персонажа без говорящих только упоминают");
        assert_eq!(mentioned[0].placed.line.speaker.as_deref(), Some("Radio Chatter"));
        assert_eq!(lines(ExcludeOwner::Character(eileen))[0].placed.line.text, "Привет.");
        assert_eq!(found[0].placed.file, mentioned[0].placed.file);

        let term = service.take_term(id, "narration", TermCategory::General).unwrap();
        let narration = lines(ExcludeOwner::Term(term));
        assert_eq!(narration.iter().map(|line| line.placed.line.source_text.as_str()).collect::<Vec<_>>(), ["Narration."]);

        service.delete_character(id, noise).unwrap();
        assert!(lines(ExcludeOwner::Character(noise)).is_empty(), "удалённой записи нечего показать");
    }
}
