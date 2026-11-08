//! `name_rules` в app.db и `name_rule_overrides` в project.db.

use rusqlite::{Connection, OptionalExtension, params};

use super::{LanguageTag, NameRule, NameRuleOverride};
use crate::storage::DbError;

fn tag(table: &'static str, column: &'static str, text: &str) -> Result<LanguageTag, DbError> {
    LanguageTag::parse(text).map_err(|error| DbError::bad_value(table, column, error))
}

pub struct NameRuleRepo<'c> {
    pub conn: &'c Connection,
}

impl NameRuleRepo<'_> {
    /// Правила на язык перевода по коду языка происхождения.
    pub fn list(&self, to: &LanguageTag) -> Result<Vec<NameRule>, DbError> {
        let rows = self
            .conn
            .prepare("SELECT from_language, rules FROM name_rules WHERE to_language = ?1 ORDER BY from_language")?
            .query_map([to.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|(from, rules)| rule(&from, to, &rules)).collect()
    }

    pub fn get(&self, from: &LanguageTag, to: &LanguageTag) -> Result<Option<NameRule>, DbError> {
        let rules: Option<String> = self
            .conn
            .query_row(
                "SELECT rules FROM name_rules WHERE from_language = ?1 AND to_language = ?2",
                [from.as_str(), to.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        rules.map(|rules| rule(from.as_str(), to, &rules)).transpose()
    }

    /// Вставляет пару или заменяет её текст.
    pub fn save(&self, rule: &NameRule) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO name_rules (from_language, to_language, rules) VALUES (?1, ?2, ?3)
             ON CONFLICT (from_language, to_language) DO UPDATE SET rules = excluded.rules",
            params![rule.from().as_str(), rule.to().as_str(), rule.rules()],
        )?;
        Ok(())
    }

    pub fn delete(&self, from: &LanguageTag, to: &LanguageTag) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM name_rules WHERE from_language = ?1 AND to_language = ?2", [from.as_str(), to.as_str()])?;
        Ok(())
    }
}

fn rule(from: &str, to: &LanguageTag, rules: &str) -> Result<NameRule, DbError> {
    NameRule::new(tag("name_rules", "from_language", from)?, to.clone(), rules).map_err(|error| DbError::bad_value("name_rules", "rules", error))
}

pub struct NameRuleOverrideRepo<'c> {
    pub conn: &'c Connection,
}

impl NameRuleOverrideRepo<'_> {
    pub fn list(&self) -> Result<Vec<NameRuleOverride>, DbError> {
        let rows = self
            .conn
            .prepare("SELECT from_language, rules FROM name_rule_overrides ORDER BY from_language")?
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(from, rules)| {
                NameRuleOverride::new(tag("name_rule_overrides", "from_language", &from)?, &rules)
                    .map_err(|error| DbError::bad_value("name_rule_overrides", "rules", error))
            })
            .collect()
    }

    pub fn save(&self, rule: &NameRuleOverride) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO name_rule_overrides (from_language, rules) VALUES (?1, ?2)
             ON CONFLICT (from_language) DO UPDATE SET rules = excluded.rules",
            params![rule.from().as_str(), rule.rules()],
        )?;
        Ok(())
    }

    pub fn delete(&self, from: &LanguageTag) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM name_rule_overrides WHERE from_language = ?1", [from.as_str()])?;
        Ok(())
    }
}
