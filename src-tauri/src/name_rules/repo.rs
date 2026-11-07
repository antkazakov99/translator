//! `name_rules` в app.db и `name_rule_overrides` в project.db.

use rusqlite::Connection;

use super::{LanguageTag, NameRule, NameRuleOverride};
use crate::storage::DbError;

pub struct NameRuleRepo<'c> {
    pub conn: &'c Connection,
}

impl NameRuleRepo<'_> {
    /// Правила на язык перевода.
    pub fn list(&self, to: &LanguageTag) -> Result<Vec<NameRule>, DbError> {
        todo!()
    }

    pub fn get(&self, from: &LanguageTag, to: &LanguageTag) -> Result<Option<NameRule>, DbError> {
        todo!()
    }

    pub fn save(&self, rule: &NameRule) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, from: &LanguageTag, to: &LanguageTag) -> Result<(), DbError> {
        todo!()
    }
}

pub struct NameRuleOverrideRepo<'c> {
    pub conn: &'c Connection,
}

impl NameRuleOverrideRepo<'_> {
    pub fn list(&self) -> Result<Vec<NameRuleOverride>, DbError> {
        todo!()
    }

    pub fn save(&self, rule: &NameRuleOverride) -> Result<(), DbError> {
        todo!()
    }

    pub fn delete(&self, from: &LanguageTag) -> Result<(), DbError> {
        todo!()
    }
}
