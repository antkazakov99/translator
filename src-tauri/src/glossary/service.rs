//! Правки глоссария без модели: разбор «Новых», связи, передача термина персонажу, удаление.

use super::{Character, CharacterId, NameKey, SceneDescription, Term, TermCategory, TermId, Token};
use crate::common::AppError;
use crate::engines::{ContainerKey, SpeakerKey};
use crate::projects::ProjectId;
use crate::state::AppState;

pub struct GlossaryService<'a> {
    pub state: &'a AppState,
}

impl GlossaryService<'_> {
    /// «Персонаж»: создаёт персонажа с именем из подписи и связывает с ним говорящего основным кодом.
    /// Подпись, не совпавшая с именем и фамилией, становится формой.
    pub fn speaker_to_character(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<CharacterId, AppError> {
        todo!()
    }

    /// «Взять всех» в «Новых говорящих».
    pub fn all_speakers_to_characters(&self, project: ProjectId) -> Result<Vec<CharacterId>, AppError> {
        todo!()
    }

    /// «Привязать» к одному или нескольким существующим персонажам.
    pub fn link_speaker(&self, project: ProjectId, speaker: &SpeakerKey, characters: &[CharacterId]) -> Result<(), AppError> {
        todo!()
    }

    pub fn unlink_speaker(&self, project: ProjectId, speaker: &SpeakerKey, character: CharacterId) -> Result<(), AppError> {
        todo!()
    }

    /// «Оставить говорящим»: запись со своим переводом подписи и родом.
    pub fn keep_as_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        todo!()
    }

    pub fn discard_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        todo!()
    }

    pub fn restore_speaker(&self, project: ProjectId, speaker: &SpeakerKey) -> Result<(), AppError> {
        todo!()
    }

    /// Персонаж без говорящих: его только упоминают.
    pub fn create_character(&self, project: ProjectId, first_name: &str) -> Result<CharacterId, AppError> {
        todo!()
    }

    pub fn save_character(&self, project: ProjectId, character: &Character) -> Result<(), AppError> {
        todo!()
    }

    /// Удаляет персонажа, его связи и заметки. Говорящие без связей и записи уходят в корзину.
    pub fn delete_character(&self, project: ProjectId, id: CharacterId) -> Result<(), AppError> {
        todo!()
    }

    pub fn take_term(&self, project: ProjectId, name: &str, category: TermCategory) -> Result<TermId, AppError> {
        todo!()
    }

    pub fn save_term(&self, project: ProjectId, term: &Term) -> Result<(), AppError> {
        todo!()
    }

    /// Удаляет термин и его заметки: название вернётся в «Новые».
    pub fn delete_term(&self, project: ProjectId, id: TermId) -> Result<(), AppError> {
        todo!()
    }

    /// Одной транзакцией: термин удаляется, название уходит в корзину `given_to_character`,
    /// у персонажа появляется форма с переводом термина.
    pub fn give_term_to_character(&self, project: ProjectId, term: TermId, character: CharacterId) -> Result<(), AppError> {
        todo!()
    }

    /// Название из корзины ещё одному персонажу: общая фамилия достаётся всем.
    pub fn give_discarded_to_character(&self, project: ProjectId, name: &NameKey, character: CharacterId) -> Result<(), AppError> {
        todo!()
    }

    pub fn discard_term_name(&self, project: ProjectId, name: &str) -> Result<(), AppError> {
        todo!()
    }

    pub fn restore_term_name(&self, project: ProjectId, name: &NameKey) -> Result<(), AppError> {
        todo!()
    }

    pub fn save_token(&self, project: ProjectId, token: &Token) -> Result<(), AppError> {
        todo!()
    }

    pub fn save_scene(&self, project: ProjectId, scene: &SceneDescription) -> Result<(), AppError> {
        todo!()
    }

    /// Сцена не передаётся модели при описании персонажа или термина.
    pub fn set_excluded_scene(&self, project: ProjectId, owner: ExcludeOwner, container: &ContainerKey, excluded: bool) -> Result<(), AppError> {
        todo!()
    }
}

/// Чья сцена исключается.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcludeOwner {
    Character(CharacterId),
    Term(TermId),
}
