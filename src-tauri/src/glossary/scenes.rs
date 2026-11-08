//! Описания сцен. Лежат отдельно от снимка и связаны с ним по ключу контейнера.

use serde::Serialize;

use crate::common::{Timestamp, non_empty};
use crate::engines::ContainerKey;

/// Название и описание сцены. Строка есть, пока заполнено хотя бы одно поле.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneDescription {
    key: ContainerKey,
    title: Option<String>,
    description: Option<String>,
    /// Какой текст прочитала модель. Не совпал с текущим: игра изменила сцену после чтения.
    text_hash: Option<String>,
    updated_at: Timestamp,
}

impl SceneDescription {
    pub fn new(key: ContainerKey, now: Timestamp) -> Self {
        Self { key, title: None, description: None, text_hash: None, updated_at: now }
    }

    pub fn restore(
        key: ContainerKey,
        title: Option<String>,
        description: Option<String>,
        text_hash: Option<String>,
        updated_at: Timestamp,
    ) -> Self {
        Self { key, title, description, text_hash, updated_at }
    }

    /// Правка пользователя.
    pub fn edit(&mut self, title: &str, description: &str, now: Timestamp) {
        self.title = non_empty(title);
        self.description = non_empty(description);
        self.updated_at = now;
    }

    /// Описание от модели пишется только в пустое поле. Возвращает, записано ли.
    pub fn fill_description(&mut self, text: &str, text_hash: &str, now: Timestamp) -> bool {
        if self.description.is_some() {
            return false;
        }
        let Some(text) = non_empty(text) else { return false };
        self.description = Some(text);
        self.text_hash = Some(text_hash.to_string());
        self.updated_at = now;
        true
    }

    /// Описание от запуска, о котором попросил пользователь: заменяет прежнее.
    pub fn replace_description(&mut self, text: &str, text_hash: &str, now: Timestamp) -> bool {
        let Some(text) = non_empty(text) else { return false };
        self.description = Some(text);
        self.text_hash = Some(text_hash.to_string());
        self.updated_at = now;
        true
    }

    /// Название от модели: тоже только в пустое поле, заполненное мог задать пользователь.
    pub fn fill_title(&mut self, text: &str, now: Timestamp) -> bool {
        if self.title.is_some() {
            return false;
        }
        let Some(text) = non_empty(text) else { return false };
        self.title = Some(text);
        self.updated_at = now;
        true
    }

    /// Игра изменила сцену после чтения.
    pub fn is_stale(&self, current_hash: &str) -> bool {
        self.text_hash.as_deref().is_some_and(|hash| hash != current_hash)
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none()
    }

    pub fn key(&self) -> &ContainerKey {
        &self.key
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn text_hash(&self) -> Option<&str> {
        self.text_hash.as_deref()
    }

    pub fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

/// Сцена текущего снимка для раздела «Сцены»: файл первой реплики, число реплик, отпечаток текста.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSource {
    pub key: ContainerKey,
    pub file: Option<String>,
    pub lines: usize,
    /// Отпечаток оригиналов реплик (`parts::scene_hash`).
    pub text_hash: String,
}
