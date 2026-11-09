//! Запуски модели по глоссарию: описания по частям сцен, имена, токены, подписи говорящих, термины.

use std::collections::HashMap;
use std::sync::Arc;

use tauri::ipc::Channel;

use super::describe_prompt::{
    CharacterAnswer, CharacterPrompt, CondensePrompt, EntryKind, EntryPartPrompt, PartPrompt, ScenePrompt, TermPrompt, TitlePrompt, clean_note,
    is_useful,
};
use super::entries::{character_names, character_scenes, entry_parts, term_names, term_scenes};
use super::parts::{MAX_PART_TEXT, Part, ScenesPart, pick_parts, scene_hash, script_line, split_lines};
use super::repo::{LineSampleRepo, NoteRepo, SampleLine, SceneDescriptionRepo, StoryLine, TermRepo};
use super::{Note, NoteOwner, SceneDescription};
use crate::common::Timestamp;
use crate::storage::DbError;
use crate::translation::TranslationRepo;
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

/// Сколько раз сжимаются заметки, которые не помещаются в запрос сборки.
const CONDENSE_ROUNDS: usize = 3;

/// Итог запуска описаний: описанные записи и ошибка, которая оборвала запуск.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    pub written: Vec<DescribeTarget>,
    pub error: Option<String>,
}

/// Как писать описание.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fill {
    /// Карточка сцены: описание заменяется.
    Replace,
    /// Группа: заменяется, но описанная сцена без новых частей пропускается.
    ReplaceUnlessRead,
    /// Перевод: только сцена без описания, описание пишется в пустое поле.
    Empty,
}

/// Сколько частей читать.
#[derive(Debug, Clone, Copy)]
enum Choice {
    Count(usize),
    Depth(ReadingDepth),
}

/// Сцена, готовая к запуску: выбранные части, прочитанные заметки этой модели и что писать.
struct ScenePlan {
    key: ContainerKey,
    parts: Vec<Part>,
    /// Заметки этой модели по отпечатку части.
    stored: HashMap<String, Note>,
    /// Текст выбранных частей: по нему берутся подсказки сборки.
    text: String,
    text_hash: String,
    need_title: bool,
    opening: bool,
    fill: Fill,
}

impl ScenePlan {
    /// Запросы плана без сжатия: непрочитанные части, сборка нескольких заметок, название.
    fn requests(&self) -> usize {
        let unread = self.parts.iter().filter(|part| !self.stored.contains_key(&part.hash)).count();
        unread + usize::from(self.parts.len() > 1) + usize::from(self.need_title)
    }
}

/// Персонаж или термин, готовый к запуску: выбранные части его сцен и прочитанные заметки этой модели.
struct EntryPlan {
    target: DescribeTarget,
    kind: EntryKind,
    name: String,
    parts: Vec<Part>,
    stored: HashMap<String, Note>,
    /// Текст выбранных частей: по нему берутся подсказки сборки.
    text: String,
}

impl EntryPlan {
    /// Непрочитанные части и сборка. У персонажа сборка всегда: из неё берётся манера речи.
    fn requests(&self) -> usize {
        let unread = self.parts.iter().filter(|part| !self.stored.contains_key(&part.hash)).count();
        let assembly = match self.kind {
            EntryKind::Character => true,
            EntryKind::Term => self.parts.len() > 1,
        };
        unread + usize::from(assembly)
    }
}

/// Запись запуска описаний.
enum Plan {
    Scene(ScenePlan),
    Entry(EntryPlan),
}

impl Plan {
    fn requests(&self) -> usize {
        match self {
            Self::Scene(plan) => plan.requests(),
            Self::Entry(plan) => plan.requests(),
        }
    }

    fn target(&self) -> DescribeTarget {
        match self {
            Self::Scene(plan) => DescribeTarget::Scene(plan.key.clone()),
            Self::Entry(plan) => plan.target.clone(),
        }
    }
}

/// Часть сцен персонажа или термина для окна.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPart {
    pub hash: String,
    pub start: String,
    pub scenes: Vec<ContainerKey>,
    /// Заметка модели действия `descriptions`, иначе самая новая.
    pub note: Option<Note>,
    pub useful: bool,
    /// Входит в выборку следующего запуска.
    pub picked: bool,
}

/// Части записи, сколько из них прочитает запуск, и исключённые сцены.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryParts {
    pub parts: Vec<EntryPart>,
    pub picked: usize,
    pub excluded: Vec<ContainerKey>,
}

/// Описания персонажей, терминов и сцен.
pub struct DescribeService<'a> {
    pub state: &'a AppState,
}

impl DescribeService<'_> {
    /// Читает `parts` частей (не меньше прочитанных) и собирает описание. Остановленный запуск продолжается
    /// со следующей непрочитанной части: заметки пишутся сразу.
    pub async fn describe(
        &self,
        project: ProjectId,
        target: DescribeTarget,
        parts: usize,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Described, AppError> {
        self.run(project, vec![target], Choice::Count(parts), Fill::Replace, one_off, progress).await
    }

    /// Группа по очереди с глубиной чтения. Заполненная запись без новых частей в запуск не входит.
    pub async fn describe_many(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        depth: ReadingDepth,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Described, AppError> {
        self.run(project, targets, Choice::Depth(depth), Fill::ReplaceUnlessRead, one_off, progress).await
    }

    /// Сбрасывает заметки владельца всех моделей: следующий запуск прочитает части заново.
    pub fn reset_notes(&self, project: ProjectId, target: &DescribeTarget) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        let owner = owner_of(target);
        Ok(open.project.write(|tx| NoteRepo { conn: tx }.delete_owner(&owner))?)
    }

    /// Части сцен персонажа или термина для окна: сцены части, заметка, войдёт ли часть в следующий запуск.
    pub fn entry_parts(&self, project: ProjectId, target: &DescribeTarget) -> Result<EntryParts, AppError> {
        let open = self.state.open_project(project)?;
        // Без моделей окно всё равно показывает части: выборка считается по заметкам любой модели.
        let model = LlmTargetService { state: self.state }
            .target(Some(&open), Action::Descriptions, None)
            .ok()
            .map(|target| target.profile.model().to_string());
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let context = GlossaryContext::new(&glossary);
            let lines = LineSampleRepo { conn: tx }.story_lines()?;
            let Some(selection) = select_entry(tx, &context, &glossary, &lines, target, false)? else { return Ok(EntryParts::default()) };
            let notes = NoteRepo { conn: tx }.all_of(&owner_of(target))?;
            let note_of = |hash: &str| {
                let own = notes.iter().find(|note| note.part_hash == hash && Some(&note.model) == model.as_ref());
                own.or_else(|| notes.iter().find(|note| note.part_hash == hash && model.is_none()))
            };
            let ready: Vec<bool> = selection.parts.iter().map(|part| note_of(&part.part.hash).is_some()).collect();
            let picked = pick_parts(&ready, ReadingDepth::default().parts_for(selection.parts.len()));
            let parts = selection
                .parts
                .iter()
                .enumerate()
                .map(|(index, part)| {
                    let note = note_of(&part.part.hash).or_else(|| notes.iter().find(|note| note.part_hash == part.part.hash)).cloned();
                    EntryPart {
                        hash: part.part.hash.clone(),
                        start: part.part.start(),
                        scenes: part.scenes.clone(),
                        useful: note.as_ref().is_some_and(|note| is_useful(&note.text)),
                        note,
                        picked: picked.contains(&index),
                    }
                })
                .collect();
            Ok(EntryParts { parts, picked: picked.len(), excluded: selection.excluded })
        })?)
    }

    /// Заметка не идёт в описание или возвращается в него.
    pub fn exclude_note(&self, project: ProjectId, target: &DescribeTarget, model: &str, part_hash: &str, excluded: bool) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.write(|tx| NoteRepo { conn: tx }.set_excluded(&owner_of(target), model, part_hash, excluded))?)
    }

    /// Части сцены с заметками всех моделей для окна: начало части и заметка, новая первой.
    pub fn scene_parts(&self, project: ProjectId, key: &ContainerKey) -> Result<Vec<PartNote>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            let parts = scene_text(tx, &mut context, key)?.map(|(parts, _)| parts).unwrap_or_default();
            let notes = NoteRepo { conn: tx }.all_of(&NoteOwner::Container(key.clone()))?;
            Ok(parts
                .into_iter()
                .map(|part| {
                    let note = notes.iter().find(|note| note.part_hash == part.hash);
                    PartNote {
                        start: part.start(),
                        note: note.map(|note| note.text.clone()),
                        model: note.map(|note| note.model.clone()),
                        useful: note.is_some_and(|note| is_useful(&note.text)),
                    }
                })
                .collect())
        })?)
    }

    async fn run(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        choice: Choice,
        fill: Fill,
        one_off: Option<ProfileId>,
        progress: Channel<RunProgress>,
    ) -> Result<Described, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Descriptions, one_off)?;
        let model = target.profile.model().to_string();
        let (plans, hints) = open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            let first = SceneDescriptionRepo { conn: tx }.first_scene()?;
            let entries = targets.iter().any(|target| !matches!(target, DescribeTarget::Scene(_)));
            let lines = if entries { LineSampleRepo { conn: tx }.story_lines()? } else { Vec::new() };
            let mut plans = Vec::new();
            for target in targets {
                let plan = match target {
                    DescribeTarget::Scene(key) => plan_scene(tx, &mut context, key, &model, choice, fill, first.as_ref())?.map(Plan::Scene),
                    other => plan_entry(tx, &context, &glossary, &lines, other, &model, choice, fill)?.map(Plan::Entry),
                };
                plans.extend(plan);
            }
            Ok((plans, context.hints))
        })?;
        let mut described = Described::default();
        if plans.is_empty() {
            return Ok(described);
        }
        let total = plans.iter().map(Plan::requests).sum();
        let mut run = BatchRun::begin(self.state, &open, Action::Descriptions, &target, progress, total)?;
        let writer = SceneWriter { open: &open, hints: &hints, target: &target, model: &model };
        for plan in plans {
            let done = described.written.len();
            let outcome = match &plan {
                Plan::Scene(scene) => writer.scene(&mut run, scene, done).await,
                Plan::Entry(entry) => writer.entry(&mut run, entry, done).await,
            };
            match outcome {
                Ok(outcome) => {
                    if outcome.description.is_some() {
                        described.written.push(plan.target());
                    }
                    if !outcome.going {
                        break;
                    }
                }
                Err(error) => {
                    run.error = Some(error.to_string());
                    break;
                }
            }
        }
        let result = run.finish(described.written)?;
        Ok(Described { written: result.items, error: result.error })
    }
}

/// Чтение сцен перед переводом: сцены без описания читаются с глубиной по умолчанию моделью действия
/// `descriptions`, описание пишется только в пустое поле.
pub(crate) struct SceneReader {
    plans: Vec<ScenePlan>,
    pub target: LlmTarget,
    model: String,
}

impl SceneReader {
    /// Планы для сцен `keys`. Сцена с описанием в план не входит.
    pub fn prepare(
        state: &AppState,
        open: &Arc<OpenProject>,
        keys: Vec<ContainerKey>,
        context: &mut GlossaryContext,
        one_off: Option<ProfileId>,
    ) -> Result<Self, AppError> {
        let target = LlmTargetService { state }.target(Some(open), Action::Descriptions, one_off)?;
        let model = target.profile.model().to_string();
        let plans = open.project.read(|tx| {
            let first = SceneDescriptionRepo { conn: tx }.first_scene()?;
            let mut plans = Vec::new();
            for key in keys {
                let choice = Choice::Depth(ReadingDepth::default());
                if let Some(plan) = plan_scene(tx, context, key, &model, choice, Fill::Empty, first.as_ref())? {
                    plans.push(plan);
                }
            }
            Ok(plans)
        })?;
        Ok(Self { plans, target, model })
    }

    /// Запросы всех планов без сжатия: для прогресса запуска.
    pub fn requests(&self) -> usize {
        self.plans.iter().map(ScenePlan::requests).sum()
    }

    pub fn wants(&self, key: &ContainerKey) -> bool {
        self.plans.iter().any(|plan| &plan.key == key)
    }

    /// Читает сцену и возвращает записанное описание. `None`: модель ничего не дала, описание уже кто-то
    /// написал или запуск остановлен. Ошибка запроса обрывает запуск.
    pub async fn read(&self, open: &Arc<OpenProject>, key: &ContainerKey, hints: &Hints, asker: &mut RunAsk<'_>) -> Result<Option<String>, AppError> {
        let Some(plan) = self.plans.iter().find(|plan| &plan.key == key) else { return Ok(None) };
        let writer = SceneWriter { open, hints, target: &self.target, model: &self.model };
        let outcome = writer.scene(asker, plan, 0).await?;
        if let Some(error) = asker.error.take() {
            return Err(error.into());
        }
        Ok(outcome.description)
    }
}

/// Часть сцены для окна.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartNote {
    pub start: String,
    pub note: Option<String>,
    pub model: Option<String>,
    /// Заметка пойдёт в описание: не пустая и не отказ.
    pub useful: bool,
}

fn owner_of(target: &DescribeTarget) -> NoteOwner {
    match target {
        DescribeTarget::Character(id) => NoteOwner::Character(*id),
        DescribeTarget::Term(id) => NoteOwner::Term(*id),
        DescribeTarget::Scene(key) => NoteOwner::Container(key.clone()),
    }
}

/// Реплики сцены строками «Говорящий: текст» с подписями из глоссария и значениями токенов, разрезанные на части,
/// и отпечаток текста сцены. `None`: сцены нет в текущем снимке.
fn scene_text(tx: &rusqlite::Connection, context: &mut GlossaryContext, key: &ContainerKey) -> Result<Option<(Vec<Part>, String)>, DbError> {
    let found = TranslationRepo { conn: tx }.scene(key);
    let mut scene = match found {
        Ok(scene) => scene,
        Err(DbError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    let text_hash = scene_hash(scene.lines.iter().map(|line| line.source_hash.as_str()));
    context.apply(&mut scene);
    let lines: Vec<String> = scene.lines.iter().map(|line| script_line(line.speaker.as_deref(), &line.source_text)).collect();
    Ok(Some((split_lines(&lines), text_hash)))
}

/// План сцены. `None`: сцену пропускаем, у неё нет реплик или она заполнена и новых частей нет.
fn plan_scene(
    tx: &rusqlite::Connection,
    context: &mut GlossaryContext,
    key: ContainerKey,
    model: &str,
    choice: Choice,
    fill: Fill,
    first: Option<&ContainerKey>,
) -> Result<Option<ScenePlan>, DbError> {
    let Some((all, text_hash)) = scene_text(tx, context, &key)? else { return Ok(None) };
    if all.is_empty() {
        return Ok(None);
    }
    let stored: HashMap<String, Note> = NoteRepo { conn: tx }
        .of(&NoteOwner::Container(key.clone()), model)?
        .into_iter()
        .map(|note| (note.part_hash.clone(), note))
        .collect();
    let ready: Vec<bool> = all.iter().map(|part| stored.contains_key(&part.hash)).collect();
    let count = match choice {
        Choice::Count(count) => count,
        Choice::Depth(depth) => depth.parts_for(all.len()),
    };
    let parts: Vec<Part> = pick_parts(&ready, count).into_iter().map(|index| all[index].clone()).collect();
    let description = SceneDescriptionRepo { conn: tx }.get(&key)?;
    let described = description.as_ref().is_some_and(|scene| scene.description().is_some());
    let skip = match fill {
        Fill::Replace => false,
        Fill::ReplaceUnlessRead => {
            let current = described && description.as_ref().is_some_and(|scene| !scene.is_stale(&text_hash));
            current && parts.iter().all(|part| stored.contains_key(&part.hash))
        }
        Fill::Empty => described,
    };
    if skip {
        return Ok(None);
    }
    let text = parts.iter().map(|part| part.text.as_str()).collect();
    let need_title = description.as_ref().is_none_or(|scene| scene.title().is_none());
    let opening = first == Some(&key);
    Ok(Some(ScenePlan { key, parts, stored, text, text_hash, need_title, opening, fill }))
}

/// Сцены записи без исключённых и части из них.
struct Selection {
    kind: EntryKind,
    name: String,
    parts: Vec<ScenesPart>,
    excluded: Vec<ContainerKey>,
    /// Описание, которое уже есть.
    described: bool,
}

/// `None`: записи нет или у персонажа нет имени, по которому его искать.
/// Сцены и части персонажа или термина. `ignore_exclusions`: части по всем сценам, как будто ничего не исключено.
fn select_entry(
    tx: &rusqlite::Connection,
    context: &GlossaryContext,
    glossary: &GlossaryData,
    lines: &[StoryLine],
    target: &DescribeTarget,
    ignore_exclusions: bool,
) -> Result<Option<Selection>, DbError> {
    Ok(match target {
        DescribeTarget::Character(id) => {
            let Some(character) = glossary.characters.iter().find(|character| character.id() == Some(*id)) else { return Ok(None) };
            let Some(name) = character.full_name() else { return Ok(None) };
            let excluded = if ignore_exclusions { Vec::new() } else { CharacterRepo { conn: tx }.excluded_containers(*id)? };
            let speakers = glossary.links.iter().filter(|link| link.character == *id).map(|link| link.speaker.clone()).collect();
            let scenes = character_scenes(lines, &speakers, &character_names(character), &excluded);
            let parts = entry_parts(lines, &scenes, context);
            Some(Selection { kind: EntryKind::Character, name, parts, excluded, described: character.description().is_some() })
        }
        DescribeTarget::Term(id) => {
            let Some(term) = glossary.terms.iter().find(|term| term.id() == Some(*id)) else { return Ok(None) };
            let excluded = if ignore_exclusions { Vec::new() } else { TermRepo { conn: tx }.excluded_containers(*id)? };
            let scenes = term_scenes(lines, &term_names(term), &excluded);
            let parts = entry_parts(lines, &scenes, context);
            Some(Selection { kind: EntryKind::Term, name: term.name().to_string(), parts, excluded, described: term.description().is_some() })
        }
        DescribeTarget::Scene(_) => None,
    })
}

/// После повторного считывания удаляет заметки, чьих частей больше нет у владельца. Части сцены берутся из её
/// реплик, у пропавшей сцены их нет. Части персонажа и термина собираются из нескольких сцен и зависят от
/// исключений: заметка остаётся, если её часть есть среди частей без исключений или при нынешних исключениях.
/// Возвращает число удалённых заметок.
pub(crate) fn prune_notes(tx: &rusqlite::Connection) -> Result<usize, DbError> {
    let notes = NoteRepo { conn: tx };
    let owners = notes.owners()?;
    if owners.is_empty() {
        return Ok(0);
    }
    let glossary = GlossaryData::load(tx)?;
    let mut context = GlossaryContext::new(&glossary);
    let lines = LineSampleRepo { conn: tx }.story_lines()?;
    let mut removed = 0;
    for owner in owners {
        let hashes: Vec<String> = match &owner {
            NoteOwner::Container(key) => {
                scene_text(tx, &mut context, key)?.map(|(parts, _)| parts.into_iter().map(|part| part.hash).collect()).unwrap_or_default()
            }
            NoteOwner::Character(id) => entry_hashes(tx, &context, &glossary, &lines, &DescribeTarget::Character(*id))?,
            NoteOwner::Term(id) => entry_hashes(tx, &context, &glossary, &lines, &DescribeTarget::Term(*id))?,
        };
        removed += notes.retain_parts(&owner, &hashes)?;
    }
    Ok(removed)
}

/// Отпечатки частей персонажа или термина без исключений и при нынешних исключениях.
/// Удалённого владельца нет в глоссарии: частей у него нет.
fn entry_hashes(
    tx: &rusqlite::Connection,
    context: &GlossaryContext,
    glossary: &GlossaryData,
    lines: &[StoryLine],
    target: &DescribeTarget,
) -> Result<Vec<String>, DbError> {
    let mut hashes = Vec::new();
    for ignore_exclusions in [true, false] {
        if let Some(selection) = select_entry(tx, context, glossary, lines, target, ignore_exclusions)? {
            hashes.extend(selection.parts.into_iter().map(|part| part.part.hash));
        }
    }
    Ok(hashes)
}

/// План персонажа или термина. `None`: читать нечего или запись заполнена и новых частей нет.
#[allow(clippy::too_many_arguments)]
fn plan_entry(
    tx: &rusqlite::Connection,
    context: &GlossaryContext,
    glossary: &GlossaryData,
    lines: &[StoryLine],
    target: DescribeTarget,
    model: &str,
    choice: Choice,
    fill: Fill,
) -> Result<Option<EntryPlan>, DbError> {
    let Some(selection) = select_entry(tx, context, glossary, lines, &target, false)? else { return Ok(None) };
    if selection.parts.is_empty() {
        return Ok(None);
    }
    let stored: HashMap<String, Note> =
        NoteRepo { conn: tx }.of(&owner_of(&target), model)?.into_iter().map(|note| (note.part_hash.clone(), note)).collect();
    let ready: Vec<bool> = selection.parts.iter().map(|part| stored.contains_key(&part.part.hash)).collect();
    let count = match choice {
        Choice::Count(count) => count,
        Choice::Depth(depth) => depth.parts_for(selection.parts.len()),
    };
    let parts: Vec<Part> = pick_parts(&ready, count).into_iter().map(|index| selection.parts[index].part.clone()).collect();
    let skip = match fill {
        Fill::Replace => false,
        Fill::ReplaceUnlessRead => selection.described && parts.iter().all(|part| stored.contains_key(&part.hash)),
        Fill::Empty => selection.described,
    };
    if skip {
        return Ok(None);
    }
    let text = parts.iter().map(|part| part.text.as_str()).collect();
    Ok(Some(EntryPlan { target, kind: selection.kind, name: selection.name, parts, stored, text }))
}

/// Итог сцены: записанное описание и идёт ли запуск дальше.
#[derive(Debug, Clone)]
struct Outcome {
    description: Option<String>,
    going: bool,
}

impl Outcome {
    const STOPPED: Self = Self { description: None, going: false };
    const NOTHING: Self = Self { description: None, going: true };
}

/// Чтение частей, сборка и запись описания одной сцены.
struct SceneWriter<'a> {
    open: &'a Arc<OpenProject>,
    hints: &'a Hints,
    target: &'a LlmTarget,
    model: &'a str,
}

impl SceneWriter<'_> {
    /// Заметки выбранных частей: готовые из базы, новые спрашиваются и пишутся сразу. Исключённые, пустые
    /// и отказы в итог не входят. `None`: запуск остановлен.
    async fn notes(
        &self,
        run: &mut impl Ask,
        owner: &NoteOwner,
        parts: &[Part],
        stored: &HashMap<String, Note>,
        written: usize,
        request: impl Fn(&Part) -> ChatRequest,
    ) -> Result<Option<Vec<String>>, AppError> {
        let mut notes = Vec::new();
        for part in parts {
            let note = match stored.get(&part.hash) {
                Some(note) if note.excluded => continue,
                Some(note) => note.text.clone(),
                None => {
                    let note = match run.send(&request(part)).await {
                        Sent::Text(text) => clean_note(&text),
                        Sent::Skipped => String::new(),
                        Sent::Stop => return Ok(None),
                    };
                    if !note.is_empty() {
                        let saved = Note {
                            owner: owner.clone(),
                            model: self.model.to_string(),
                            part_hash: part.hash.clone(),
                            text: note.clone(),
                            excluded: false,
                            created_at: Timestamp::now(),
                        };
                        self.open.project.write(|tx| NoteRepo { conn: tx }.save(&saved))?;
                    }
                    if !run.step(written) {
                        return Ok(None);
                    }
                    note
                }
            };
            if is_useful(&note) {
                notes.push(note);
            }
        }
        Ok(Some(notes))
    }

    /// Описание персонажа или термина по заметкам частей. Запуск заменяет описание, у персонажа и манеру речи.
    async fn entry(&self, run: &mut impl Ask, plan: &EntryPlan, written: usize) -> Result<Outcome, AppError> {
        let owner = owner_of(&plan.target);
        let request = |part: &Part| EntryPartPrompt::build(plan.kind, &plan.name, &part.text, self.hints, self.target);
        let Some(notes) = self.notes(run, &owner, &plan.parts, &plan.stored, written, request).await? else { return Ok(Outcome::STOPPED) };
        if notes.is_empty() {
            return Ok(Outcome::NOTHING);
        }
        let mut going = true;
        let answer = if let (EntryKind::Term, [_], [note]) = (plan.kind, plan.parts.as_slice(), notes.as_slice()) {
            Some(CharacterAnswer { description: note.clone(), speech: None })
        } else {
            let Some(notes) = self.condense(run, notes, written).await else { return Ok(Outcome::STOPPED) };
            let request = match plan.kind {
                EntryKind::Character => CharacterPrompt::build(&plan.name, &notes, &plan.text, self.hints, self.target),
                EntryKind::Term => TermPrompt::build(&plan.name, &notes, &plan.text, self.hints, self.target),
            };
            let text = match run.send(&request).await {
                Sent::Text(text) => text,
                Sent::Skipped => String::new(),
                Sent::Stop => return Ok(Outcome::STOPPED),
            };
            going = run.step(written);
            match plan.kind {
                EntryKind::Character => CharacterPrompt::accept(&text),
                EntryKind::Term => Some(clean_note(&text)).filter(|text| !text.is_empty()).map(|description| CharacterAnswer { description, speech: None }),
            }
        };
        let Some(answer) = answer else { return Ok(Outcome { description: None, going }) };
        self.open.project.write(|tx| match &plan.target {
            DescribeTarget::Character(id) => {
                let repo = CharacterRepo { conn: tx };
                let mut character = repo.get(*id)?;
                character.set_description(&answer.description);
                if let Some(speech) = &answer.speech {
                    character.set_speech(speech);
                }
                repo.save(&character).map(|_| ())
            }
            DescribeTarget::Term(id) => {
                let repo = TermRepo { conn: tx };
                let mut term = repo.get(*id)?;
                term.set_description(&answer.description);
                repo.save(&term).map(|_| ())
            }
            DescribeTarget::Scene(_) => Ok(()),
        })?;
        Ok(Outcome { description: Some(answer.description), going })
    }

    /// Возвращает записанное описание и идёт ли запуск дальше.
    async fn scene(&self, run: &mut impl Ask, plan: &ScenePlan, written: usize) -> Result<Outcome, AppError> {
        let owner = NoteOwner::Container(plan.key.clone());
        let request = |part: &Part| PartPrompt::build(&part.text, self.hints, self.target);
        let Some(notes) = self.notes(run, &owner, &plan.parts, &plan.stored, written, request).await? else { return Ok(Outcome::STOPPED) };
        if notes.is_empty() {
            return Ok(Outcome::NOTHING);
        }

        let mut going = true;
        let description = if let ([_], [note]) = (plan.parts.as_slice(), notes.as_slice()) {
            note.clone()
        } else {
            let Some(notes) = self.condense(run, notes, written).await else { return Ok(Outcome::STOPPED) };
            let description = match run.send(&ScenePrompt::build(&notes, &plan.text, self.hints, self.target)).await {
                Sent::Text(text) => clean_note(&text),
                Sent::Skipped => String::new(),
                Sent::Stop => return Ok(Outcome::STOPPED),
            };
            going = run.step(written);
            description
        };
        if description.is_empty() {
            return Ok(Outcome { description: None, going });
        }
        let changed = self.open.project.write(|tx| {
            let repo = SceneDescriptionRepo { conn: tx };
            let mut scene = repo.get(&plan.key)?.unwrap_or_else(|| SceneDescription::new(plan.key.clone(), Timestamp::now()));
            let changed = match plan.fill {
                Fill::Empty => scene.fill_description(&description, &plan.text_hash, Timestamp::now()),
                Fill::Replace | Fill::ReplaceUnlessRead => scene.replace_description(&description, &plan.text_hash, Timestamp::now()),
            };
            if changed {
                repo.save(&scene)?;
            }
            Ok(changed)
        })?;
        if !changed {
            return Ok(Outcome { description: None, going });
        }
        if !going || !plan.need_title {
            return Ok(Outcome { description: Some(description), going });
        }

        let title = match run.send(&TitlePrompt::build(&description, plan.opening, self.target)).await {
            Sent::Text(text) => TitlePrompt::accept(&text),
            Sent::Skipped => None,
            Sent::Stop => return Ok(Outcome { description: Some(description), going: false }),
        };
        if let Some(title) = title {
            self.open.project.write(|tx| {
                let repo = SceneDescriptionRepo { conn: tx };
                let Some(mut scene) = repo.get(&plan.key)? else { return Ok(()) };
                if scene.fill_title(&title, Timestamp::now()) {
                    repo.save(&scene)?;
                }
                Ok(())
            })?;
        }
        let going = run.step(written + 1);
        Ok(Outcome { description: Some(description), going })
    }

    /// Сжимает заметки группами, пока они не поместятся в запрос сборки. `None`: запуск остановлен.
    async fn condense(&self, run: &mut impl Ask, mut notes: Vec<String>, written: usize) -> Option<Vec<String>> {
        for _ in 0..CONDENSE_ROUNDS {
            if CondensePrompt::fits(&notes) {
                break;
            }
            let groups = CondensePrompt::groups(notes);
            run.grow(groups.len());
            let mut condensed = Vec::new();
            for group in groups {
                match run.send(&CondensePrompt::build(&group, self.target)).await {
                    Sent::Text(text) => {
                        let text = clean_note(&text);
                        if text.is_empty() { condensed.extend(group) } else { condensed.push(text) }
                    }
                    Sent::Skipped => condensed.extend(group),
                    Sent::Stop => return None,
                }
                if !run.step(written) {
                    return None;
                }
            }
            notes = condensed;
        }
        if !CondensePrompt::fits(&notes) {
            let size: usize = notes.iter().map(|note| note.chars().count()).sum();
            let keep = (notes.len() * MAX_PART_TEXT / size.max(1)).max(1);
            notes = spread(&notes, keep);
        }
        Some(notes)
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
pub(crate) enum Sent {
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

    /// Запуск узнал о запросах, которых не было в плане, например о сжатии заметок.
    fn grow(&mut self, more: usize) {
        self.report.total += more as u32;
        self.run.report(self.report.clone());
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

/// Запросы запуска, в котором читаются сцены: свой запуск описаний или чужой запуск, например перевода.
pub(crate) trait Ask {
    /// Запрос без отчёта. Ошибка запоминается и обрывает запуск.
    async fn send(&mut self, request: &ChatRequest) -> Sent;
    /// Запрос готов. `false`: запуск остановлен.
    fn step(&mut self, written: usize) -> bool;
    /// Запросов больше, чем было в плане.
    fn grow(&mut self, more: usize);
}

impl Ask for BatchRun<'_> {
    async fn send(&mut self, request: &ChatRequest) -> Sent {
        BatchRun::send(self, request).await
    }

    fn step(&mut self, written: usize) -> bool {
        BatchRun::step(self, written)
    }

    fn grow(&mut self, more: usize) {
        BatchRun::grow(self, more);
    }
}

/// Запросы чтения сцены внутри запуска перевода: прогресс общий, ошибка запоминается для запуска.
pub(crate) struct RunAsk<'a> {
    pub run: &'a LlmRun,
    pub target: &'a LlmTarget,
    pub progress: &'a mut RunProgress,
    pub error: Option<LlmError>,
}

impl Ask for RunAsk<'_> {
    async fn send(&mut self, request: &ChatRequest) -> Sent {
        if self.run.cancelled() || self.error.is_some() {
            return Sent::Stop;
        }
        match ask(LlmClient::for_kind(self.target.provider.kind()), self.target, request, self.run).await {
            Ok(Some(text)) => Sent::Text(text),
            Ok(None) => Sent::Skipped,
            Err(error) => {
                self.error = Some(error);
                Sent::Stop
            }
        }
    }

    fn step(&mut self, _written: usize) -> bool {
        self.progress.done += 1;
        self.run.report(self.progress.clone());
        !self.run.cancelled()
    }

    fn grow(&mut self, more: usize) {
        self.progress.total += more as u32;
        self.run.report(self.progress.clone());
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
    fn scenes_are_described_from_part_notes_and_named_once() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            let answer = if system.contains("name a scene") { "«Знакомство с Эйлин»." } else { "**Эйлин** здоровается с героем." };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let start = ContainerKey::new("label:start").unwrap();
        let left = ContainerKey::new("label:left").unwrap();

        let described =
            tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(start.clone()), usize::MAX, None, silent())).unwrap();
        assert_eq!(described, Described { written: vec![DescribeTarget::Scene(start.clone())], error: None });
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 2, "одна часть сразу становится описанием, потом название");
            let part = user_text(&requests[0]);
            assert!(part.contains("Script:\n\nEileen: Hello.\nNarration.\nRadio Chatter: Static.\n"), "{part}");
            assert!(system_text(&requests[1]).ends_with("This is the first scene of the game."));
        }
        let open = state.open_project(id).unwrap();
        let (scene, sources) = open
            .project
            .read(|tx| {
                let repo = SceneDescriptionRepo { conn: tx };
                Ok((repo.get(&start)?.unwrap(), repo.sources()?))
            })
            .unwrap();
        assert_eq!(scene.description(), Some("Эйлин здоровается с героем."));
        assert_eq!(scene.title(), Some("Знакомство с Эйлин"));
        let source = sources.iter().find(|source| source.key == start).unwrap();
        assert!(!scene.is_stale(&source.text_hash), "описание помнит текст, который прочитала модель");

        let described = tauri::async_runtime::block_on(service.describe_many(
            id,
            vec![DescribeTarget::Scene(start.clone()), DescribeTarget::Scene(left.clone())],
            ReadingDepth::default(),
            None,
            silent(),
        ))
        .unwrap();
        assert_eq!(described.written, [DescribeTarget::Scene(left.clone())], "заполненная сцена без новых частей пропущена");
        assert_eq!(seen.lock().unwrap().len(), 4);

        tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(start.clone()), usize::MAX, None, silent())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 4, "заметка готова, название есть: запросов нет");

        let parts = service.scene_parts(id, &start).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].note.as_deref(), Some("Эйлин здоровается с героем."));
        assert!(parts[0].useful);
        assert_eq!(service.reset_notes(id, &DescribeTarget::Scene(start.clone())).unwrap(), 1);
        assert_eq!(service.scene_parts(id, &start).unwrap()[0].note, None);
    }

    #[test]
    fn characters_and_terms_are_described_from_their_scenes() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            // Сборка тоже говорит о заметках, поэтому проверяется первой.
            let answer = if system.contains("about one character") {
                "```json\n{\"description\": \"Эйлин встречает героя.\", \"speech\": \"Говорит на «ты».\"}\n```"
            } else if system.contains("notes about the character") {
                "Эйлин здоровается и показывает дорогу."
            } else if system.contains("notes about the term") {
                "Сторона, куда ведёт дорога."
            } else {
                "Лишняя сборка."
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let side = glossary.take_term(id, "side", TermCategory::General).unwrap();
        let service = DescribeService { state: &state };
        let character = DescribeTarget::Character(eileen);
        let term = DescribeTarget::Term(side);

        let parts = service.entry_parts(id, &character).unwrap();
        assert_eq!(parts.parts.len(), 1, "две маленькие сцены в одной части");
        let scenes: Vec<&str> = parts.parts[0].scenes.iter().map(ContainerKey::as_str).collect();
        assert_eq!(scenes, ["label:start", "label:left"]);
        assert_eq!((parts.picked, parts.parts[0].note.is_none()), (1, true));

        let described =
            tauri::async_runtime::block_on(service.describe_many(id, vec![character.clone(), term.clone()], ReadingDepth::default(), None, silent()))
                .unwrap();
        assert_eq!(described, Described { written: vec![character.clone(), term.clone()], error: None });
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 3, "часть и сборка персонажа, единственная часть термина");
            let part = user_text(&requests[0]);
            assert!(part.contains("Eileen: Hello.\n") && part.contains("---\nEileen: Left side.\n"), "{part}");
            assert!(part.ends_with("Notes about Eileen:"), "{part}");
        }
        let open = state.open_project(id).unwrap();
        let (person, item) = open.project.read(|tx| Ok((CharacterRepo { conn: tx }.get(eileen)?, TermRepo { conn: tx }.get(side)?))).unwrap();
        assert_eq!((person.description(), person.speech()), (Some("Эйлин встречает героя."), Some("Говорит на «ты».")));
        assert_eq!(item.description(), Some("Сторона, куда ведёт дорога."), "заметка единственной части сразу описание");

        let parts = service.entry_parts(id, &character).unwrap();
        let note = parts.parts[0].note.clone().unwrap();
        service.exclude_note(id, &character, &note.model, &note.part_hash, true).unwrap();
        assert!(service.entry_parts(id, &character).unwrap().parts[0].note.as_ref().unwrap().excluded);

        let left = ContainerKey::new("label:left").unwrap();
        glossary.set_excluded_scene(id, crate::glossary::ExcludeOwner::Character(eileen), &left, true).unwrap();
        let parts = service.entry_parts(id, &character).unwrap();
        assert_eq!(parts.excluded, std::slice::from_ref(&left));
        let scenes: Vec<&str> = parts.parts[0].scenes.iter().map(ContainerKey::as_str).collect();
        assert_eq!(scenes, ["label:start"], "исключённая сцена модели не даётся");
        assert!(parts.parts[0].note.is_none(), "у новой части заметки нет");
    }

    #[test]
    fn notes_of_parts_that_are_gone_are_pruned() {
        let (url, _) = fake_model(|body| {
            let system = system_text(body);
            let answer = if system.contains("about one character") {
                "```json
{\"description\": \"Эйлин встречает героя.\"}
```"
            } else if system.contains("name a scene") {
                "Знакомство"
            } else {
                "Эйлин здоровается."
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let service = DescribeService { state: &state };
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let character = DescribeTarget::Character(eileen);
        let start = ContainerKey::new("label:start").unwrap();
        let left = ContainerKey::new("label:left").unwrap();
        tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(start.clone()), usize::MAX, None, silent())).unwrap();
        tauri::async_runtime::block_on(service.describe(id, character.clone(), usize::MAX, None, silent())).unwrap();
        // Заметка при исключённой сцене: её часть есть только при нынешних исключениях.
        glossary.set_excluded_scene(id, crate::glossary::ExcludeOwner::Character(eileen), &left, true).unwrap();
        tauri::async_runtime::block_on(service.describe(id, character.clone(), usize::MAX, None, silent())).unwrap();

        let open = state.open_project(id).unwrap();
        let count = |sql: &str| -> i64 { open.project.read(|tx| Ok(tx.query_row(sql, [], |row| row.get(0))?)).unwrap() };
        let real = count("SELECT count(*) FROM notes");
        assert!(real >= 3, "сцена и две части персонажа: {real}");
        open.project
            .write(|tx| {
                for (kind, owner) in [("container", "label:start"), ("container", "label:gone"), ("character", "999999")] {
                    tx.execute(
                        "INSERT INTO notes (owner_kind, owner_ref, model, part_hash, text, created_at) VALUES (?1, ?2, 'm', 'stale', 'x', 0)",
                        rusqlite::params![kind, owner],
                    )?;
                }
                tx.execute(
                    "INSERT INTO notes (owner_kind, owner_ref, model, part_hash, text, created_at) VALUES ('character', ?1, 'm', 'stale', 'x', 0)",
                    [eileen.get()],
                )?;
                Ok(())
            })
            .unwrap();

        assert_eq!(open.project.write(|tx| prune_notes(tx)).unwrap(), 4);
        assert_eq!(count("SELECT count(*) FROM notes"), real);
        assert_eq!(count("SELECT count(*) FROM notes WHERE part_hash = 'stale'"), 0);
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
