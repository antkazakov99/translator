//! Библиотека правил имён.

use super::{LanguageTag, NameRule};
use crate::common::AppError;
use crate::projects::ProjectId;
use crate::state::AppState;

pub struct NameRuleService<'a> {
    pub state: &'a AppState,
}

impl NameRuleService<'_> {
    pub fn list(&self, to: &LanguageTag) -> Result<Vec<NameRule>, AppError> {
        todo!()
    }

    pub fn save(&self, rule: &NameRule) -> Result<(), AppError> {
        todo!()
    }

    pub fn delete(&self, from: &LanguageTag, to: &LanguageTag) -> Result<(), AppError> {
        todo!()
    }

    /// Новые пары из ответа модели дополняют библиотеку, существующие не заменяются. Возвращает число добавленных.
    pub fn merge(&self, rules: &[NameRule]) -> Result<usize, AppError> {
        todo!()
    }

    /// Правила для запроса модели: своё правило проекта важнее общего.
    pub fn effective(&self, project: ProjectId, from: &[LanguageTag]) -> Result<Vec<NameRule>, AppError> {
        todo!()
    }
}
