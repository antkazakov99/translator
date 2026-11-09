//! Считывание игры: снимок в project.db, перенос перевода на новую версию, «Пропавшие».

pub mod commands;
pub mod matching;
pub mod model;
pub mod repo;
pub mod service;

pub use matching::{MatchKind, MatchPlan, MatchedPair, Matcher, MatchSegment};
pub use model::{FileId, MissingSegment, Scan, ScanId, ScanReport, SegmentId};
pub use repo::{EdgeRepo, ScanRepo};
pub use service::ScanService;
