//! Токены: переменные в тексте игры вроде `[mc]`.

use serde::Serialize;

use crate::common::{ValidationError, non_empty};

/// Токен. Значение заменяет его целиком при чтении сцен и переводе, без значения модель видит пояснение.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Token {
    /// Без скобок: `mc`, `friend.name`.
    name: String,
    value: Option<String>,
    note: Option<String>,
}

impl Token {
    pub fn new(name: &str) -> Result<Self, ValidationError> {
        let name = name.trim().trim_start_matches('[').trim_end_matches(']').trim();
        if name.is_empty() {
            return Err(ValidationError::Empty("token"));
        }
        Ok(Self { name: name.to_string(), value: None, note: None })
    }

    pub fn restore(name: &str, value: Option<String>, note: Option<String>) -> Result<Self, ValidationError> {
        let mut token = Self::new(name)?;
        token.value = value;
        token.note = note;
        Ok(token)
    }

    pub fn set_value(&mut self, text: &str) {
        self.value = non_empty(text);
    }

    pub fn set_note(&mut self, text: &str) {
        self.note = non_empty(text);
    }

    /// Как токен записан в тексте игры.
    pub fn placeholder(&self) -> String {
        format!("[{}]", self.name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Пустая запись не хранится.
    pub fn is_empty(&self) -> bool {
        self.value.is_none() && self.note.is_none()
    }
}
