//! Глоссарий в запросе модели: подсказки, подписи и род говорящих, значения токенов.

use std::collections::HashMap;

use super::text::fill_tokens;
use super::{Hints, Scene};
use crate::common::Gender;
use crate::engines::SpeakerKey;
use crate::glossary::{CharacterId, GlossaryData, NameField, NameOrigin, speaker_gender};
use crate::name_rules::{LanguageTag, NameRule};

/// Глоссарий, разобранный для запросов одного запуска.
#[derive(Debug, Clone, Default)]
pub struct GlossaryContext {
    pub hints: Hints,
    /// Подпись и род по ключу говорящего.
    speakers: HashMap<SpeakerKey, Speaker>,
    /// Значения токенов: токен со значением модель не видит, она верила бы имени токена больше, чем пояснению.
    values: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
struct Speaker {
    /// Имя единственного связанного персонажа.
    name: Option<String>,
    gender: Option<Gender>,
    /// Перевод подписи из записи говорящего.
    translation: Option<String>,
}

impl GlossaryContext {
    pub fn new(data: &GlossaryData) -> Self {
        let mut hints = Hints::default();
        for character in &data.characters {
            let gender = character.gender();
            let pairs = [
                (character.full_name(), character.translated_full_name()),
                (character.first_name().map(str::to_string), character.translated_first_name().map(str::to_string)),
                (character.last_name().map(str::to_string), character.translated_last_name().map(str::to_string)),
            ];
            let forms = character.forms().iter().map(|form| (Some(form.text().to_string()), form.translation().map(str::to_string)));
            for (original, translated) in pairs.into_iter().chain(forms) {
                if let (Some(original), Some(translated)) = (original, translated) {
                    hints.names.push((original, translated, gender));
                }
            }
        }
        for term in &data.terms {
            let Some(translation) = term.translation() else { continue };
            hints.terms.push((term.name().to_string(), translation.to_string(), term.is_guess()));
            for form in term.forms() {
                hints.terms.push((form.text().to_string(), form.translation().unwrap_or(translation).to_string(), term.is_guess()));
            }
        }
        let mut values = HashMap::new();
        for token in &data.tokens {
            match (token.value(), token.note()) {
                (Some(value), _) => {
                    values.insert(token.name().to_string(), value.to_string());
                }
                (None, Some(note)) => hints.tokens.push((token.name().to_string(), note.to_string())),
                (None, None) => {}
            }
        }

        let mut speakers: HashMap<SpeakerKey, Speaker> = HashMap::new();
        for link in &data.links {
            speakers.entry(link.speaker.clone()).or_default();
        }
        for (key, speaker) in &mut speakers {
            let linked: Vec<_> = data
                .links
                .iter()
                .filter(|link| &link.speaker == key)
                .filter_map(|link| data.characters.iter().find(|character| character.id() == Some(link.character)))
                .collect();
            if let [only] = linked.as_slice() {
                speaker.name = only.full_name();
            }
            let genders: Vec<Option<Gender>> = linked.iter().map(|character| character.gender()).collect();
            let entry = data.entries.iter().find(|entry| entry.key() == key);
            speaker.gender = speaker_gender(entry, &genders);
        }
        for entry in &data.entries {
            let speaker = speakers.entry(entry.key().clone()).or_default();
            speaker.translation = entry.translation().map(str::to_string);
            speaker.gender = speaker_gender(Some(entry), &[]).or(speaker.gender);
        }
        Self { hints, speakers, values }
    }

    /// Происхождение имён и правила их языков: правила идут в запрос, если имя упомянуто в репликах пакета.
    /// Берётся только действующее происхождение, полное имя получает языки имени и фамилии.
    pub fn with_name_rules(&mut self, data: &GlossaryData, origins: &HashMap<CharacterId, Vec<NameOrigin>>, rules: &[NameRule]) {
        let ruled = |origin: &NameOrigin| -> Vec<String> {
            origin.languages.iter().filter(|language| rules.iter().any(|rule| rule.from() == *language)).map(ToString::to_string).collect()
        };
        for character in &data.characters {
            let Some(own) = character.id().and_then(|id| origins.get(&id)) else { continue };
            let current: Vec<&NameOrigin> = own.iter().filter(|origin| origin.is_current(character)).collect();
            let of = |field: NameField| current.iter().filter(|origin| origin.field == field).flat_map(|origin| ruled(origin)).collect::<Vec<_>>();
            let (first, last) = (of(NameField::First), of(NameField::Last));
            let mut add = |name: Option<&str>, codes: Vec<String>| {
                if let Some(name) = name.filter(|_| !codes.is_empty()) {
                    self.hints.name_origins.push((name.to_string(), codes));
                }
            };
            let mut full = first.clone();
            full.extend(last.iter().filter(|code| !first.contains(code)).cloned());
            add(character.full_name().as_deref(), full);
            add(character.first_name(), first);
            add(character.last_name(), last);
            for origin in current.iter().filter(|origin| origin.field == NameField::Form) {
                add(Some(&origin.source), ruled(origin));
            }
        }
        self.hints.name_rules = rules.iter().map(|rule| (rule.from().to_string(), rule.rules().to_string())).collect();
    }

    /// Языки имён, у которых есть действующее происхождение: по ним сервис берёт правила.
    pub fn name_languages(data: &GlossaryData, origins: &HashMap<CharacterId, Vec<NameOrigin>>) -> Vec<LanguageTag> {
        let mut languages: Vec<LanguageTag> = Vec::new();
        for character in &data.characters {
            let Some(own) = character.id().and_then(|id| origins.get(&id)) else { continue };
            for language in own.iter().filter(|origin| origin.is_current(character)).flat_map(|origin| &origin.languages) {
                if !languages.contains(language) {
                    languages.push(language.clone());
                }
            }
        }
        languages
    }

    /// Подписи и род говорящих по глоссарию, значения токенов в оригинале и переводе. Перевод подписи из записи
    /// говорящего идёт в подсказку имён: модель увидит его рядом с подписью.
    pub fn apply(&mut self, scene: &mut Scene) {
        for line in &mut scene.lines {
            if let Some(speaker) = line.speaker_key.as_ref().and_then(|key| self.speakers.get(key)) {
                if let Some(name) = &speaker.name {
                    line.speaker = Some(name.clone());
                }
                line.speaker_gender = speaker.gender.or(line.speaker_gender);
                if let (Some(label), Some(translation)) = (&line.speaker, &speaker.translation)
                    && !self.hints.names.iter().any(|(original, _, _)| original == label)
                {
                    self.hints.names.push((label.clone(), translation.clone(), speaker.gender));
                }
            }
            if !self.values.is_empty() {
                line.source_text = fill_tokens(&line.source_text, &self.values);
                line.text = fill_tokens(&line.text, &self.values);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Timestamp;
    use crate::glossary::{Character, CharacterId, Form, SpeakerEntry, SpeakerLink, Term, TermCategory, Token};
    use crate::translation::batcher::tests::{line, scene};

    fn key(text: &str) -> SpeakerKey {
        SpeakerKey::new(text).unwrap()
    }

    #[test]
    fn glossary_gives_names_terms_speakers_and_token_values() {
        let text = |value: &str| Some(value.to_string());
        let anna = Character::restore(
            CharacterId::from_row(1),
            text("Anna"),
            text("Lee"),
            text("Анна"),
            text("Ли"),
            Some(Gender::Female),
            None,
            None,
            vec![Form::new("Annie", Some("Энни")).unwrap()],
        );
        let mut entry = SpeakerEntry::new(key("label:Radio"), Timestamp::from_secs(0));
        entry.set_translation("Радио");
        entry.set_gender(Some(Gender::Male));
        let term = Term::guess("Silver Key", "Серебряный ключ", TermCategory::Item).unwrap();
        let mut mother = Token::new("mom").unwrap();
        mother.set_value("mother");
        let mut hero = Token::new("mc").unwrap();
        hero.set_note("имя героя");
        let data = GlossaryData {
            characters: vec![anna],
            links: vec![SpeakerLink { speaker: key("code:a"), character: CharacterId::from_row(1), is_primary: true }],
            entries: vec![entry],
            terms: vec![term],
            tokens: vec![mother, hero],
        };

        let mut context = GlossaryContext::new(&data);
        assert!(context.hints.names.contains(&("Anna Lee".to_string(), "Анна Ли".to_string(), Some(Gender::Female))));
        assert!(context.hints.names.contains(&("Annie".to_string(), "Энни".to_string(), Some(Gender::Female))));
        assert_eq!(context.hints.terms, [("Silver Key".to_string(), "Серебряный ключ".to_string(), true)]);
        assert_eq!(context.hints.tokens, [("mc".to_string(), "имя героя".to_string())]);

        let mut lines = vec![line(1, "Hi, [mom]. I'm [mc].", ""), line(2, "Static.", "")];
        lines[0].speaker = Some("a".to_string());
        lines[0].speaker_key = Some(key("code:a"));
        lines[1].speaker = Some("Radio".to_string());
        lines[1].speaker_key = Some(key("label:Radio"));
        let mut scene = scene(lines);
        context.apply(&mut scene);
        assert_eq!(scene.lines[0].speaker.as_deref(), Some("Anna Lee"));
        assert_eq!(scene.lines[0].speaker_gender, Some(Gender::Female));
        assert_eq!(scene.lines[0].source_text, "Hi, mother. I'm [mc].");
        assert_eq!(scene.lines[1].speaker_gender, Some(Gender::Male));
        assert!(context.hints.names.contains(&("Radio".to_string(), "Радио".to_string(), Some(Gender::Male))));
    }

    #[test]
    fn name_rules_follow_current_origins_of_names_with_rules() {
        let text = |value: &str| Some(value.to_string());
        let anja = Character::restore(
            CharacterId::from_row(1),
            text("Anja"),
            text("Karanovic"),
            None,
            None,
            Some(Gender::Female),
            None,
            None,
            vec![Form::new("Mrs. Karanovic", None).unwrap()],
        );
        let data = GlossaryData { characters: vec![anja], links: Vec::new(), entries: Vec::new(), terms: Vec::new(), tokens: Vec::new() };
        let tag = |code: &str| LanguageTag::parse(code).unwrap();
        let origin = |field: NameField, source: &str, context: &str, languages: &[&str]| NameOrigin {
            field,
            source: source.to_string(),
            context: context.to_string(),
            languages: languages.iter().map(|code| tag(code)).collect(),
            reason: None,
        };
        let origins = HashMap::from([(
            CharacterId::from_row(1),
            vec![
                origin(NameField::First, "Anja", "Anja Karanovic", &["sr"]),
                origin(NameField::Last, "Karanovic", "Anja Karanovic", &["hr", "sr"]),
                origin(NameField::Form, "Mrs. Karanovic", "Anja Karanovic", &["sr"]),
                origin(NameField::Form, "Mrs. Karanovic", "Anja Petrovic", &["de"]),
            ],
        )]);
        assert_eq!(GlossaryContext::name_languages(&data, &origins), [tag("sr"), tag("hr")], "устаревшее происхождение не в счёт");

        let mut context = GlossaryContext::new(&data);
        context.with_name_rules(&data, &origins, &[NameRule::new(tag("sr"), tag("ru"), "j это й").unwrap()]);
        let sr = || vec!["sr".to_string()];
        assert_eq!(
            context.hints.name_origins,
            [
                ("Anja Karanovic".to_string(), sr()),
                ("Anja".to_string(), sr()),
                ("Karanovic".to_string(), sr()),
                ("Mrs. Karanovic".to_string(), sr()),
            ]
        );
        assert_eq!(context.hints.name_rules, [("sr".to_string(), "j это й".to_string())]);
    }
}
