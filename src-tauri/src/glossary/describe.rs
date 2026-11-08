//! Запуски модели по глоссарию: описания по частям сцен, имена, токены, подписи говорящих, термины.

use std::collections::HashMap;
use std::sync::Arc;

use tauri::ipc::Channel;

use super::repo::{LineSampleRepo, SampleLine};
use super::names::{
    CheckPrompt, NameItem, NameSuggestion, NamesPrompt, ORIGIN_BATCH, OriginsPrompt, cast, form_items, missing_rules, name_items, with_origins,
};
use super::suggest::{
    SPEAKER_SAMPLES, SpeakerItem, SpeakerSuggestion, SpeakersPrompt, TOKEN_SAMPLES, TermItem, TermSuggestion, TermsPrompt, TokenItem,
    TokenSuggestion, TokensPrompt, sample_line, spread, token_line,
};
use super::{Character, CharacterId, CharacterRepo, GlossaryData, NameOrigin, SpeakerRepo, TermId};
use crate::common::ValidationError;
use crate::name_rules::{LanguageTag, NameRule, NameRuleService, RulesDraftPrompt};
use crate::translation::Hints;
use crate::common::AppError;
use crate::engines::{ContainerKey, SpeakerKey};
use crate::llm::{Action, ChatRequest, LlmClient, LlmError, LlmRun, LlmTarget, LlmTargetService, ProfileId, RunProgress, RunStatus};
use crate::projects::ProjectId;
use crate::state::{AppState, OpenProject};
use crate::translation::hints::GlossaryContext;
use crate::translation::text::{mentions, token_names};

/// Сколько раз спрашивается пакет при сбое сети.
const ATTEMPTS: usize = 3;
/// Сколько реплик с названием термина просматривается в поиске примера: граница слова проверяется в Rust.
const EXAMPLE_CANDIDATES: usize = 20;

/// Предложения запуска. Остановка и ошибка не теряют готового: `error` говорит, что запуск оборвался.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestions<T> {
    pub items: Vec<T>,
    pub error: Option<String>,
}

/// Что описывается общим путём: части, заметки по частям, описание по заметкам.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeTarget {
    Character(CharacterId),
    Term(TermId),
    /// Сцена: части это её реплики, заметка единственной части сразу становится описанием.
    Scene(ContainerKey),
}

/// Глубина чтения в массовом обновлении: минимум и максимум частей и желаемый процент.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingDepth {
    pub min_parts: usize,
    pub max_parts: usize,
    pub percent: u8,
}

impl Default for ReadingDepth {
    fn default() -> Self {
        Self { min_parts: 5, max_parts: 50, percent: 30 }
    }
}

impl ReadingDepth {
    /// Процент от всех частей округляется вверх и ограничивается минимумом и максимумом.
    /// У кого частей меньше минимума, тот читается целиком (`partsForDepth`).
    pub fn parts_for(&self, total: usize) -> usize {
        if total <= self.min_parts {
            return total;
        }
        let wanted = (total * usize::from(self.percent)).div_ceil(100);
        wanted.clamp(self.min_parts, self.max_parts.max(self.min_parts)).min(total)
    }
}

/// Описания персонажей, терминов и сцен.
pub struct DescribeService<'a> {
    pub state: &'a AppState,
}

impl DescribeService<'_> {
    /// Читает `parts` частей и собирает описание. Описание по готовым заметкам собирается и после остановки.
    pub async fn describe(
        &self,
        project: ProjectId,
        target: DescribeTarget,
        parts: usize,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<(), AppError> {
        todo!()
    }

    /// Группа по очереди с глубиной чтения. Заполненная запись без новых частей в запуск не входит.
    pub async fn describe_many(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        depth: ReadingDepth,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<(), AppError> {
        todo!()
    }

    /// Сбрасывает заметки владельца.
    pub fn reset_notes(&self, project: ProjectId, target: &DescribeTarget) -> Result<usize, AppError> {
        todo!()
    }
}

/// Имена и другие запуски по записям глоссария.
pub struct NamesService<'a> {
    pub state: &'a AppState,
}

impl NamesService<'_> {
    /// Имена без перевода всей группой в одном запуске: разбор происхождения тех, у кого его нет, правила их
    /// языков, перевод имён и фамилий, потом форм от написанных имён, и проверка каждого пакета перевода.
    /// Происхождение записывается сразу, написания возвращаются для отбора.
    pub async fn translate_names(
        &self,
        project: ProjectId,
        characters: Vec<CharacterId>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<NamesFound, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Names, one_off)?;
        let (glossary, stored) = open.project.read(|tx| Ok((GlossaryData::load(tx)?, CharacterRepo { conn: tx }.all_origins()?)))?;
        let mut known: Vec<(CharacterId, NameOrigin)> =
            stored.into_iter().flat_map(|(id, origins)| origins.into_iter().map(move |origin| (id, origin))).collect();
        let context = GlossaryContext::new(&glossary);
        let cast = cast(&glossary.characters);
        let chosen: Vec<&Character> =
            glossary.characters.iter().filter(|character| character.id().is_some_and(|id| characters.contains(&id))).collect();
        let mut names = name_items(&chosen);
        let mut forms = form_items(&chosen, &[]);
        with_origins(&mut names, known.iter().map(|(id, origin)| (*id, origin)));
        with_origins(&mut forms, known.iter().map(|(id, origin)| (*id, origin)));
        if names.is_empty() && forms.is_empty() {
            return Ok(NamesFound::default());
        }
        let unknown: Vec<NameItem> = names.iter().chain(&forms).filter(|item| item.languages.is_empty()).cloned().collect();
        let origin_batches: Vec<&[NameItem]> = unknown.chunks(ORIGIN_BATCH).collect();
        let size = target.settings.batch_size.get();
        let total = origin_batches.len() + 2 * (names.len().div_ceil(size) + forms.len().div_ceil(size));
        let mut run = BatchRun::begin(self.state, &open, Action::Names, &target, progress, total)?;
        let mut items: Vec<NameSuggestion> = Vec::new();
        let mut missing = Vec::new();

        let mut going = true;
        for batch in origin_batches {
            match run.send(&OriginsPrompt::build(batch, &cast, &target)).await {
                Sent::Text(text) => {
                    let found = OriginsPrompt::accept(&text, batch);
                    open.project.write(|tx| {
                        let repo = CharacterRepo { conn: tx };
                        for (id, origin) in &found {
                            repo.save_origins(*id, std::slice::from_ref(origin))?;
                        }
                        Ok(())
                    })?;
                    known.extend(found);
                }
                Sent::Skipped => {}
                Sent::Stop => {
                    going = false;
                    break;
                }
            }
            if !run.step(0) {
                going = false;
                break;
            }
        }

        if going {
            with_origins(&mut names, known.iter().map(|(id, origin)| (*id, origin)));
            with_origins(&mut forms, known.iter().map(|(id, origin)| (*id, origin)));
            let languages: Vec<LanguageTag> = names.iter().chain(&forms).flat_map(|item| item.languages.iter().cloned()).collect();
            let rules = NameRuleService { state: self.state }.effective(project, &languages)?;
            missing = missing_rules(&names.iter().chain(&forms).cloned().collect::<Vec<_>>(), &rules);
            let spell = Spell { cast: &cast, hints: &context.hints, rules: &rules, target: &target };
            for batch in chunks(names, &target) {
                going = spell.batch(&mut run, &batch, &mut items).await;
                if !going {
                    break;
                }
            }
            if going {
                let mut forms = form_items(&chosen, &items);
                with_origins(&mut forms, known.iter().map(|(id, origin)| (*id, origin)));
                for batch in chunks(forms, &target) {
                    if !spell.batch(&mut run, &batch, &mut items).await {
                        break;
                    }
                }
            }
        }
        Ok(NamesFound { suggestions: run.finish(items)?, missing_rules: missing })
    }

    /// Черновик правил передачи имён с языка `from` на язык перевода проекта. `existing`: текст, который
    /// черновик дополняет. В базу ничего не пишется.
    pub async fn draft_rules(
        &self,
        project: ProjectId,
        from: LanguageTag,
        existing: Option<String>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Suggestions<String>, AppError> {
        if !crate::name_rules::has_rules(&from) {
            return Err(ValidationError::BadLanguage(from.to_string()).into());
        }
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Names, one_off)?;
        let request = RulesDraftPrompt::build(&from, existing.as_deref(), &target);
        run_batches(self.state, &open, Action::Names, &target, progress, vec![request], |_, text| {
            RulesDraftPrompt::accept(text).into_iter().collect()
        })
        .await
    }

    /// Перевод, категория и описание терминов. Модель видит своё описание термина и реплику с ним.
    pub async fn translate_terms(
        &self,
        project: ProjectId,
        terms: Vec<TermId>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Suggestions<TermSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Terms, one_off)?;
        let (items, glossary) = open.project.read(|tx| {
            let data = GlossaryData::load(tx)?;
            let samples = LineSampleRepo { conn: tx };
            let mut items = Vec::new();
            for id in &terms {
                let Some(term) = data.terms.iter().find(|term| term.id() == Some(*id)) else { continue };
                let lower = term.name().to_lowercase();
                let example = samples
                    .containing(term.name(), Some(EXAMPLE_CANDIDATES))?
                    .into_iter()
                    .find(|line| mentions(&line.text.to_lowercase(), &lower));
                items.push((term.clone(), example));
            }
            Ok((items, data))
        })?;
        let context = GlossaryContext::new(&glossary);
        let values = token_values(&glossary);
        let items: Vec<TermItem> = items
            .into_iter()
            .filter_map(|(term, example)| {
                Some(TermItem {
                    id: term.id()?,
                    name: term.name().to_string(),
                    category: term.category(),
                    description: term.description().map(str::to_string),
                    example: example.map(|line| sample_line(line.speaker.as_deref(), &line.text, &values)),
                })
            })
            .collect();
        let batches = chunks(items, &target);
        let requests = batches.iter().map(|batch| TermsPrompt::build(batch, &context.hints, &target)).collect();
        run_batches(self.state, &open, Action::Terms, &target, progress, requests, |index, text| TermsPrompt::accept(text, &batches[index])).await
    }

    /// Пояснения и значения токенов по репликам с `[___]` вместо токена. Токен без реплик пропускается: судить не о чем.
    pub async fn suggest_tokens(
        &self,
        project: ProjectId,
        tokens: Vec<String>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Suggestions<TokenSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Tokens, one_off)?;
        let (lines, glossary) = open.project.read(|tx| {
            let samples = LineSampleRepo { conn: tx };
            let mut lines = Vec::new();
            for name in &tokens {
                let found: Vec<SampleLine> = samples
                    .containing(&format!("[{name}"), None)?
                    .into_iter()
                    .filter(|line| token_names(&line.text).any(|found| found == name.as_str()))
                    .collect();
                lines.push((name.clone(), found));
            }
            Ok((lines, GlossaryData::load(tx)?))
        })?;
        let context = GlossaryContext::new(&glossary);
        let values = token_values(&glossary);
        let items: Vec<TokenItem> = lines
            .into_iter()
            .filter(|(_, found)| !found.is_empty())
            .map(|(name, found)| {
                let lines =
                    spread(&found, TOKEN_SAMPLES).iter().map(|line| token_line(line.speaker.as_deref(), &line.text, &name, &values)).collect();
                TokenItem { name, lines }
            })
            .collect();
        let batches = chunks(items, &target);
        let requests = batches.iter().map(|batch| TokensPrompt::build(batch, &context.hints, &target)).collect();
        run_batches(self.state, &open, Action::Tokens, &target, progress, requests, |index, text| TokensPrompt::accept(text, &batches[index])).await
    }

    /// Перевод подписи и род говорящих по нескольким их репликам.
    pub async fn suggest_speakers(
        &self,
        project: ProjectId,
        speakers: Vec<SpeakerKey>,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Suggestions<SpeakerSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Speakers, one_off)?;
        let (found, glossary) = open.project.read(|tx| {
            let current = SpeakerRepo { conn: tx }.current()?;
            let samples = LineSampleRepo { conn: tx };
            let mut found = Vec::new();
            for key in &speakers {
                let Some(speaker) = current.iter().find(|item| &item.speaker.key == key) else { continue };
                let label = speaker.speaker.label.clone().unwrap_or_else(|| key_tail(key).to_string());
                found.push((key.clone(), label, samples.of_speaker(key)?));
            }
            Ok((found, GlossaryData::load(tx)?))
        })?;
        let context = GlossaryContext::new(&glossary);
        let values = token_values(&glossary);
        let items: Vec<SpeakerItem> = found
            .into_iter()
            .map(|(key, label, lines)| {
                let lines = spread(&lines, SPEAKER_SAMPLES).iter().map(|line| sample_line(None, &line.text, &values)).collect();
                SpeakerItem { key, label, lines }
            })
            .collect();
        let batches = chunks(items, &target);
        let requests = batches.iter().map(|batch| SpeakersPrompt::build(batch, &context.hints, &target)).collect();
        run_batches(self.state, &open, Action::Speakers, &target, progress, requests, |index, text| SpeakersPrompt::accept(text, &batches[index]))
            .await
    }

    /// Запрос для чат-бота без API: правила и данные одним текстом.
    pub fn chat_request(&self, project: ProjectId, characters: &[CharacterId]) -> Result<String, AppError> {
        todo!()
    }

    /// Ответ чат-бота проходит те же проверки и попадает в тот же отбор.
    pub fn read_chat_answer(&self, project: ProjectId, answer: &str) -> Result<serde_json::Value, AppError> {
        todo!()
    }
}

/// Значения токенов: подставляются в реплики образцов, как при переводе.
fn token_values(glossary: &GlossaryData) -> HashMap<String, String> {
    glossary.tokens.iter().filter_map(|token| Some((token.name().to_string(), token.value()?.to_string()))).collect()
}

fn key_tail(key: &SpeakerKey) -> &str {
    key.as_str().split_once(':').map_or(key.as_str(), |(_, tail)| tail)
}

/// Пакеты по `batch_size` действия.
fn chunks<T: Clone>(items: Vec<T>, target: &LlmTarget) -> Vec<Vec<T>> {
    items.chunks(target.settings.batch_size.get()).map(<[T]>::to_vec).collect()
}

/// Запуск предложений: место запуска, запросы по очереди, разбор, итог в `llm_runs`. Ответ модели ждётся вне транзакции,
/// а в базу ничего не пишется. `accept` получает номер пакета и текст ответа.
async fn run_batches<T>(
    state: &AppState,
    open: &Arc<OpenProject>,
    action: Action,
    target: &LlmTarget,
    progress: Channel<RunProgress>,
    requests: Vec<ChatRequest>,
    accept: impl Fn(usize, &str) -> Vec<T>,
) -> Result<Suggestions<T>, AppError> {
    if requests.is_empty() {
        return Ok(Suggestions { items: Vec::new(), error: None });
    }
    let mut run = BatchRun::begin(state, open, action, target, progress, requests.len())?;
    let mut items = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        if !run.ask(request, &mut items, |text| accept(index, text)).await {
            break;
        }
    }
    run.finish(items)
}

/// Итог перевода имён: написания и языки имён, для которых не нашлось правил.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamesFound {
    pub suggestions: Suggestions<NameSuggestion>,
    pub missing_rules: Vec<LanguageTag>,
}

impl<T> Default for Suggestions<T> {
    fn default() -> Self {
        Self { items: Vec::new(), error: None }
    }
}

/// Перевод одного пакета имён с проверкой.
struct Spell<'a> {
    cast: &'a [String],
    hints: &'a Hints,
    rules: &'a [NameRule],
    target: &'a LlmTarget,
}

impl Spell<'_> {
    /// Перевод и проверка: два шага прогресса. `false`: запуск остановлен или оборвался, написания пакета
    /// остаются непроверенными.
    async fn batch(&self, run: &mut BatchRun<'_>, batch: &[NameItem], items: &mut Vec<NameSuggestion>) -> bool {
        let start = items.len();
        let request = NamesPrompt::build(batch, self.cast, self.hints, self.rules, self.target);
        if !run.ask(&request, items, |text| NamesPrompt::accept(text, batch)).await {
            CheckPrompt::unchecked(&mut items[start..]);
            return false;
        }
        if let Some(request) = CheckPrompt::build(batch, &items[start..], self.hints, self.rules, self.target) {
            match run.send(&request).await {
                Sent::Text(text) => CheckPrompt::apply(&text, batch, &mut items[start..]),
                Sent::Skipped => CheckPrompt::unchecked(&mut items[start..]),
                Sent::Stop => {
                    CheckPrompt::unchecked(&mut items[start..]);
                    return false;
                }
            }
        }
        run.step(items.len())
    }
}

/// Ответ на запрос пакета.
enum Sent {
    Text(String),
    /// Модель отказалась отвечать: пакет пропускается.
    Skipped,
    /// Запуск остановлен или оборвался.
    Stop,
}

/// Ход запуска предложений по пакетам: место запуска, прогресс, ошибка, которая обрывает запуск.
struct BatchRun<'a> {
    run: LlmRun,
    client: LlmClient,
    target: &'a LlmTarget,
    report: RunProgress,
    error: Option<String>,
}

impl<'a> BatchRun<'a> {
    fn begin(
        state: &AppState,
        open: &Arc<OpenProject>,
        action: Action,
        target: &'a LlmTarget,
        progress: Channel<RunProgress>,
        total: usize,
    ) -> Result<Self, AppError> {
        let run = LlmRun::begin(state, Some(open.clone()), action, target, progress)?;
        let report = RunProgress { total: total as u32, ..RunProgress::default() };
        run.report(report.clone());
        Ok(Self { run, client: LlmClient::for_kind(target.provider.kind()), target, report, error: None })
    }

    /// Пакет: запрос, разбор в `items`, отчёт. `false`: запуск остановлен или оборвался, дальше не спрашивать.
    async fn ask<T>(&mut self, request: &ChatRequest, items: &mut Vec<T>, accept: impl FnOnce(&str) -> Vec<T>) -> bool {
        match self.send(request).await {
            Sent::Text(text) => {
                let accepted = accept(&text);
                if accepted.is_empty() {
                    tracing::warn!("Из ответа модели ничего не взято:\n{text}");
                }
                items.extend(accepted);
            }
            Sent::Skipped => {}
            Sent::Stop => return false,
        }
        self.step(items.len())
    }

    /// Запрос без отчёта. Ошибка запоминается и обрывает запуск.
    async fn send(&mut self, request: &ChatRequest) -> Sent {
        if self.run.cancelled() || self.error.is_some() {
            return Sent::Stop;
        }
        match ask(self.client, self.target, request, &self.run).await {
            Ok(Some(text)) => Sent::Text(text),
            Ok(None) => {
                tracing::warn!("Модель отказалась отвечать на пакет, он пропущен");
                Sent::Skipped
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Sent::Stop
            }
        }
    }

    /// Пакет готов: отчёт с числом предложений. `false`: запуск остановлен.
    fn step(&mut self, written: usize) -> bool {
        self.report.done += 1;
        self.report.written = written as u32;
        self.run.report(self.report.clone());
        !self.run.cancelled()
    }

    /// Итог в `llm_runs`. Готовое не теряется и при ошибке.
    fn finish<T>(self, items: Vec<T>) -> Result<Suggestions<T>, AppError> {
        let status = match &self.error {
            Some(_) => RunStatus::Failed,
            None if self.run.cancelled() => RunStatus::Cancelled,
            None => RunStatus::Done,
        };
        self.run.finish(status, self.error.as_deref())?;
        Ok(Suggestions { items, error: self.error })
    }
}

/// Один запрос с повтором при сбое сети. `None`: модель отказалась отвечать.
async fn ask(client: LlmClient, target: &LlmTarget, request: &ChatRequest, run: &LlmRun) -> Result<Option<String>, LlmError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match client.send(target, request).await {
            Ok(text) => return Ok(Some(text)),
            Err(LlmError::Refused) => return Ok(None),
            Err(error) if error.is_transient() && attempt < ATTEMPTS && !run.cancelled() => {
                tracing::warn!("Запрос к модели не удался, повтор: {error}");
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::{GlossaryService, NameField, TermCategory};
    use crate::llm::client::tests::{completion, fake_model};
    use crate::llm::{LlmSettingsService, Profile, Provider, ProviderKind};
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    /// Открытый проект по синтетической игре с моделью на `url`.
    fn project(dir: &std::path::Path, url: &str) -> (AppState, ProjectId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        let settings = LlmSettingsService { state: &state };
        let provider = Provider::new("LM Studio", ProviderKind::Local, url, 0).unwrap();
        settings.save_provider(&provider, None).unwrap();
        settings.save_profile(&Profile::new(provider.id(), "", "gemma", 0).unwrap()).unwrap();
        (state, id)
    }

    fn user_text(body: &serde_json::Value) -> String {
        body["messages"][1]["content"].as_str().unwrap_or_default().to_string()
    }

    fn silent() -> Channel<RunProgress> {
        Channel::new(|_| Ok(()))
    }

    #[test]
    fn terms_and_speakers_come_back_as_suggestions_without_writing() {
        let (url, seen) = fake_model(|body| {
            let user = user_text(body);
            let answer = if user.contains("Speaker labels:") {
                r#"[{"id": 1, "translation": "Радио", "gender": "unknown"}]"#
            } else {
                r#"[{"id": 1, "translation": "Сторона", "category": "location", "note": "Левая сторона"}]"#
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let term = glossary.take_term(id, "side", TermCategory::General).unwrap();
        let radio = SpeakerKey::new("label:Radio Chatter").unwrap();
        glossary.keep_as_speaker(id, &radio).unwrap();
        let service = NamesService { state: &state };

        let terms = tauri::async_runtime::block_on(service.translate_terms(id, vec![term], None, silent())).unwrap();
        assert_eq!(
            terms,
            Suggestions {
                items: vec![TermSuggestion {
                    id: term,
                    translation: "Сторона".to_string(),
                    category: Some(TermCategory::Location),
                    description: Some("Левая сторона".to_string()),
                }],
                error: None,
            }
        );
        let request = user_text(&seen.lock().unwrap()[0]);
        assert!(request.contains(r#"example: "Eileen: Left side.""#), "{request}");

        let speakers = tauri::async_runtime::block_on(service.suggest_speakers(id, vec![radio.clone()], None, silent())).unwrap();
        assert_eq!(speakers.items, [SpeakerSuggestion { key: radio, translation: "Радио".to_string(), gender: None }]);
        let request = user_text(&seen.lock().unwrap()[1]);
        assert!(request.contains("Label: Radio Chatter\nLines:\nStatic.\n"), "{request}");

        let open = state.open_project(id).unwrap();
        let (translation, runs): (Option<String>, i64) = open
            .project
            .read(|tx| {
                Ok((
                    tx.query_row("SELECT translation FROM terms", [], |row| row.get(0))?,
                    tx.query_row("SELECT count(*) FROM llm_runs WHERE status = 'done'", [], |row| row.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!((translation, runs), (None, 2), "предложения в базу не пишутся, запуски записаны");
        assert!(!state.llm_run.is_active());
    }

    fn system_text(body: &serde_json::Value) -> String {
        body["messages"][0]["content"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn names_get_origins_rules_spelling_and_check_in_one_run() {
        let (url, seen) = fake_model(|body| {
            let (system, user) = (system_text(body), user_text(body));
            let answer = if system.contains("linguistic origins") {
                r#"[{"id": 1, "languages": ["ga", "en"], "reason": "Ирландское имя"}, {"id": 2, "languages": ["en"]}, {"id": 3, "languages": ["en"]}]"#
            } else if system.contains("review proposed") && user.contains("Miss Lee") {
                r#"[{"id": 1, "variants": [{"source": "Miss Lee", "translated": "Мисс Ли"}]}]"#
            } else if system.contains("review proposed") {
                r#"[{"id": 1, "variants": [{"source": "Eileen", "translated": "Айлин", "description": "Ирландское чтение"}, {"translated": "Эйлин"}], "issues": ["Чтение спорное"]}]"#
            } else if user.contains("form of") {
                r#"[{"id": 1, "translated_name": "Мисс Ли"}]"#
            } else {
                r#"[{"id": 1, "translated_name": "Эйлин"}, {"id": 2, "translated_name": "Ли"}]"#
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let eileen = glossary.create_character(id, "Eileen").unwrap();
        let open = state.open_project(id).unwrap();
        let mut character = open.project.read(|tx| GlossaryData::load(tx)).unwrap().characters.remove(0);
        character.set_last_name("Lee");
        character.set_gender(Some(crate::common::Gender::Female));
        character.add_form(crate::glossary::Form::new("Miss Lee", None).unwrap()).unwrap();
        glossary.save_character(id, &character).unwrap();
        open.project
            .write(|tx| {
                tx.execute("INSERT INTO llm_action_settings (action_id, batch_size) VALUES ('names', 5)", [])?;
                Ok(())
            })
            .unwrap();

        let rules = NameRuleService { state: &state };
        rules.save_shared(id, &LanguageTag::parse("en").unwrap(), "Ee это и").unwrap();

        let service = NamesService { state: &state };
        let found = tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent())).unwrap();
        assert_eq!(found.suggestions.error, None);
        assert_eq!(found.missing_rules, [LanguageTag::parse("ga").unwrap()]);
        let spelled: Vec<(NameField, &str, Vec<&str>, Vec<&str>)> = found
            .suggestions
            .items
            .iter()
            .map(|item| {
                (
                    item.field,
                    item.source.as_str(),
                    item.variants.iter().map(|variant| variant.translation.as_str()).collect(),
                    item.issues.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            spelled,
            [
                (NameField::First, "Eileen", vec!["Айлин", "Эйлин"], vec!["Происхождение неоднозначно: проверьте написание.", "Чтение спорное"]),
                (NameField::Last, "Lee", vec!["Ли"], vec!["Проверка пропустила это имя."]),
                (NameField::Form, "Miss Lee", vec!["Мисс Ли"], vec![]),
            ],
            "проверка заменила написание имени, пропущенная фамилия осталась с замечанием"
        );
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 5, "разбор, перевод и проверка имён, перевод и проверка формы");
            let origins = user_text(&requests[0]);
            assert!(origins.contains("- id: 3, name: \"Miss Lee\", another way to call the character, owner: \"Eileen Lee\"\n"), "{origins}");
            let names = user_text(&requests[1]);
            assert!(names.contains("All names in this game: Eileen Lee\n"), "{names}");
            assert!(names.contains("- id: 1: ga, en (Ирландское имя)\n- id: 2: en\n"), "{names}");
            assert!(names.contains("[en]\nEe это и\n"), "{names}");
            assert!(user_text(&requests[3]).contains("- id: 1, name: \"Miss Lee\", name of a woman, form of Айлин Ли\n"), "форма от проверенного имени");
        }
        let open = state.open_project(id).unwrap();
        let (translated, origins): (Option<String>, i64) = open
            .project
            .read(|tx| {
                Ok((
                    tx.query_row("SELECT translated_first_name FROM characters", [], |row| row.get(0))?,
                    tx.query_row("SELECT count(*) FROM name_origins", [], |row| row.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!((translated, origins), (None, 3), "написания в базу не пишутся, происхождение записано");

        tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 9, "второй раз происхождение не спрашивается");
    }

    #[test]
    fn own_rules_of_the_project_win_over_shared_ones() {
        let (url, _) = fake_model(|_| completion("[]"));
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let rules = NameRuleService { state: &state };
        let (sr, de) = (LanguageTag::parse("sr").unwrap(), LanguageTag::parse("de").unwrap());
        rules.save_shared(id, &sr, "общее").unwrap();
        rules.save_shared(id, &de, "немецкое").unwrap();
        rules.save_own(id, &sr, "своё").unwrap();
        let effective = rules.effective(id, &[sr.clone(), LanguageTag::undetermined(), sr.clone()]).unwrap();
        assert_eq!(effective.iter().map(NameRule::rules).collect::<Vec<_>>(), ["своё"]);

        let eileen = GlossaryService { state: &state }.create_character(id, "Eileen").unwrap();
        let open = state.open_project(id).unwrap();
        let origin = NameOrigin {
            field: NameField::First,
            source: "Eileen".to_string(),
            context: "Eileen".to_string(),
            languages: vec![LanguageTag::parse("ga").unwrap(), LanguageTag::undetermined()],
            reason: None,
        };
        open.project.write(|tx| CharacterRepo { conn: tx }.save_origins(eileen, std::slice::from_ref(&origin))).unwrap();
        assert_eq!(open.project.read(|tx| CharacterRepo { conn: tx }.origins(eileen)).unwrap(), [origin]);

        let overview: Vec<(String, usize, Option<String>, Option<String>)> = rules
            .overview(id)
            .unwrap()
            .into_iter()
            .map(|item| (item.language.to_string(), item.names, item.rules, item.own))
            .collect();
        assert_eq!(
            overview,
            [
                ("ga".to_string(), 1, None, None),
                ("de".to_string(), 0, Some("немецкое".to_string()), None),
                ("sr".to_string(), 0, Some("общее".to_string()), Some("своё".to_string())),
            ],
            "сначала языки имён проекта, und не язык правил"
        );
        rules.save_own(id, &sr, " ").unwrap();
        rules.save_shared(id, &de, "").unwrap();
        assert_eq!(rules.overview(id).unwrap().len(), 2);
    }

    #[test]
    fn tokens_without_lines_send_nothing() {
        let (url, seen) = fake_model(|_| completion("[]"));
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let found = tauri::async_runtime::block_on(NamesService { state: &state }.suggest_tokens(id, vec!["mc".to_string()], None, silent()));
        assert_eq!(found.unwrap(), Suggestions { items: Vec::new(), error: None });
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_request_keeps_earlier_suggestions() {
        let (url, _) = fake_model(|body| {
            if user_text(body).contains("\"first\"") {
                completion(r#"[{"id": 1, "translation": "Первый"}]"#)
            } else {
                serde_json::json!({ "error": "boom" }).to_string()
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let first = glossary.take_term(id, "first", TermCategory::General).unwrap();
        let second = glossary.take_term(id, "second", TermCategory::General).unwrap();
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                tx.execute("INSERT INTO llm_action_settings (action_id, batch_size) VALUES ('terms', 1)", [])?;
                Ok(())
            })
            .unwrap();

        let found =
            tauri::async_runtime::block_on(NamesService { state: &state }.translate_terms(id, vec![first, second], None, silent())).unwrap();
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].id, first);
        assert!(found.error.is_some(), "второй пакет оборвал запуск");
        let failed: i64 =
            open.project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM llm_runs WHERE status = 'failed'", [], |row| row.get(0))?)).unwrap();
        assert_eq!(failed, 1);
    }


    #[test]
    fn depth_rounds_up_and_clamps() {
        let depth = ReadingDepth::default();
        assert_eq!(depth.parts_for(3), 3);
        assert_eq!(depth.parts_for(10), 5);
        assert_eq!(depth.parts_for(21), 7);
        assert_eq!(depth.parts_for(1000), 50);
    }
}
