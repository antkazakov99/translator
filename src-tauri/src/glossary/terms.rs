//! Термины и корзина названий.

use serde::Serialize;

use super::{Form, NameKey};
use crate::common::{Timestamp, ValidationError, non_empty, row_id};

row_id!(TermId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

/// Термин. Предположение модели (`guess`) идёт в запросы с пометкой «не подтверждено».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Term {
    id: Option<TermId>,
    name: String,
    name_key: NameKey,
    translation: Option<String>,
    category: TermCategory,
    guess: bool,
    description: Option<String>,
    forms: Vec<Form>,
}

impl Term {
    /// Взят пользователем из «Новых».
    pub fn new(name: &str, category: TermCategory) -> Result<Self, ValidationError> {
        let name_key = NameKey::new(name).ok_or(ValidationError::Empty("name"))?;
        Ok(Self {
            id: None,
            name: name.trim().to_string(),
            name_key,
            translation: None,
            category,
            guess: false,
            description: None,
            forms: Vec::new(),
        })
    }

    /// Новое название из ответа модели при переводе реплик.
    pub fn guess(name: &str, translation: &str, category: TermCategory) -> Result<Self, ValidationError> {
        let mut term = Self::new(name, category)?;
        term.translation = non_empty(translation);
        term.guess = true;
        Ok(term)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: TermId,
        name: &str,
        translation: Option<String>,
        category: TermCategory,
        guess: bool,
        description: Option<String>,
        forms: Vec<Form>,
    ) -> Result<Self, ValidationError> {
        let mut term = Self::new(name, category)?;
        term.id = Some(id);
        term.translation = translation;
        term.guess = guess;
        term.description = description;
        term.forms = forms;
        Ok(term)
    }

    /// Подтверждение пользователем.
    pub fn confirm(&mut self) {
        self.guess = false;
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

    pub fn is_guess(&self) -> bool {
        self.guess
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

/// Название в корзине: его вычитает раздел «Новые термины».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscardedTerm {
    pub text: String,
    pub key: NameKey,
    pub reason: DiscardReason,
    pub discarded_at: Timestamp,
}

/// Кандидат в термины из `term_candidates`, которого нет в глоссарии и корзине.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermCandidate {
    pub text: String,
    pub occurrences: u32,
    pub example: String,
}
