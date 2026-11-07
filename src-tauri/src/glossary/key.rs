//! Ключ сравнения имён и названий.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// NFC, Unicode-свёртка регистра, без пробелов по краям. Так «Город» и «город» совпадают, чего не даёт `COLLATE NOCASE`.
/// Смена правила требует миграции, которая пересчитает все `*_key`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NameKey(String);

impl NameKey {
    /// `None` для пустого текста: у пустого имени ключа нет.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let nfc: String = text.nfc().collect();
        let folded = caseless::default_case_fold_str(&nfc);
        Some(Self(folded.nfc().collect()))
    }

    /// Ключ из базы как есть: его уже посчитал Rust при записи.
    pub fn from_stored(key: String) -> Self {
        Self(key)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::NameKey;

    #[test]
    fn folds_case_beyond_ascii() {
        assert_eq!(NameKey::new(" Город "), NameKey::new("город"));
        assert_eq!(NameKey::new("STRASSE"), NameKey::new("straße"));
        assert_eq!(NameKey::new("   "), None);
    }

    #[test]
    fn normalizes_composed_letters() {
        // «й» одной буквой и «и» с кратким знаком.
        assert_eq!(NameKey::new("\u{0439}"), NameKey::new("\u{0438}\u{0306}"));
    }
}
