//! Глоссарий: говорящие и их связи с персонажами, персонажи, термины, токены, описания сцен, заметки модели.

pub mod analyze;
pub mod characters;
pub mod commands;
pub mod describe;
pub mod describe_prompt;
pub mod entries;
pub mod key;
pub mod names;
pub mod notes;
pub mod parts;
pub mod repo;
pub mod scenes;
pub mod service;
pub mod speakers;
pub mod suggest;
pub mod terms;
pub mod tokens;

pub use analyze::{Finding, FoundKind, NameParts};
pub use characters::{Character, CharacterId, Form, NameField, NameOrigin};
pub(crate) use describe::prune_notes;
pub use describe::{DescribeService, DescribeTarget, Described, NamesFound, NamesService, Pending, ReadingDepth, Suggestions};
pub use key::NameKey;
pub use notes::{Note, NoteOwner};
pub use repo::{CharacterRepo, LineSampleRepo, NoteRepo, ReadingDepthRepo, SampleLine, SceneDescriptionRepo, StoryLine, SpeakerRepo, SpeakerUse, TermRepo, TokenRepo};
pub use names::{NameSuggestion, NameVariant};
pub use suggest::{SpeakerSuggestion, TermSuggestion, TokenSuggestion};
pub use scenes::{SceneDescription, SceneSource};
pub use service::{ExcludeOwner, GlossaryData, GlossaryOverview, GlossaryService, NamePick};
pub use speakers::{Speaker, SpeakerEntry, SpeakerLink, speaker_gender};
pub use terms::{DiscardReason, DiscardedTerm, Term, TermCategory, TermId};
pub use tokens::Token;
