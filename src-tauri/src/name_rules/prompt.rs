//! Черновик правил передачи имён моделью: промпт и разбор ответа.

use super::LanguageTag;
use crate::llm::{ChatRequest, LlmTarget};

const DRAFT_ANSWER_TOKENS: u32 = 3072;

const DRAFT_SYSTEM: &str = "Draft practical rules in Russian for spelling personal names and surnames from the requested language in Russian. \
Include digraphs, pronunciation, omitted diacritics, established names and exceptions. \
Distinguish nominative name spelling, case declension and gender-specific surname formation: they are different operations. \
Do not create -a/-ova/-eva or any other feminine ending merely from a character's gender. Do not import surname formation from a different language. \
Preserve the exact supplied nominative surname unless the actual language of use demonstrably requires a distinct form. \
If a rule is uncertain, omit it rather than invent it. Follow supplied source notes. Do not claim to have consulted other sources. \
Return plain text only, without Markdown, at most 400 Russian words, concise enough for a translation prompt.";

pub struct RulesDraftPrompt;

impl RulesDraftPrompt {
    /// `existing`: правила в библиотеке, их поправки пользователя черновик сохраняет.
    pub fn build(from: &LanguageTag, existing: Option<&str>, target: &LlmTarget) -> ChatRequest {
        let existing = existing.map(str::trim).filter(|text| !text.is_empty()).unwrap_or("(none)");
        ChatRequest {
            system: DRAFT_SYSTEM.to_string(),
            user: format!(
                "Language of the names: {from} (BCP 47 code)\n\
                 Existing library rules (may contain user corrections and source references):\n{existing}\n\n\
                 Preserve explicit corrections, constraints and source references of the existing text when rewriting. \
                 Never reintroduce an example or transformation it explicitly prohibits. Do not replace it with rules of another language."
            ),
            temperature: target.settings.temperature,
            max_tokens: DRAFT_ANSWER_TOKENS,
        }
    }

    /// Текст без блока кода вокруг. `None`: модель ничего не написала.
    pub fn accept(text: &str) -> Option<String> {
        let text = text.trim();
        let text = text.strip_prefix("```").map_or(text, |inner| {
            let inner = inner.split_once('\n').map_or(inner, |(_, rest)| rest);
            inner.trim_end().strip_suffix("```").unwrap_or(inner)
        });
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::prompt::tests::target;

    #[test]
    fn draft_keeps_existing_rules_and_drops_code_fences() {
        let request = RulesDraftPrompt::build(&LanguageTag::parse("sr").unwrap(), Some(" j это й "), &target());
        assert!(request.user.starts_with("Language of the names: sr (BCP 47 code)\n"));
        assert!(request.user.contains("source references):\nj это й\n\n"));
        assert_eq!(RulesDraftPrompt::accept("```text\nj это й\n```\n").as_deref(), Some("j это й"));
        assert_eq!(RulesDraftPrompt::accept("  \n"), None);
    }
}
