//! Говорящие: кто говорит в игре по снимку, запись говорящего и связи с персонажами.

use serde::Serialize;

use super::{CharacterId, NameKey};
use crate::common::{Gender, Timestamp, non_empty};
use crate::engines::SpeakerKey;

/// Говорящий из снимка (`speakers`). Это не персонаж: роль без персонажа или подпись на нескольких.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Speaker {
    pub key: SpeakerKey,
    /// Значения задаёт движок: `code`, `label`.
    pub kind: String,
    /// Подпись, которую показывает игра.
    pub label: Option<String>,
}

impl Speaker {
    pub fn label_key(&self) -> Option<NameKey> {
        self.label.as_deref().and_then(NameKey::new)
    }
}

/// Запись говорящего (`speaker_entries`): перевод подписи и род. У связанного с персонажами говорящего
/// её перевод подписи важнее перевода по именам.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpeakerEntry {
    key: SpeakerKey,
    translation: Option<String>,
    gender: Option<Gender>,
    created_at: Timestamp,
}

impl SpeakerEntry {
    /// «Оставить говорящим» из «Новых».
    pub fn new(key: SpeakerKey, now: Timestamp) -> Self {
        Self { key, translation: None, gender: None, created_at: now }
    }

    pub fn restore(key: SpeakerKey, translation: Option<String>, gender: Option<Gender>, created_at: Timestamp) -> Self {
        Self { key, translation, gender, created_at }
    }

    pub fn set_translation(&mut self, text: &str) {
        self.translation = non_empty(text);
    }

    pub fn set_gender(&mut self, gender: Option<Gender>) {
        self.gender = gender;
    }

    pub fn key(&self) -> &SpeakerKey {
        &self.key
    }

    pub fn translation(&self) -> Option<&str> {
        self.translation.as_deref()
    }

    pub fn gender(&self) -> Option<Gender> {
        self.gender
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }
}

/// Связь говорящего с персонажем (`speaker_characters`). Основной код подписывает персонажа без имени, он один.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpeakerLink {
    pub speaker: SpeakerKey,
    pub character: CharacterId,
    pub is_primary: bool,
}

/// Род говорящего в подсказках модели: свой у записи, иначе у единственного связанного персонажа.
/// `None` у говорящего с несколькими персонажами без своего рода: согласование во множественном числе.
pub fn speaker_gender(entry: Option<&SpeakerEntry>, linked: &[Option<Gender>]) -> Option<Gender> {
    entry.and_then(SpeakerEntry::gender).or(match linked {
        [only] => *only,
        _ => None,
    })
}
