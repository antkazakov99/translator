//! Предложения модели по записям глоссария: написание имён, перевод терминов и подписей говорящих, значения
//! токенов, вопрос об одинаковых переводах, черновик правил имён. Промпты и разбор ответа лежат в `suggest.rs`
//! и `names.rs`, пишет ответ `GlossaryService::accept_*` и `fill_names`. Здесь порядок шагов: что спросить,
//! что сделать с ответом (`Taking`, `NameTaking`) и что осталось цепочке автоперевода (`names_work`, `glossary_work`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::entries::term_names;
use super::mentions::mentioned;
use super::names::{
    CheckPrompt, NameItem, NameSuggestion, NamesPrompt, ORIGIN_BATCH, OriginsPrompt, ask_key, cast, form_items, missing_rules, name_items,
    requests as name_requests, unasked, with_origins,
};
use super::repo::{AskKind, AskRepo, LineSampleRepo, PairCheckRepo, SampleLine};
use super::resolve::merge_pair;
use super::run::{BatchRun, Sent, Suggestions};
use super::same_entry::{self, SameEntryPrompt};
use super::service::{GlossaryService, NamePick, game_tokens};
use super::suggest::{
    SPEAKER_SAMPLES, SpeakerItem, SpeakerSuggestion, SpeakersPrompt, TERM_SAMPLES, TOKEN_SAMPLES, TermItem, TermSuggestion, TermsPrompt, TokenItem,
    TokenSuggestion, TokensPrompt, player_input_note, sample_line, spread, token_line,
};
use super::{Character, CharacterId, CharacterRepo, GlossaryData, NameKey, NameOrigin, SpeakerRepo, TermId};
use crate::common::{AppError, Timestamp, ValidationError};
use crate::engines::{SpeakerKey, VariableSource};
use crate::llm::{Action, ChatRequest, LlmTarget, LlmTargetService, ProfileId, RunHost};
use crate::name_rules::{LanguageTag, NameRule, NameRuleService, RulesDraftPrompt};
use crate::projects::ProjectId;
use crate::scan::ScanService;
use crate::storage::DbError;
use crate::state::{AppState, OpenProject};
use crate::translation::Hints;
use crate::translation::hints::GlossaryContext;
use crate::translation::text::token_names;

/// Что запуск предложений делает с ответом модели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taking {
    /// Окно глоссария: годное возвращается в отбор, в базу ничего не пишется.
    Offer,
    /// Цепочка автоперевода: годное пишется после каждого пакета, а запись, на которую годного ответа нет,
    /// запоминается в `glossary_asks` и больше цепочкой не спрашивается.
    Write,
}

/// Что запуск делает с написаниями имён.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameTaking {
    /// Окно глоссария: все написания идут в отбор, отметок нет.
    Offer,
    /// Цепочка, «Авто»: написание, на котором перевод и проверка сошлись, пишется сразу, остальные идут в отбор.
    Agreed,
    /// Цепочка, «Ручной выбор»: все написания идут в отбор.
    Picks,
}

/// Вид предложений по записям глоссария: чем различаются запуски терминов, подписей и токенов. Ход запуска
/// у них один (`SuggestService::offer`).
trait Offer {
    type Item: Clone;
    type Found;
    const ACTION: Action;
    const ASK: AskKind;

    /// Название записи для строки хода.
    fn label(item: &Self::Item) -> &str;
    fn build(batch: &[Self::Item], hints: &Hints, target: &LlmTarget) -> ChatRequest;
    fn accept(text: &str, batch: &[Self::Item]) -> Vec<Self::Found>;
    /// Предложение отвечает на запись пакета.
    fn answers(found: &Self::Found, item: &Self::Item) -> bool;
    /// Ключ отметки «спрошено».
    fn ask_key(item: &Self::Item) -> Option<String>;
    /// Ответ пакета идёт в подсказки следующих пакетов запуска.
    fn learn(hints: &mut Hints, batch: &[Self::Item], found: &[Self::Found]);
    fn write(service: &GlossaryService, project: ProjectId, found: &[Self::Found]) -> Result<usize, AppError>;
}

impl Offer for TermsPrompt {
    type Item = TermItem;
    type Found = TermSuggestion;
    const ACTION: Action = Action::Terms;
    const ASK: AskKind = AskKind::Term;

    fn label(item: &TermItem) -> &str {
        &item.name
    }

    fn build(batch: &[TermItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        TermsPrompt::build(batch, hints, target)
    }

    fn accept(text: &str, batch: &[TermItem]) -> Vec<TermSuggestion> {
        TermsPrompt::accept(text, batch)
    }

    fn answers(found: &TermSuggestion, item: &TermItem) -> bool {
        found.id == item.id
    }

    fn ask_key(item: &TermItem) -> Option<String> {
        NameKey::new(&item.name).map(|key| key.as_str().to_string())
    }

    fn learn(hints: &mut Hints, batch: &[TermItem], found: &[TermSuggestion]) {
        for suggestion in found {
            if let Some(item) = batch.iter().find(|item| item.id == suggestion.id) {
                hints.terms.push((item.name.clone(), suggestion.translation.clone()));
            }
        }
    }

    fn write(service: &GlossaryService, project: ProjectId, found: &[TermSuggestion]) -> Result<usize, AppError> {
        service.accept_terms(project, found)
    }
}

impl Offer for SpeakersPrompt {
    type Item = SpeakerItem;
    type Found = SpeakerSuggestion;
    const ACTION: Action = Action::Speakers;
    const ASK: AskKind = AskKind::Speaker;

    fn label(item: &SpeakerItem) -> &str {
        &item.label
    }

    fn build(batch: &[SpeakerItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        SpeakersPrompt::build(batch, hints, target)
    }

    fn accept(text: &str, batch: &[SpeakerItem]) -> Vec<SpeakerSuggestion> {
        SpeakersPrompt::accept(text, batch)
    }

    fn answers(found: &SpeakerSuggestion, item: &SpeakerItem) -> bool {
        found.key == item.key
    }

    fn ask_key(item: &SpeakerItem) -> Option<String> {
        Some(item.key.as_str().to_string())
    }

    /// Подпись с переводом идёт в имена: следующая подпись с тем же словом («Waitress 2») переведётся так же.
    fn learn(hints: &mut Hints, batch: &[SpeakerItem], found: &[SpeakerSuggestion]) {
        for suggestion in found {
            if let Some(item) = batch.iter().find(|item| item.key == suggestion.key) {
                hints.names.push((item.label.clone(), suggestion.translation.clone(), suggestion.gender));
            }
        }
    }

    fn write(service: &GlossaryService, project: ProjectId, found: &[SpeakerSuggestion]) -> Result<usize, AppError> {
        service.accept_speakers(project, found)
    }
}

impl Offer for TokensPrompt {
    type Item = TokenItem;
    type Found = TokenSuggestion;
    const ACTION: Action = Action::Tokens;
    const ASK: AskKind = AskKind::Token;

    fn label(item: &TokenItem) -> &str {
        &item.name
    }

    fn build(batch: &[TokenItem], hints: &Hints, target: &LlmTarget) -> ChatRequest {
        TokensPrompt::build(batch, hints, target)
    }

    fn accept(text: &str, batch: &[TokenItem]) -> Vec<TokenSuggestion> {
        TokensPrompt::accept(text, batch)
    }

    fn answers(found: &TokenSuggestion, item: &TokenItem) -> bool {
        found.name == item.name
    }

    fn ask_key(item: &TokenItem) -> Option<String> {
        Some(item.name.clone())
    }

    /// Пояснение токена без значения идёт в подсказки: в репликах следующих пакетов он остаётся токеном.
    fn learn(hints: &mut Hints, _batch: &[TokenItem], found: &[TokenSuggestion]) {
        for suggestion in found {
            if let (None, Some(note)) = (&suggestion.value, &suggestion.note) {
                hints.tokens.push((suggestion.name.clone(), note.clone()));
            }
        }
    }

    fn write(service: &GlossaryService, project: ProjectId, found: &[TokenSuggestion]) -> Result<usize, AppError> {
        service.accept_tokens(project, found)
    }
}

/// Имена и другие запуски по записям глоссария.
pub struct SuggestService<'a> {
    pub state: &'a AppState,
}

impl SuggestService<'_> {
    /// Имена и фамилии без перевода, потом формы: разбор происхождения тех, у кого его нет, правила их языков,
    /// написание и проверка каждого пакета. Происхождение записывается сразу. Что делается с написаниями, решает
    /// `taking`. Форма строится от записанного написания: формы персонажа, чьё имя ждёт перевода или отбора,
    /// остаются следующему запуску (`NamesFound::waiting_forms`). В цепочке имя, о котором она уже спрашивала,
    /// не спрашивается снова.
    pub async fn translate_names(
        &self,
        project: ProjectId,
        characters: Vec<CharacterId>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
        taking: NameTaking,
    ) -> Result<NamesFound, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Names, one_off)?;
        let chain = taking != NameTaking::Offer;
        let (glossary, stored, asked) = open.project.read(|tx| {
            let asked = if chain { AskRepo { conn: tx }.asked(AskKind::Name)? } else { HashSet::new() };
            Ok((GlossaryData::load(tx)?, CharacterRepo { conn: tx }.all_origins()?, asked))
        })?;
        let cast = cast(&glossary.characters);
        let chosen: Vec<&Character> =
            glossary.characters.iter().filter(|character| character.id().is_some_and(|id| characters.contains(&id))).collect();
        let known: Vec<(CharacterId, NameOrigin)> = stored.into_iter().flat_map(|(id, origins)| origins.into_iter().map(move |origin| (id, origin))).collect();
        let origins = || known.iter().map(|(id, origin)| (*id, origin));
        let mut names = unasked(name_items(&chosen), &asked);
        with_origins(&mut names, origins());
        // Формы персонажа, чьё имя сейчас спрашивается, ждут: основа формы это записанное написание.
        let waiting: HashSet<CharacterId> = names.iter().flat_map(|item| item.owners.iter().map(|owner| owner.character)).collect();
        let settled: Vec<&Character> = chosen.iter().copied().filter(|character| character.id().is_some_and(|id| !waiting.contains(&id))).collect();
        let mut forms = unasked(form_items(&settled), &asked);
        with_origins(&mut forms, origins());
        let waiting_forms = unasked(form_items(&chosen), &asked).len() - forms.len();
        if names.is_empty() && forms.is_empty() {
            return Ok(NamesFound { waiting_forms, ..NamesFound::default() });
        }

        let size = target.settings.batch_size.get();
        let total = name_requests(&names, &[], size) + name_requests(&[], &forms, size);
        let run = BatchRun::begin(self.state, &open, Action::Names, &target, host, total)?;
        let mut spelling = Spelling {
            state: self.state,
            open: &open,
            project,
            target: &target,
            cast,
            taking,
            hints: GlossaryContext::new(&glossary).hints,
            known,
            rules: Vec::new(),
            missing: Vec::new(),
            picks: Vec::new(),
            written: 0,
            run,
        };

        let going = spelling.spell("Написание имён", names).await?;
        let mut left = waiting_forms;
        // Цепочка без отбора: имена записаны или отмечены, формы их владельцев больше не ждут.
        if going && chain && spelling.picks.is_empty() && waiting_forms > 0 {
            let (fresh, asked) =
                open.project.read(|tx| Ok((CharacterRepo { conn: tx }.list()?, AskRepo { conn: tx }.asked(AskKind::Name)?)))?;
            let chosen: Vec<&Character> = fresh.iter().filter(|character| character.id().is_some_and(|id| characters.contains(&id))).collect();
            let all = unasked(form_items(&chosen), &asked);
            spelling.run.grow(name_requests(&[], &all, size).saturating_sub(name_requests(&[], &forms, size)));
            forms = all;
            left = 0;
        }
        if going {
            spelling.spell("Написание форм", forms).await?;
        }
        let Spelling { run, picks, written, missing, .. } = spelling;
        let mut suggestions = run.finish(picks)?;
        suggestions.written = written;
        Ok(NamesFound { suggestions, missing_rules: missing, waiting_forms: left })
    }

    /// Черновик правил передачи имён с языка `from` на язык перевода проекта. `existing`: текст, который
    /// черновик дополняет. В базу ничего не пишется.
    pub async fn draft_rules(
        &self,
        project: ProjectId,
        from: LanguageTag,
        existing: Option<String>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Suggestions<String>, AppError> {
        from.validate_registered()?;
        if !crate::name_rules::has_rules(&from) {
            return Err(ValidationError::BadLanguage(from.to_string()).into());
        }
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Names, one_off)?;
        let request = RulesDraftPrompt::build(&from, existing.as_deref(), &target);
        let mut run = BatchRun::begin(self.state, &open, Action::Names, &target, host, 1)?;
        let mut items = Vec::new();
        run.note(format!("Черновик правил имён: {from}"));
        run.ask(&request, &mut items, |text| RulesDraftPrompt::accept(text).into_iter().collect()).await;
        run.finish(items)
    }

    /// Перевод и категория терминов. Модель видит своё описание термина и несколько реплик с ним: упомянут ли
    /// термин в реплике, решает `mentions::mentioned` с ответами модели, поэтому «I hope so» примером термина
    /// «Hope» не станет. Термины идут от коротких названий к длинным: перевод «PPF» попадает в подсказки
    /// пакета с «PPF Headquarters».
    pub async fn translate_terms(
        &self,
        project: ProjectId,
        terms: Vec<TermId>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
        taking: Taking,
    ) -> Result<Suggestions<TermSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
        let (mut items, glossary) = open.project.read(|tx| {
            let data = GlossaryData::load(tx)?;
            let values = token_values(&data);
            let lines = LineSampleRepo { conn: tx }.all()?;
            let lowered: Vec<String> = lines.iter().map(|line| line.text.to_lowercase()).collect();
            let mut items = Vec::new();
            for id in &terms {
                let Some(term) = data.terms.iter().find(|term| term.id() == Some(*id)) else { continue };
                let names = term_names(term);
                let wanted: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
                let checks = data.term_checks.get(id);
                let found: Vec<&SampleLine> = lines
                    .iter()
                    .zip(&lowered)
                    .filter(|(line, lower)| wanted.iter().any(|name| lower.contains(name)) && mentioned(&line.text, &line.source_hash, &names, checks))
                    .map(|(line, _)| line)
                    .collect();
                items.push(TermItem {
                    id: *id,
                    name: term.name().to_string(),
                    category: term.category(),
                    description: term.description().map(str::to_string),
                    examples: spread(&found, TERM_SAMPLES).iter().map(|line| sample_line(line.speaker.as_deref(), &line.text, &values)).collect(),
                });
            }
            Ok((items, data))
        })?;
        items.sort_by_key(|item| item.name.split_whitespace().count());
        self.offer::<TermsPrompt>(project, &open, GlossaryContext::new(&glossary).hints, items, one_off, host.into(), taking).await
    }

    /// Вопросы «одно ли это» о терминах с одинаковым переводом («Fey Court» и «Fairy Court», оба «Двор фей»):
    /// «да» сливает пару, остальное запоминается, и пара больше не спрашивается. Возвращает строки слияний.
    pub async fn settle_same_translations(
        &self,
        project: ProjectId,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Suggestions<String>, AppError> {
        let open = self.state.open_project(project)?;
        let questions = open.project.read(|tx| same_entry::translation_questions(tx))?;
        if questions.is_empty() {
            return Ok(Suggestions::default());
        }
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Terms, one_off)?;
        let mut run = BatchRun::begin(self.state, &open, Action::Terms, &target, host, questions.len())?;
        let mut merged = Vec::new();
        for question in &questions {
            let Some(pair) = question.pair() else { continue };
            run.note(format!("одно ли это {} и {}", question.a.label(), question.b.label()));
            let same = match run.send(&SameEntryPrompt::build(question, &target)).await {
                Sent::Text(text) => SameEntryPrompt::accept(&text),
                Sent::Skipped => None,
                Sent::Stop => break,
            };
            if same == Some(true) {
                merged.extend(open.project.write(|tx| merge_pair(tx, pair))?);
            } else {
                // Ответ «нет», отказ и ответ, который не разобрать: пара остаётся двумя записями.
                open.project.write(|tx| PairCheckRepo { conn: tx }.mark(pair.a, pair.b, false, Timestamp::now()))?;
            }
            if !run.step(merged.len()) {
                break;
            }
        }
        let written = merged.len();
        let mut found = run.finish(merged)?;
        found.written = written;
        Ok(found)
    }

    /// Пояснения и значения токенов по репликам с `[___]` вместо токена. Токен без реплик пропускается: судить не о чем.
    ///
    /// Что скрипты игры говорят о переменной, отдаёт движок (`ScanService::text_variables`). Токен, значение
    /// которого вводит игрок, модели не отправляется: он сразу получает пояснение без значения, тем же путём,
    /// что ответ модели (цепочка пишет, окно получает в отбор). Константа идёт в запрос со значением из скрипта,
    /// у меняющейся переменной значение из ответа не берётся (`TokenItem::takes_value`).
    pub async fn suggest_tokens(
        &self,
        project: ProjectId,
        tokens: Vec<String>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
        taking: Taking,
    ) -> Result<Suggestions<TokenSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
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
        let lines: Vec<(String, Vec<SampleLine>)> = lines.into_iter().filter(|(_, found)| !found.is_empty()).collect();
        if lines.is_empty() {
            return Ok(Suggestions::default());
        }
        // Движок читает все скрипты снимка: долго, поэтому вне потоков запросов.
        let names: Vec<String> = lines.iter().map(|(name, _)| name.clone()).collect();
        let reader = open.clone();
        let variables = tauri::async_runtime::spawn_blocking(move || ScanService::text_variables(&reader, &names))
            .await
            .map_err(|_| AppError::Cancelled)??;
        let mut sources: HashMap<String, VariableSource> = variables.into_iter().map(|variable| (variable.name, variable.source)).collect();

        let values = token_values(&glossary);
        let mut hints = GlossaryContext::new(&glossary).hints;
        let mut typed: Vec<TokenSuggestion> = Vec::new();
        let mut items: Vec<TokenItem> = Vec::new();
        for (name, found) in lines {
            match sources.remove(&name).unwrap_or_default() {
                VariableSource::PlayerInput { prompt } => {
                    let note = player_input_note(prompt.as_deref());
                    // В репликах остальных токенов этот остаётся токеном: пояснение идёт в их подсказки.
                    hints.tokens.push((name.clone(), note.clone()));
                    typed.push(TokenSuggestion { name, value: None, note: Some(note) });
                }
                source => {
                    let lines =
                        spread(&found, TOKEN_SAMPLES).iter().map(|line| token_line(line.speaker.as_deref(), &line.text, &name, &values)).collect();
                    items.push(TokenItem { name, lines, source });
                }
            }
        }
        let written = match taking {
            Taking::Write => GlossaryService { state: self.state }.accept_tokens(project, &typed)?,
            Taking::Offer => 0,
        };
        let mut found = self.offer::<TokensPrompt>(project, &open, hints, items, one_off, host.into(), taking).await?;
        found.written += written;
        found.items.splice(0..0, typed);
        Ok(found)
    }

    /// Перевод подписи и род говорящих по нескольким их репликам.
    pub async fn suggest_speakers(
        &self,
        project: ProjectId,
        speakers: Vec<SpeakerKey>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
        taking: Taking,
    ) -> Result<Suggestions<SpeakerSuggestion>, AppError> {
        let open = self.state.open_project(project)?;
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
        let values = token_values(&glossary);
        let items: Vec<SpeakerItem> = found
            .into_iter()
            .map(|(key, label, lines)| {
                let lines = spread(&lines, SPEAKER_SAMPLES).iter().map(|line| sample_line(None, &line.text, &values)).collect();
                SpeakerItem { key, label, lines }
            })
            .collect();
        self.offer::<SpeakersPrompt>(project, &open, GlossaryContext::new(&glossary).hints, items, one_off, host.into(), taking).await
    }

    /// Запуск предложений пакетами: запрос, разбор, итог в `llm_runs`. Ответ модели ждётся вне транзакции.
    /// `hints`: подсказки глоссария на начало запуска. Ответ пакета идёт в подсказки следующих: знание растёт
    /// внутри запуска. `Taking::Write` пишет годное после каждого пакета и запоминает записи, на которые годного
    /// ответа нет.
    #[allow(clippy::too_many_arguments)]
    async fn offer<O: Offer>(
        &self,
        project: ProjectId,
        open: &Arc<OpenProject>,
        mut hints: Hints,
        items: Vec<O::Item>,
        one_off: Option<ProfileId>,
        host: RunHost,
        taking: Taking,
    ) -> Result<Suggestions<O::Found>, AppError> {
        // Без записей модель не нужна: запуск проходит и там, где она не настроена.
        if items.is_empty() {
            return Ok(Suggestions::default());
        }
        let target = LlmTargetService { state: self.state }.target(Some(open), O::ACTION, one_off)?;
        let batches = chunks(items, &target);
        if batches.is_empty() {
            return Ok(Suggestions::default());
        }
        let mut run = BatchRun::begin(self.state, open, O::ACTION, &target, host, batches.len())?;
        let mut found: Vec<O::Found> = Vec::new();
        let mut written = 0;
        for batch in &batches {
            run.note(batch_note(batch.iter().map(|item| O::label(item))));
            let accepted = match run.send(&O::build(batch, &hints, &target)).await {
                Sent::Text(text) => {
                    let accepted = O::accept(&text, batch);
                    if accepted.is_empty() {
                        tracing::warn!("Из ответа модели ничего не взято:\n{text}");
                    }
                    accepted
                }
                Sent::Skipped => Vec::new(),
                Sent::Stop => break,
            };
            O::learn(&mut hints, batch, &accepted);
            if taking == Taking::Write {
                written += O::write(&GlossaryService { state: self.state }, project, &accepted)?;
                let missed: Vec<String> =
                    batch.iter().filter(|item| !accepted.iter().any(|found| O::answers(found, item))).filter_map(|item| O::ask_key(item)).collect();
                open.project.write(|tx| {
                    let asks = AskRepo { conn: tx };
                    missed.iter().try_for_each(|key| asks.mark(O::ASK, key, Timestamp::now()))
                })?;
            }
            found.extend(accepted);
            if !run.step(if taking == Taking::Write { written } else { found.len() }) {
                break;
            }
        }
        let mut suggestions = run.finish(found)?;
        suggestions.written = written;
        Ok(suggestions)
    }
}

/// Записи глоссария без перевода, о которых цепочка ещё не спрашивала: работа шага `glossary`.
#[derive(Debug, Default)]
pub(crate) struct GlossaryWork {
    pub terms: Vec<TermId>,
    pub speakers: Vec<SpeakerKey>,
    pub tokens: Vec<String>,
    /// Пары терминов с одинаковым переводом, про которые модель ещё не отвечала, одно ли это.
    pub pairs: usize,
}

impl GlossaryWork {
    /// Примерное число запросов по размерам пакетов действий: термины, вопросы о парах, подписи, токены.
    pub(crate) fn requests(&self, batch: impl Fn(Action) -> usize) -> usize {
        let batches = |action: Action, items: usize| items.div_ceil(batch(action).max(1));
        batches(Action::Terms, self.terms.len())
            + self.pairs.min(same_entry::MAX_TRANSLATION_QUESTIONS)
            + batches(Action::Speakers, self.speakers.len())
            + batches(Action::Tokens, self.tokens.len())
    }
}

/// Имена без перевода, о которых цепочка ещё не спрашивала: работа шага `names`.
#[derive(Debug, Default)]
pub(crate) struct NamesWork {
    /// Персонажи с таким именем, фамилией или формой.
    pub characters: Vec<CharacterId>,
    /// Имена и фамилии: одно написание одного вида и рода считается один раз.
    pub names: usize,
    pub forms: usize,
    /// Примерное число запросов: разбор происхождения, написание и проверка.
    pub requests: usize,
}

/// Имена, фамилии и формы без перевода, о которых цепочка ещё не спрашивала (`glossary_asks`), и запросы на них
/// при пакете `batch`: разбор происхождения, написание и проверка.
pub(crate) fn names_work(tx: &rusqlite::Connection, batch: usize) -> Result<NamesWork, DbError> {
    let characters = CharacterRepo { conn: tx }.list()?;
    let origins = CharacterRepo { conn: tx }.all_origins()?;
    let asked = AskRepo { conn: tx }.asked(AskKind::Name)?;
    let all: Vec<&Character> = characters.iter().collect();
    let known: Vec<_> = origins.iter().flat_map(|(id, list)| list.iter().map(move |origin| (*id, origin))).collect();
    let mut names = unasked(name_items(&all), &asked);
    let mut forms = unasked(form_items(&all), &asked);
    with_origins(&mut names, known.iter().copied());
    with_origins(&mut forms, known.iter().copied());
    let mut owners: Vec<CharacterId> = Vec::new();
    for owner in names.iter().chain(&forms).flat_map(|item| &item.owners) {
        if !owners.contains(&owner.character) {
            owners.push(owner.character);
        }
    }
    Ok(NamesWork { characters: owners, names: names.len(), forms: forms.len(), requests: name_requests(&names, &forms, batch) })
}

/// Записи глоссария без перевода, о которых цепочка ещё не спрашивала (`glossary_asks`): термины, говорящие
/// текущего снимка без перевода подписи, токены из текста игры без значения и пояснения. И пары терминов
/// с одинаковым переводом без ответа модели, одно ли это.
///
/// Говорящий со своей записью (перевод или род задал пользователь) берётся всегда. Говорящий без записи, без
/// персонажа и не в корзине берётся, только когда разбор прочитал все сцены (`analyzed`): до этого он ещё может
/// оказаться персонажем, а запись с переводом закрыла бы его от разбора.
pub(crate) fn glossary_work(tx: &rusqlite::Connection, analyzed: bool) -> Result<GlossaryWork, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let asks = AskRepo { conn: tx };
    let (asked_terms, asked_speakers, asked_tokens) = (asks.asked(AskKind::Term)?, asks.asked(AskKind::Speaker)?, asks.asked(AskKind::Token)?);
    let speakers = SpeakerRepo { conn: tx };
    let current = speakers.current()?;
    let discarded = speakers.discarded()?;
    let terms = glossary
        .terms
        .iter()
        .filter(|term| term.translation().is_none() && !asked_terms.contains(term.name_key().as_str()))
        .filter_map(|term| term.id())
        .collect();
    let speakers = current
        .iter()
        .filter(|item| !asked_speakers.contains(item.speaker.key.as_str()))
        .filter(|item| {
            let key = &item.speaker.key;
            match glossary.entries.iter().find(|entry| entry.key() == key) {
                Some(entry) => entry.translation().is_none(),
                None => {
                    analyzed
                        && item.lines > 0
                        && !glossary.links.iter().any(|link| &link.speaker == key)
                        && !discarded.iter().any(|speaker| &speaker.key == key)
                }
            }
        })
        .map(|item| item.speaker.key.clone())
        .collect();
    let tokens = game_tokens(tx)?
        .into_keys()
        .filter(|name| !asked_tokens.contains(name))
        .filter(|name| glossary.tokens.iter().find(|token| token.name() == name).is_none_or(|token| token.value().is_none() && token.note().is_none()))
        .collect();
    let pairs = same_entry::translation_pairs(tx, &glossary)?.len();
    Ok(GlossaryWork { terms, speakers, tokens, pairs })
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

/// Записи пакета для строки хода: первые пять через запятую, остальные числом.
fn item_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    const SHOWN: usize = 5;
    let names: Vec<&str> = names.collect();
    let mut text = names.iter().take(SHOWN).copied().collect::<Vec<_>>().join(", ");
    if names.len() > SHOWN {
        text.push_str(&format!(" и ещё {}", names.len() - SHOWN));
    }
    text
}

/// «В пакете: Rusty Anchor, Old Lantern»: строка хода запуска предложений.
fn batch_note<'a>(names: impl Iterator<Item = &'a str>) -> String {
    format!("в пакете: {}", item_names(names))
}

/// Итог перевода имён: написания для отбора и языки имён, для которых не нашлось правил.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamesFound {
    /// Написания для отбора. `written`: сколько персонажей цепочка записала сама.
    pub suggestions: Suggestions<NameSuggestion>,
    pub missing_rules: Vec<LanguageTag>,
    /// Формы, которые ждут записи имени владельца: их спросит следующий запуск.
    pub waiting_forms: usize,
}

/// Ход перевода имён: что уже известно запуску и что он дал.
struct Spelling<'a> {
    state: &'a AppState,
    open: &'a Arc<OpenProject>,
    project: ProjectId,
    target: &'a LlmTarget,
    cast: Vec<String>,
    taking: NameTaking,
    /// Подсказки глоссария: пополняются написаниями этого запуска.
    hints: Hints,
    /// Происхождение имён: из базы и разобранное этим запуском.
    known: Vec<(CharacterId, NameOrigin)>,
    rules: Vec<NameRule>,
    missing: Vec<LanguageTag>,
    /// Написания для отбора.
    picks: Vec<NameSuggestion>,
    /// Сколько персонажей записано без отбора.
    written: usize,
    run: BatchRun<'a>,
}

impl Spelling<'_> {
    /// Имена одного вида: разбор происхождения тех, у кого его нет, правила их языков, написание и проверка
    /// пакетами. `false`: запуск остановлен или оборвался.
    async fn spell(&mut self, label: &str, mut items: Vec<NameItem>) -> Result<bool, AppError> {
        if items.is_empty() {
            return Ok(true);
        }
        self.with_origins(&mut items);
        if !self.origins(&items).await? {
            return Ok(false);
        }
        self.with_origins(&mut items);
        let languages: Vec<LanguageTag> = items.iter().flat_map(|item| item.languages.iter().cloned()).collect();
        self.rules = NameRuleService { state: self.state }.effective(self.project, &languages)?;
        for language in missing_rules(&items, &self.rules) {
            if !self.missing.contains(&language) {
                self.missing.push(language);
            }
        }
        let batches = chunks(items, self.target);
        let count = batches.len();
        for (index, batch) in batches.iter().enumerate() {
            if !self.batch(&format!("{label}: пакет {} из {count}", index + 1), batch).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn with_origins(&self, items: &mut [NameItem]) {
        with_origins(items, self.known.iter().map(|(id, origin)| (*id, origin)));
    }

    /// Разбор происхождения имён без него. Ответ пишется сразу. `false`: запуск остановлен или оборвался.
    async fn origins(&mut self, items: &[NameItem]) -> Result<bool, AppError> {
        let unknown: Vec<NameItem> = items.iter().filter(|item| item.languages.is_empty()).cloned().collect();
        let batches: Vec<&[NameItem]> = unknown.chunks(ORIGIN_BATCH).collect();
        let count = batches.len();
        for (index, batch) in batches.into_iter().enumerate() {
            self.run.note(format!("Происхождение имён: пакет {} из {count}: {}", index + 1, item_names(batch.iter().map(|item| item.name.as_str()))));
            match self.run.send(&OriginsPrompt::build(batch, &self.cast, self.target)).await {
                Sent::Text(text) => {
                    let found = OriginsPrompt::accept(&text, batch);
                    self.open.project.write(|tx| {
                        let repo = CharacterRepo { conn: tx };
                        found.iter().try_for_each(|(id, origin)| repo.save_origins(*id, std::slice::from_ref(origin)))
                    })?;
                    self.known.extend(found);
                    let invalid = OriginsPrompt::invalid(&text, batch);
                    if !invalid.is_empty() {
                        self.run.grow(1);
                        if !self.run.step(self.shown()) {
                            return Ok(false);
                        }
                        self.run.note("Происхождение имён: исправление неверных кодов языков".to_string());
                        let mut repaired = match self.run.send(&OriginsPrompt::repair(&invalid, &self.cast, self.target)).await {
                            Sent::Text(text) => OriginsPrompt::accept(&text, &invalid),
                            Sent::Skipped => Vec::new(),
                            Sent::Stop => return Ok(false),
                        };
                        repaired.extend(OriginsPrompt::unresolved(&invalid, &repaired));
                        self.open.project.write(|tx| {
                            let repo = CharacterRepo { conn: tx };
                            repaired.iter().try_for_each(|(id, origin)| repo.save_origins(*id, std::slice::from_ref(origin)))
                        })?;
                        self.known.extend(repaired);
                    }
                }
                Sent::Skipped => {}
                Sent::Stop => return Ok(false),
            }
            if !self.run.step(self.shown()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Число для хода запуска: записанное в цепочке «Авто», иначе написания для отбора.
    fn shown(&self) -> usize {
        if self.taking == NameTaking::Agreed { self.written } else { self.picks.len() }
    }

    /// Написание и проверка пакета: два шага прогресса. Написания идут в подсказки следующих пакетов. Цепочка
    /// пишет бесспорное и запоминает спрошенное. `false`: запуск остановлен или оборвался, написания пакета
    /// остались непроверенными.
    async fn batch(&mut self, label: &str, batch: &[NameItem]) -> Result<bool, AppError> {
        self.run.note(format!("{label}: {}", item_names(batch.iter().map(|item| item.name.as_str()))));
        let request = NamesPrompt::build(batch, &self.cast, &self.hints, &self.rules, self.target);
        let mut spelled = match self.run.send(&request).await {
            Sent::Text(text) => {
                let spelled = NamesPrompt::accept(&text, batch);
                if spelled.is_empty() {
                    tracing::warn!("Из ответа модели ничего не взято:\n{text}");
                }
                spelled
            }
            Sent::Skipped => Vec::new(),
            Sent::Stop => return Ok(false),
        };
        let mut going = self.run.step(self.shown());
        match CheckPrompt::build(batch, &spelled, &self.hints, &self.rules, self.target) {
            Some(request) if going => {
                self.run.note(format!("{label}: проверка написаний"));
                match self.run.send(&request).await {
                    Sent::Text(text) => CheckPrompt::apply(&text, batch, &mut spelled),
                    Sent::Skipped => CheckPrompt::unchecked(&mut spelled),
                    Sent::Stop => {
                        CheckPrompt::unchecked(&mut spelled);
                        going = false;
                    }
                }
            }
            _ => CheckPrompt::unchecked(&mut spelled),
        }
        self.take(batch, spelled)?;
        Ok(going && self.run.step(self.shown()))
    }

    /// Что делается с написаниями пакета: подсказки, запись бесспорного, отметки спрошенного, отбор.
    fn take(&mut self, batch: &[NameItem], spelled: Vec<NameSuggestion>) -> Result<(), AppError> {
        for item in batch {
            let first = spelled.iter().find(|found| found.field == item.field && found.source == item.name).and_then(|found| found.variants.first());
            if let Some(variant) = first {
                self.hints.names.push((item.name.clone(), variant.translation.clone(), item.gender));
            }
        }
        let (written, picks): (Vec<NameSuggestion>, Vec<NameSuggestion>) =
            spelled.into_iter().partition(|found| self.taking == NameTaking::Agreed && found.agreed);
        if !written.is_empty() {
            let chosen: Vec<NamePick> = written
                .iter()
                .filter_map(|found| {
                    let variant = found.variants.first()?;
                    Some(NamePick { character: found.character, field: found.field, source: found.source.clone(), translation: variant.translation.clone() })
                })
                .collect();
            self.written += GlossaryService { state: self.state }.fill_names(self.project, &chosen)?;
        }
        if self.taking != NameTaking::Offer {
            let asked: Vec<String> = batch
                .iter()
                .flat_map(|item| item.owners.iter().map(move |owner| (item, owner.character)))
                .filter(|(item, character)| !written.iter().any(|found| found.character == *character && found.field == item.field && found.source == item.name))
                .filter_map(|(item, character)| ask_key(character, item.field, &item.name))
                .collect();
            self.open.project.write(|tx| {
                let asks = AskRepo { conn: tx };
                asked.iter().try_for_each(|key| asks.mark(AskKind::Name, key, Timestamp::now()))
            })?;
        }
        self.picks.extend(picks);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::describe::tests::{project, project_with, silent, system_text, user_text};
    use crate::glossary::{NameField, TermCategory, TermRepo};
    use crate::llm::client::tests::{completion, fake_model};

    #[test]
    fn terms_and_speakers_come_back_as_suggestions_without_writing() {
        let (url, seen) = fake_model(|body| {
            let user = user_text(body);
            let answer = if user.contains("Speaker labels:") {
                r#"[{"id": 1, "translation": "Радио", "gender": "unknown"}]"#
            } else {
                r#"[{"id": 1, "translation": "Сторона", "category": "location"}]"#
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let term = glossary.take_term(id, "side", TermCategory::General).unwrap();
        let radio = SpeakerKey::new("label:Radio Chatter").unwrap();
        glossary.keep_as_speaker(id, &radio).unwrap();
        let service = SuggestService { state: &state };

        let terms = tauri::async_runtime::block_on(service.translate_terms(id, vec![term], None, silent(), Taking::Offer)).unwrap();
        assert_eq!(
            terms,
            Suggestions {
                items: vec![TermSuggestion { id: term, translation: "Сторона".to_string(), category: Some(TermCategory::Location) }],
                error: None,
                written: 0,
            }
        );
        let request = user_text(&seen.lock().unwrap()[0]);
        assert!(request.contains(r#"examples: ["Eileen: Left side."]"#), "{request}");

        let speakers = tauri::async_runtime::block_on(service.suggest_speakers(id, vec![radio.clone()], None, silent(), Taking::Offer)).unwrap();
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

    fn asked(state: &AppState, project: ProjectId, kind: AskKind) -> Vec<String> {
        let open = state.open_project(project).unwrap();
        let mut keys: Vec<String> = open.project.read(|tx| AskRepo { conn: tx }.asked(kind)).unwrap().into_iter().collect();
        keys.sort();
        keys
    }

    #[test]
    fn chain_writes_terms_batch_by_batch_and_remembers_what_got_no_answer() {
        let (url, seen) = fake_model(|body| {
            let user = user_text(body);
            let answer = if user.contains(r#"term: "Left side""#) {
                r#"[{"id": 1, "translation": "Левая сторона"}]"#
            } else if user.contains(r#"term: "Left""#) {
                r#"[{"id": 1, "translation": "Лево", "category": "location"}]"#
            } else {
                "[]"
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let side = glossary.take_term(id, "Left side", TermCategory::Item).unwrap();
        let left = glossary.take_term(id, "Left", TermCategory::General).unwrap();
        let hello = glossary.take_term(id, "Hello", TermCategory::General).unwrap();
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                tx.execute("INSERT INTO llm_action_settings (action_id, batch_size) VALUES ('terms', 1)", [])?;
                Ok(())
            })
            .unwrap();

        let service = SuggestService { state: &state };
        let found = tauri::async_runtime::block_on(service.translate_terms(id, vec![side, left, hello], None, silent(), Taking::Write)).unwrap();
        assert_eq!((found.written, found.error), (2, None));
        let requests: Vec<String> = seen.lock().unwrap().iter().map(user_text).collect();
        assert_eq!(requests.len(), 3);
        // Короткие названия раньше длинных. «Go left» из меню примером термина «Left» не стало: решает написание.
        assert!(requests[0].contains(r#"term: "Left", current_category: "general", examples: ["Eileen: Left side."]"#), "{}", requests[0]);
        assert!(requests[1].contains(r#"term: "Hello""#), "{}", requests[1]);
        assert!(requests[2].starts_with("Known terms: Left = Лево\n\n"), "перевод первого пакета в подсказках третьего: {}", requests[2]);

        let terms = open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap();
        let shown: Vec<(&str, Option<&str>, TermCategory)> = terms.iter().map(|term| (term.name(), term.translation(), term.category())).collect();
        assert_eq!(
            shown,
            [
                ("Hello", None, TermCategory::General),
                ("Left", Some("Лево"), TermCategory::Location),
                ("Left side", Some("Левая сторона"), TermCategory::Item),
            ],
            "годное записано после пакета, категорию получил только термин general"
        );
        assert_eq!(asked(&state, id, AskKind::Term), ["hello"], "термин без годного ответа запомнен");

        // Окно отметок не ставит и ничего не пишет.
        let offered = tauri::async_runtime::block_on(service.translate_terms(id, vec![hello], None, silent(), Taking::Offer)).unwrap();
        assert_eq!((offered.items.len(), offered.written), (0, 0));
        assert_eq!(seen.lock().unwrap().len(), 4, "кнопка окна спрашивает и отмеченное");
    }

    #[test]
    fn terms_with_one_translation_are_asked_about_and_the_answer_is_kept() {
        let (url, seen) = fake_model(|body| {
            let answer = if user_text(body).contains("Fey Court") { r#"{"same": true}"# } else { "Не уверен." };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let take = |name: &str, category: TermCategory| glossary.take_term(id, name, category).unwrap();
        let (fey, fairy) = (take("Fey Court", TermCategory::Location), take("Fairy Court", TermCategory::Organization));
        let (inn, tavern) = (take("Inn", TermCategory::Location), take("Tavern", TermCategory::Location));
        let suggestion = |id, translation: &str| TermSuggestion { id, translation: translation.to_string(), category: None };
        let written = glossary
            .accept_terms(id, &[suggestion(fey, "Двор фей"), suggestion(fairy, "Двор фей"), suggestion(inn, "Таверна"), suggestion(tavern, "Таверна")])
            .unwrap();
        assert_eq!(written, 4);
        let open = state.open_project(id).unwrap();
        let names = || open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap().iter().map(|term| term.name().to_string()).collect::<Vec<_>>();
        assert_eq!(names(), ["Fairy Court", "Fey Court", "Inn", "Tavern"], "запись перевода сама ничего не сливает");

        let service = SuggestService { state: &state };
        let found = tauri::async_runtime::block_on(service.settle_same_translations(id, None, silent())).unwrap();
        assert_eq!(found.items, ["Fairy Court → Fey Court (объединено)"]);
        assert_eq!((found.written, found.error), (1, None));
        assert_eq!(names(), ["Fey Court", "Inn", "Tavern"], "слито только то, про что модель сказала «одно»");
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(user_text(&requests[0]).starts_with("A (term, location): Fey Court\nB (term, organization): Fairy Court\n"), "{}", user_text(&requests[0]));
        }

        // Ответ, который не разобрать, тоже запомнен: пара остаётся двумя записями и больше не спрашивается.
        let again = tauri::async_runtime::block_on(service.settle_same_translations(id, None, silent())).unwrap();
        assert!(again.items.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert!(open.project.read(|tx| same_entry::translation_pairs(tx, &GlossaryData::load(tx)?)).unwrap().is_empty());
    }

    #[test]
    fn rules_reject_unregistered_codes_but_old_rules_can_be_read_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://localhost:1");
        let service = NameRuleService { state: &state };
        let invalid = LanguageTag::parse("jp").unwrap();
        assert!(service.save_shared(id, &invalid, "правило").is_err());
        assert!(service.save_own(id, &invalid, "правило").is_err());
        let rule = NameRule::new(invalid.clone(), LanguageTag::parse("ru").unwrap(), "старое правило").unwrap();
        state.app.write(|tx| crate::name_rules::NameRuleRepo { conn: tx }.save(&rule)).unwrap();
        let open = state.open_project(id).unwrap();
        let own = crate::name_rules::NameRuleOverride::new(invalid.clone(), "старое правило проекта").unwrap();
        open.project.write(|tx| crate::name_rules::NameRuleOverrideRepo { conn: tx }.save(&own)).unwrap();
        assert_eq!(service.overview(id).unwrap().len(), 1);
        service.save_shared(id, &invalid, "").unwrap();
        service.save_own(id, &invalid, "").unwrap();
        assert!(service.overview(id).unwrap().is_empty());
    }

    #[test]
    fn names_retry_invalid_language_once_and_keep_the_result() {
        for (invalid, corrected) in [("jp", true), ("jp", false), ("grk", true), ("grk", false)] {
            let (url, seen) = fake_model(move |body| {
                if system_text(body).contains("linguistic origins") {
                    if corrected && user_text(body).contains("A previous answer") {
                        completion(r#"[{"id":1,"languages":["ja"]}]"#)
                    } else {
                        completion(&format!(r#"[{{"id":1,"languages":["{invalid}"]}}]"#))
                    }
                } else {
                    completion("[]")
                }
            });
            let dir = tempfile::tempdir().unwrap();
            let (state, id) = project(dir.path(), &url);
            let character = GlossaryService { state: &state }.create_character(id, "Yuki").unwrap();
            let service = SuggestService { state: &state };
            let result =
                tauri::async_runtime::block_on(service.translate_names(id, vec![character], None, silent(), NameTaking::Offer)).unwrap();
            assert!(result.suggestions.error.is_none());
            let open = state.open_project(id).unwrap();
            let origins = open.project.read(|tx| CharacterRepo { conn: tx }.origins(character)).unwrap();
            assert_eq!(origins[0].languages[0].as_str(), if corrected { "ja" } else { "und" });
            if !corrected {
                assert!(origins[0].reason.as_ref().unwrap().contains("повторного запроса"));
            }
            tauri::async_runtime::block_on(service.translate_names(id, vec![character], None, silent(), NameTaking::Offer)).unwrap();
            let requests = seen.lock().unwrap();
            assert_eq!(requests.iter().filter(|request| system_text(request).contains("linguistic origins")).count(), 2);
        }
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

        let service = SuggestService { state: &state };
        let found = tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent(), NameTaking::Offer)).unwrap();
        assert_eq!(found.suggestions.error, None);
        assert_eq!(found.missing_rules, [LanguageTag::parse("ga").unwrap()]);
        assert_eq!(found.waiting_forms, 1, "форма ждёт, пока запишут имя и фамилию");
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
                (
                    NameField::First,
                    "Eileen",
                    vec!["Айлин", "Эйлин"],
                    vec!["Происхождение неоднозначно: проверьте написание.", "Перевод и проверка разошлись: выберите написание.", "Чтение спорное"]
                ),
                (NameField::Last, "Lee", vec!["Ли"], vec!["Проверка пропустила это имя."]),
            ],
            "проверка заменила написание имени, пропущенная фамилия осталась с замечанием"
        );
        assert!(!found.suggestions.items.iter().any(|item| item.agreed));
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 3, "разбор, перевод и проверка имён: форма ждёт");
            let origins = user_text(&requests[0]);
            assert!(origins.contains("- id: 1, name: \"Eileen\", given name, owner: \"Eileen Lee\"\n"), "{origins}");
            assert!(!origins.contains("Miss Lee"), "{origins}");
            let names = user_text(&requests[1]);
            assert!(names.contains("All names in this game: Eileen Lee\n"), "{names}");
            assert!(names.contains("- id: 1: ga, en (Ирландское имя)\n- id: 2: en\n"), "{names}");
            assert!(names.contains("[en]\nEe это и\n"), "{names}");
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
        assert_eq!((translated, origins), (None, 2), "написания в базу не пишутся, происхождение записано");

        // Выбор пользователя записан: форма строится от него, а не от первого варианта модели.
        let pick = |field, source: &str, translation: &str| NamePick { character: eileen, field, source: source.to_string(), translation: translation.to_string() };
        assert_eq!(glossary.fill_names(id, &[pick(NameField::First, "Eileen", "Эйлин"), pick(NameField::Last, "Lee", "Ли")]).unwrap(), 1);
        let forms = tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent(), NameTaking::Offer)).unwrap();
        assert_eq!(forms.waiting_forms, 0);
        let form = &forms.suggestions.items[0];
        assert_eq!((form.field, form.source.as_str(), form.variants[0].translation.as_str(), form.agreed), (NameField::Form, "Miss Lee", "Мисс Ли", true));
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 6, "разбор, перевод и проверка формы");
            assert!(user_text(&requests[4]).contains("- id: 1, name: \"Miss Lee\", name of a woman, form of Эйлин Ли\n"), "{}", user_text(&requests[4]));
        }

        tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent(), NameTaking::Offer)).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 8, "второй раз происхождение не спрашивается");
    }

    #[test]
    fn chain_writes_agreed_spellings_and_asks_the_rest_once() {
        let (url, seen) = fake_model(|body| {
            let (system, user) = (system_text(body), user_text(body));
            let answer = if system.contains("linguistic origins") {
                r#"[{"id": 1, "languages": ["en"]}, {"id": 2, "languages": ["en"]}]"#
            } else if system.contains("review proposed") && user.contains("Miss Lee") {
                r#"[{"id": 1, "variants": [{"source": "Miss Lee", "translated": "Мисс Ли"}]}]"#
            } else if system.contains("review proposed") {
                r#"[{"id": 1, "variants": [{"source": "Eileen", "translated": "Айлин"}]}, {"id": 2, "variants": [{"source": "Lee", "translated": "Ли"}]}]"#
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
        let character = || open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        let mut edited = character();
        edited.set_last_name("Lee");
        edited.add_form(crate::glossary::Form::new("Miss Lee", None).unwrap()).unwrap();
        glossary.save_character(id, &edited).unwrap();
        open.project
            .write(|tx| {
                tx.execute("INSERT INTO llm_action_settings (action_id, batch_size) VALUES ('names', 5)", [])?;
                Ok(())
            })
            .unwrap();
        let service = SuggestService { state: &state };
        let run = |taking| tauri::async_runtime::block_on(service.translate_names(id, vec![eileen], None, silent(), taking)).unwrap();

        // Фамилия бесспорна и записана. Имя спорно: оно идёт в отбор, а форма ждёт его.
        let first = run(NameTaking::Agreed);
        assert_eq!(first.suggestions.written, 1);
        let picks: Vec<(&str, &str)> = first.suggestions.items.iter().map(|item| (item.source.as_str(), item.variants[0].translation.as_str())).collect();
        assert_eq!(picks, [("Eileen", "Айлин")]);
        assert_eq!(first.waiting_forms, 1);
        assert_eq!((character().translated_first_name().map(str::to_string), character().translated_last_name().map(str::to_string)), (None, Some("Ли".to_string())));
        assert_eq!(asked(&state, id, AskKind::Name), [format!("{}:first:eileen", eileen.get())]);
        assert_eq!(seen.lock().unwrap().len(), 3);

        // Отбор отброшен: имя больше не спрашивается, форма строится от того, что записано.
        let second = run(NameTaking::Agreed);
        assert_eq!((second.suggestions.written, second.suggestions.items.len(), second.waiting_forms), (1, 0, 0));
        assert_eq!(character().forms()[0].translation(), Some("Мисс Ли"));
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 6, "разбор, перевод и проверка одной формы");
            assert!(user_text(&requests[4]).contains("name: \"Miss Lee\", name of a character, form of Ли\n"), "{}", user_text(&requests[4]));
        }

        // Делать больше нечего, а ручной режим отдал бы в отбор и бесспорное.
        let third = run(NameTaking::Picks);
        assert_eq!((third.suggestions.items.len(), seen.lock().unwrap().len()), (0, 6));
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
        let found = tauri::async_runtime::block_on(SuggestService { state: &state }.suggest_tokens(id, vec!["mc".to_string()], None, silent(), Taking::Write));
        assert_eq!(found.unwrap(), Suggestions::default());
        assert!(seen.lock().unwrap().is_empty());
    }

    /// Скрипт с переменными: ввод игрока, константа, переменная с двумя значениями, незаданная и константа без слов.
    const TOKENS_SCRIPT: &str = r#"define mom = _("Mom")
default title = "Mr."
define config.version = "1.0"

label names:
    $ hero = renpy.input("{i}What is your name?{/i}", length=12).strip()
    if hero == "Ann":
        $ title = "Ms."
    e "Hi, [hero]! Ask [mom]."
    e "Good day, [title] [hero!c]."
    e "Take [thing] with you."
    e "Version [config.version]."
    return
"#;

    const HERO_NOTE: &str = "Значение вводит игрок. Вопрос игры: «What is your name?»";

    fn token_names_of(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// Токены проекта: имя, значение, пояснение.
    fn stored_tokens(state: &AppState, project: ProjectId) -> Vec<(String, Option<String>, Option<String>)> {
        let open = state.open_project(project).unwrap();
        let tokens = open.project.read(|tx| crate::glossary::TokenRepo { conn: tx }.list()).unwrap();
        let mut shown: Vec<_> =
            tokens.iter().map(|token| (token.name().to_string(), token.value().map(str::to_string), token.note().map(str::to_string))).collect();
        shown.sort();
        shown
    }

    #[test]
    fn chain_takes_token_facts_from_the_scripts_of_the_game() {
        let (url, seen) = fake_model(|_| {
            completion(
                r#"[{"id": 1, "note": "Как герой зовёт мать", "value": "Мама"}, {"id": 2, "note": "Обращение к герою", "value": "господин"},
                    {"id": 3, "note": "", "value": ""}, {"id": 4, "note": "Версия игры", "value": "1.0"}]"#,
            )
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project_with(dir.path(), &url, &[("tokens.rpy", TOKENS_SCRIPT)]);
        let service = SuggestService { state: &state };
        let tokens = token_names_of(&["hero", "mom", "title", "thing", "config.version", "nobody"]);
        let found = tauri::async_runtime::block_on(service.suggest_tokens(id, tokens.clone(), None, silent(), Taking::Write)).unwrap();
        assert_eq!((found.written, found.error), (4, None));

        let requests: Vec<String> = seen.lock().unwrap().iter().map(user_text).collect();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.starts_with(&format!("Tokens: [hero] = {HERO_NOTE}\n\n")), "пояснение вводимого токена в подсказках: {request}");
        assert!(request.contains("\nid: 1\nScript value: \"Mom\"\nLines:\nEileen: Hi, [hero]! Ask [___].\n"), "{request}");
        assert!(request.contains("\nid: 2\nChanges during the game. Script values: \"Mr.\", \"Ms.\"\nLines:\nEileen: Good day, [___] [hero!c].\n"), "{request}");
        assert!(request.contains("\nid: 3\nLines:\nEileen: Take [___] with you.\n"), "{request}");
        assert!(request.contains("\nid: 4\nScript value: \"1.0\"\nLines:\n"), "{request}");
        assert!(!request.contains("\nid: 5\n"), "токен, который вводит игрок, и токен без реплик в запрос не идут: {request}");

        let token = |name: &str, value: Option<&str>, note: Option<&str>| (name.to_string(), value.map(str::to_string), note.map(str::to_string));
        assert_eq!(
            stored_tokens(&state, id),
            [
                token("config.version", None, Some("Версия игры")),
                token("hero", None, Some(HERO_NOTE)),
                token("mom", Some("Мама"), Some("Как герой зовёт мать")),
                token("title", None, Some("Обращение к герою")),
            ],
            "значение записано только константе со словами, вводимый токен получил пояснение без модели"
        );
        assert_eq!(asked(&state, id, AskKind::Token), ["thing"], "токен без годного ответа запомнен");

        // Записанное цепочке больше не работа. «nobody» в репликах нет: его нет и в работе.
        let open = state.open_project(id).unwrap();
        assert!(open.project.read(|tx| glossary_work(tx, true)).unwrap().tokens.is_empty());
    }

    #[test]
    fn typed_in_token_comes_to_the_window_without_the_model_and_fills_only_an_empty_note() {
        let dir = tempfile::tempdir().unwrap();
        // Модели по этому адресу нет: запрос оборвал бы запуск.
        let (state, id) = project_with(dir.path(), "http://127.0.0.1:9/v1", &[("tokens.rpy", TOKENS_SCRIPT)]);
        let service = SuggestService { state: &state };
        let run = |taking| tauri::async_runtime::block_on(service.suggest_tokens(id, token_names_of(&["hero"]), None, silent(), taking)).unwrap();

        let offered = run(Taking::Offer);
        let suggestion = TokenSuggestion { name: "hero".to_string(), value: None, note: Some(HERO_NOTE.to_string()) };
        assert_eq!(offered, Suggestions { items: vec![suggestion.clone()], error: None, written: 0 });
        assert!(stored_tokens(&state, id).is_empty(), "окно само не пишет");
        let runs: i64 =
            state.open_project(id).unwrap().project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM llm_runs", [], |row| row.get(0))?)).unwrap();
        assert_eq!(runs, 0, "запуска модели не было");

        // Своё пояснение остаётся: запись идёт только в пустое поле.
        let glossary = GlossaryService { state: &state };
        let own = TokenSuggestion { name: "hero".to_string(), value: None, note: Some("Имя героя".to_string()) };
        assert_eq!(glossary.accept_tokens(id, &[own]).unwrap(), 1);
        let written = run(Taking::Write);
        assert_eq!((written.written, written.items), (0, vec![suggestion]));
        assert_eq!(stored_tokens(&state, id), [("hero".to_string(), None, Some("Имя героя".to_string()))]);
    }

    /// Шаг токенов на настоящей игре из `TRANSLATOR_TEST_VARIABLES_GAME` (папка `game`): печатает токены, которые
    /// получили пояснение без модели, и первый запрос к модели. Модель поддельная и отвечает пустым списком.
    #[test]
    #[ignore = "Читает настоящую игру: TRANSLATOR_TEST_VARIABLES_GAME"]
    fn token_step_reads_a_real_game() {
        use crate::glossary::describe::tests::project_of;

        let Some(game) = crate::engines::renpy::local_test_game("TRANSLATOR_TEST_VARIABLES_GAME") else { return };
        let (url, seen) = fake_model(|_| completion("[]"));
        let dir = tempfile::tempdir().unwrap();
        let root = crate::projects::GameRoot::new(std::path::Path::new(&game).parent().unwrap()).unwrap();
        let (state, id) = project_of(dir.path(), &url, root);
        let tokens = state.open_project(id).unwrap().project.read(|tx| glossary_work(tx, true)).unwrap().tokens;
        let started = std::time::Instant::now();
        let found = tauri::async_runtime::block_on(SuggestService { state: &state }.suggest_tokens(id, tokens.clone(), None, silent(), Taking::Offer)).unwrap();
        let requests = seen.lock().unwrap();
        println!("{game}: токенов {}, без модели {}, запросов {}, шаг {:?}", tokens.len(), found.items.len(), requests.len(), started.elapsed());
        for item in &found.items {
            println!("[{}] {}", item.name, item.note.as_deref().unwrap_or_default());
        }
        if let Some(first) = requests.first() {
            println!("{}", user_text(first));
        }
        assert_eq!(found.error, None);
        assert!(found.items.iter().all(|item| item.value.is_none()), "без модели значение не пишется");
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
            tauri::async_runtime::block_on(SuggestService { state: &state }.translate_terms(id, vec![first, second], None, silent(), Taking::Offer))
                .unwrap();
        assert_eq!(found.items.len(), 1);
        assert_eq!(found.items[0].id, first);
        assert!(found.error.is_some(), "второй пакет оборвал запуск");
        let failed: i64 =
            open.project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM llm_runs WHERE status = 'failed'", [], |row| row.get(0))?)).unwrap();
        assert_eq!(failed, 1);
    }

    #[test]
    fn speaker_without_entry_waits_for_the_analysis() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let glossary = GlossaryService { state: &state };
        let radio = SpeakerKey::new("label:Radio Chatter").unwrap();
        glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        glossary.keep_as_speaker(id, &radio).unwrap();
        let open = state.open_project(id).unwrap();
        let speakers = |analyzed: bool| open.project.read(|tx| glossary_work(tx, analyzed)).unwrap().speakers;
        assert_eq!(speakers(false), std::slice::from_ref(&radio), "говорящий со своей записью берётся всегда");

        // Запись убрана: до конца разбора говорящий ещё может оказаться персонажем.
        glossary.delete_speaker_entry(id, &radio).unwrap();
        assert!(speakers(false).is_empty());
        assert_eq!(speakers(true), std::slice::from_ref(&radio), "после разбора подпись говорящего без персонажа переводится");

        // Перевод от модели создаёт запись сам.
        let found = SpeakerSuggestion { key: radio.clone(), translation: "Радио".to_string(), gender: None };
        assert_eq!(glossary.accept_speakers(id, &[found]).unwrap(), 1);
        assert!(speakers(true).is_empty());

        glossary.delete_speaker_entry(id, &radio).unwrap();
        glossary.discard_speaker(id, &radio).unwrap();
        assert!(speakers(true).is_empty(), "говорящий в корзине не переводится");
    }
}
