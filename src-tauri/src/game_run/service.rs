//! Цепочка «Вся игра»: сводка того, что осталось, и шаги по очереди. Шаги вызывают сервисы действий как есть,
//! внутри одной `RunChain`: место запуска занято на всю цепочку, у каждого вызова своя строка `llm_runs`.

use std::sync::Arc;

use tauri::ipc::Channel;

use super::{GameRun, GameStep, NamesMode, StepResult, StepWork, WorkCount, ordered};
use crate::common::AppError;
use crate::engines::{ContainerKey, SpeakerKey};
use crate::glossary::names::{ORIGIN_BATCH, form_items, name_items, with_origins};
use crate::glossary::service::game_tokens;
use crate::glossary::{
    Character, CharacterId, CharacterRepo, DescribeService, DescribeTarget, Described, GlossaryData, GlossaryService, NamePick, NamesFound,
    NamesService, SpeakerRepo, TermId,
};
use crate::llm::{Action, LlmTargetService, RunChain, RunProgress};
use crate::projects::ProjectId;
use crate::state::{AppState, OpenProject};
use crate::storage::DbError;
use crate::translation::{LinesMode, TranslationScope, TranslationService};

pub struct GameRunService<'a> {
    pub state: &'a AppState,
}

/// Записи глоссария без перевода: работа шага `glossary`.
#[derive(Debug, Default)]
struct GlossaryWork {
    terms: Vec<TermId>,
    speakers: Vec<SpeakerKey>,
    tokens: Vec<String>,
    /// Персонажи с именем, фамилией или формой без перевода.
    characters: Vec<CharacterId>,
}

/// Чем кончился шаг.
#[derive(Debug, Default)]
struct StepOutcome {
    written: usize,
    error: Option<String>,
    names: Option<NamesFound>,
}

impl StepOutcome {
    fn from_described(described: Described) -> Self {
        Self { written: described.written.len() + described.added.len(), error: described.error, names: None }
    }
}

impl GameRunService<'_> {
    /// Что осталось каждому шагу и примерное число запросов.
    pub fn overview(&self, project: ProjectId) -> Result<Vec<StepWork>, AppError> {
        let open = self.state.open_project(project)?;
        let scenes = TranslationService { state: self.state }.story_scenes(project)?;
        let all: Vec<ContainerKey> = scenes.iter().filter_map(|scene| scene.key.clone()).collect();
        let unread: Vec<ContainerKey> =
            scenes.iter().filter(|scene| !scene.described || scene.stale).filter_map(|scene| scene.key.clone()).collect();
        let describe = DescribeService { state: self.state };
        let lines = TranslationService { state: self.state };

        let analysis = describe.pending_analysis(project, all)?;
        let (glossary, entries) = open.project.read(|tx| Ok((glossary_work(tx)?, entry_targets(&GlossaryData::load(tx)?))))?;
        let glossary_requests = self.glossary_requests(&open, &glossary)?;
        let entries = describe.pending(project, entries)?;
        let scenes = describe.pending(project, unread.into_iter().map(DescribeTarget::Scene).collect())?;
        let translate = lines.pending(project, LinesMode::Translate, false)?;
        let review = lines.pending(project, LinesMode::Review, true)?;

        let work = |step, left: Vec<(&'static str, usize)>, requests| StepWork {
            step,
            left: left.into_iter().map(|(label, count)| WorkCount { label, count }).collect(),
            requests,
        };
        Ok(vec![
            work(GameStep::Analysis, vec![("сцен с непрочитанными частями", analysis.items)], analysis.requests),
            work(
                GameStep::Glossary,
                vec![
                    ("терминов без перевода", glossary.terms.len()),
                    ("подписей без перевода", glossary.speakers.len()),
                    ("токенов без значения", glossary.tokens.len()),
                    ("персонажей с именами без перевода", glossary.characters.len()),
                ],
                glossary_requests,
            ),
            work(GameStep::Entries, vec![("персонажей и терминов без описания", entries.items)], entries.requests),
            work(GameStep::Scenes, vec![("сцен без описания", scenes.items)], scenes.requests),
            work(GameStep::Lines, vec![("реплик без перевода", translate.items)], translate.requests),
            work(GameStep::Review, vec![("реплик не вычитано", review.items)], review.requests),
        ])
    }

    /// Отмеченные шаги по порядку одной цепочкой. Остановка и ошибка обрывают цепочку после текущего запроса,
    /// готовое остаётся. Ручной выбор имён останавливает цепочку на отборе: окно продолжает её с `rest`.
    pub async fn run(&self, project: ProjectId, steps: &[GameStep], names: NamesMode, progress: Channel<RunProgress>) -> Result<GameRun, AppError> {
        let steps = ordered(steps);
        let chain = RunChain::begin(self.state, progress)?;
        let count = steps.len();
        let mut result = GameRun::default();
        for (index, step) in steps.iter().copied().enumerate() {
            if chain.cancelled() {
                result.stopped = true;
                break;
            }
            chain.stage(index + 1, count, step.title());
            let outcome = match self.step(project, step, names, &chain, index + 1, count).await {
                Ok(outcome) => outcome,
                Err(error) => StepOutcome { error: Some(error.to_string()), ..StepOutcome::default() },
            };
            if let Some(error) = outcome.error {
                result.error = Some(format!("{}: {error}", step.title()));
                result.names = outcome.names;
                break;
            }
            if chain.cancelled() {
                result.stopped = true;
                result.names = outcome.names;
                break;
            }
            result.finished.push(StepResult { step, written: outcome.written });
            if outcome.names.is_some() {
                result.names = outcome.names;
                result.rest = steps[index + 1..].to_vec();
                break;
            }
        }
        Ok(result)
    }

    /// Написания имён из отбора в пустые поля персонажей. Возвращает число изменённых персонажей.
    pub fn write_names(&self, project: ProjectId, picks: &[NamePick]) -> Result<usize, AppError> {
        GlossaryService { state: self.state }.fill_names(project, picks)
    }

    async fn step(
        &self,
        project: ProjectId,
        step: GameStep,
        names: NamesMode,
        chain: &RunChain,
        index: usize,
        count: usize,
    ) -> Result<StepOutcome, AppError> {
        let describe = DescribeService { state: self.state };
        let lines = TranslationService { state: self.state };
        match step {
            GameStep::Analysis => {
                let keys = self.scene_keys(project, |_| true)?;
                Ok(StepOutcome::from_described(describe.analyze(project, keys, None, chain).await?))
            }
            GameStep::Glossary => self.glossary(project, names, chain, index, count).await,
            GameStep::Entries => {
                let open = self.state.open_project(project)?;
                let targets = open.project.read(|tx| Ok(entry_targets(&GlossaryData::load(tx)?)))?;
                Ok(StepOutcome::from_described(describe.describe_many(project, targets, None, chain).await?))
            }
            GameStep::Scenes => {
                let keys = self.scene_keys(project, |scene| !scene.described || scene.stale)?;
                let targets = keys.into_iter().map(DescribeTarget::Scene).collect();
                Ok(StepOutcome::from_described(describe.describe_many(project, targets, None, chain).await?))
            }
            GameStep::Lines => {
                let run = lines.translate(project, TranslationScope::Project, None, chain).await?;
                Ok(StepOutcome { written: run.written, ..StepOutcome::default() })
            }
            GameStep::Review => {
                let run = lines.review_unreviewed(project, TranslationScope::Project, None, chain).await?;
                Ok(StepOutcome { written: run.written, ..StepOutcome::default() })
            }
        }
    }

    /// Термины, подписи и токены принимаются без отбора, имена по `names`. Каждая часть своим запуском.
    async fn glossary(&self, project: ProjectId, names: NamesMode, chain: &RunChain, index: usize, count: usize) -> Result<StepOutcome, AppError> {
        let open = self.state.open_project(project)?;
        let work = open.project.read(|tx| glossary_work(tx))?;
        let suggest = NamesService { state: self.state };
        let write = GlossaryService { state: self.state };
        let mut outcome = StepOutcome::default();
        let stop = |error: &Option<String>| error.is_some() || chain.cancelled();

        if !work.terms.is_empty() {
            chain.stage(index, count, "Перевод терминов");
            let found = suggest.translate_terms(project, work.terms, None, chain).await?;
            outcome.written += write.accept_terms(project, &found.items)?;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        if !work.speakers.is_empty() {
            chain.stage(index, count, "Перевод подписей");
            let found = suggest.suggest_speakers(project, work.speakers, None, chain).await?;
            outcome.written += write.accept_speakers(project, &found.items)?;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        if !work.tokens.is_empty() {
            chain.stage(index, count, "Значения токенов");
            let found = suggest.suggest_tokens(project, work.tokens, None, chain).await?;
            outcome.written += write.accept_tokens(project, &found.items)?;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        if !work.characters.is_empty() {
            chain.stage(index, count, "Перевод имён");
            let found = suggest.translate_names(project, work.characters, None, chain).await?;
            outcome.error = found.suggestions.error.clone();
            match names {
                NamesMode::Auto => outcome.written += write.fill_names(project, &first_variants(&found))?,
                NamesMode::Manual if !found.suggestions.items.is_empty() => outcome.names = Some(found),
                NamesMode::Manual => {}
            }
        }
        Ok(outcome)
    }

    /// Ключи сцен по сюжету, которые подходят шагу.
    fn scene_keys(&self, project: ProjectId, wanted: impl Fn(&crate::translation::SceneProgress) -> bool) -> Result<Vec<ContainerKey>, AppError> {
        let scenes = TranslationService { state: self.state }.story_scenes(project)?;
        Ok(scenes.iter().filter(|scene| wanted(scene)).filter_map(|scene| scene.key.clone()).collect())
    }

    /// Запросы шага `glossary` по размерам пакетов действий: термины, подписи, токены, потом имена
    /// (разбор происхождения, написание и проверка имён и форм).
    fn glossary_requests(&self, open: &Arc<OpenProject>, work: &GlossaryWork) -> Result<usize, AppError> {
        let batches = |action: Action, items: usize| items.div_ceil(self.batch_size(open, action));
        let (glossary, origins) = open.project.read(|tx| Ok((GlossaryData::load(tx)?, CharacterRepo { conn: tx }.all_origins()?)))?;
        let chosen: Vec<&Character> =
            glossary.characters.iter().filter(|character| character.id().is_some_and(|id| work.characters.contains(&id))).collect();
        let known: Vec<_> = origins.iter().flat_map(|(id, list)| list.iter().map(move |origin| (*id, origin))).collect();
        let mut names = name_items(&chosen);
        let mut forms = form_items(&chosen, &[]);
        with_origins(&mut names, known.iter().copied());
        with_origins(&mut forms, known.iter().copied());
        let unknown = names.iter().chain(&forms).filter(|item| item.languages.is_empty()).count();
        Ok(batches(Action::Terms, work.terms.len())
            + batches(Action::Speakers, work.speakers.len())
            + batches(Action::Tokens, work.tokens.len())
            + unknown.div_ceil(ORIGIN_BATCH)
            + 2 * (batches(Action::Names, names.len()) + batches(Action::Names, forms.len())))
    }

    /// Размер пакета действия. Без настроенной модели значение из кода.
    fn batch_size(&self, open: &Arc<OpenProject>, action: Action) -> usize {
        LlmTargetService { state: self.state }
            .target(Some(open), action, None)
            .map_or_else(|_| action.default_batch_size(), |target| target.settings.batch_size)
            .get()
    }
}

/// Первое написание каждого имени: промпт ставит первым самое устоявшееся.
fn first_variants(found: &NamesFound) -> Vec<NamePick> {
    found
        .suggestions
        .items
        .iter()
        .filter_map(|item| {
            let variant = item.variants.first()?;
            Some(NamePick { character: item.character, field: item.field, source: item.source.clone(), translation: variant.translation.clone() })
        })
        .collect()
}

/// Персонажи с именем и термины без описания: работа шага `entries`.
fn entry_targets(glossary: &GlossaryData) -> Vec<DescribeTarget> {
    let characters = glossary
        .characters
        .iter()
        .filter(|character| character.description().is_none() && character.full_name().is_some())
        .filter_map(|character| Some(DescribeTarget::Character(character.id()?)));
    let terms = glossary.terms.iter().filter(|term| term.description().is_none()).filter_map(|term| Some(DescribeTarget::Term(term.id()?)));
    characters.chain(terms).collect()
}

/// Записи глоссария без перевода, как их видят разделы глоссария: термины, записи говорящих текущего снимка,
/// токены из текста игры без значения и пояснения, персонажи с непереведённым именем, фамилией или формой.
fn glossary_work(tx: &rusqlite::Connection) -> Result<GlossaryWork, DbError> {
    let glossary = GlossaryData::load(tx)?;
    let current = SpeakerRepo { conn: tx }.current()?;
    let terms = glossary.terms.iter().filter(|term| term.translation().is_none()).filter_map(|term| term.id()).collect();
    let speakers = current
        .iter()
        .map(|item| &item.speaker.key)
        .filter(|key| glossary.entries.iter().any(|entry| entry.key() == *key && entry.translation().is_none()))
        .cloned()
        .collect();
    let tokens = game_tokens(tx)?
        .into_keys()
        .filter(|name| glossary.tokens.iter().find(|token| token.name() == name).is_none_or(|token| token.value().is_none() && token.note().is_none()))
        .collect();
    let characters = glossary
        .characters
        .iter()
        .filter(|character| {
            (character.first_name().is_some() && character.translated_first_name().is_none())
                || (character.last_name().is_some() && character.translated_last_name().is_none())
                || character.forms().iter().any(|form| form.translation().is_none())
        })
        .filter_map(Character::id)
        .collect();
    Ok(GlossaryWork { terms, speakers, tokens, characters })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::{NameField, TermCategory, TermRepo};
    use crate::llm::client::tests::{completion, fake_model};
    use crate::llm::{LlmSettingsService, Profile, Provider, ProviderKind};
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    /// Открытый проект по синтетической игре с моделью на `url`. Говорящий `e` стал персонажем Eileen,
    /// подпись «Radio Chatter» оставлена говорящим без перевода.
    fn project(dir: &std::path::Path, url: &str) -> (AppState, ProjectId, CharacterId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        let settings = LlmSettingsService { state: &state };
        let provider = Provider::new("LM Studio", ProviderKind::Local, url, 0).unwrap();
        settings.save_provider(&provider, None).unwrap();
        settings.save_profile(&Profile::new(provider.id(), "", "gemma", 0).unwrap()).unwrap();
        let glossary = GlossaryService { state: &state };
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        glossary.keep_as_speaker(id, &SpeakerKey::new("label:Radio Chatter").unwrap()).unwrap();
        (state, id, eileen)
    }

    fn text(body: &serde_json::Value, index: usize) -> String {
        body["messages"][index]["content"].as_str().unwrap_or_default().to_string()
    }

    /// Модель на все действия цепочки: ответ по виду запроса.
    fn reply(body: &serde_json::Value) -> String {
        let (system, user) = (text(body, 0), text(body, 1));
        let answer = if system.contains("proper names") {
            r#"[{"kind": "location", "name": "Left side"}]"#.to_string()
        } else if system.contains("terms of a visual novel") {
            r#"[{"id": 1, "translation": "Левая сторона", "category": "location"}]"#.to_string()
        } else if system.contains("speaker labels") {
            r#"[{"id": 1, "translation": "Радио", "gender": "unknown"}]"#.to_string()
        } else if system.contains("linguistic origins") {
            r#"[{"id": 1, "languages": ["en"]}]"#.to_string()
        } else if system.contains("review proposed") {
            r#"[{"id": 1, "variants": [{"source": "Eileen", "translated": "Эйлин"}, {"source": "Eileen", "translated": "Айлин"}]}]"#.to_string()
        } else if system.contains("personal names") {
            r#"[{"id": 1, "translated_name": "Эйлин", "variants": [{"source": "Eileen", "translated": "Айлин"}]}]"#.to_string()
        } else if user.contains("Lines:\n") {
            let lines = user.split("Lines:\n").nth(1).unwrap_or_default();
            let word = if lines.contains("\"draft\"") { "Правка" } else { "Перевод" };
            let number = regex::Regex::new(r#""id": "(\d+)""#).unwrap();
            let items: Vec<serde_json::Value> = number
                .captures_iter(lines)
                .map(|caps| serde_json::json!({ "id": &caps[1], "text": format!("{word} {}", &caps[1]) }))
                .collect();
            serde_json::Value::Array(items).to_string()
        } else if system.contains("name a scene") {
            "Налево".to_string()
        } else if system.contains("glossary entry about one character") {
            r#"{"description": "Проводница.", "speech": "Вежливо."}"#.to_string()
        } else {
            "Описание.".to_string()
        };
        completion(&answer)
    }

    fn silent() -> Channel<RunProgress> {
        Channel::new(|_| Ok(()))
    }

    fn count(state: &AppState, project: ProjectId, sql: &str) -> i64 {
        let open = state.open_project(project).unwrap();
        open.project.read(|tx| Ok(tx.query_row(sql, [], |row| row.get(0))?)).unwrap()
    }

    fn empty_steps(state: &AppState, project: ProjectId) -> Vec<GameStep> {
        GameRunService { state }.overview(project).unwrap().into_iter().filter(StepWork::is_empty).map(|work| work.step).collect()
    }

    fn steps(result: &GameRun) -> Vec<GameStep> {
        result.finished.iter().map(|done| done.step).collect()
    }

    #[test]
    fn whole_game_runs_every_step_in_one_slot_and_leaves_nothing() {
        let (url, _) = fake_model(reply);
        let dir = tempfile::tempdir().unwrap();
        let (state, id, eileen) = project(dir.path(), &url);
        let service = GameRunService { state: &state };
        assert_eq!(empty_steps(&state, id), [GameStep::Review], "до перевода вычитывать нечего");

        let result = tauri::async_runtime::block_on(service.run(id, &GameStep::ALL, NamesMode::Auto, silent())).unwrap();
        assert_eq!((result.error.as_deref(), result.stopped, result.names.is_some()), (None, false, false));
        assert_eq!(steps(&result), GameStep::ALL);
        assert!(!state.llm_run.is_active(), "место освобождено");

        let open = state.open_project(id).unwrap();
        let (glossary, terms) = open.project.read(|tx| Ok((GlossaryData::load(tx)?, TermRepo { conn: tx }.list()?))).unwrap();
        let character = glossary.characters.iter().find(|character| character.id() == Some(eileen)).unwrap();
        assert_eq!(character.translated_first_name(), Some("Эйлин"), "в авто берётся первый вариант");
        assert_eq!(character.description(), Some("Проводница."));
        assert_eq!(
            (terms[0].name(), terms[0].translation(), terms[0].category(), terms[0].description()),
            ("Left side", Some("Левая сторона"), TermCategory::Location, Some("Описание."))
        );
        assert_eq!(glossary.entries[0].translation(), Some("Радио"), "подпись принята без отбора");
        assert_eq!(count(&state, id, "SELECT count(*) FROM scene_descriptions WHERE description IS NOT NULL"), 2);
        assert_eq!(
            count(&state, id, "SELECT count(*) FROM translations WHERE text LIKE 'Правка %'"),
            count(&state, id, "SELECT count(*) FROM translations WHERE text LIKE '%Перевод%' OR text LIKE 'Правка %'"),
            "вычитаны все переведённые моделью"
        );
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status <> 'done'"), 0);
        let actions = count(&state, id, "SELECT count(DISTINCT action_id) FROM llm_runs");
        assert_eq!(actions, 7, "у каждого вызова своя строка: analysis, terms, speakers, names, descriptions, lines, review");
        assert_eq!(empty_steps(&state, id), GameStep::ALL, "повторять нечего");
    }

    #[test]
    fn manual_names_stop_the_chain_for_review_and_rest_continues() {
        let (url, _) = fake_model(reply);
        let dir = tempfile::tempdir().unwrap();
        let (state, id, eileen) = project(dir.path(), &url);
        let service = GameRunService { state: &state };

        let result = tauri::async_runtime::block_on(service.run(id, &[GameStep::Lines, GameStep::Glossary], NamesMode::Manual, silent())).unwrap();
        assert_eq!(steps(&result), [GameStep::Glossary]);
        assert_eq!(result.rest, [GameStep::Lines], "перевод реплик ждёт отбора имён");
        let names = result.names.clone().unwrap();
        let item = &names.suggestions.items[0];
        assert_eq!((item.character, item.field), (eileen, NameField::First));
        assert_eq!(item.variants.iter().map(|variant| variant.translation.as_str()).collect::<Vec<_>>(), ["Эйлин", "Айлин"]);
        assert_eq!(count(&state, id, "SELECT count(*) FROM translations WHERE run_id IS NOT NULL"), 0);
        assert!(!state.llm_run.is_active(), "отбор идёт без занятого места");
        let open = state.open_project(id).unwrap();
        let first = || open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap().translated_first_name().map(str::to_string);
        assert_eq!(first(), None, "имя без отбора не записано");

        let pick = NamePick { character: eileen, field: NameField::First, source: "Eileen".to_string(), translation: "Айлин".to_string() };
        assert_eq!(service.write_names(id, std::slice::from_ref(&pick)).unwrap(), 1);
        assert_eq!(first().as_deref(), Some("Айлин"));
        assert_eq!(service.write_names(id, &[NamePick { translation: "Эйлин".to_string(), ..pick }]).unwrap(), 0, "заполненное не перезаписывается");

        let rest = tauri::async_runtime::block_on(service.run(id, &result.rest, NamesMode::Manual, silent())).unwrap();
        assert_eq!(steps(&rest), [GameStep::Lines]);
        assert!(count(&state, id, "SELECT count(*) FROM translations WHERE run_id IS NOT NULL") > 0);
    }

    #[test]
    fn stopped_chain_continues_from_the_same_place() {
        let slot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<crate::llm::RunSlot>>>> = Default::default();
        let stop = slot.clone();
        let (url, seen) = fake_model(move |body| {
            if text(body, 0).contains("proper names")
                && let Some(slot) = stop.lock().unwrap().take()
            {
                slot.cancel();
            }
            reply(body)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id, _) = project(dir.path(), &url);
        *slot.lock().unwrap() = Some(state.llm_run.clone());
        let service = GameRunService { state: &state };
        let analyses = || seen.lock().unwrap().iter().filter(|body| text(body, 0).contains("proper names")).count();
        let chosen = [GameStep::Analysis, GameStep::Lines];

        let result = tauri::async_runtime::block_on(service.run(id, &chosen, NamesMode::Auto, silent())).unwrap();
        assert!(result.stopped && result.finished.is_empty() && result.error.is_none(), "{result:?}");
        assert_eq!(analyses(), 1, "остановка после текущего запроса");
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE action_id = 'lines'"), 0, "следующий шаг не начат");
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status = 'cancelled' AND action_id = 'analysis'"), 1);
        assert!(!state.llm_run.is_active());

        let result = tauri::async_runtime::block_on(service.run(id, &chosen, NamesMode::Auto, silent())).unwrap();
        assert_eq!(steps(&result), chosen);
        assert_eq!(analyses(), 2, "прочитанная часть пропущена, разобрана только вторая сцена");
    }

    #[test]
    fn error_stops_the_chain_with_the_step_name() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id, _) = project(dir.path(), "http://127.0.0.1:9/v1");
        let service = GameRunService { state: &state };
        let result = tauri::async_runtime::block_on(service.run(id, &[GameStep::Glossary, GameStep::Lines], NamesMode::Auto, silent())).unwrap();
        assert!(result.error.as_deref().is_some_and(|error| error.starts_with("Переводы глоссария: ")), "{:?}", result.error);
        assert!(result.finished.is_empty());
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE action_id = 'lines'"), 0);
        assert!(!state.llm_run.is_active());
    }

    #[test]
    fn second_chain_waits_for_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id, _) = project(dir.path(), "http://127.0.0.1:9/v1");
        let held = RunChain::begin(&state, silent()).unwrap();
        let busy = tauri::async_runtime::block_on(GameRunService { state: &state }.run(id, &[GameStep::Lines], NamesMode::Auto, silent()));
        assert!(matches!(busy, Err(AppError::RunActive)));
        drop(held);
        assert!(!state.llm_run.is_active());
    }
}
