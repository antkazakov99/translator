//! Персонаж: имя и фамилия с переводами, другие формы, род, описание.

use serde::{Deserialize, Serialize};

use super::NameKey;
use crate::common::{Gender, ValidationError, non_empty, row_id};
use crate::name_rules::LanguageTag;

row_id!(CharacterId);

/// Чем форма приходится имени персонажа. От вида зависит, как форму пишет перевод имён и может ли она стать
/// главным именем или фамилией. У формы термина вида нет: всегда `Other`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum FormKind {
    /// Другое написание имени: короткая форма, полная, опечатка («Dimi» у «Dimitria»).
    First,
    /// Другая фамилия или другое её написание («Saint-Saens» у «Margaret Dvorak»).
    Last,
    /// Всё остальное: прозвище, титул, обращение, подпись из игры.
    #[default]
    Other,
}

impl FormKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Last => "last",
            Self::Other => "other",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "first" => Ok(Self::First),
            "last" => Ok(Self::Last),
            "other" => Ok(Self::Other),
            _ => Err(ValidationError::UnknownValue { field: "character_forms.kind", value: text.to_string() }),
        }
    }
}

/// Другая форма имени или названия: обращение, прозвище, сокращение, подпись из игры.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Form {
    text: String,
    key: NameKey,
    translation: Option<String>,
    kind: FormKind,
}

impl Form {
    /// Форма вида `Other`.
    pub fn new(text: &str, translation: Option<&str>) -> Result<Self, ValidationError> {
        Self::named(text, translation, FormKind::Other)
    }

    pub fn named(text: &str, translation: Option<&str>, kind: FormKind) -> Result<Self, ValidationError> {
        let key = NameKey::new(text).ok_or(ValidationError::Empty("form"))?;
        Ok(Self {
            text: text.trim().to_string(),
            key,
            translation: translation.and_then(non_empty),
            kind,
        })
    }

    pub fn kind(&self) -> FormKind {
        self.kind
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn key(&self) -> &NameKey {
        &self.key
    }

    pub fn translation(&self) -> Option<&str> {
        self.translation.as_deref()
    }

    pub fn set_translation(&mut self, text: &str) {
        self.translation = non_empty(text);
    }
}

/// Персонаж. Ключи имени и фамилии модель держит вместе с полями.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Character {
    /// `None` у ещё не сохранённого.
    id: Option<CharacterId>,
    first_name: Option<String>,
    last_name: Option<String>,
    translated_first_name: Option<String>,
    translated_last_name: Option<String>,
    gender: Option<Gender>,
    /// Манера речи.
    speech: Option<String>,
    description: Option<String>,
    forms: Vec<Form>,
}

impl Character {
    /// Из подписи говорящего («Персонаж» в «Новых») или вручную.
    pub fn new(first_name: &str) -> Self {
        Self {
            id: None,
            first_name: non_empty(first_name),
            last_name: None,
            translated_first_name: None,
            translated_last_name: None,
            gender: None,
            speech: None,
            description: None,
            forms: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: CharacterId,
        first_name: Option<String>,
        last_name: Option<String>,
        translated_first_name: Option<String>,
        translated_last_name: Option<String>,
        gender: Option<Gender>,
        speech: Option<String>,
        description: Option<String>,
        forms: Vec<Form>,
    ) -> Self {
        let mut character = Self {
            id: Some(id),
            first_name,
            last_name,
            translated_first_name,
            translated_last_name,
            gender,
            speech,
            description,
            forms: Vec::new(),
        };
        for form in forms {
            character.push_form(form);
        }
        character
    }

    /// Смена имени убирает формы, совпавшие с новым именем, а их перевод переходит в пустое поле перевода.
    pub fn set_first_name(&mut self, name: &str) {
        self.first_name = non_empty(name);
        self.clean_forms();
    }

    pub fn set_last_name(&mut self, name: &str) {
        self.last_name = non_empty(name);
        self.clean_forms();
    }

    pub fn set_translated_names(&mut self, first: &str, last: &str) {
        self.translated_first_name = non_empty(first);
        self.translated_last_name = non_empty(last);
    }

    /// Написание модели в пустое поле: имя, фамилию или форму с написанием `source`. Заполненное не трогается.
    /// Возвращает, записано ли.
    pub fn fill_spelling(&mut self, field: NameField, source: &str, translation: &str) -> bool {
        let Some(translation) = non_empty(translation) else { return false };
        match field {
            NameField::First if self.translated_first_name.is_none() && self.first_name.is_some() => {
                self.translated_first_name = Some(translation);
                true
            }
            NameField::Last if self.translated_last_name.is_none() && self.last_name.is_some() => {
                self.translated_last_name = Some(translation);
                true
            }
            NameField::Form => {
                let key = NameKey::new(source);
                match self.forms.iter_mut().find(|form| Some(form.key()) == key.as_ref() && form.translation.is_none()) {
                    Some(form) => {
                        form.translation = Some(translation);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    pub fn set_gender(&mut self, gender: Option<Gender>) {
        self.gender = gender;
    }

    pub fn set_speech(&mut self, text: &str) {
        self.speech = non_empty(text);
    }

    pub fn set_description(&mut self, text: &str) {
        self.description = non_empty(text);
    }

    /// Форма не может повторять имя, фамилию, полное имя или другую форму: иначе одно написание получит два перевода.
    pub fn add_form(&mut self, form: Form) -> Result<(), ValidationError> {
        if self.name_keys().contains(form.key()) || self.forms.iter().any(|own| own.key() == form.key()) {
            return Err(ValidationError::DuplicateForm(form.text().to_string()));
        }
        self.forms.push(form);
        Ok(())
    }

    pub fn remove_form(&mut self, key: &NameKey) {
        self.forms.retain(|form| form.key() != key);
    }

    /// Имя (`First`) или фамилия (`Last`) и формы того же вида: написания, из которых выбирается главное.
    pub fn spellings(&self, kind: FormKind) -> Vec<&str> {
        let name = match kind {
            FormKind::First => self.first_name(),
            FormKind::Last => self.last_name(),
            FormKind::Other => None,
        };
        name.into_iter().chain(self.forms.iter().filter(|form| form.kind == kind && kind != FormKind::Other).map(Form::text)).collect()
    }

    /// Форма вида «имя» или «фамилия» становится главным именем или фамилией. Прежнее написание уходит формой
    /// того же вида, переводы остаются при своих написаниях. `false`: такой формы нет или она вида `Other`.
    pub fn promote(&mut self, key: &NameKey) -> bool {
        let Some(index) = self.forms.iter().position(|form| form.key() == key && form.kind != FormKind::Other) else { return false };
        let form = self.forms.remove(index);
        let kind = form.kind;
        let (name, translated) = match kind {
            FormKind::First => (&mut self.first_name, &mut self.translated_first_name),
            _ => (&mut self.last_name, &mut self.translated_last_name),
        };
        let old = name.replace(form.text).map(|text| (text, translated.take()));
        *translated = form.translation;
        if let Some((text, translation)) = old
            && let Ok(old) = Form::named(&text, translation.as_deref(), kind)
        {
            self.forms.push(old);
        }
        self.clean_forms();
        true
    }

    /// Чистка форм (`uniqueForms`): уходят копии имён и других форм, а их перевод заполняет пустое поле.
    pub fn clean_forms(&mut self) {
        let forms = std::mem::take(&mut self.forms);
        for form in forms {
            self.push_form(form);
        }
    }

    fn push_form(&mut self, form: Form) {
        let key = form.key().clone();
        if Some(&key) == self.first_name_key().as_ref() {
            if self.translated_first_name.is_none() {
                self.translated_first_name = form.translation.clone();
            }
        } else if Some(&key) == self.last_name_key().as_ref() {
            if self.translated_last_name.is_none() {
                self.translated_last_name = form.translation.clone();
            }
        } else if let Some(own) = self.forms.iter_mut().find(|own| own.key() == &key) {
            if own.translation.is_none() {
                own.translation = form.translation;
            }
        } else if Some(&key) != self.full_name().as_deref().and_then(NameKey::new).as_ref() {
            self.forms.push(form);
        }
    }

    /// Имя и фамилия через пробел: подпись карточки и подпись говорящего в запросах модели.
    pub fn full_name(&self) -> Option<String> {
        match (&self.first_name, &self.last_name) {
            (Some(first), Some(last)) => Some(format!("{first} {last}")),
            (Some(name), None) | (None, Some(name)) => Some(name.clone()),
            (None, None) => None,
        }
    }

    pub fn translated_full_name(&self) -> Option<String> {
        match (&self.translated_first_name, &self.translated_last_name) {
            (Some(first), Some(last)) => Some(format!("{first} {last}")),
            (Some(name), None) | (None, Some(name)) => Some(name.clone()),
            (None, None) => None,
        }
    }

    pub fn first_name_key(&self) -> Option<NameKey> {
        self.first_name.as_deref().and_then(NameKey::new)
    }

    pub fn last_name_key(&self) -> Option<NameKey> {
        self.last_name.as_deref().and_then(NameKey::new)
    }

    /// Ключи имени, фамилии и полного имени.
    pub fn name_keys(&self) -> Vec<NameKey> {
        [self.first_name_key(), self.last_name_key(), self.full_name().as_deref().and_then(NameKey::new)]
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn id(&self) -> Option<CharacterId> {
        self.id
    }

    pub fn first_name(&self) -> Option<&str> {
        self.first_name.as_deref()
    }

    pub fn last_name(&self) -> Option<&str> {
        self.last_name.as_deref()
    }

    pub fn translated_first_name(&self) -> Option<&str> {
        self.translated_first_name.as_deref()
    }

    pub fn translated_last_name(&self) -> Option<&str> {
        self.translated_last_name.as_deref()
    }

    pub fn gender(&self) -> Option<Gender> {
        self.gender
    }

    pub fn speech(&self) -> Option<&str> {
        self.speech.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn forms(&self) -> &[Form] {
        &self.forms
    }
}

/// Поле имени, происхождение которого определила модель.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum NameField {
    First,
    Last,
    Form,
}

impl NameField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Last => "last",
            Self::Form => "form",
        }
    }

    pub fn parse(text: &str) -> Result<Self, ValidationError> {
        match text {
            "first" => Ok(Self::First),
            "last" => Ok(Self::Last),
            "form" => Ok(Self::Form),
            _ => Err(ValidationError::UnknownValue { field: "name_origins.field", value: text.to_string() }),
        }
    }
}

/// Происхождение имени (`name_origins`). По нему выбираются правила имён.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameOrigin {
    pub field: NameField,
    /// Разобранное написание.
    pub source: String,
    /// Полное имя персонажа во время разбора.
    pub context: String,
    pub languages: Vec<LanguageTag>,
    pub reason: Option<String>,
}

impl NameOrigin {
    /// Действует, пока у персонажа то же полное имя и в поле то же написание (`currentNameOrigins` в 1.0).
    pub fn is_current(&self, character: &Character) -> bool {
        if character.full_name().as_deref() != Some(self.context.as_str()) {
            return false;
        }
        let source = NameKey::new(&self.source);
        match self.field {
            NameField::First => source == character.first_name_key(),
            NameField::Last => source == character.last_name_key(),
            NameField::Form => character.forms().iter().any(|form| Some(form.key()) == source.as_ref()),
        }
    }
}
