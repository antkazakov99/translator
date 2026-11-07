//! Код языка и правило имён.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::common::ValidationError;

/// Код BCP 47 в канонической записи: `sr`, `sr-Latn`, `pt-BR`. `und` значит «неизвестен».
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LanguageTag(String);

impl LanguageTag {
    pub const UNDETERMINED: &str = "und";

    /// Проверяет подтеги и приводит регистр: язык строчными, письменность с заглавной, регион заглавными.
    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        let bad = || ValidationError::BadLanguage(text.to_string());
        let text = text.trim().replace('_', "-");
        let mut parts = Vec::new();
        for (index, part) in text.split('-').enumerate() {
            if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(bad());
            }
            let canonical = if index == 0 {
                if !(2..=8).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_alphabetic()) {
                    return Err(bad());
                }
                part.to_ascii_lowercase()
            } else if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
                let mut script = part.to_ascii_lowercase();
                script[..1].make_ascii_uppercase();
                script
            } else if (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
                || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))
            {
                part.to_ascii_uppercase()
            } else {
                part.to_ascii_lowercase()
            };
            parts.push(canonical);
        }
        Ok(Self(parts.join("-")))
    }

    pub fn undetermined() -> Self {
        Self(Self::UNDETERMINED.to_string())
    }

    pub fn is_undetermined(&self) -> bool {
        self.0 == Self::UNDETERMINED
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Правило передачи имён с языка происхождения на язык перевода. Общее на все игры.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameRule {
    from: LanguageTag,
    to: LanguageTag,
    rules: String,
}

impl NameRule {
    /// Пустые правила не сохраняются, для неизвестного происхождения правил нет.
    pub fn new(from: LanguageTag, to: LanguageTag, rules: &str) -> Result<Self, ValidationError> {
        if from.is_undetermined() || to.is_undetermined() {
            return Err(ValidationError::BadLanguage(LanguageTag::UNDETERMINED.to_string()));
        }
        let rules = rules.trim();
        if rules.is_empty() {
            return Err(ValidationError::Empty("rules"));
        }
        Ok(Self { from, to, rules: rules.to_string() })
    }

    pub fn from(&self) -> &LanguageTag {
        &self.from
    }

    pub fn to(&self) -> &LanguageTag {
        &self.to
    }

    pub fn rules(&self) -> &str {
        &self.rules
    }
}

/// Своя традиция написания имён в игре: важнее общего правила. Язык перевода берётся из проекта.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameRuleOverride {
    from: LanguageTag,
    rules: String,
}

impl NameRuleOverride {
    pub fn new(from: LanguageTag, rules: &str) -> Result<Self, ValidationError> {
        let rules = rules.trim();
        if rules.is_empty() {
            return Err(ValidationError::Empty("rules"));
        }
        Ok(Self { from, rules: rules.to_string() })
    }

    pub fn from(&self) -> &LanguageTag {
        &self.from
    }

    pub fn rules(&self) -> &str {
        &self.rules
    }
}
