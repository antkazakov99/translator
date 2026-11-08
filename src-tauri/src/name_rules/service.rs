//! Библиотека правил имён.

use std::collections::{BTreeMap, HashSet};

use super::{LanguageTag, NameRule, NameRuleOverride, NameRuleOverrideRepo, NameRuleRepo};
use crate::common::AppError;
use crate::glossary::CharacterRepo;
use crate::projects::{ProjectId, ProjectInfoRepo};
use crate::state::AppState;

/// Язык имён для окна: сколько имён проекта на нём, общие правила и свои правила проекта.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageRules {
    pub language: LanguageTag,
    pub names: usize,
    pub rules: Option<String>,
    pub own: Option<String>,
}

/// Происхождение, для которого правил не бывает: неизвестное (`und`) и роли с описаниями (`zxx`).
pub fn has_rules(language: &LanguageTag) -> bool {
    !language.is_undetermined() && language.as_str() != "zxx"
}

pub struct NameRuleService<'a> {
    pub state: &'a AppState,
}

impl NameRuleService<'_> {
    pub fn list(&self, to: &LanguageTag) -> Result<Vec<NameRule>, AppError> {
        Ok(self.state.app.read(|tx| NameRuleRepo { conn: tx }.list(to))?)
    }

    pub fn save(&self, rule: &NameRule) -> Result<(), AppError> {
        Ok(self.state.app.write(|tx| NameRuleRepo { conn: tx }.save(rule))?)
    }

    pub fn delete(&self, from: &LanguageTag, to: &LanguageTag) -> Result<(), AppError> {
        Ok(self.state.app.write(|tx| NameRuleRepo { conn: tx }.delete(from, to))?)
    }

    /// Новые пары дополняют библиотеку, существующие не заменяются. Возвращает число добавленных.
    pub fn merge(&self, rules: &[NameRule]) -> Result<usize, AppError> {
        Ok(self.state.app.write(|tx| {
            let repo = NameRuleRepo { conn: tx };
            let mut added = 0;
            for rule in rules {
                if repo.get(rule.from(), rule.to())?.is_none() {
                    repo.save(rule)?;
                    added += 1;
                }
            }
            Ok(added)
        })?)
    }

    pub fn target_language(&self, project: ProjectId) -> Result<LanguageTag, AppError> {
        Ok(self.state.open_project(project)?.project.read(|tx| ProjectInfoRepo { conn: tx }.target_language())?)
    }

    /// Правила для запроса модели на язык перевода проекта: своё правило проекта важнее общего.
    pub fn effective(&self, project: ProjectId, from: &[LanguageTag]) -> Result<Vec<NameRule>, AppError> {
        let open = self.state.open_project(project)?;
        let (to, own) = open.project.read(|tx| Ok((ProjectInfoRepo { conn: tx }.target_language()?, NameRuleOverrideRepo { conn: tx }.list()?)))?;
        let mut seen = HashSet::new();
        let mut rules = Vec::new();
        for language in from.iter().filter(|language| has_rules(language) && seen.insert(*language)) {
            match own.iter().find(|rule| rule.from() == language) {
                Some(rule) => rules.push(NameRule::new(language.clone(), to.clone(), rule.rules())?),
                None => rules.extend(self.state.app.read(|tx| NameRuleRepo { conn: tx }.get(language, &to))?),
            }
        }
        Ok(rules)
    }

    /// Своё правило проекта. Пустой текст удаляет его.
    pub fn save_own(&self, project: ProjectId, from: &LanguageTag, rules: &str) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        let rule = (!rules.trim().is_empty()).then(|| NameRuleOverride::new(from.clone(), rules)).transpose()?;
        Ok(open.project.write(|tx| {
            let repo = NameRuleOverrideRepo { conn: tx };
            match &rule {
                Some(rule) => repo.save(rule),
                None => repo.delete(from),
            }
        })?)
    }

    /// Общее правило на язык перевода проекта. Пустой текст удаляет его.
    pub fn save_shared(&self, project: ProjectId, from: &LanguageTag, rules: &str) -> Result<(), AppError> {
        let to = self.target_language(project)?;
        if rules.trim().is_empty() {
            return self.delete(from, &to);
        }
        self.save(&NameRule::new(from.clone(), to, rules)?)
    }

    /// Языки для окна: сначала языки имён проекта по числу имён, потом остальные с правилами, по коду.
    pub fn overview(&self, project: ProjectId) -> Result<Vec<LanguageRules>, AppError> {
        let open = self.state.open_project(project)?;
        let (to, own, origins) = open.project.read(|tx| {
            Ok((ProjectInfoRepo { conn: tx }.target_language()?, NameRuleOverrideRepo { conn: tx }.list()?, CharacterRepo { conn: tx }.all_origins()?))
        })?;
        let shared = self.list(&to)?;
        let mut languages: BTreeMap<LanguageTag, LanguageRules> = BTreeMap::new();
        fn entry<'m>(languages: &'m mut BTreeMap<LanguageTag, LanguageRules>, language: &LanguageTag) -> &'m mut LanguageRules {
            languages.entry(language.clone()).or_insert_with(|| LanguageRules { language: language.clone(), names: 0, rules: None, own: None })
        }
        for language in origins.values().flatten().flat_map(|origin| &origin.languages).filter(|language| has_rules(language)) {
            entry(&mut languages, language).names += 1;
        }
        for rule in &shared {
            entry(&mut languages, rule.from()).rules = Some(rule.rules().to_string());
        }
        for rule in &own {
            entry(&mut languages, rule.from()).own = Some(rule.rules().to_string());
        }
        let mut list: Vec<LanguageRules> = languages.into_values().collect();
        list.sort_by(|one, other| other.names.cmp(&one.names).then_with(|| one.language.cmp(&other.language)));
        Ok(list)
    }
}
