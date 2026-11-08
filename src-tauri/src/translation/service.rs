//! Порядок шагов перевода и редактуры. Ответ модели ждётся вне транзакции, каждый пакет пишется короткой.

use std::collections::HashMap;
use std::sync::Arc;

use tauri::ipc::Channel;

use super::batcher::LINE_CONTEXT;
use super::hints::GlossaryContext;
use super::text::is_wordless;
use super::{
    Batch, Batcher, Draft, FileProgress, Hints, LineState, LinesAnswer, LinesMode, LinesPrompt, Scene, SceneLine, SentState,
    TranslationRepo, TranslationScope,
};
use crate::common::{AppError, Timestamp};
use crate::glossary::{GlossaryData, Term, TermRepo};
use crate::llm::{Action, LlmClient, LlmError, LlmRun, LlmTarget, LlmTargetService, ProfileId, RunId, RunProgress, RunStatus};
use crate::projects::ProjectId;
use crate::scan::{FileId, SegmentId};
use crate::state::{AppState, OpenProject};

/// Сколько раз спрашивается пакет: при сбое сети и пока в нём остаются реплики без перевода.
const ATTEMPTS: usize = 3;

pub struct TranslationService<'a> {
    pub state: &'a AppState,
}

impl TranslationService<'_> {
    /// Заполняет пустые реплики. Возвращает число переведённых.
    /// Описание сцены пока берётся готовым: чтение сцены перед переводом придёт с описаниями сцен.
    pub async fn translate(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        self.run(project, LinesMode::Translate, scope, one_off, progress).await
    }

    /// Вычитывает готовые реплики, исправления становятся черновиками. Возвращает число исправленных.
    pub async fn review(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        self.run(project, LinesMode::Review, scope, one_off, progress).await
    }

    /// Запуск: цель, сцены, место запуска, пакеты, итог в `llm_runs`. Остановка не ошибка: готовое остаётся.
    async fn run(
        &self,
        project: ProjectId,
        mode: LinesMode,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        let action = match mode {
            LinesMode::Translate => Action::Lines,
            LinesMode::Review => Action::Review,
        };
        let target = LlmTargetService { state: self.state }.target(Some(&open), action, one_off)?;
        let (mut scenes, glossary) =
            open.project.read(|tx| Ok((load_scenes(&TranslationRepo { conn: tx }, &scope)?, GlossaryData::load(tx)?)))?;
        let only = match scope {
            TranslationScope::Line(line) => Some(line),
            _ => None,
        };
        // Что ушло бы модели без глоссария: по этому проверяется запись, ведь в базе токены не заменены.
        let sent: HashMap<SegmentId, SentState> =
            scenes.iter().flat_map(|scene| &scene.lines).map(|line| (line.id, line.sent())).collect();
        let mut glossary = GlossaryContext::new(&glossary);
        for scene in &mut scenes {
            glossary.apply(scene);
        }
        let run = LlmRun::begin(self.state, Some(open.clone()), action, &target, progress)?;
        let result = match run.id() {
            Some(id) => {
                let mut job = Job { open: &open, target: &target, run: &run, id, mode, hints: glossary.hints, sent, progress: RunProgress::default() };
                job.lines(scenes, only).await
            }
            None => Err(AppError::ProjectNotOpen),
        };
        let status = match &result {
            Err(_) => RunStatus::Failed,
            Ok(_) if run.cancelled() => RunStatus::Cancelled,
            Ok(_) => RunStatus::Done,
        };
        let error = result.as_ref().err().map(ToString::to_string);
        run.finish(status, error.as_deref())?;
        result
    }

    /// Сброс перевода: пустые правки, как ручная очистка строки.
    pub fn reset(&self, project: ProjectId, scope: TranslationScope) -> Result<usize, AppError> {
        todo!()
    }

    /// Файлы с репликами и прогресс по ним.
    pub fn files(&self, project: ProjectId) -> Result<Vec<FileProgress>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| TranslationRepo { conn: tx }.files())?)
    }

    /// Реплики файла по сценам, в порядке файла.
    pub fn file_scenes(&self, project: ProjectId, file: FileId) -> Result<Vec<Scene>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| TranslationRepo { conn: tx }.file_scenes(file))?)
    }

    /// Правка пользователя одной транзакцией с историей. Возвращает реплику после правки.
    pub fn save_text(&self, project: ProjectId, line: SegmentId, text: &str) -> Result<SceneLine, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| {
            let repo = TranslationRepo { conn: tx };
            repo.save_user_text(line, text, Timestamp::now())?;
            repo.line(line)
        })?)
    }

    /// Подтверждает черновики модели. Возвращает число подтверждённых.
    pub fn confirm(&self, project: ProjectId, lines: &[SegmentId]) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| TranslationRepo { conn: tx }.confirm(lines, Timestamp::now()))?)
    }

    /// Откат запуска модели. Возвращает число откаченных и оставленных реплик.
    pub fn rollback(&self, project: ProjectId, run: RunId) -> Result<(usize, usize), AppError> {
        todo!()
    }
}

/// Сцены запуска. Проект берёт файлы по порядку.
fn load_scenes(repo: &TranslationRepo, scope: &TranslationScope) -> Result<Vec<Scene>, crate::storage::DbError> {
    match scope {
        TranslationScope::Scenes(keys) => keys.iter().map(|key| repo.scene(key)).collect(),
        TranslationScope::File(file) => repo.file_scenes(*file),
        TranslationScope::Line(line) => Ok(vec![repo.scene_of(*line)?]),
        TranslationScope::Project => {
            let mut scenes = Vec::new();
            for file in repo.files()? {
                scenes.extend(repo.file_scenes(file.id)?);
            }
            Ok(scenes)
        }
    }
}

/// Идущий запуск перевода или редактуры. Сцены держит у себя: записанный черновик сразу виден следующим пакетам.
struct Job<'r> {
    open: &'r Arc<OpenProject>,
    target: &'r LlmTarget,
    run: &'r LlmRun,
    id: RunId,
    mode: LinesMode,
    hints: Hints,
    /// Текст реплики в базе, без значений токенов: с ним сверяется запись черновика.
    sent: HashMap<SegmentId, SentState>,
    progress: RunProgress,
}

impl Job<'_> {
    /// Пакеты всех сцен по порядку. `only` ограничивает запуск одной репликой.
    async fn lines(&mut self, mut scenes: Vec<Scene>, only: Option<SegmentId>) -> Result<usize, AppError> {
        if self.mode == LinesMode::Translate {
            self.fill_wordless(&mut scenes, only)?;
        }
        let plans: Vec<Vec<Batch>> = scenes
            .iter()
            .map(|scene| match only {
                Some(line) => single(scene, self.mode, line).into_iter().collect(),
                None => Batcher::split(scene, self.mode, self.target.settings.batch_size),
            })
            .collect();
        self.progress.total = plans.iter().map(Vec::len).sum::<usize>() as u32;
        self.report(None);

        for (scene, batches) in scenes.iter_mut().zip(plans) {
            let name = scene.title.clone().or_else(|| scene.key.as_ref().map(|key| key.to_string()));
            for batch in batches {
                if self.run.cancelled() {
                    return Ok(self.progress.written as usize);
                }
                self.report(name.clone());
                match self.mode {
                    LinesMode::Translate => self.translate(scene, &batch).await?,
                    LinesMode::Review => self.ask(scene, &batch).await?,
                }
                self.progress.done += 1;
                self.report(name.clone());
            }
        }
        Ok(self.progress.written as usize)
    }

    /// Слабая модель бросает конец длинного пакета, а ответ может не разобраться. Повтор спрашивает
    /// каждую группу пустых реплик подряд отдельным запросом.
    async fn translate(&mut self, scene: &mut Scene, batch: &Batch) -> Result<(), AppError> {
        for attempt in 0..ATTEMPTS {
            let spans = if attempt == 0 { vec![batch.clone()] } else { Batcher::blank_runs(scene, batch) };
            if spans.is_empty() {
                break;
            }
            for span in spans {
                if self.run.cancelled() {
                    return Ok(());
                }
                self.ask(scene, &span).await?;
            }
        }
        Ok(())
    }

    /// Один запрос: промпт, ответ, проверка, запись черновиков короткой транзакцией.
    async fn ask(&mut self, scene: &mut Scene, batch: &Batch) -> Result<(), AppError> {
        let request = LinesPrompt::build(self.mode, scene, batch, &self.hints, self.target);
        let client = LlmClient::for_kind(self.target.provider.kind());
        let mut attempt = 0;
        let text = loop {
            attempt += 1;
            match client.send(self.target, &request).await {
                Ok(text) => break text,
                Err(LlmError::Refused) => {
                    tracing::warn!("Модель отказалась переводить пакет сцены, реплики остаются пустыми");
                    return Ok(());
                }
                Err(error) if error.is_transient() && attempt < ATTEMPTS && !self.run.cancelled() => {
                    tracing::warn!("Запрос к модели не удался, повтор: {error}");
                }
                Err(error) => return Err(error.into()),
            }
        };
        let Some(answer) = LinesAnswer::parse(&text) else {
            tracing::warn!("Ответ модели не разобран:\n{text}");
            return Ok(());
        };
        let drafts = answer.accept(self.mode, scene, batch);
        self.save(scene, &drafts)?;
        if self.mode == LinesMode::Translate {
            self.save_terms(answer.new_terms(scene, batch))?;
        }
        Ok(())
    }

    /// Новые термины ложатся в глоссарий предположениями и сразу идут в следующие запросы с пометкой:
    /// иначе один термин расходился бы по пакетам. Название, уже известное глоссарию, не пишется.
    fn save_terms(&mut self, terms: Vec<Term>) -> Result<(), AppError> {
        if terms.is_empty() {
            return Ok(());
        }
        let saved = self.open.project.write(|tx| {
            let repo = TermRepo { conn: tx };
            let mut saved = Vec::new();
            for term in terms {
                if !repo.is_known(term.name_key())? {
                    repo.save(&term)?;
                    saved.push(term);
                }
            }
            Ok(saved)
        })?;
        for term in saved {
            if let Some(translation) = term.translation() {
                self.hints.terms.push((term.name().to_string(), translation.to_string(), true));
            }
        }
        Ok(())
    }

    /// Реплика без букв вроде «...» переводится сама собой: модель отвечала на неё пустым списком.
    fn fill_wordless(&mut self, scenes: &mut [Scene], only: Option<SegmentId>) -> Result<(), AppError> {
        for scene in scenes {
            let drafts: Vec<Draft> = scene
                .lines
                .iter()
                .filter(|line| only.is_none_or(|id| id == line.id))
                .filter(|line| line.is_blank() && !line.source_text.trim().is_empty() && is_wordless(&line.source_text))
                .map(|line| Draft { line: line.id, text: line.source_text.clone(), sent: line.sent() })
                .collect();
            self.save(scene, &drafts)?;
        }
        Ok(())
    }

    /// Пишет черновики и показывает их следующим пакетам сцены. Отправленное сверяется с текстом из базы,
    /// а не с текстом, где глоссарий подставил значения токенов.
    fn save(&mut self, scene: &mut Scene, drafts: &[Draft]) -> Result<(), AppError> {
        if drafts.is_empty() {
            return Ok(());
        }
        let drafts: Vec<Draft> = drafts
            .iter()
            .map(|draft| Draft { sent: self.sent.get(&draft.line).cloned().unwrap_or_else(|| draft.sent.clone()), ..draft.clone() })
            .collect();
        let drafts = drafts.as_slice();
        for draft in drafts {
            self.sent.insert(draft.line, SentState { text: draft.text.clone(), source_hash: draft.sent.source_hash.clone() });
        }
        let now = Timestamp::now();
        let written = self.open.project.write(|tx| {
            let repo = TranslationRepo { conn: tx };
            match self.mode {
                LinesMode::Translate => repo.save_drafts(drafts, self.id, now),
                LinesMode::Review => repo.replace_with_drafts(drafts, self.id, now),
            }
        })?;
        self.progress.written += written as u32;
        for draft in drafts {
            if let Some(line) = scene.lines.iter_mut().find(|line| line.id == draft.line) {
                line.text.clone_from(&draft.text);
                line.state = LineState::Draft;
            }
        }
        Ok(())
    }

    fn report(&mut self, message: Option<String>) {
        self.progress.message = message;
        self.run.report(self.progress.clone());
    }
}

/// Пакет из одной реплики с репликами перед ней. Реплика, которая не нужна режиму, пакета не даёт.
fn single(scene: &Scene, mode: LinesMode, line: SegmentId) -> Option<Batch> {
    let index = scene.lines.iter().position(|item| item.id == line)?;
    if !mode.wants(&scene.lines[index]) {
        return None;
    }
    let context = scene.lines[index.saturating_sub(LINE_CONTEXT)..index].iter().map(|item| item.id).collect();
    Some(Batch { lines: vec![line], context })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;
    use crate::translation::LineState;

    /// Открытый проект по синтетической игре.
    fn project(dir: &std::path::Path) -> (AppState, ProjectId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        (state, id)
    }

    fn revisions(state: &AppState, project: ProjectId) -> i64 {
        let open = state.open_project(project).unwrap();
        open.project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))?)).unwrap()
    }

    #[test]
    fn file_lines_come_by_scenes_in_file_order() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };

        let files = service.files(id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "game/script.rpy");
        assert_eq!(files[0].translated, 0);

        let scenes = service.file_scenes(id, files[0].id).unwrap();
        let keys: Vec<_> = scenes.iter().map(|scene| scene.key.as_ref().map(|key| key.as_str())).collect();
        assert_eq!(keys, [Some("label:start"), Some("label:left")]);
        let first = &scenes[0].lines[0];
        assert_eq!(first.source_text, "Hello.");
        assert_eq!(first.speaker.as_deref(), Some("Eileen"));
        assert_eq!(first.state, LineState::Empty);
        assert!(first.location.is_some());
    }

    #[test]
    fn user_edit_writes_history_once_and_clearing_keeps_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let line = service.file_scenes(id, file).unwrap()[0].lines[0].id;

        let saved = service.save_text(id, line, "Привет.").unwrap();
        assert_eq!((saved.text.as_str(), saved.state), ("Привет.", LineState::Done));
        service.save_text(id, line, "Привет.").unwrap();
        assert_eq!(revisions(&state, id), 1, "тот же текст историю не пишет");
        assert_eq!(service.files(id).unwrap()[0].translated, 1);

        let cleared = service.save_text(id, line, "   ").unwrap();
        assert_eq!((cleared.text.as_str(), cleared.state), ("", LineState::Empty));
        assert_eq!(revisions(&state, id), 2);
        assert_eq!(service.files(id).unwrap()[0].translated, 0);
    }

    #[test]
    fn confirming_a_draft_makes_it_done_and_changed_source_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let line = service.file_scenes(id, file).unwrap()[0].lines[0].id;
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                tx.execute(
                    "INSERT INTO translations (segment_id, text, draft, source_hash, updated_at)
                     VALUES (?1, 'Здравствуй.', 1, 'старый', 0)",
                    [line.get()],
                )?;
                Ok(())
            })
            .unwrap();

        let draft = &service.file_scenes(id, file).unwrap()[0].lines[0];
        assert_eq!(draft.state, LineState::Draft);
        assert!(draft.source_changed);
        assert_eq!(service.files(id).unwrap()[0].drafts, 1);

        assert_eq!(service.confirm(id, &[line, line]).unwrap(), 1);
        let done = &service.file_scenes(id, file).unwrap()[0].lines[0];
        assert_eq!((done.text.as_str(), done.state), ("Здравствуй.", LineState::Done));
        assert_eq!(revisions(&state, id), 1);
    }

    use crate::llm::client::tests::{completion, fake_model};
    use crate::llm::{LlmSettingsService, Profile, Provider, ProviderKind};
    use crate::translation::{Draft, SentState};

    /// Модель в настройках приложения: локальный сервер по адресу `url`.
    fn add_model(state: &AppState, url: &str) {
        let settings = LlmSettingsService { state };
        let provider = Provider::new("LM Studio", ProviderKind::Local, url, 0).unwrap();
        settings.save_provider(&provider, None).unwrap();
        settings.save_profile(&Profile::new(provider.id(), "", "gemma", 0).unwrap()).unwrap();
    }

    /// Модель отвечает на каждую реплику с номером: переводом или, в редактуре, правкой.
    fn answer_every_line(body: &serde_json::Value) -> String {
        let user = body["messages"][1]["content"].as_str().unwrap_or_default();
        let lines = user.split("Lines:\n").nth(1).unwrap_or_default();
        let word = if lines.contains("\"draft\"") { "Правка" } else { "Перевод" };
        let number = regex::Regex::new(r#""id": "(\d+)""#).unwrap();
        let items: Vec<serde_json::Value> = number
            .captures_iter(lines)
            .map(|caps| serde_json::json!({ "id": &caps[1], "text": format!("{word} {}", &caps[1]) }))
            .collect();
        completion(&serde_json::Value::Array(items).to_string())
    }

    fn silent() -> Channel<RunProgress> {
        Channel::new(|_| Ok(()))
    }

    fn count(state: &AppState, project: ProjectId, sql: &str) -> i64 {
        let open = state.open_project(project).unwrap();
        open.project.read(|tx| Ok(tx.query_row(sql, [], |row| row.get(0))?)).unwrap()
    }

    fn lines(service: &TranslationService, project: ProjectId, file: FileId) -> Vec<SceneLine> {
        service.file_scenes(project, file).unwrap().into_iter().flat_map(|scene| scene.lines).collect()
    }

    #[test]
    fn translation_writes_drafts_by_batches_and_records_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let (url, seen) = fake_model(answer_every_line);
        add_model(&state, &url);
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].clone();

        let translated = tauri::async_runtime::block_on(service.translate(id, TranslationScope::File(file.id), None, silent())).unwrap();
        assert_eq!(translated, file.total);
        let all = lines(&service, id, file.id);
        assert!(all.iter().all(|line| line.state == LineState::Draft && line.text.starts_with("Перевод")), "{all:?}");
        assert_eq!(count(&state, id, "SELECT count(*) FROM revisions WHERE kind = 'llm' AND run_id IS NOT NULL"), file.total as i64);
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status = 'done'"), 1);
        assert!(!state.llm_run.is_active(), "место освобождено");

        let requests = seen.lock().unwrap().len();
        assert_eq!(tauri::async_runtime::block_on(service.translate(id, TranslationScope::Project, None, silent())).unwrap(), 0);
        assert_eq!(seen.lock().unwrap().len(), requests, "переводить нечего, запросов нет");
    }

    #[test]
    fn glossary_reaches_the_prompt_and_new_terms_become_guesses() {
        use crate::common::Gender;
        use crate::engines::SpeakerKey;
        use crate::glossary::{CharacterRepo, GlossaryService};

        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let glossary = GlossaryService { state: &state };
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let open = state.open_project(id).unwrap();
        let mut person = open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        person.set_translated_names("Эйлин", "");
        person.set_gender(Some(Gender::Female));
        glossary.save_character(id, &person).unwrap();

        let (url, seen) = fake_model(|body| {
            let reply = serde_json::from_str::<serde_json::Value>(&answer_every_line(body)).unwrap();
            let text = reply["choices"][0]["message"]["content"].as_str().unwrap();
            let mut items: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
            items.push(serde_json::json!({ "term": "Left Side", "translation": "Левая сторона", "category": "location" }));
            items.push(serde_json::json!({ "term": "Narnia", "translation": "Нарния" }));
            completion(&serde_json::Value::Array(items).to_string())
        });
        add_model(&state, &url);
        let service = TranslationService { state: &state };
        tauri::async_runtime::block_on(service.translate(id, TranslationScope::Project, None, silent())).unwrap();

        let requests = seen.lock().unwrap();
        let first = requests[0]["messages"][1]["content"].as_str().unwrap();
        assert!(first.contains("Speakers:\nEileen: female"), "{first}");
        assert!(first.contains("Russian names: Eileen = Эйлин"), "{first}");

        let terms = glossary.overview(id).unwrap().data.terms;
        assert_eq!(terms.len(), 1, "выдуманный термин без упоминания не сохраняется");
        assert_eq!((terms[0].name(), terms[0].translation(), terms[0].is_guess()), ("Left Side", Some("Левая сторона"), true));
    }

    #[test]
    fn review_replaces_translations_with_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let (url, _) = fake_model(answer_every_line);
        add_model(&state, &url);
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let first = lines(&service, id, file)[0].id;
        service.save_text(id, first, "Здравствуй.").unwrap();

        let fixed = tauri::async_runtime::block_on(service.review(id, TranslationScope::Line(first), None, silent())).unwrap();
        assert_eq!(fixed, 1);
        let line = &lines(&service, id, file)[0];
        assert_eq!((line.text.as_str(), line.state), ("Правка 1", LineState::Draft));
    }

    #[test]
    fn second_run_waits_and_late_answer_does_not_overwrite_user_edit() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        add_model(&state, "http://127.0.0.1:9/v1");
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let open = state.open_project(id).unwrap();
        let target = LlmTargetService { state: &state }.target(Some(&open), Action::Lines, None).unwrap();

        let held = LlmRun::begin(&state, Some(open.clone()), Action::Lines, &target, silent()).unwrap();
        let busy = tauri::async_runtime::block_on(service.translate(id, TranslationScope::File(file), None, silent()));
        assert!(matches!(busy, Err(AppError::RunActive)));

        let line = lines(&service, id, file)[0].clone();
        service.save_text(id, line.id, "Своё.").unwrap();
        let late = Draft { line: line.id, text: "Модель.".to_string(), sent: SentState { text: String::new(), source_hash: line.source_hash } };
        let run = held.id().unwrap();
        assert_eq!(open.project.write(|tx| TranslationRepo { conn: tx }.save_drafts(&[late], run, Timestamp::now())).unwrap(), 0);
        assert_eq!(lines(&service, id, file)[0].text, "Своё.");

        drop(held);
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status = 'failed'"), 1, "запуск без finish стал failed");
        assert!(!state.llm_run.is_active());
    }

    #[test]
    fn unreachable_server_fails_the_run_and_frees_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        add_model(&state, "http://127.0.0.1:9/v1");
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let result = tauri::async_runtime::block_on(service.translate(id, TranslationScope::File(file), None, silent()));
        assert!(matches!(result, Err(AppError::Llm(_))));
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status = 'failed' AND error IS NOT NULL"), 1);
        assert!(!state.llm_run.is_active());
    }

    #[test]
    fn closed_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        ProjectService { state: &state }.close(id);
        assert!(matches!(TranslationService { state: &state }.files(id), Err(AppError::ProjectNotOpen)));
    }
}
