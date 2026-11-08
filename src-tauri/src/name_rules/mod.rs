//! Правила передачи имён по паре языков: общие в app.db и свои у проекта.

pub mod model;
pub mod prompt;
pub mod repo;
pub mod service;

pub use model::{LanguageTag, NameRule, NameRuleOverride};
pub use prompt::RulesDraftPrompt;
pub use repo::{NameRuleOverrideRepo, NameRuleRepo};
pub use service::{LanguageRules, NameRuleService, has_rules};
