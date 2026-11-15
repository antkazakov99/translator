//! Цепочка автоперевода: сводка того, что осталось, и шаги по очереди. Шаги вызывают сервисы действий как есть,
//! внутри одной `RunChain`: место запуска занято на всю цепочку, у каждого вызова своя строка `llm_runs`.
//! Что осталось шагу, считает сервис действия: цепочка своей логики действий не держит.

use std::sync::Arc;

use tauri::ipc::Channel;

use super::{GameRun, GameStep, NamesMode, StepResult, StepWork, WorkCount, ordered};
use crate::common::AppError;
use crate::engines::ContainerKey;
use crate::glossary::describe::entries_work;
use crate::glossary::same_entry::translation_pairs;
use crate::glossary::suggest_service::{glossary_work, names_work};
use crate::glossary::{
    CharacterRepo, DescribeService, DescribeTarget, Described, GlossaryData, GlossaryService, NamePick, NameTaking, NamesFound, NoteRepo,
    SceneDescriptionRepo, SpeakerRepo, SuggestService, Taking, TermRepo, TokenRepo,
};
use crate::llm::{Action, LlmRunRepo, LlmTargetService, RunChain, RunProgress};
use crate::projects::ProjectId;
use crate::state::{AppState, OpenProject};
use crate::translation::{LinesMode, TranslationRepo, TranslationScope, TranslationService};
use crate::glossary::repo::{AnalyzedPartRepo, AskRepo, MentionCheckRepo, PairCheckRepo};

pub struct GameRunService<'a> {
    pub state: &'a AppState,
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
        let analyzed = analysis.items == 0;
        let names_batch = self.batch_size(&open, Action::Names);
        let (names, glossary, entries) =
            open.project.read(|tx| Ok((names_work(tx, names_batch)?, glossary_work(tx, analyzed)?, entries_work(&GlossaryData::load(tx)?))))?;
        let glossary_requests = glossary.requests(|action| self.batch_size(&open, action));
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
                GameStep::Names,
                vec![("имён и фамилий без перевода", names.names), ("форм без перевода", names.forms)],
                names.requests,
            ),
            work(
                GameStep::Glossary,
                vec![
                    ("терминов без перевода", glossary.terms.len()),
                    ("пар терминов с одним переводом", glossary.pairs),
                    ("подписей без перевода", glossary.speakers.len()),
                    ("токенов без значения", glossary.tokens.len()),
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
    /// готовое остаётся. Отбор имён останавливает цепочку: окно продолжает её с `rest`, и первым в них стоит
    /// сам шаг имён. Заход шага после отбора спрашивает только то, о чём ещё не спрашивал: формы.
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
                result.rest = steps[index..].to_vec();
                break;
            }
        }
        Ok(result)
    }

    /// Сброс всего прогресса одной транзакцией: проект как сразу после считывания игры. Уходят переводы реплик
    /// с историей, отметки вычитки и разбора, заметки, описания сцен, весь глоссарий и законченные запуски.
    /// Снимок игры, настройки моделей проекта, свои правила имён и замены шрифтов остаются.
    pub fn reset(&self, project: ProjectId) -> Result<(), AppError> {
        if self.state.llm_run.is_active() {
            return Err(AppError::RunActive);
        }
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| {
            TranslationRepo { conn: tx }.clear()?;
            AnalyzedPartRepo { conn: tx }.clear()?;
            PairCheckRepo { conn: tx }.clear()?;
            MentionCheckRepo { conn: tx }.clear()?;
            AskRepo { conn: tx }.clear()?;
            NoteRepo { conn: tx }.clear()?;
            SceneDescriptionRepo { conn: tx }.clear()?;
            SpeakerRepo { conn: tx }.clear()?;
            CharacterRepo { conn: tx }.clear()?;
            TermRepo { conn: tx }.clear()?;
            TokenRepo { conn: tx }.clear()?;
            LlmRunRepo { conn: tx }.clear_finished()?;
            Ok(())
        })?)
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
            GameStep::Names => self.names(project, names, chain).await,
            GameStep::Glossary => self.glossary(project, chain, index, count).await,
            GameStep::Entries => {
                let open = self.state.open_project(project)?;
                let targets = open.project.read(|tx| Ok(entries_work(&GlossaryData::load(tx)?)))?;
                Ok(StepOutcome::from_described(describe.describe_unasked(project, targets, None, chain).await?))
            }
            GameStep::Scenes => {
                let keys = self.scene_keys(project, |scene| !scene.described || scene.stale)?;
                let targets = keys.into_iter().map(DescribeTarget::Scene).collect();
                Ok(StepOutcome::from_described(describe.describe_unasked(project, targets, None, chain).await?))
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

    /// Имена, о которых цепочка ещё не спрашивала. «Авто» пишет написания, на которых перевод и проверка
    /// сошлись, спорные идут в отбор; ручной режим отдаёт в отбор все. Отбор останавливает цепочку.
    async fn names(&self, project: ProjectId, mode: NamesMode, chain: &RunChain) -> Result<StepOutcome, AppError> {
        let open = self.state.open_project(project)?;
        let work = open.project.read(|tx| names_work(tx, 1))?;
        if work.characters.is_empty() {
            return Ok(StepOutcome::default());
        }
        let taking = match mode {
            NamesMode::Auto => NameTaking::Agreed,
            NamesMode::Manual => NameTaking::Picks,
        };
        let found = SuggestService { state: self.state }.translate_names(project, work.characters, None, chain, taking).await?;
        let mut outcome = StepOutcome { written: found.suggestions.written, error: found.suggestions.error.clone(), names: None };
        if !found.suggestions.items.is_empty() {
            outcome.names = Some(found);
        }
        Ok(outcome)
    }

    /// Термины, подписи и токены принимаются без отбора: годное пишется после каждого пакета. После терминов
    /// модель отвечает, одно ли это термины с одинаковым переводом. Каждая часть своим запуском.
    async fn glossary(&self, project: ProjectId, chain: &RunChain, index: usize, count: usize) -> Result<StepOutcome, AppError> {
        let open = self.state.open_project(project)?;
        let scenes = TranslationService { state: self.state }.story_scenes(project)?;
        let all = scenes.iter().filter_map(|scene| scene.key.clone()).collect();
        let analyzed = DescribeService { state: self.state }.pending_analysis(project, all)?.items == 0;
        let work = open.project.read(|tx| glossary_work(tx, analyzed))?;
        let suggest = SuggestService { state: self.state };
        let mut outcome = StepOutcome::default();
        let stop = |error: &Option<String>| error.is_some() || chain.cancelled();

        if !work.terms.is_empty() {
            chain.stage(index, count, "Перевод терминов");
            let found = suggest.translate_terms(project, work.terms, None, chain, Taking::Write).await?;
            outcome.written += found.written;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        // Пары считаются заново: одинаковый перевод мог дать перевод терминов этого же захода.
        if open.project.read(|tx| Ok(!translation_pairs(tx, &GlossaryData::load(tx)?)?.is_empty()))? {
            chain.stage(index, count, "Термины с одним переводом");
            let found = suggest.settle_same_translations(project, None, chain).await?;
            outcome.written += found.written;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        if !work.speakers.is_empty() {
            chain.stage(index, count, "Перевод подписей");
            let found = suggest.suggest_speakers(project, work.speakers, None, chain, Taking::Write).await?;
            outcome.written += found.written;
            outcome.error = found.error;
            if stop(&outcome.error) {
                return Ok(outcome);
            }
        }
        if !work.tokens.is_empty() {
            chain.stage(index, count, "Значения токенов");
            let found = suggest.suggest_tokens(project, work.tokens, None, chain, Taking::Write).await?;
            outcome.written += found.written;
            outcome.error = found.error;
        }
        Ok(outcome)
    }

    /// Ключи сцен по сюжету, которые подходят шагу.
    fn scene_keys(&self, project: ProjectId, wanted: impl Fn(&crate::translation::SceneProgress) -> bool) -> Result<Vec<ContainerKey>, AppError> {
        let scenes = TranslationService { state: self.state }.story_scenes(project)?;
        Ok(scenes.iter().filter(|scene| wanted(scene)).filter_map(|scene| scene.key.clone()).collect())
    }

    /// Размер пакета действия. Без настроенной модели значение из кода.
    fn batch_size(&self, open: &Arc<OpenProject>, action: Action) -> usize {
        LlmTargetService { state: self.state }
            .target(Some(open), action, None)
            .map_or_else(|_| action.default_batch_size(), |target| target.settings.batch_size)
            .get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::SpeakerKey;
    use crate::glossary::{CharacterId, NameField, TermCategory, TermRepo};
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
        let (url, seen) = fake_model(reply);
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
        assert_eq!(actions, 7, "у каждого вызова своя строка: analysis, names, terms, speakers, descriptions, lines, review");
        {
            let requests = seen.lock().unwrap();
            let at = |part: &str| requests.iter().position(|body| text(body, 0).contains(part)).unwrap();
            assert!(at("personal names") < at("terms of a visual novel"), "имена переводятся раньше терминов и подписей");
        }
        assert_eq!(empty_steps(&state, id), GameStep::ALL, "повторять нечего");

        // Окно запускает отмеченные шаги без сводки: шаг без работы проходит без запросов к модели.
        let asked = seen.lock().unwrap().len();
        let again = tauri::async_runtime::block_on(service.run(id, &GameStep::ALL, NamesMode::Auto, silent())).unwrap();
        assert_eq!((again.error.as_deref(), again.stopped), (None, false));
        assert_eq!(steps(&again), GameStep::ALL);
        assert_eq!(seen.lock().unwrap().len(), asked, "делать нечего: к модели запросов нет");
    }

    #[test]
    fn manual_names_stop_the_chain_for_review_and_rest_continues() {
        let (url, _) = fake_model(reply);
        let dir = tempfile::tempdir().unwrap();
        let (state, id, eileen) = project(dir.path(), &url);
        let service = GameRunService { state: &state };

        let result = tauri::async_runtime::block_on(service.run(id, &[GameStep::Lines, GameStep::Names], NamesMode::Manual, silent())).unwrap();
        assert_eq!(steps(&result), [GameStep::Names]);
        assert_eq!(result.rest, [GameStep::Names, GameStep::Lines], "перевод реплик ждёт отбора, шаг имён после него спросит формы");
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
        assert_eq!(steps(&rest), [GameStep::Names, GameStep::Lines], "второй заход имён прошёл без отбора");
        assert!(rest.names.is_none());
        assert!(count(&state, id, "SELECT count(*) FROM translations WHERE run_id IS NOT NULL") > 0);
    }

    #[test]
    fn auto_names_stop_only_on_disputed_spellings_and_ask_them_once() {
        // Проверка ставит первым другое написание: перевод и проверка разошлись.
        let (url, seen) = fake_model(|body| {
            if text(body, 0).contains("review proposed") {
                completion(r#"[{"id": 1, "variants": [{"source": "Eileen", "translated": "Айлин"}, {"source": "Eileen", "translated": "Эйлин"}]}]"#)
            } else {
                reply(body)
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id, eileen) = project(dir.path(), &url);
        let service = GameRunService { state: &state };
        let spelled = || seen.lock().unwrap().iter().filter(|body| text(body, 0).contains("expert in personal names")).count();
        let open = state.open_project(id).unwrap();
        let first = || open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap().translated_first_name().map(str::to_string);

        let result = tauri::async_runtime::block_on(service.run(id, &[GameStep::Names, GameStep::Lines], NamesMode::Auto, silent())).unwrap();
        assert_eq!((steps(&result), result.rest.clone()), (vec![GameStep::Names], vec![GameStep::Names, GameStep::Lines]));
        let names = result.names.unwrap();
        assert_eq!(names.suggestions.items[0].variants[0].translation, "Айлин");
        assert_eq!(first(), None, "спорное написание без отбора не пишется");
        assert_eq!(spelled(), 1);

        // Отбор отброшен: цепочка идёт дальше и имя второй раз не спрашивает, шаг становится пустым.
        let rest = tauri::async_runtime::block_on(service.run(id, &result.rest, NamesMode::Auto, silent())).unwrap();
        assert_eq!(steps(&rest), [GameStep::Names, GameStep::Lines]);
        assert!(rest.names.is_none());
        assert_eq!(spelled(), 1);
        assert!(empty_steps(&state, id).contains(&GameStep::Names));
        assert_eq!(count(&state, id, "SELECT count(*) FROM glossary_asks WHERE kind = 'name'"), 1);
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
    fn chain_asks_about_an_entry_without_a_description_once() {
        // Сборка описания персонажа приходит без JSON: описания из неё не взять.
        let (url, seen) = fake_model(|body| {
            if text(body, 0).contains("glossary entry about one character") { completion("Не знаю.") } else { reply(body) }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id, eileen) = project(dir.path(), &url);
        let service = GameRunService { state: &state };
        let assemblies = || seen.lock().unwrap().iter().filter(|body| text(body, 0).contains("glossary entry about one character")).count();
        assert!(!empty_steps(&state, id).contains(&GameStep::Entries));

        let result = tauri::async_runtime::block_on(service.run(id, &[GameStep::Entries], NamesMode::Auto, silent())).unwrap();
        assert_eq!((steps(&result), result.error.as_deref()), (vec![GameStep::Entries], None));
        assert_eq!(result.finished[0].written, 0);
        assert_eq!(assemblies(), 1);
        assert_eq!(count(&state, id, "SELECT count(*) FROM glossary_asks WHERE kind = 'description'"), 1);
        assert!(empty_steps(&state, id).contains(&GameStep::Entries), "запись без годного ответа сводка не считает");

        // Ни шаг описаний, ни описание записей сцены перед её описанием о ней больше не спрашивают.
        let again = tauri::async_runtime::block_on(service.run(id, &[GameStep::Entries, GameStep::Scenes], NamesMode::Auto, silent())).unwrap();
        assert_eq!((steps(&again), again.error.as_deref()), (vec![GameStep::Entries, GameStep::Scenes], None));
        assert_eq!(assemblies(), 1);

        // Кнопка записи в окне глоссария спрашивает, не глядя на отметку.
        let owner = crate::glossary::ExcludeOwner::Character(eileen);
        let described = tauri::async_runtime::block_on(DescribeService { state: &state }.describe_entry(id, owner, None, silent())).unwrap();
        assert!(described.written.is_empty() && described.error.is_none(), "{described:?}");
        assert_eq!(assemblies(), 2);
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
    fn reset_returns_the_project_to_a_fresh_scan() {
        let (url, _) = fake_model(reply);
        let dir = tempfile::tempdir().unwrap();
        let (state, id, _) = project(dir.path(), &url);
        let service = GameRunService { state: &state };
        let segments = count(&state, id, "SELECT count(*) FROM segments");
        tauri::async_runtime::block_on(service.run(id, &GameStep::ALL, NamesMode::Auto, silent())).unwrap();
        assert!(count(&state, id, "SELECT count(*) FROM translations") > 0);

        let held = RunChain::begin(&state, silent()).unwrap();
        assert!(matches!(service.reset(id), Err(AppError::RunActive)), "во время запуска сброса нет");
        drop(held);

        service.reset(id).unwrap();
        for table in [
            "translations", "revisions", "reviewed_lines", "analyzed_parts", "notes", "scene_descriptions", "characters", "character_forms",
            "speaker_characters", "speaker_entries", "discarded_speakers", "terms", "term_forms", "discarded_terms", "tokens", "llm_runs",
            "character_mention_checks", "term_mention_checks", "glossary_asks",
        ] {
            assert_eq!(count(&state, id, &format!("SELECT count(*) FROM {table}")), 0, "{table}");
        }
        assert_eq!(count(&state, id, "SELECT count(*) FROM segments"), segments, "снимок игры остался");
        assert_eq!(
            empty_steps(&state, id),
            [GameStep::Names, GameStep::Glossary, GameStep::Entries, GameStep::Review],
            "глоссарий пуст: разбор, описания сцен и перевод снова доступны"
        );
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
