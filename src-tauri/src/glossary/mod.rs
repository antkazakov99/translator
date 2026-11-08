//! Глоссарий: говорящие и их связи с персонажами, персонажи, термины, токены, описания сцен, заметки модели.

pub mod candidates;
pub mod characters;
pub mod commands;
pub mod describe;
pub mod key;
pub mod names;
pub mod notes;
pub mod repo;
pub mod scenes;
pub mod service;
pub mod speakers;
pub mod suggest;
pub mod terms;
pub mod tokens;

pub use characters::{Character, CharacterId, Form, NameField, NameOrigin};
pub use describe::{DescribeService, DescribeTarget, NamesService, ReadingDepth, Suggestions};
pub use key::NameKey;
pub use notes::{Note, NoteOwner};
pub use repo::{CharacterRepo, LineSampleRepo, NoteRepo, SampleLine, SceneDescriptionRepo, SpeakerRepo, SpeakerUse, TermRepo, TokenRepo};
pub use names::{NameSuggestion, NameVariant};
pub use suggest::{SpeakerSuggestion, TermSuggestion, TokenSuggestion};
pub use scenes::SceneDescription;
pub use service::{ExcludeOwner, GlossaryData, GlossaryOverview, GlossaryService};
pub use speakers::{Speaker, SpeakerEntry, SpeakerLink, speaker_gender};
pub use terms::{DiscardReason, DiscardedTerm, Term, TermCandidate, TermCategory, TermId};
pub use tokens::Token;
