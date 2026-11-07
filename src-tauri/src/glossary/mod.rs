//! Глоссарий: говорящие и их связи с персонажами, персонажи, термины, токены, описания сцен, заметки модели.

pub mod characters;
pub mod describe;
pub mod key;
pub mod notes;
pub mod repo;
pub mod scenes;
pub mod service;
pub mod speakers;
pub mod terms;
pub mod tokens;

pub use characters::{Character, CharacterId, Form, NameField, NameOrigin};
pub use describe::{DescribeService, DescribeTarget, NamesService, ReadingDepth};
pub use key::NameKey;
pub use notes::{Note, NoteOwner};
pub use repo::{CharacterRepo, NoteRepo, SceneDescriptionRepo, SpeakerRepo, TermRepo, TokenRepo};
pub use scenes::SceneDescription;
pub use service::{ExcludeOwner, GlossaryService};
pub use speakers::{Speaker, SpeakerEntry, SpeakerLink, speaker_gender};
pub use terms::{DiscardReason, DiscardedTerm, Term, TermCandidate, TermCategory, TermId};
pub use tokens::Token;
