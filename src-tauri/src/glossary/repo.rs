//! Таблицы глоссария в project.db.

use rusqlite::Connection;

use super::{
    Character, CharacterId, DiscardedTerm, NameKey, NameOrigin, Note, NoteOwner, SceneDescription, Speaker, SpeakerEntry,
    SpeakerLink, Term, TermId, Token,
};
use super::terms::TermCandidate;
use crate::common::Timestamp;
use crate::engines::{ContainerKey, SpeakerKey};
use crate::storage::DbError;

/// `speakers`, `speaker_entries`, `speaker_characters`, `discarded_speakers`.
pub struct SpeakerRepo<'c> {
    pub conn: &'c Connection,
}

impl SpeakerRepo<'_> {
    /// Говорящие текущего снимка, которых пользователь ещё не разобрал.
    pub fn new_speakers(&self) -> Result<Vec<Speaker>, DbError> {
        todo!()
    }

    pub fn entries(&self) -> Result<Vec<SpeakerEntry>, DbError> {
        todo!()
    }

    pub fn save_entry(&self, entry: &SpeakerEntry) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete_entry(&self, key: &SpeakerKey) -> Result<(), DbError> {
        todo!()
    }

    pub fn links_of(&self, character: CharacterId) -> Result<Vec<SpeakerLink>, DbError> {
        todo!()
    }

    pub fn link(&self, link: &SpeakerLink) -> Result<(), DbError> {
        todo!()
    }

    pub fn unlink(&self, speaker: &SpeakerKey, character: CharacterId) -> Result<(), DbError> {
        todo!()
    }

    /// Говорящие персонажа без связей и записи: после удаления персонажа уходят в корзину.
    pub fn orphans(&self) -> Result<Vec<SpeakerKey>, DbError> {
        todo!()
    }

    pub fn discard(&self, key: &SpeakerKey, now: Timestamp) -> Result<(), DbError> {
        todo!()
    }

    pub fn restore_discarded(&self, key: &SpeakerKey) -> Result<(), DbError> {
        todo!()
    }
}

/// `characters`, `character_forms`, `name_origins`, `character_excluded_containers`.
pub struct CharacterRepo<'c> {
    pub conn: &'c Connection,
}

impl CharacterRepo<'_> {
    pub fn get(&self, id: CharacterId) -> Result<Character, DbError> {
        todo!()
    }

    pub fn list(&self) -> Result<Vec<Character>, DbError> {
        todo!()
    }

    /// Вставляет нового или заменяет поля и формы целиком. Возвращает id.
    pub fn save(&self, character: &Character) -> Result<CharacterId, DbError> {
        todo!()
    }

    /// Связи и формы удаляются каскадом, заметки удаляет сервис в той же транзакции.
    pub fn delete(&self, id: CharacterId) -> Result<(), DbError> {
        todo!()
    }

    pub fn origins(&self, id: CharacterId) -> Result<Vec<NameOrigin>, DbError> {
        todo!()
    }

    pub fn save_origins(&self, id: CharacterId, origins: &[NameOrigin]) -> Result<(), DbError> {
        todo!()
    }

    pub fn excluded_containers(&self, id: CharacterId) -> Result<Vec<ContainerKey>, DbError> {
        todo!()
    }

    pub fn set_excluded(&self, id: CharacterId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        todo!()
    }
}

/// `terms`, `term_forms`, `term_excluded_containers`, `discarded_terms`, `term_candidates`.
pub struct TermRepo<'c> {
    pub conn: &'c Connection,
}

impl TermRepo<'_> {
    pub fn get(&self, id: TermId) -> Result<Term, DbError> {
        todo!()
    }

    pub fn list(&self) -> Result<Vec<Term>, DbError> {
        todo!()
    }

    pub fn save(&self, term: &Term) -> Result<TermId, DbError> {
        todo!()
    }

    pub fn delete(&self, id: TermId) -> Result<(), DbError> {
        todo!()
    }

    /// «Новые термины»: кандидаты без терминов, имён персонажей, подписей разобранных говорящих и корзины.
    pub fn new_candidates(&self) -> Result<Vec<TermCandidate>, DbError> {
        todo!()
    }

    /// Пересобирает `term_candidates` и строку `derived` одной транзакцией.
    pub fn replace_candidates(&self, candidates: &[TermCandidate], version: u32) -> Result<(), DbError> {
        todo!()
    }

    /// Есть ли название среди терминов, имён и корзины: так проверяются предположения модели.
    pub fn is_known(&self, key: &NameKey) -> Result<bool, DbError> {
        todo!()
    }

    pub fn discarded(&self) -> Result<Vec<DiscardedTerm>, DbError> {
        todo!()
    }

    pub fn discard(&self, term: &DiscardedTerm) -> Result<(), DbError> {
        todo!()
    }

    pub fn restore_discarded(&self, key: &NameKey) -> Result<(), DbError> {
        todo!()
    }

    pub fn excluded_containers(&self, id: TermId) -> Result<Vec<ContainerKey>, DbError> {
        todo!()
    }

    pub fn set_excluded(&self, id: TermId, container: &ContainerKey, excluded: bool) -> Result<(), DbError> {
        todo!()
    }
}

/// `tokens`.
pub struct TokenRepo<'c> {
    pub conn: &'c Connection,
}

impl TokenRepo<'_> {
    pub fn list(&self) -> Result<Vec<Token>, DbError> {
        todo!()
    }

    /// Пустой токен удаляется.
    pub fn save(&self, token: &Token) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, name: &str) -> Result<(), DbError> {
        todo!()
    }
}

/// `scene_descriptions`.
pub struct SceneDescriptionRepo<'c> {
    pub conn: &'c Connection,
}

impl SceneDescriptionRepo<'_> {
    pub fn get(&self, key: &ContainerKey) -> Result<Option<SceneDescription>, DbError> {
        todo!()
    }

    pub fn list(&self) -> Result<Vec<SceneDescription>, DbError> {
        todo!()
    }

    /// Пустое описание удаляет строку.
    pub fn save(&self, scene: &SceneDescription) -> Result<(), DbError> {
        todo!()
    }
}

/// `notes`.
pub struct NoteRepo<'c> {
    pub conn: &'c Connection,
}

impl NoteRepo<'_> {
    pub fn of(&self, owner: &NoteOwner, model: &str) -> Result<Vec<Note>, DbError> {
        todo!()
    }

    pub fn save(&self, note: &Note) -> Result<(), DbError> {
        todo!()
    }

    pub fn set_excluded(&self, owner: &NoteOwner, model: &str, part_hash: &str, excluded: bool) -> Result<(), DbError> {
        todo!()
    }

    /// Явный сброс или удаление владельца.
    pub fn delete_owner(&self, owner: &NoteOwner) -> Result<usize, DbError> {
        todo!()
    }

    /// После повторного считывания: удаляет заметки, чьих частей нет среди всех частей владельца.
    pub fn retain_parts(&self, owner: &NoteOwner, part_hashes: &[String]) -> Result<usize, DbError> {
        todo!()
    }
}
