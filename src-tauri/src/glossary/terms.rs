//! Термины и корзина названий.

use serde::Serialize;

use super::{Form, NameKey};
use crate::common::{Timestamp, ValidationError, non_empty, row_id};

row_id!(TermId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum TermCategory {
    General,
    Location,
    Organization,
    Title,
    Item,
    Event,
}

impl TermCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Location => "location",
            Self::Organization => "organization",
            Self::Title => "title",
            Self::Item => "item",
            Self::Event => "event",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "general" => Ok(Self::General),
            "location" => Ok(Self::Location),
            "organization" => Ok(Self::Organization),
            "title" => Ok(Self::Title),
            "item" => Ok(Self::Item),
            "event" => Ok(Self::Event),
            _ => Err(ValidationError::UnknownValue { field: "terms.category", value: text.to_string() }),
        }
    }
}

/// Термин глоссария.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Term {
    id: Option<TermId>,
    name: String,
    name_key: NameKey,
    translation: Option<String>,
    category: TermCategory,
    description: Option<String>,
    forms: Vec<Form>,
}

impl Term {
    /// Новый термин: взят пользователем или из отбора разбора сцены.
    pub fn new(name: &str, category: TermCategory) -> Result<Self, ValidationError> {
        let name_key = NameKey::new(name).ok_or(ValidationError::Empty("name"))?;
        Ok(Self {
            id: None,
            name: name.trim().to_string(),
            name_key,
            translation: None,
            category,
            description: None,
            forms: Vec::new(),
        })
    }

    pub fn restore(
        id: TermId,
        name: &str,
        translation: Option<String>,
        category: TermCategory,
        description: Option<String>,
        forms: Vec<Form>,
    ) -> Result<Self, ValidationError> {
        let mut term = Self::new(name, category)?;
        term.id = Some(id);
        term.translation = translation;
        term.description = description;
        term.forms = forms;
        Ok(term)
    }

    pub fn rename(&mut self, name: &str) -> Result<(), ValidationError> {
        self.name_key = NameKey::new(name).ok_or(ValidationError::Empty("name"))?;
        self.name = name.trim().to_string();
        self.forms.retain(|form| form.key() != &self.name_key);
        Ok(())
    }

    pub fn set_translation(&mut self, text: &str) {
        self.translation = non_empty(text);
    }

    pub fn set_category(&mut self, category: TermCategory) {
        self.category = category;
    }

    pub fn set_description(&mut self, text: &str) {
        self.description = non_empty(text);
    }

    pub fn add_form(&mut self, form: Form) -> Result<(), ValidationError> {
        if form.key() == &self.name_key || self.forms.iter().any(|own| own.key() == form.key()) {
            return Err(ValidationError::DuplicateForm(form.text().to_string()));
        }
        self.forms.push(form);
        Ok(())
    }

    pub fn remove_form(&mut self, key: &NameKey) {
        self.forms.retain(|form| form.key() != key);
    }

    pub fn id(&self) -> Option<TermId> {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn name_key(&self) -> &NameKey {
        &self.name_key
    }

    pub fn translation(&self) -> Option<&str> {
        self.translation.as_deref()
    }

    pub fn category(&self) -> TermCategory {
        self.category
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn forms(&self) -> &[Form] {
        &self.forms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscardReason {
    /// Мусорное название.
    Junk,
    /// Передано персонажу: из корзины его можно передать ещё одному.
    GivenToCharacter,
}

impl DiscardReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Junk => "junk",
            Self::GivenToCharacter => "given_to_character",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "junk" => Ok(Self::Junk),
            "given_to_character" => Ok(Self::GivenToCharacter),
            _ => Err(ValidationError::UnknownValue { field: "discarded_terms.reason", value: text.to_string() }),
        }
    }
}

/// Поля термина, который пользователь убрал в корзину: с ними он возвращается.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeptTerm {
    pub translation: Option<String>,
    pub category: TermCategory,
    pub description: Option<String>,
    pub forms: Vec<Form>,
}

/// Название в корзине: разбор сцены его не предлагает.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscardedTerm {
    pub text: String,
    pub key: NameKey,
    pub reason: DiscardReason,
    pub discarded_at: Timestamp,
    /// Поля удалённого термина. `None`: в корзине только название (мусор от разбора, название, переданное персонажу).
    pub kept: Option<KeptTerm>,
}

impl DiscardedTerm {
    /// Только название: мусор от разбора сцены или название, переданное персонажу.
    pub fn name(text: &str, key: NameKey, reason: DiscardReason, now: Timestamp) -> Self {
        Self { text: text.trim().to_string(), key, reason, discarded_at: now, kept: None }
    }

    /// Термин, который пользователь убрал в корзину: поля остаются для возврата.
    pub fn trashed(term: &Term, now: Timestamp) -> Self {
        Self {
            text: term.name.clone(),
            key: term.name_key.clone(),
            reason: DiscardReason::Junk,
            discarded_at: now,
            kept: Some(KeptTerm {
                translation: term.translation.clone(),
                category: term.category,
                description: term.description.clone(),
                forms: term.forms.clone(),
            }),
        }
    }

    /// Термин для возврата из корзины: с сохранёнными полями, без них категории `general`. Форма, которая
    /// повторяет название или другую форму, пропускается.
    pub fn into_term(self) -> Result<Term, ValidationError> {
        let Some(kept) = self.kept else {
            return Term::new(&self.text, TermCategory::General);
        };
        let mut term = Term::new(&self.text, kept.category)?;
        term.translation = kept.translation;
        term.description = kept.description;
        for form in kept.forms {
            let _ = term.add_form(form);
        }
        Ok(term)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trashed_term_returns_with_its_fields() {
        let mut term = Term::new("People's Police Force", TermCategory::Organization).unwrap();
        term.set_translation("Народная полиция");
        term.set_description("Полиция города.");
        term.add_form(Form::new("PPF", Some("НП")).unwrap()).unwrap();

        let discarded = DiscardedTerm::trashed(&term, Timestamp::from_secs(1));
        assert_eq!(discarded.reason, DiscardReason::Junk);
        assert_eq!(discarded.key, *term.name_key());
        assert_eq!(discarded.into_term().unwrap(), term);
    }

    #[test]
    fn name_without_fields_returns_as_general_term() {
        let key = NameKey::new("Silver Key").unwrap();
        let discarded = DiscardedTerm::name(" Silver Key ", key, DiscardReason::GivenToCharacter, Timestamp::from_secs(1));
        let term = discarded.into_term().unwrap();
        assert_eq!((term.name(), term.category(), term.translation()), ("Silver Key", TermCategory::General, None));
    }
}
