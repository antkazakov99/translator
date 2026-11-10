//! Порядок шагов перевода и редактуры. Ответ модели ждётся вне транзакции, каждый пакет пишется короткой.

use std::collections::HashMap;
use std::sync::Arc;

use tauri::ipc::Channel;

use super::batcher::LINE_CONTEXT;
use super::hints::GlossaryContext;
use super::text::is_wordless;
use super::{
    Batch, Batcher, Draft, FileProgress, FileScenes, Hints, LineRun, LineState, LinesAnswer, LinesMode, LinesPrompt, Scene, SceneHit, SceneLine, SceneProgress, SceneRef, SentState,
    TranslationRepo, TranslationScope, scene_hits,
};
use crate::common::{AppError, Timestamp};
use crate::glossary::describe::{RunAsk, SceneReader};
use crate::glossary::{CharacterRepo, GlossaryData, SceneDescriptionRepo};
use crate::name_rules::NameRuleService;
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
    /// Заполняет пустые реплики. Возвращает запуск и число переведённых.
    /// Описание сцены пока берётся готовым: чтение сцены перед переводом придёт с описаниями сцен.
    pub async fn translate(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<LinesRun, AppError> {
        self.run(project, LinesMode::Translate, scope, one_off, progress).await
    }

    /// Вычитывает готовые реплики, исправления становятся черновиками. Возвращает запуск и число исправленных.
    pub async fn review(
        &self,
        project: ProjectId,
        scope: TranslationScope,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<LinesRun, AppError> {
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
    ) -> Result<LinesRun, AppError> {
        let open = self.state.open_project(project)?;
        let action = match mode {
            LinesMode::Translate => Action::Lines,
            LinesMode::Review => Action::Review,
        };
        let target = LlmTargetService { state: self.state }.target(Some(&open), action, one_off)?;
        let (mut scenes, glossary, origins) = open.project.read(|tx| {
            Ok((load_scenes(&TranslationRepo { conn: tx }, &scope)?, GlossaryData::load(tx)?, CharacterRepo { conn: tx }.all_origins()?))
        })?;
        let rules = NameRuleService { state: self.state }.effective(project, &GlossaryContext::name_languages(&glossary, &origins))?;
        let only = match scope {
            TranslationScope::Line(line) => Some(line),
            _ => None,
        };
        // Что ушло бы модели без глоссария: по этому проверяется запись, ведь в базе токены не заменены.
        let sent: HashMap<SegmentId, SentState> =
            scenes.iter().flat_map(|scene| &scene.lines).map(|line| (line.id, line.sent())).collect();
        let mut context = GlossaryContext::new(&glossary);
        context.with_name_rules(&glossary, &origins, &rules);
        let mut glossary = context;
        for scene in &mut scenes {
            glossary.apply(scene);
        }
        // Перевод сцены без описания начинается с её чтения. Перевод одной реплики это быстрая правка, он сцену не читает.
        let unread: Vec<_> = scenes
            .iter()
            .filter(|scene| scene.description.is_none() && scene.lines.iter().any(|line| mode.wants(line)))
            .filter_map(|scene| scene.key.clone())
            .collect();
        let reader = match (mode, only) {
            (LinesMode::Translate, None) if !unread.is_empty() => Some(SceneReader::prepare(self.state, &open, unread, &mut glossary, one_off)?),
            _ => None,
        };
        let run = LlmRun::begin(self.state, Some(open.clone()), action, &target, progress)?;
        let result = match run.id() {
            Some(id) => {
                let mut job =
                    Job { open: &open, target: &target, run: &run, id, mode, hints: glossary.hints, sent, progress: RunProgress::default(), reader };
                job.lines(scenes, only).await.map(|written| LinesRun { run: id, written })
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

    /// Очистка перевода области: пустые правки пользователя, как ручная очистка строки, одной транзакцией.
    /// Уже пустые реплики не трогаются. Возвращает число очищенных.
    pub fn reset(&self, project: ProjectId, scope: TranslationScope) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        let now = Timestamp::now();
        Ok(open.project.write(|tx| {
            let repo = TranslationRepo { conn: tx };
            let lines: Vec<SegmentId> = load_scenes(&repo, &scope)?
                .iter()
                .flat_map(|scene| &scene.lines)
                .filter(|line| !line.text.trim().is_empty())
                .filter(|line| !matches!(scope, TranslationScope::Line(only) if only != line.id))
                .map(|line| line.id)
                .collect();
            let mut cleared = 0;
            for line in lines {
                if repo.save_user_text(line, "", now)? {
                    cleared += 1;
                }
            }
            Ok(cleared)
        })?)
    }

    /// Последние запуски перевода и редактуры для меню «Запуски».
    pub fn line_runs(&self, project: ProjectId) -> Result<Vec<LineRun>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| TranslationRepo { conn: tx }.line_runs(LINE_RUNS))?)
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

    /// Файлы с прогрессом и их сцены для таблицы одним чтением. «Устарело» сверяет отпечаток описания
    /// с текстом сцены в снимке.
    pub fn scene_tree(&self, project: ProjectId) -> Result<Vec<FileScenes>, AppError> {
        let open = self.state.open_project(project)?;
        let (files, mut scenes, sources, described) = open.project.read(|tx| {
            let repo = TranslationRepo { conn: tx };
            let descriptions = SceneDescriptionRepo { conn: tx };
            Ok((repo.files()?, repo.scene_progress()?, descriptions.sources()?, descriptions.list()?))
        })?;
        let hashes: HashMap<_, _> = sources.into_iter().map(|source| (source.key, source.text_hash)).collect();
        let described: HashMap<_, _> = described.into_iter().map(|scene| (scene.key().clone(), scene)).collect();
        for scene in &mut scenes {
            let Some(key) = &scene.key else { continue };
            scene.stale = match (described.get(key), hashes.get(key)) {
                (Some(description), Some(hash)) => description.is_stale(hash),
                _ => false,
            };
        }
        let mut by_file: HashMap<FileId, Vec<SceneProgress>> = HashMap::new();
        for scene in scenes {
            by_file.entry(scene.file).or_default().push(scene);
        }
        Ok(files.into_iter().map(|file| FileScenes { scenes: by_file.remove(&file.id).unwrap_or_default(), file }).collect())
    }

    /// Сцены, где в оригинале или действующем переводе реплик есть `query`, и число таких реплик.
    pub fn search(&self, project: ProjectId, query: &str) -> Result<Vec<SceneHit>, AppError> {
        if query.trim().chars().count() < super::search::MIN_QUERY {
            return Ok(Vec::new());
        }
        let open = self.state.open_project(project)?;
        let texts = open.project.read(|tx| TranslationRepo { conn: tx }.search_texts())?;
        Ok(scene_hits(&texts, query))
    }

    /// Реплики одной сцены по сюжету или реплики файла вне сцен по контейнерам.
    pub fn scene_lines(&self, project: ProjectId, scene: SceneRef) -> Result<Vec<Scene>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let repo = TranslationRepo { conn: tx };
            match scene {
                SceneRef::Scene(key) => Ok(vec![repo.scene(&key)?]),
                SceneRef::Loose(file) => repo.loose_scenes(file),
            }
        })?)
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

    /// Откат запуска модели одной транзакцией. Идущий запуск не откатывается. Возвращает число откаченных
    /// и оставленных реплик.
    pub fn rollback(&self, project: ProjectId, run: RunId) -> Result<(usize, usize), AppError> {
        let open = self.state.open_project(project)?;
        match open.project.read(|tx| TranslationRepo { conn: tx }.run_status(run))? {
            None => Err(crate::storage::DbError::NotFound.into()),
            Some(RunStatus::Running) => Err(AppError::RunActive),
            Some(_) => Ok(open.project.write(|tx| TranslationRepo { conn: tx }.rollback_run(run, Timestamp::now()))?),
        }
    }
}

/// Сколько последних запусков показывает меню «Запуски».
const LINE_RUNS: usize = 15;

/// Итог запуска перевода или редактуры.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinesRun {
    pub run: RunId,
    /// Записанные реплики.
    pub written: usize,
}

/// Сцены запуска. Проект берёт файлы по порядку.
fn load_scenes(repo: &TranslationRepo, scope: &TranslationScope) -> Result<Vec<Scene>, crate::storage::DbError> {
    match scope {
        TranslationScope::Scenes(keys) => keys.iter().map(|key| repo.scene(key)).collect(),
        TranslationScope::File(file) => repo.file_scenes(*file),
        TranslationScope::Line(line) => Ok(vec![repo.scene_of(*line)?]),
        TranslationScope::Loose(file) => repo.loose_scenes(*file),
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
    /// Чтение сцен без описания перед их переводом.
    reader: Option<SceneReader>,
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
        let reading = self.reader.as_ref().map_or(0, SceneReader::requests);
        self.progress.total = (plans.iter().map(Vec::len).sum::<usize>() + reading) as u32;
        self.report(None);

        for (scene, batches) in scenes.iter_mut().zip(plans) {
            let name = scene.title.clone().or_else(|| scene.key.as_ref().map(|key| key.to_string()));
            if !batches.is_empty() {
                self.read(scene, name.as_deref()).await?;
            }
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

    /// Читает сцену без описания, если она в плане чтения: описание пишется в пустое поле и сразу идёт
    /// в запросы её перевода.
    async fn read(&mut self, scene: &mut Scene, name: Option<&str>) -> Result<(), AppError> {
        let (Some(reader), Some(key)) = (&self.reader, &scene.key) else { return Ok(()) };
        if !reader.wants(key) || self.run.cancelled() {
            return Ok(());
        }
        self.progress.message = Some(format!("Чтение сцены: {}", name.unwrap_or(key.as_str())));
        self.run.report(self.progress.clone());
        let mut asker = RunAsk { run: self.run, target: &reader.target, progress: &mut self.progress, error: None };
        if let Some(description) = reader.read(self.open, key, &self.hints, &mut asker).await? {
            scene.description = Some(description);
        }
        Ok(())
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
    use crate::engines::ContainerKey;
    use crate::glossary::GlossaryService;
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
    fn scene_tree_groups_scenes_by_file_with_loose_lines_last() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        // Реплики вне сцен: одна без контейнера, одна на экране.
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                Ok(tx.execute_batch(
                    "INSERT INTO containers (key, kind, file_id, last_seen_scan)
                       SELECT 'screen:hud', 'screen', file_id, last_seen_scan FROM containers WHERE key = 'label:left';
                     UPDATE segments SET container_id = NULL WHERE source_text = 'Static.';
                     UPDATE segments SET container_id = (SELECT id FROM containers WHERE key = 'screen:hud') WHERE source_text = 'Who?';",
                )?)
            })
            .unwrap();
        let start = ContainerKey::new("label:start").unwrap();
        let glossary = GlossaryService { state: &state };
        glossary.edit_scene(id, &start, "Начало", "Эйлин здоровается.").unwrap();
        // Модель читала другой текст сцены.
        open.project.write(|tx| Ok(tx.execute("UPDATE scene_descriptions SET text_hash = 'old'", [])?)).unwrap();
        let loose_line = lines(&service, id, service.files(id).unwrap()[0].id).into_iter().find(|line| line.source_text == "Static.").unwrap();
        service.save_text(id, loose_line.id, "Помехи.").unwrap();

        let tree = service.scene_tree(id).unwrap();
        assert_eq!(tree.len(), 1);
        let group = &tree[0];
        let keys: Vec<_> = group.scenes.iter().map(|scene| scene.key.as_ref().map(|key| key.as_str())).collect();
        assert_eq!(keys, [Some("label:start"), Some("label:left"), None]);
        assert_eq!(group.scenes.iter().map(|scene| scene.total).sum::<usize>(), group.file.total, "каждая реплика файла в одной сцене");
        let (first, left, loose) = (&group.scenes[0], &group.scenes[1], &group.scenes[2]);
        assert_eq!((first.title.as_deref(), first.described, first.stale), (Some("Начало"), true, true));
        assert_eq!((left.described, left.stale), (false, false));
        assert_eq!((loose.total, loose.translated), (2, 1));

        let read = service.scene_lines(id, SceneRef::Scene(start.clone())).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].description.as_deref(), Some("Эйлин здоровается."));
        assert!(read[0].lines.iter().all(|line| line.source_text != "Static." && line.source_text != "Who?"));
        let outside = service.scene_lines(id, SceneRef::Loose(group.file.id)).unwrap();
        let outside: Vec<_> =
            outside.iter().map(|scene| (scene.key.as_ref().map(|key| key.as_str()), scene.lines.len())).collect();
        assert_eq!(outside, [(None, 1), (Some("screen:hud"), 1)], "в порядке файла по контейнерам");

        assert_eq!(service.reset(id, TranslationScope::Loose(group.file.id)).unwrap(), 1);
        assert_eq!(service.scene_tree(id).unwrap()[0].scenes[2].translated, 0);
    }

    #[test]
    fn search_finds_scenes_by_original_and_translation() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let service = TranslationService { state: &state };
        let keys = |hits: Vec<SceneHit>| -> Vec<(Option<String>, usize)> {
            hits.into_iter().map(|hit| (hit.scene.map(|key| key.to_string()), hit.lines)).collect()
        };

        assert_eq!(keys(service.search(id, "hello").unwrap()), [(Some("label:start".to_string()), 2)], "две реплики «Hello.»");
        assert_eq!(keys(service.search(id, "LEFT SIDE").unwrap()), [(Some("label:left".to_string()), 1)]);
        assert!(service.search(id, "привет").unwrap().is_empty());

        let file = service.files(id).unwrap()[0].id;
        let left = lines(&service, id, file).into_iter().find(|line| line.source_text == "Left side.").unwrap();
        service.save_text(id, left.id, "Левая сторона, привет.").unwrap();
        assert_eq!(keys(service.search(id, "ПРИВЕТ").unwrap()), [(Some("label:left".to_string()), 1)], "ищется и перевод");
        assert!(service.search(id, "l").unwrap().is_empty(), "один символ не ищется");
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

        let translated = tauri::async_runtime::block_on(service.translate(id, TranslationScope::File(file.id), None, silent())).unwrap().written;
        assert_eq!(translated, file.total);
        let all = lines(&service, id, file.id);
        assert!(all.iter().all(|line| line.state == LineState::Draft && line.text.starts_with("Перевод")), "{all:?}");
        assert_eq!(count(&state, id, "SELECT count(*) FROM revisions WHERE kind = 'llm' AND run_id IS NOT NULL"), file.total as i64);
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs WHERE status = 'done'"), 1);
        assert!(!state.llm_run.is_active(), "место освобождено");

        let requests = seen.lock().unwrap().len();
        assert_eq!(tauri::async_runtime::block_on(service.translate(id, TranslationScope::Project, None, silent())).unwrap().written, 0);
        assert_eq!(seen.lock().unwrap().len(), requests, "переводить нечего, запросов нет");
    }

    #[test]
    fn glossary_reaches_the_prompt_and_terms_in_the_answer_are_ignored() {
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
        let first = requests
            .iter()
            .filter_map(|request| request["messages"][1]["content"].as_str())
            .find(|user| user.contains("Lines:"))
            .unwrap();
        assert!(first.contains("Speakers:\nEileen: female"), "{first}");
        assert!(first.contains("Russian names: Eileen = Эйлин"), "{first}");

        assert!(glossary.overview(id).unwrap().data.terms.is_empty(), "термины ищет разбор сцены, а не перевод");
        let written: i64 = open.project.read(|tx| Ok(tx.query_row("SELECT count(*) FROM translations", [], |row| row.get(0))?)).unwrap();
        assert!(written > 0, "реплики рядом с терминами в ответе записаны");
    }

    #[test]
    fn scene_without_description_is_read_before_its_translation() {

        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let (url, seen) = fake_model(|body| {
            let system = body["messages"][0]["content"].as_str().unwrap_or_default();
            if system.contains("read a part") {
                completion("Эйлин здоровается с героем.")
            } else if system.contains("name a scene") {
                completion("Приветствие")
            } else {
                answer_every_line(body)
            }
        });
        add_model(&state, &url);
        let service = TranslationService { state: &state };
        let glossary = GlossaryService { state: &state };
        let left = ContainerKey::new("label:left").unwrap();
        glossary.edit_scene(id, &left, "", "Своё описание.").unwrap();

        tauri::async_runtime::block_on(service.translate(id, TranslationScope::Project, None, silent())).unwrap();
        let users: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["messages"][1]["content"].as_str().unwrap_or_default().to_string())
            .collect();
        let translations: Vec<&String> = users.iter().filter(|user| user.contains("Lines:")).collect();
        assert_eq!(users.len(), translations.len() + 2, "одна сцена прочитана: часть и название");
        assert!(translations[0].contains("Scene: Эйлин здоровается с героем."), "{}", translations[0]);
        assert!(translations.iter().any(|user| user.contains("Scene: Своё описание.")), "своё описание не трогается");
        let start = ContainerKey::new("label:start").unwrap();
        let read = service.scene_lines(id, SceneRef::Scene(start.clone())).unwrap();
        assert_eq!((read[0].description.as_deref(), read[0].title.as_deref()), (Some("Эйлин здоровается с героем."), Some("Приветствие")));
        assert_eq!(count(&state, id, "SELECT count(*) FROM llm_runs"), 1, "чтение идёт в запуске перевода");

        glossary.edit_scene(id, &start, "", "").unwrap();
        let file = service.files(id).unwrap()[0].id;
        let line = lines(&service, id, file)[0].id;
        service.save_text(id, line, "").unwrap();
        let before = seen.lock().unwrap().len();
        tauri::async_runtime::block_on(service.translate(id, TranslationScope::Line(line), None, silent())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), before + 1, "перевод одной реплики сцену не читает");
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

        let fixed = tauri::async_runtime::block_on(service.review(id, TranslationScope::Line(first), None, silent())).unwrap().written;
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
    fn rollback_returns_untouched_lines_to_their_state_before_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        let (url, _) = fake_model(answer_every_line);
        add_model(&state, &url);
        let service = TranslationService { state: &state };
        let file = service.files(id).unwrap()[0].id;
        let all = lines(&service, id, file);
        let (first, second) = (all[0].id, all[1].id);
        service.save_text(id, first, "Своё.").unwrap();
        service.save_text(id, first, "").unwrap();

        let run = tauri::async_runtime::block_on(service.translate(id, TranslationScope::File(file), None, silent())).unwrap();
        assert_eq!(run.written, all.len());
        service.save_text(id, second, "Поправлено.").unwrap();
        let runs = service.line_runs(id).unwrap();
        assert_eq!((runs[0].id, runs[0].action, runs[0].status, runs[0].lines), (run.run, Action::Lines, RunStatus::Done, all.len()));

        assert_eq!(service.rollback(id, run.run).unwrap(), (all.len() - 1, 1), "поправленную после запуска не трогаем");
        let after = lines(&service, id, file);
        assert_eq!((after[0].text.as_str(), after[0].state), ("", LineState::Empty), "очищенная до запуска снова пуста");
        assert_eq!(after[1].text, "Поправлено.");
        assert!(after[2..].iter().all(|line| line.state == LineState::Empty), "{after:?}");
        let others = format!("SELECT count(*) FROM translations WHERE segment_id NOT IN ({}, {})", first.get(), second.get());
        assert_eq!(count(&state, id, &others), 0, "у реплик, которых не было в translations до запуска, строк нет");
        assert_eq!(service.rollback(id, run.run).unwrap(), (0, all.len()), "второй откат ничего не меняет");

        assert_eq!(service.reset(id, TranslationScope::File(file)).unwrap(), 1);
        assert_eq!(service.reset(id, TranslationScope::File(file)).unwrap(), 0, "пустые не трогаются");
        assert!(lines(&service, id, file).iter().all(|line| line.text.is_empty()));
        assert!(matches!(service.rollback(id, RunId::from_row(999)), Err(AppError::Db(_))));
    }

    #[test]
    fn closed_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path());
        ProjectService { state: &state }.close(id);
        assert!(matches!(TranslationService { state: &state }.files(id), Err(AppError::ProjectNotOpen)));
    }
}
