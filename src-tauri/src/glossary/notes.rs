//! Заметки модели по частям сцен. Стоят времени и денег, поэтому лежат в project.db.

use serde::Serialize;

use super::{CharacterId, TermId};
use crate::common::{Timestamp, ValidationError};
use crate::engines::ContainerKey;

/// Чья заметка.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum NoteOwner {
    Character(CharacterId),
    Term(TermId),
    Container(ContainerKey),
}

impl NoteOwner {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Character(_) => "character",
            Self::Term(_) => "term",
            Self::Container(_) => "container",
        }
    }

    pub fn reference(&self) -> String {
        match self {
            Self::Character(id) => id.get().to_string(),
            Self::Term(id) => id.get().to_string(),
            Self::Container(key) => key.as_str().to_string(),
        }
    }

    pub fn parse(kind: &str, reference: &str) -> Result<Self, ValidationError> {
        let bad = || ValidationError::UnknownValue { field: "notes.owner", value: format!("{kind}:{reference}") };
        match kind {
            "character" => reference.parse().map(|id| Self::Character(CharacterId::from_row(id))).map_err(|_| bad()),
            "term" => reference.parse().map(|id| Self::Term(TermId::from_row(id))).map_err(|_| bad()),
            "container" => ContainerKey::new(reference).map(Self::Container),
            _ => Err(bad()),
        }
    }
}

/// Заметка по одной части. Ключ зависит от текста части: другая модель или изменённая сцена дают новую заметку.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    pub owner: NoteOwner,
    pub model: String,
    /// Отпечаток текста части.
    pub part_hash: String,
    pub text: String,
    pub created_at: Timestamp,
}
