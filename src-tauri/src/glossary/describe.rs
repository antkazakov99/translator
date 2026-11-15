//! Описания моделью: разбор сцен, описания сцен, персонажей и терминов по частям. Ход запуска лежит в `run.rs`,
//! предложения по записям глоссария (имена, термины, подписи, токены) в `suggest_service.rs`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::analyze::{AnalysisPrompt, Asked, PartLine, merge, questions};
use super::mentions::Casings;
use super::relations::Pair;
use super::repo::MentionCheckRepo;
use super::resolve::{known_for_prompt, merge_pair, save_scene_found, scene_cast};
use super::run::{Ask, BatchRun, RunAsk, Sent};
use super::same_entry::{self, SameEntryPrompt};
use super::service::ExcludeOwner;
use super::describe_prompt::{
    CharacterAnswer, CharacterPrompt, CondensePrompt, EntryKind, EntryPartPrompt, NotesAbout, PartPrompt, ScenePrompt, TermPrompt, TitlePrompt,
    clean_note, is_useful,
};
use super::entries::{character_names, character_scenes, entry_parts, scene_entries, term_names, term_scenes};
use super::parts::{MAX_PART_TEXT, Part, pick_parts, scene_hash, script_line, split_lines};
use super::repo::{
    AnalyzedPartRepo, AskKind, AskRepo, LineSampleRepo, NoteRepo, PairCheckRepo, ReadingDepthRepo, SceneDescriptionRepo, StoryLine, TermRepo,
};
use super::{Note, NoteOwner, SceneDescription};
use crate::common::Timestamp;
use crate::storage::DbError;
use crate::translation::TranslationRepo;
use super::suggest::spread;
use super::{CharacterId, CharacterRepo, GlossaryData, TermId};
use crate::common::ValidationError;
use crate::translation::Hints;
use crate::common::AppError;
use crate::engines::ContainerKey;
use crate::llm::{Action, ChatRequest, LlmTarget, LlmTargetService, ProfileId, RunHost};
use crate::projects::ProjectId;
use crate::state::{AppState, OpenProject};
use crate::translation::hints::GlossaryContext;

/// Что описывается общим путём: части, заметки по частям, описание по заметкам.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeTarget {
    Character(CharacterId),
    Term(TermId),
    /// Сцена: части это её реплики, заметка единственной части сразу становится описанием.
    Scene(ContainerKey),
}

impl From<ExcludeOwner> for DescribeTarget {
    fn from(owner: ExcludeOwner) -> Self {
        match owner {
            ExcludeOwner::Character(id) => Self::Character(id),
            ExcludeOwner::Term(id) => Self::Term(id),
        }
    }
}

/// Глубина чтения в массовом обновлении: минимум и максимум частей и желаемый процент.
/// Одна на приложение (`reading_depth` в app.db), без неё действует `default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadingDepth {
    min_parts: usize,
    /// `None`: без предела.
    max_parts: Option<usize>,
    percent: u8,
}

impl Default for ReadingDepth {
    fn default() -> Self {
        Self { min_parts: 5, max_parts: Some(50), percent: 60 }
    }
}

impl ReadingDepth {
    /// Минимум не меньше одной части, максимум (если есть) не меньше минимума, процент от 1 до 100.
    pub fn new(min_parts: usize, max_parts: Option<usize>, percent: u8) -> Result<Self, ValidationError> {
        if min_parts == 0 {
            return Err(ValidationError::OutOfRange("минимум частей"));
        }
        if max_parts.is_some_and(|max| max < min_parts) {
            return Err(ValidationError::OutOfRange("максимум частей меньше минимума"));
        }
        if !(1..=100).contains(&percent) {
            return Err(ValidationError::OutOfRange("процент чтения"));
        }
        Ok(Self { min_parts, max_parts, percent })
    }

    pub fn min_parts(&self) -> usize {
        self.min_parts
    }

    pub fn max_parts(&self) -> Option<usize> {
        self.max_parts
    }

    pub fn percent(&self) -> u8 {
        self.percent
    }

    /// Процент от всех частей округляется вверх и ограничивается минимумом и максимумом, если он есть.
    /// У кого частей меньше минимума, тот читается целиком (`partsFor` в окне).
    pub fn parts_for(&self, total: usize) -> usize {
        if total <= self.min_parts {
            return total;
        }
        let wanted = (total * usize::from(self.percent)).div_ceil(100).max(self.min_parts);
        let capped = match self.max_parts {
            Some(max) => wanted.min(max.max(self.min_parts)),
            None => wanted,
        };
        capped.min(total)
    }

    /// Глубина приложения: без строки в app.db значение по умолчанию.
    pub fn load(state: &AppState) -> Result<Self, AppError> {
        Ok(state.app.read(|tx| ReadingDepthRepo { conn: tx }.get())?)
    }
}

/// Сколько раз сжимаются заметки, которые не помещаются в запрос сборки.
const CONDENSE_ROUNDS: usize = 3;

/// Итог запуска описаний: описанные записи, добавленные разбором сцен имена и названия и ошибка,
/// которая оборвала запуск.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    pub written: Vec<DescribeTarget>,
    pub added: Vec<String>,
    pub error: Option<String>,
}

/// Сколько работы у запуска: записи и примерное число запросов.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pending {
    pub items: usize,
    pub requests: usize,
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
    /// «Сцена «Утро»»: начало строки хода запуска. Без названия ключ сцены.
    label: String,
    /// Отпечатки частей сцены, которые уже прочитал разбор.
    analyzed: HashSet<String>,
    /// Реплики сцены, как они стоят в частях: по ним разбор задаёт вопросы о спорных упоминаниях.
    lines: Vec<PartLine>,
}

impl ScenePlan {
    /// Запросы плана без сжатия: непрочитанные части, сборка нескольких заметок, название.
    fn requests(&self) -> usize {
        let unread = self.parts.iter().filter(|part| !self.stored.contains_key(&part.hash)).count();
        unread + usize::from(self.parts.len() > 1) + usize::from(self.need_title)
    }

    /// Выбранные части, которые разбор ещё не читал.
    fn unanalyzed(&self) -> Vec<&Part> {
        self.parts.iter().filter(|part| !self.analyzed.contains(&part.hash)).collect()
    }
}

/// Персонаж или термин, готовый к запуску: выбранные части его сцен и прочитанные заметки этой модели.
struct EntryPlan {
    owner: ExcludeOwner,
    name: String,
    parts: Vec<Part>,
    stored: HashMap<String, Note>,
    /// Текст выбранных частей: по нему берутся подсказки сборки.
    text: String,
    /// `Empty`: описание и манера речи пишутся только в пустое поле.
    fill: Fill,
}

impl EntryPlan {
    fn kind(&self) -> EntryKind {
        match self.owner {
            ExcludeOwner::Character(_) => EntryKind::Character,
            ExcludeOwner::Term(_) => EntryKind::Term,
        }
    }

    /// О чём заметки плана: для запроса сжатия.
    fn about(&self) -> NotesAbout<'_> {
        match self.owner {
            ExcludeOwner::Character(_) => NotesAbout::Character(&self.name),
            ExcludeOwner::Term(_) => NotesAbout::Term(&self.name),
        }
    }

    /// Непрочитанные части и сборка. У персонажа сборка всегда: из неё берётся манера речи.
    fn requests(&self) -> usize {
        let unread = self.parts.iter().filter(|part| !self.stored.contains_key(&part.hash)).count();
        let assembly = match self.owner {
            ExcludeOwner::Character(_) => true,
            ExcludeOwner::Term(_) => self.parts.len() > 1,
        };
        unread + usize::from(assembly)
    }

    /// «Персонаж «Lis»» или «Термин «Rusty Anchor»»: начало строки хода запуска.
    fn label(&self) -> String {
        match self.owner {
            ExcludeOwner::Character(_) => format!("Персонаж «{}»", self.name),
            ExcludeOwner::Term(_) => format!("Термин «{}»", self.name),
        }
    }

    /// Ключ отметки «спрошено» в `glossary_asks`: запись и выбранные части. С другими частями (новые сцены
    /// записи, другая глубина чтения) запись спросится снова.
    fn ask_key(&self) -> String {
        let parts = scene_hash(self.parts.iter().map(|part| part.hash.as_str()));
        match self.owner {
            ExcludeOwner::Character(id) => format!("character:{}:{parts}", id.get()),
            ExcludeOwner::Term(id) => format!("term:{}:{parts}", id.get()),
        }
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
            Self::Entry(plan) => plan.owner.into(),
        }
    }

    /// Персонаж или термин, о котором цепочка уже спрашивала с этими частями и не получила описания.
    fn asked(&self, asked: &HashSet<String>) -> bool {
        match self {
            Self::Scene(_) => false,
            Self::Entry(plan) => asked.contains(&plan.ask_key()),
        }
    }
}

/// Части сцен персонажа или термина для окна.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryParts {
    /// Сколько частей у записи.
    pub parts: usize,
    /// У скольких есть заметка любой модели.
    pub read: usize,
    /// Сколько прочитает следующий запуск.
    pub picked: usize,
    /// Сцены, которые модели не даются.
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
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        self.run(project, vec![target], Choice::Count(parts), Fill::Replace, false, one_off, host).await
    }

    /// Группа по очереди с глубиной чтения приложения. Заполненная запись без новых частей в запуск не входит.
    pub async fn describe_many(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        let depth = ReadingDepth::load(self.state)?;
        self.run(project, targets, Choice::Depth(depth), Fill::ReplaceUnlessRead, false, one_off, host).await
    }

    /// Кнопка записи в окне глоссария: персонаж или термин с глубиной чтения приложения. Описание заменяется
    /// и тогда, когда новых частей нет: оно собирается из готовых заметок.
    pub async fn describe_entry(
        &self,
        project: ProjectId,
        owner: ExcludeOwner,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        let depth = ReadingDepth::load(self.state)?;
        self.run(project, vec![owner.into()], Choice::Depth(depth), Fill::Replace, false, one_off, host).await
    }

    /// Группа цепочки автоперевода: как `describe_many`, но персонаж и термин, о которых цепочка уже спросила
    /// и не получила описания (`glossary_asks`), в запуск не входят, а запись, оставшаяся после запуска без
    /// описания, запоминается. То же с записями сцены перед её описанием. Описание и манера речи персонажа
    /// и описание термина пишутся только в пустое поле.
    pub async fn describe_unasked(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        let depth = ReadingDepth::load(self.state)?;
        self.run(project, targets, Choice::Depth(depth), Fill::ReplaceUnlessRead, true, one_off, host).await
    }

    /// Сколько работы у группы `describe_unasked`: записи, которые войдут в запуск, и запросы без сжатия,
    /// разбора сцен и описания записей сцены.
    pub fn pending(&self, project: ProjectId, targets: Vec<DescribeTarget>) -> Result<Pending, AppError> {
        let open = self.state.open_project(project)?;
        let model = self.notes_model(&open, Action::Descriptions);
        let choice = Choice::Depth(ReadingDepth::load(self.state)?);
        let plans = open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            unasked(tx, plan_all(tx, &mut context, &glossary, targets, &model, choice, Fill::ReplaceUnlessRead, Fill::Empty)?)
        })?;
        Ok(Pending { items: plans.len(), requests: plans.iter().map(Plan::requests).sum() })
    }

    /// Сколько работы у разбора `analyze`: сцены с непрочитанными разбором частями и число таких частей.
    pub fn pending_analysis(&self, project: ProjectId, keys: Vec<ContainerKey>) -> Result<Pending, AppError> {
        let open = self.state.open_project(project)?;
        let plans = self.analysis_plans(&open, keys)?;
        Ok(Pending { items: plans.len(), requests: plans.iter().map(|plan| plan.unanalyzed().len()).sum() })
    }

    /// Разбор сцен без описания: части, которые прочитало бы описание с глубиной чтения приложения, ищутся
    /// новые персонажи и термины и сразу пишутся в глоссарий. Прочитанные разбором части пропускаются.
    pub async fn analyze(
        &self,
        project: ProjectId,
        keys: Vec<ContainerKey>,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        let open = self.state.open_project(project)?;
        let target = LlmTargetService { state: self.state }.target(Some(&open), Action::Analysis, one_off)?;
        let plans = self.analysis_plans(&open, keys)?;
        if plans.is_empty() {
            return Ok(Described::default());
        }
        let (mut known, mut casings) = open.project.read(|tx| Ok((known_for_prompt(tx)?, game_casings(tx)?)))?;
        let total = plans.iter().map(|plan| plan.unanalyzed().len()).sum();
        let mut run = BatchRun::begin(self.state, &open, Action::Analysis, &target, host, total)?;
        let mut added = Vec::new();
        for plan in &plans {
            match analyze_scene(&open, &mut run, &target, plan, &mut known, &mut casings, added.len()).await {
                Ok((found, going)) => {
                    added.extend(found);
                    if !going {
                        break;
                    }
                }
                Err(error) => {
                    run.fail(&error);
                    break;
                }
            }
        }
        let result = run.finish(Vec::<()>::new())?;
        Ok(Described { written: Vec::new(), added, error: result.error })
    }

    /// Сцены с непрочитанными разбором частями: части те же, что выбрало бы описание группы.
    fn analysis_plans(&self, open: &Arc<OpenProject>, keys: Vec<ContainerKey>) -> Result<Vec<ScenePlan>, AppError> {
        let model = self.notes_model(open, Action::Descriptions);
        let choice = Choice::Depth(ReadingDepth::load(self.state)?);
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            let mut plans = Vec::new();
            for key in keys {
                if let Some(plan) = plan_scene(tx, &mut context, key, &model, choice, Fill::Replace, None)?
                    && !plan.unanalyzed().is_empty()
                {
                    plans.push(plan);
                }
            }
            Ok(plans)
        })?)
    }

    /// Модель заметок действия. Без настроенной модели пусто: выборка частей идёт без прочитанных заметок.
    fn notes_model(&self, open: &Arc<OpenProject>, action: Action) -> String {
        LlmTargetService { state: self.state }.target(Some(open), action, None).map(|target| target.profile.model().to_string()).unwrap_or_default()
    }

    /// Сбрасывает заметки владельца всех моделей: следующий запуск прочитает части заново.
    pub fn reset_notes(&self, project: ProjectId, target: &DescribeTarget) -> Result<usize, AppError> {
        let open = self.state.open_project(project)?;
        let owner = owner_of(target);
        Ok(open.project.write(|tx| NoteRepo { conn: tx }.delete_owner(&owner))?)
    }

    /// Части сцен персонажа или термина для окна: сколько их, сколько прочитано и сколько прочитает запуск.
    pub fn entry_parts(&self, project: ProjectId, owner: ExcludeOwner) -> Result<EntryParts, AppError> {
        let open = self.state.open_project(project)?;
        // Без настроенной модели выборка считается по заметкам любой модели.
        let model = self.notes_model(&open, Action::Descriptions);
        let depth = ReadingDepth::load(self.state)?;
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let context = GlossaryContext::new(&glossary);
            let lines = LineSampleRepo { conn: tx }.story_lines()?;
            let Some(selection) = select_entry(tx, &context, &glossary, &lines, owner, false)? else { return Ok(EntryParts::default()) };
            let notes = NoteRepo { conn: tx }.all_of(&owner_of(&owner.into()))?;
            let noted = |part: &Part, own: bool| notes.iter().any(|note| note.part_hash == part.hash && (!own || model.is_empty() || note.model == model));
            let ready: Vec<bool> = selection.parts.iter().map(|part| noted(part, true)).collect();
            Ok(EntryParts {
                parts: selection.parts.len(),
                read: selection.parts.iter().filter(|part| noted(part, false)).count(),
                picked: pick_parts(&ready, depth.parts_for(selection.parts.len())).len(),
                excluded: selection.excluded,
            })
        })?)
    }

    /// Части сцены с заметками всех моделей для окна: начало части и заметка, новая первой.
    pub fn scene_parts(&self, project: ProjectId, key: &ContainerKey) -> Result<Vec<PartNote>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            let parts = scene_text(tx, &mut context, key)?.map(|(parts, _, _)| parts).unwrap_or_default();
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

    /// `chain`: запуск цепочки автоперевода. Он не спрашивает о персонаже и термине, по которым уже не получил
    /// описания, запоминает новые такие записи и пишет описание записи только в пустое поле.
    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        project: ProjectId,
        targets: Vec<DescribeTarget>,
        choice: Choice,
        fill: Fill,
        chain: bool,
        one_off: Option<ProfileId>,
        host: impl Into<RunHost>,
    ) -> Result<Described, AppError> {
        let open = self.state.open_project(project)?;
        // Без записей модель не нужна: запуск проходит и там, где она не настроена.
        if targets.is_empty() {
            return Ok(Described::default());
        }
        let service = LlmTargetService { state: self.state };
        let target = service.target(Some(&open), Action::Descriptions, one_off)?;
        let model = target.profile.model().to_string();
        // Сцена сначала разбирается: новые персонажи и термины пишутся в глоссарий до её описания.
        let analysis = if targets.iter().any(|target| matches!(target, DescribeTarget::Scene(_))) {
            Some(service.target(Some(&open), Action::Analysis, one_off)?)
        } else {
            None
        };
        let depth = ReadingDepth::load(self.state)?;
        let (plans, hints, mut known, mut casings) = open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let mut context = GlossaryContext::new(&glossary);
            let entries = if chain { Fill::Empty } else { fill };
            let plans = plan_all(tx, &mut context, &glossary, targets, &model, choice, fill, entries)?;
            let plans = if chain { unasked(tx, plans)? } else { plans };
            let (known, casings) = if analysis.is_some() { (known_for_prompt(tx)?, game_casings(tx)?) } else { (Vec::new(), Casings::new([])) };
            Ok((plans, context.hints, known, casings))
        })?;
        let mut described = Described::default();
        if plans.is_empty() {
            return Ok(described);
        }
        let analyses: usize = match analysis {
            Some(_) => plans.iter().map(|plan| if let Plan::Scene(scene) = plan { scene.unanalyzed().len() } else { 0 }).sum(),
            None => 0,
        };
        let total = plans.iter().map(Plan::requests).sum::<usize>() + analyses;
        let mut run = BatchRun::begin(self.state, &open, Action::Descriptions, &target, host, total)?;
        let writer = SceneWriter { open: &open, hints: &hints, target: &target, model: &model, chain };
        for plan in plans {
            let done = described.written.len();
            let outcome = match &plan {
                Plan::Scene(scene) => {
                    let mut going = true;
                    if let Some(analysis) = &analysis {
                        match analyze_scene(&open, &mut run, analysis, scene, &mut known, &mut casings, done).await {
                            Ok((added, still)) => {
                                described.added.extend(added);
                                going = still;
                            }
                            Err(error) => {
                                run.fail(&error);
                                break;
                            }
                        }
                        if going {
                            match writer.scene_entries(&mut run, &scene.key, depth, done).await {
                                Ok((written, still)) => {
                                    described.written.extend(written);
                                    going = still;
                                }
                                Err(error) => {
                                    run.fail(&error);
                                    break;
                                }
                            }
                        }
                    }
                    if !going {
                        break;
                    }
                    writer.scene(&mut run, scene, done).await
                }
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
                    run.fail(&error);
                    break;
                }
            }
        }
        let result = run.finish(described.written)?;
        Ok(Described { written: result.items, added: described.added, error: result.error })
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
        let choice = Choice::Depth(ReadingDepth::load(state)?);
        let plans = open.project.read(|tx| {
            let first = SceneDescriptionRepo { conn: tx }.first_scene()?;
            let mut plans = Vec::new();
            for key in keys {
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
        let writer = SceneWriter { open, hints, target: &self.target, model: &self.model, chain: false };
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

/// Регистры слов всех реплик игры: по ним разбор решает, о каком упоминании спросить модель.
fn game_casings(tx: &rusqlite::Connection) -> Result<Casings, DbError> {
    Ok(Casings::new(LineSampleRepo { conn: tx }.story_lines()?.into_iter().map(|line| line.text)))
}

/// Разбор частей сцены моделью действия `analysis` и запись новых персонажей и терминов в глоссарий. Вместе
/// с поиском модель отвечает на вопросы о спорных упоминаниях известных записей, ответы пишутся по репликам.
/// Остановленный разбор записывает найденное до остановки. Возвращает добавленное и идёт ли запуск дальше.
#[allow(clippy::too_many_arguments)]
async fn analyze_scene(
    open: &Arc<OpenProject>,
    run: &mut BatchRun<'_>,
    target: &LlmTarget,
    plan: &ScenePlan,
    known: &mut Vec<String>,
    casings: &mut Casings,
    written: usize,
) -> Result<(Vec<String>, bool), AppError> {
    let mut found = Vec::new();
    let mut going = true;
    let parts = plan.unanalyzed();
    let total = parts.len();
    let mut read = Vec::new();
    let (cast, glossary) = open.project.read(|tx| Ok((scene_cast(tx, &plan.key)?, GlossaryData::load(tx)?)))?;
    let entries = Asked::all(&glossary);
    let mut answers: Vec<(ExcludeOwner, String, bool)> = Vec::new();
    let cast_ids: Vec<CharacterId> = cast.iter().map(|member| member.id).collect();
    let mut suggested: Vec<Pair> = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        run.note(format!("{}: поиск персонажей и терминов, часть {} из {total}", plan.label, index + 1));
        let mut asking = questions(&part.text, &plan.lines, &entries, casings);
        asking.retain(|question| !answers.iter().any(|(owner, hash, _)| *owner == question.owner && *hash == question.hash));
        match run.send_to(target, &AnalysisPrompt::build(&part.text, known, &cast, &asking, target)).await {
            Sent::Text(text) => {
                found.extend(AnalysisPrompt::accept(&text, &part.text));
                for (number, used) in AnalysisPrompt::checks(&text, asking.len()) {
                    if let Some(question) = asking.get(number) {
                        answers.push((question.owner, question.hash.clone(), used));
                    }
                }
                for pair in AnalysisPrompt::same_people(&text, &cast_ids).into_iter().filter_map(|(a, b)| Pair::people(a, b)) {
                    if !suggested.contains(&pair) {
                        suggested.push(pair);
                    }
                }
            }
            Sent::Skipped => {}
            Sent::Stop => {
                going = false;
                break;
            }
        }
        read.push(part.hash.as_str());
        if !run.step(written) {
            going = false;
            break;
        }
    }
    let merged = merge(found);
    // Находки и отметки прочитанных частей одной транзакцией: повторный разбор не пропустит часть без её находок.
    let saved = open.project.write(|tx| {
        let marks = AnalyzedPartRepo { conn: tx };
        let now = Timestamp::now();
        for hash in &read {
            marks.mark(&plan.key, hash, now)?;
        }
        let checks = MentionCheckRepo { conn: tx };
        for (owner, hash, used) in &answers {
            checks.mark(*owner, hash, *used, now)?;
        }
        save_scene_found(tx, &merged, Some(&plan.key))
    })?;
    let mut added = saved.added;
    if !added.is_empty() {
        run.note(format!("{}: добавлено в глоссарий: {}", plan.label, added.join(", ")));
    }
    if going {
        // Сначала пары, о которых модель сказала при записи находок, потом её `same_person`.
        let mut pairs: Vec<Pair> = saved.claims.iter().map(|claim| claim.pair).collect();
        for pair in suggested {
            if !pairs.contains(&pair) {
                pairs.push(pair);
            }
        }
        let (joined, still) = check_pairs(open, run, target, plan, &pairs, written).await?;
        added.extend(joined);
        going = still;
    }
    known.extend(added.iter().cloned());
    Ok((added, going))
}

/// Вопросы модели, одно ли это пары записей (`same_entry`): пары от разбора и пары, которые видит скрипт. Ответ
/// «да» сливает пару, «нет» запоминается. Число запросов запуска растёт на число вопросов. Возвращает итог
/// слияний и идёт ли запуск дальше.
async fn check_pairs(
    open: &Arc<OpenProject>,
    run: &mut BatchRun<'_>,
    target: &LlmTarget,
    plan: &ScenePlan,
    claimed: &[Pair],
    written: usize,
) -> Result<(Vec<String>, bool), AppError> {
    let questions = open.project.read(|tx| same_entry::questions(tx, &plan.key, claimed))?;
    if questions.is_empty() {
        return Ok((Vec::new(), true));
    }
    run.grow(questions.len());
    let mut joined = Vec::new();
    for question in &questions {
        let Some(pair) = question.pair() else { continue };
        let ask = if question.a.is_person() && question.b.is_person() { "один ли человек" } else { "одно ли это" };
        run.note(format!("{}: {ask} {} и {}", plan.label, question.a.label(), question.b.label()));
        match run.send_to(target, &SameEntryPrompt::build(question, target)).await {
            Sent::Text(text) => {
                let merged = match SameEntryPrompt::accept(&text) {
                    Some(true) => open.project.write(|tx| merge_pair(tx, pair))?,
                    Some(false) => {
                        open.project.write(|tx| PairCheckRepo { conn: tx }.mark(pair.a, pair.b, false, Timestamp::now()))?;
                        None
                    }
                    None => None,
                };
                if let Some(text) = merged {
                    run.note(format!("{}: {text}", plan.label));
                    joined.push(text);
                }
            }
            Sent::Skipped => {}
            Sent::Stop => return Ok((joined, false)),
        }
        if !run.step(written) {
            return Ok((joined, false));
        }
    }
    Ok((joined, true))
}

/// Планы записей запуска описаний по порядку. Пропущенные записи (нечего читать, заполнены) в план не входят.
/// `fill`: как писать описание сцены, `entries`: персонажа и термина.
#[allow(clippy::too_many_arguments)]
fn plan_all(
    tx: &rusqlite::Connection,
    context: &mut GlossaryContext,
    glossary: &GlossaryData,
    targets: Vec<DescribeTarget>,
    model: &str,
    choice: Choice,
    fill: Fill,
    entries: Fill,
) -> Result<Vec<Plan>, DbError> {
    let first = SceneDescriptionRepo { conn: tx }.first_scene()?;
    let lines = if targets.iter().all(|target| matches!(target, DescribeTarget::Scene(_))) {
        Vec::new()
    } else {
        LineSampleRepo { conn: tx }.story_lines()?
    };
    let mut plans = Vec::new();
    for target in targets {
        let owner = match target {
            DescribeTarget::Scene(key) => {
                plans.extend(plan_scene(tx, context, key, model, choice, fill, first.as_ref())?.map(Plan::Scene));
                continue;
            }
            DescribeTarget::Character(id) => ExcludeOwner::Character(id),
            DescribeTarget::Term(id) => ExcludeOwner::Term(id),
        };
        plans.extend(plan_entry(tx, context, glossary, &lines, owner, model, choice, entries)?.map(Plan::Entry));
    }
    Ok(plans)
}

/// Планы без персонажей и терминов, о которых цепочка уже спрашивала и не получила описания (`glossary_asks`).
fn unasked(tx: &rusqlite::Connection, mut plans: Vec<Plan>) -> Result<Vec<Plan>, DbError> {
    let asked = AskRepo { conn: tx }.asked(AskKind::Description)?;
    plans.retain(|plan| !plan.asked(&asked));
    Ok(plans)
}

/// Персонажи с именем и термины без описания: работа шага `entries` цепочки автоперевода. Записи, о которых
/// цепочка уже спрашивала, отсеивают `pending` и `describe_unasked`: отметка зависит от выбранных частей.
pub(crate) fn entries_work(glossary: &GlossaryData) -> Vec<DescribeTarget> {
    let characters = glossary
        .characters
        .iter()
        .filter(|character| character.description().is_none() && character.full_name().is_some())
        .filter_map(|character| Some(DescribeTarget::Character(character.id()?)));
    let terms = glossary.terms.iter().filter(|term| term.description().is_none()).filter_map(|term| Some(DescribeTarget::Term(term.id()?)));
    characters.chain(terms).collect()
}

fn owner_of(target: &DescribeTarget) -> NoteOwner {
    match target {
        DescribeTarget::Character(id) => NoteOwner::Character(*id),
        DescribeTarget::Term(id) => NoteOwner::Term(*id),
        DescribeTarget::Scene(key) => NoteOwner::Container(key.clone()),
    }
}

/// Части сцены, отпечаток её текста и реплики, как они стоят в частях.
type SceneText = (Vec<Part>, String, Vec<PartLine>);

/// Реплики сцены строками «Говорящий: текст» с подписями из глоссария и значениями токенов, разрезанные на части,
/// отпечаток текста сцены и сами реплики. `None`: сцены нет в текущем снимке.
fn scene_text(tx: &rusqlite::Connection, context: &mut GlossaryContext, key: &ContainerKey) -> Result<Option<SceneText>, DbError> {
    let found = TranslationRepo { conn: tx }.scene(key);
    let mut scene = match found {
        Ok(scene) => scene,
        Err(DbError::NotFound) => return Ok(None),
        Err(error) => return Err(error),
    };
    let text_hash = scene_hash(scene.lines.iter().map(|line| line.source_hash.as_str()));
    context.apply(&mut scene);
    let lines: Vec<PartLine> = scene
        .lines
        .iter()
        .map(|line| PartLine {
            script: script_line(line.speaker.as_deref(), &line.source_text),
            text: line.source_text.replace('\n', " "),
            hash: line.source_hash.clone(),
        })
        .collect();
    let scripts: Vec<String> = lines.iter().map(|line| line.script.clone()).collect();
    Ok(Some((split_lines(&scripts), text_hash, lines)))
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
    let Some((all, text_hash, lines)) = scene_text(tx, context, &key)? else { return Ok(None) };
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
    let name = description.as_ref().and_then(|scene| scene.title()).map_or_else(|| key.to_string(), str::to_string);
    let label = format!("Сцена «{name}»");
    let analyzed = AnalyzedPartRepo { conn: tx }.of(&key)?;
    Ok(Some(ScenePlan { key, parts, stored, text, text_hash, need_title, opening, fill, label, analyzed, lines }))
}

/// Сцены записи без исключённых и части из них.
struct Selection {
    name: String,
    parts: Vec<Part>,
    excluded: Vec<ContainerKey>,
    /// Описание, которое уже есть.
    described: bool,
}

/// Сцены и части персонажа или термина. `ignore_exclusions`: части по всем сценам, как будто ничего не исключено.
/// `None`: записи нет или у персонажа нет имени, по которому его искать.
fn select_entry(
    tx: &rusqlite::Connection,
    context: &GlossaryContext,
    glossary: &GlossaryData,
    lines: &[StoryLine],
    owner: ExcludeOwner,
    ignore_exclusions: bool,
) -> Result<Option<Selection>, DbError> {
    Ok(match owner {
        ExcludeOwner::Character(id) => {
            let Some(character) = glossary.characters.iter().find(|character| character.id() == Some(id)) else { return Ok(None) };
            let Some(name) = character.full_name() else { return Ok(None) };
            let excluded = if ignore_exclusions { Vec::new() } else { CharacterRepo { conn: tx }.excluded_containers(id)? };
            let speakers = glossary.links.iter().filter(|link| link.character == id).map(|link| link.speaker.clone()).collect();
            let scenes = character_scenes(lines, &speakers, &character_names(character), glossary.character_checks.get(&id), &excluded);
            let parts = entry_parts(lines, &scenes, context);
            Some(Selection { name, parts, excluded, described: character.description().is_some() })
        }
        ExcludeOwner::Term(id) => {
            let Some(term) = glossary.terms.iter().find(|term| term.id() == Some(id)) else { return Ok(None) };
            let excluded = if ignore_exclusions { Vec::new() } else { TermRepo { conn: tx }.excluded_containers(id)? };
            let scenes = term_scenes(lines, &term_names(term), glossary.term_checks.get(&id), &excluded);
            let parts = entry_parts(lines, &scenes, context);
            Some(Selection { name: term.name().to_string(), parts, excluded, described: term.description().is_some() })
        }
    })
}

/// После повторного считывания удаляет заметки, чьих частей больше нет у владельца. Части сцены берутся из её
/// реплик, у пропавшей сцены их нет. Части персонажа и термина собираются из нескольких сцен и зависят от
/// исключений: заметка остаётся, если её часть есть среди частей без исключений или при нынешних исключениях.
/// Возвращает число удалённых заметок.
pub(crate) fn prune_notes(tx: &rusqlite::Connection) -> Result<usize, DbError> {
    let notes = NoteRepo { conn: tx };
    let owners = notes.owners()?;
    if owners.is_empty() && (AnalyzedPartRepo { conn: tx }).scenes()?.is_empty() {
        return Ok(0);
    }
    let glossary = GlossaryData::load(tx)?;
    let mut context = GlossaryContext::new(&glossary);
    let lines = LineSampleRepo { conn: tx }.story_lines()?;
    let mut removed = 0;
    for owner in owners {
        let hashes: Vec<String> = match &owner {
            NoteOwner::Container(key) => {
                scene_text(tx, &mut context, key)?.map(|(parts, _, _)| parts.into_iter().map(|part| part.hash).collect()).unwrap_or_default()
            }
            NoteOwner::Character(id) => entry_hashes(tx, &context, &glossary, &lines, ExcludeOwner::Character(*id))?,
            NoteOwner::Term(id) => entry_hashes(tx, &context, &glossary, &lines, ExcludeOwner::Term(*id))?,
        };
        removed += notes.retain_parts(&owner, &hashes)?;
    }
    // Отметки разбора уходят вместе с частями: изменённая часть сцены разбирается заново.
    let marks = AnalyzedPartRepo { conn: tx };
    for key in marks.scenes()? {
        let hashes: Vec<String> =
            scene_text(tx, &mut context, &key)?.map(|(parts, _, _)| parts.into_iter().map(|part| part.hash).collect()).unwrap_or_default();
        marks.retain(&key, &hashes)?;
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
    owner: ExcludeOwner,
) -> Result<Vec<String>, DbError> {
    let mut hashes = Vec::new();
    for ignore_exclusions in [true, false] {
        if let Some(selection) = select_entry(tx, context, glossary, lines, owner, ignore_exclusions)? {
            hashes.extend(selection.parts.into_iter().map(|part| part.hash));
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
    owner: ExcludeOwner,
    model: &str,
    choice: Choice,
    fill: Fill,
) -> Result<Option<EntryPlan>, DbError> {
    let Some(selection) = select_entry(tx, context, glossary, lines, owner, false)? else { return Ok(None) };
    if selection.parts.is_empty() {
        return Ok(None);
    }
    let stored: HashMap<String, Note> =
        NoteRepo { conn: tx }.of(&owner_of(&owner.into()), model)?.into_iter().map(|note| (note.part_hash.clone(), note)).collect();
    let ready: Vec<bool> = selection.parts.iter().map(|part| stored.contains_key(&part.hash)).collect();
    let count = match choice {
        Choice::Count(count) => count,
        Choice::Depth(depth) => depth.parts_for(selection.parts.len()),
    };
    let parts: Vec<Part> = pick_parts(&ready, count).into_iter().map(|index| selection.parts[index].clone()).collect();
    let skip = match fill {
        Fill::Replace => false,
        Fill::ReplaceUnlessRead => selection.described && parts.iter().all(|part| stored.contains_key(&part.hash)),
        Fill::Empty => selection.described,
    };
    if skip {
        return Ok(None);
    }
    let text = parts.iter().map(|part| part.text.as_str()).collect();
    Ok(Some(EntryPlan { owner, name: selection.name, parts, stored, text, fill }))
}

/// Пишет ответ модели в запись. `Fill::Empty`: описание и манера речи только в пустое поле, заполненное за время
/// запуска остаётся как есть. Род пишется только персонажу без рода. `false`: записи уже нет (удалена или слита
/// за время запуска) или её описание заполнено.
fn write_entry(tx: &rusqlite::Connection, owner: ExcludeOwner, fill: Fill, answer: &CharacterAnswer) -> Result<bool, DbError> {
    let empty_only = fill == Fill::Empty;
    match owner {
        ExcludeOwner::Character(id) => {
            let repo = CharacterRepo { conn: tx };
            let mut character = match repo.get(id) {
                Ok(character) => character,
                Err(DbError::NotFound) => return Ok(false),
                Err(error) => return Err(error),
            };
            if empty_only && character.description().is_some() {
                return Ok(false);
            }
            character.set_description(&answer.description);
            if let Some(speech) = &answer.speech
                && !(empty_only && character.speech().is_some())
            {
                character.set_speech(speech);
            }
            if character.gender().is_none() {
                character.set_gender(answer.gender);
            }
            repo.save(&character)?;
        }
        ExcludeOwner::Term(id) => {
            let repo = TermRepo { conn: tx };
            let mut term = match repo.get(id) {
                Ok(term) => term,
                Err(DbError::NotFound) => return Ok(false),
                Err(error) => return Err(error),
            };
            if empty_only && term.description().is_some() {
                return Ok(false);
            }
            term.set_description(&answer.description);
            repo.save(&term)?;
        }
    }
    Ok(true)
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
    /// Запуск цепочки автоперевода: записи без описания запоминаются в `glossary_asks` и снова не спрашиваются.
    chain: bool,
}

impl SceneWriter<'_> {
    /// Цепочка запоминает персонажа или термин, о котором спросила и не получила описания.
    fn unanswered(&self, plan: &EntryPlan) -> Result<(), AppError> {
        if self.chain {
            self.open.project.write(|tx| AskRepo { conn: tx }.mark(AskKind::Description, &plan.ask_key(), Timestamp::now()))?;
        }
        Ok(())
    }

    /// Заметки выбранных частей: готовые из базы, новые спрашиваются и пишутся сразу. Пустые и отказы в итог
    /// не входят. `None`: запуск остановлен.
    #[allow(clippy::too_many_arguments)]
    async fn notes(
        &self,
        run: &mut impl Ask,
        label: &str,
        owner: &NoteOwner,
        parts: &[Part],
        stored: &HashMap<String, Note>,
        written: usize,
        request: impl Fn(&Part) -> ChatRequest,
    ) -> Result<Option<Vec<String>>, AppError> {
        let mut notes = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let note = match stored.get(&part.hash) {
                Some(note) => note.text.clone(),
                None => {
                    run.note(format!("{label}: заметка к части {} из {}", index + 1, parts.len()));
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

    /// Персонажи и термины сцены без описания описываются с глубиной чтения приложения: так новые записи разбора
    /// и старые пустые получают описание до описания сцены. Возвращает описанные и идёт ли запуск дальше.
    async fn scene_entries(
        &self,
        run: &mut BatchRun<'_>,
        key: &ContainerKey,
        depth: ReadingDepth,
        written: usize,
    ) -> Result<(Vec<DescribeTarget>, bool), AppError> {
        let choice = Choice::Depth(depth);
        let plans = self.open.project.read(|tx| {
            let glossary = GlossaryData::load(tx)?;
            let context = GlossaryContext::new(&glossary);
            let samples = LineSampleRepo { conn: tx };
            let (characters, terms) = scene_entries(&glossary, &samples.scene_lines(key)?);
            if characters.is_empty() && terms.is_empty() {
                return Ok(Vec::new());
            }
            let lines = samples.story_lines()?;
            let owners = characters.into_iter().map(ExcludeOwner::Character).chain(terms.into_iter().map(ExcludeOwner::Term));
            let mut plans = Vec::new();
            for owner in owners {
                plans.extend(plan_entry(tx, &context, &glossary, &lines, owner, self.model, choice, Fill::Empty)?);
            }
            if self.chain {
                let asked = AskRepo { conn: tx }.asked(AskKind::Description)?;
                plans.retain(|plan| !asked.contains(&plan.ask_key()));
            }
            Ok(plans)
        })?;
        run.grow(plans.iter().map(EntryPlan::requests).sum());
        let mut done = Vec::new();
        for plan in plans {
            let outcome = self.entry(run, &plan, written).await?;
            if outcome.description.is_some() {
                done.push(plan.owner.into());
            }
            if !outcome.going {
                return Ok((done, false));
            }
        }
        Ok((done, true))
    }

    /// Описание персонажа или термина по заметкам частей. Единственная полезная заметка о термине сразу
    /// становится описанием, у персонажа сборка идёт всегда: из неё берутся манера речи и род.
    async fn entry(&self, run: &mut impl Ask, plan: &EntryPlan, written: usize) -> Result<Outcome, AppError> {
        let owner = owner_of(&plan.owner.into());
        let label = plan.label();
        let kind = plan.kind();
        let request = |part: &Part| EntryPartPrompt::build(kind, &plan.name, &part.text, self.hints, self.target);
        let Some(notes) = self.notes(run, &label, &owner, &plan.parts, &plan.stored, written, request).await? else {
            return Ok(Outcome::STOPPED);
        };
        if notes.is_empty() {
            self.unanswered(plan)?;
            return Ok(Outcome::NOTHING);
        }
        let mut going = true;
        let answer = if let (EntryKind::Term, [note]) = (kind, notes.as_slice()) {
            Some(CharacterAnswer { description: note.clone(), ..CharacterAnswer::default() })
        } else {
            let Some(notes) = self.condense(run, &label, plan.about(), notes, written).await else { return Ok(Outcome::STOPPED) };
            run.note(format!("{label}: сборка описания"));
            let request = match kind {
                EntryKind::Character => CharacterPrompt::build(&plan.name, &notes, &plan.text, self.hints, self.target),
                EntryKind::Term => TermPrompt::build(&plan.name, &notes, &plan.text, self.hints, self.target),
            };
            let text = match run.send(&request).await {
                Sent::Text(text) => text,
                Sent::Skipped => String::new(),
                Sent::Stop => return Ok(Outcome::STOPPED),
            };
            going = run.step(written);
            match kind {
                EntryKind::Character => CharacterPrompt::accept(&text),
                EntryKind::Term => Some(clean_note(&text)).filter(|text| !text.is_empty()).map(|description| CharacterAnswer { description, ..CharacterAnswer::default() }),
            }
        };
        let Some(answer) = answer else {
            self.unanswered(plan)?;
            return Ok(Outcome { description: None, going });
        };
        let saved = self.open.project.write(|tx| write_entry(tx, plan.owner, plan.fill, &answer))?;
        Ok(Outcome { description: saved.then_some(answer.description), going })
    }

    /// Возвращает записанное описание и идёт ли запуск дальше.
    async fn scene(&self, run: &mut impl Ask, plan: &ScenePlan, written: usize) -> Result<Outcome, AppError> {
        let owner = NoteOwner::Container(plan.key.clone());
        let request = |part: &Part| PartPrompt::build(&part.text, self.hints, self.target);
        let Some(notes) = self.notes(run, &plan.label, &owner, &plan.parts, &plan.stored, written, request).await? else {
            return Ok(Outcome::STOPPED);
        };
        if notes.is_empty() {
            return Ok(Outcome::NOTHING);
        }

        let mut going = true;
        let description = if let ([_], [note]) = (plan.parts.as_slice(), notes.as_slice()) {
            note.clone()
        } else {
            let Some(notes) = self.condense(run, &plan.label, NotesAbout::Scene, notes, written).await else { return Ok(Outcome::STOPPED) };
            run.note(format!("{}: сборка описания", plan.label));
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

        run.note(format!("{}: название", plan.label));
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
    async fn condense(&self, run: &mut impl Ask, label: &str, about: NotesAbout<'_>, mut notes: Vec<String>, written: usize) -> Option<Vec<String>> {
        for _ in 0..CONDENSE_ROUNDS {
            if CondensePrompt::fits(&notes) {
                break;
            }
            let groups = CondensePrompt::groups(notes);
            run.grow(groups.len());
            let mut condensed = Vec::new();
            let count = groups.len();
            for (index, group) in groups.into_iter().enumerate() {
                run.note(format!("{label}: сжатие заметок, группа {} из {count}", index + 1));
                match run.send(&CondensePrompt::build(&group, about, self.target)).await {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tauri::ipc::Channel;
    use super::super::analyze::{Finding, FoundKind, NameParts};
    use super::super::relations;
    use super::super::resolve::{Claim, save_found};
    use crate::glossary::{Character, FormKind, GlossaryService, SpeakerRepo, Term, TermCategory};

    use crate::llm::client::tests::{completion, fake_model};
    use crate::engines::SpeakerKey;
    use crate::llm::{LlmSettingsService, Profile, Provider, ProviderKind, RunProgress};
    use crate::projects::ProjectService;
    use crate::state::tests::memory_secrets;

    /// Открытый проект по синтетической игре с моделью на `url`.
    pub(crate) fn project(dir: &std::path::Path, url: &str) -> (AppState, ProjectId) {
        project_with(dir, url, &[])
    }

    /// То же с дополнительными скриптами игры: имя файла в папке `game` и его текст.
    pub(crate) fn project_with(dir: &std::path::Path, url: &str, scripts: &[(&str, &str)]) -> (AppState, ProjectId) {
        let root = crate::engines::renpy::snapshot::tests::sample_game(&dir.join("game"));
        for (name, text) in scripts {
            std::fs::write(dir.join("game").join("game").join(name), text).unwrap();
        }
        project_of(dir, url, root)
    }

    /// Открытый проект по игре `root` с моделью на `url`. Данные приложения лежат в `dir`.
    pub(crate) fn project_of(dir: &std::path::Path, url: &str, root: crate::projects::GameRoot) -> (AppState, ProjectId) {
        let state = AppState::start(dir.join("data"), memory_secrets()).unwrap();
        let service = ProjectService { state: &state };
        let id = service.create(root).unwrap().id();
        service.open(id).unwrap();
        let settings = LlmSettingsService { state: &state };
        let provider = Provider::new("LM Studio", ProviderKind::Local, url, 0).unwrap();
        settings.save_provider(&provider, None).unwrap();
        settings.save_profile(&Profile::new(provider.id(), "", "gemma", 0).unwrap()).unwrap();
        (state, id)
    }

    pub(crate) fn user_text(body: &serde_json::Value) -> String {
        body["messages"][1]["content"].as_str().unwrap_or_default().to_string()
    }

    pub(crate) fn silent() -> Channel<RunProgress> {
        Channel::new(|_| Ok(()))
    }

    pub(crate) fn system_text(body: &serde_json::Value) -> String {
        body["messages"][0]["content"].as_str().unwrap_or_default().to_string()
    }

    /// Канал, который запоминает строки хода запуска без повторов подряд.
    fn recorded() -> (Channel<RunProgress>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let notes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = notes.clone();
        let channel = Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(json) = body
                && let Ok(progress) = serde_json::from_str::<serde_json::Value>(&json)
                && let Some(message) = progress["message"].as_str()
            {
                let mut notes = sink.lock().unwrap();
                if notes.last().map(String::as_str) != Some(message) {
                    notes.push(message.to_string());
                }
            }
            Ok(())
        });
        (channel, notes)
    }

    #[test]
    fn scenes_are_described_from_part_notes_and_named_once() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            let answer = if system.contains("proper names") {
                "[]"
            } else if system.contains("name a scene") {
                "«Знакомство с Эйлин»."
            } else {
                "**Эйлин** здоровается с героем."
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let start = ContainerKey::new("label:start").unwrap();
        let left = ContainerKey::new("label:left").unwrap();

        let described =
            tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(start.clone()), usize::MAX, None, silent())).unwrap();
        assert_eq!(described, Described { written: vec![DescribeTarget::Scene(start.clone())], added: Vec::new(), error: None });
        {
            let requests = seen.lock().unwrap();
            assert_eq!(requests.len(), 3, "разбор, одна часть сразу становится описанием, потом название");
            assert!(system_text(&requests[0]).contains("proper names"), "сначала разбор");
            let part = user_text(&requests[1]);
            assert!(part.contains("Script:\n\nEileen: Hello.\nNarration.\nRadio Chatter: Static.\n"), "{part}");
            assert!(system_text(&requests[2]).ends_with("This is the first scene of the game."));
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
            None,
            silent(),
        ))
        .unwrap();
        assert_eq!(described.written, [DescribeTarget::Scene(left.clone())], "заполненная сцена без новых частей пропущена");
        assert_eq!(seen.lock().unwrap().len(), 6, "пропущенная сцена не разбирается");

        tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(start.clone()), usize::MAX, None, silent())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 6, "заметка готова, название есть, часть разобрана: запросов нет");

        let parts = service.scene_parts(id, &start).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].note.as_deref(), Some("Эйлин здоровается с героем."));
        assert!(parts[0].useful);
        assert_eq!(service.reset_notes(id, &DescribeTarget::Scene(start.clone())).unwrap(), 1);
        assert_eq!(service.scene_parts(id, &start).unwrap()[0].note, None);
    }

    #[test]
    fn scene_analysis_adds_new_entries_and_describes_them_before_the_scene() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            let answer = if system.contains("proper names") {
                r#"[{"kind": "location", "name": "Left side"}, {"kind": "character", "name": "Eileen", "gender": "female"}, {"kind": "character", "name": "Ghost"}]"#
            } else if system.contains("notes about the term") {
                "Сторона дороги."
            } else if system.contains("name a scene") {
                "Налево"
            } else {
                "Эйлин показывает дорогу."
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let left = ContainerKey::new("label:left").unwrap();
        GlossaryService { state: &state }.keep_as_speaker(id, &SpeakerKey::new("code:e").unwrap()).unwrap();

        let (channel, notes) = recorded();
        let described = tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(left.clone()), usize::MAX, None, channel)).unwrap();
        assert_eq!(described.added, ["Left side"], "подпись разобранного говорящего и выдуманное имя не добавлены");
        assert_eq!(
            *notes.lock().unwrap(),
            [
                "Сцена «label:left»: поиск персонажей и терминов, часть 1 из 1",
                "Сцена «label:left»: добавлено в глоссарий: Left side",
                "Термин «Left side»: заметка к части 1 из 1",
                "Сцена «label:left»: заметка к части 1 из 1",
                "Сцена «label:left»: название",
            ],
            "ход запуска говорит, что делает модель"
        );
        assert_eq!(described.error, None);
        let open = state.open_project(id).unwrap();
        let terms = open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap();
        assert_eq!(terms.len(), 1);
        assert_eq!((terms[0].category(), terms[0].description()), (TermCategory::Location, Some("Сторона дороги.")));
        let term = terms[0].id().unwrap();
        assert_eq!(described.written, [DescribeTarget::Term(term), DescribeTarget::Scene(left.clone())], "сначала термин, потом сцена");
        {
            let requests = seen.lock().unwrap();
            let systems: Vec<String> = requests.iter().map(system_text).collect();
            assert_eq!(systems.len(), 4, "разбор, термин, часть сцены, название");
            assert!(systems[0].contains("proper names") && systems[1].contains("notes about the term"));
            assert_eq!(requests[0]["temperature"].as_f64(), Some(0.0), "разбор идёт с настройками своего действия");
        }

        let entries = GlossaryService { state: &state }.scene_entries(id, &left).unwrap();
        assert_eq!((entries.0.len(), entries.1.len()), (0, 1), "у сцены один термин, персонажей в глоссарии нет");
        tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(left.clone()), usize::MAX, None, silent())).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 4, "разобранная часть не разбирается снова, термин описан, заметка сцены готова");

        open.project.write(|tx| Ok(tx.execute("DELETE FROM analyzed_parts", [])?)).unwrap();
        tauri::async_runtime::block_on(service.describe(id, DescribeTarget::Scene(left.clone()), usize::MAX, None, silent())).unwrap();
        let requests = seen.lock().unwrap();
        assert!(user_text(&requests[4]).starts_with("Known names and terms: Left side, Eileen\n"), "{}", user_text(&requests[4]));
        assert_eq!(requests.len(), 5, "без отметки часть разбирается снова, описанный термин не описывается снова");
    }

    #[test]
    fn characters_and_terms_are_described_from_their_scenes() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            // Сборка тоже говорит о заметках, поэтому проверяется первой.
            let answer = if system.contains("about one character") {
                "```json\n{\"description\": \"Эйлин встречает героя.\", \"speech\": \"Говорит на «ты».\", \"gender\": \"female\"}\n```"
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

        let owner = ExcludeOwner::Character(eileen);
        let counts = |parts: &EntryParts| (parts.parts, parts.read, parts.picked);
        assert_eq!(counts(&service.entry_parts(id, owner).unwrap()), (1, 0, 1), "две маленькие сцены в одной части");

        let described =
            tauri::async_runtime::block_on(service.describe_many(id, vec![character.clone(), term.clone()], None, silent()))
                .unwrap();
        assert_eq!(described, Described { written: vec![character.clone(), term.clone()], added: Vec::new(), error: None });
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
        assert_eq!(person.gender(), Some(crate::common::Gender::Female), "пустой род заполнен по описанию");
        assert_eq!(item.description(), Some("Сторона, куда ведёт дорога."), "заметка единственной части сразу описание");

        assert_eq!(counts(&service.entry_parts(id, owner).unwrap()), (1, 1, 1));

        // Группа описанную запись без новых частей пропускает, кнопка записи собирает описание заново из заметки.
        let mut edited = person.clone();
        edited.set_description("Старое описание.");
        glossary.save_character(id, &edited).unwrap();
        let again = tauri::async_runtime::block_on(service.describe_many(id, vec![character.clone()], None, silent())).unwrap();
        assert!(again.written.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 3);
        let replaced = tauri::async_runtime::block_on(service.describe_entry(id, owner, None, silent())).unwrap();
        assert_eq!(replaced.written, std::slice::from_ref(&character));
        assert_eq!(seen.lock().unwrap().len(), 4, "только сборка");
        let person = open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        assert_eq!(person.description(), Some("Эйлин встречает героя."));

        let left = ContainerKey::new("label:left").unwrap();
        glossary.set_excluded_scene(id, owner, &left, true).unwrap();
        let parts = service.entry_parts(id, owner).unwrap();
        assert_eq!(parts.excluded, std::slice::from_ref(&left));
        assert_eq!(counts(&parts), (1, 0, 1), "у новой части заметки нет");
        tauri::async_runtime::block_on(service.describe_entry(id, owner, None, silent())).unwrap();
        let part = user_text(&seen.lock().unwrap()[4]);
        assert!(part.contains("Eileen: Hello.\n") && !part.contains("Left side"), "исключённая сцена модели не даётся: {part}");
    }

    #[test]
    fn chain_writes_an_entry_only_into_empty_fields() {
        let (url, _) = fake_model(|body| {
            let answer = if system_text(body).contains("about one character") {
                r#"{"description": "Эйлин встречает героя.", "speech": "Говорит на «ты»."}"#
            } else {
                "Эйлин здоровается."
            };
            completion(answer)
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let glossary = GlossaryService { state: &state };
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let open = state.open_project(id).unwrap();
        let get = || open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        let mut person = get();
        person.set_speech("Сухо и коротко.");
        glossary.save_character(id, &person).unwrap();

        let target = DescribeTarget::Character(eileen);
        let described =
            tauri::async_runtime::block_on(DescribeService { state: &state }.describe_unasked(id, vec![target.clone()], None, silent())).unwrap();
        assert_eq!(described.written, [target]);
        let person = get();
        assert_eq!((person.description(), person.speech()), (Some("Эйлин встречает героя."), Some("Сухо и коротко.")), "своя манера речи осталась");

        // Описание, которое появилось за время запуска, цепочка не трогает, а кнопка записи заменяет.
        let answer = CharacterAnswer { description: "Новое.".to_string(), speech: Some("Иначе.".to_string()), gender: None };
        let owner = ExcludeOwner::Character(eileen);
        assert!(!open.project.write(|tx| write_entry(tx, owner, Fill::Empty, &answer)).unwrap());
        assert_eq!(get().description(), Some("Эйлин встречает героя."));
        assert!(open.project.write(|tx| write_entry(tx, owner, Fill::Replace, &answer)).unwrap());
        let person = get();
        assert_eq!((person.description(), person.speech()), (Some("Новое."), Some("Иначе.")));
        // Записи уже нет (удалена или слита за время запуска): писать некуда, запуск идёт дальше.
        let gone = ExcludeOwner::Term(TermId::from_row(999_999));
        assert!(!open.project.write(|tx| write_entry(tx, gone, Fill::Replace, &answer)).unwrap());
    }

    fn found(name: &str, kind: FoundKind, same_as: Option<&str>) -> Finding {
        Finding { name: name.to_string(), kind, gender: None, same_as: same_as.map(str::to_string), other_than: None, parts: None }
    }

    fn person(name: &str, first: Option<&str>, last: Option<&str>, same_as: Option<&str>) -> Finding {
        Finding { parts: NameParts::check(name, first, last), ..found(name, FoundKind::Character, same_as) }
    }

    #[test]
    fn findings_that_name_a_known_entry_become_its_forms() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let glossary = GlossaryService { state: &state };
        let police = glossary.take_term(id, "People's Police Force", TermCategory::Organization).unwrap();
        let eileen = glossary.create_character(id, "Eileen").unwrap();
        let open = state.open_project(id).unwrap();
        let mut character = open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        character.set_last_name("Lee");
        glossary.save_character(id, &character).unwrap();

        let org = FoundKind::Term(TermCategory::Organization);
        let added = open
            .project
            .write(|tx| {
                save_found(
                    tx,
                    &[
                        found("PPF", org, Some("People's Police Force")),
                        found("Mr. Lee", FoundKind::Character, Some("Eileen Lee")),
                        found("the Order", org, Some("Order of the Dawn")),
                        found("Order of the Dawn", org, None),
                        found("Ghost", FoundKind::Character, Some("Nobody")),
                    ],
                )
            })
            .unwrap();
        assert_eq!(
            added,
            ["PPF → People's Police Force", "Order of the Dawn", "Ghost", "the Order → Order of the Dawn", "Eileen (говорящий) → Eileen Lee"],
            "обращение с фамилией не форма, говорящий привязан по имени"
        );

        let (terms, character) = open.project.read(|tx| Ok((TermRepo { conn: tx }.list()?, CharacterRepo { conn: tx }.get(eileen)?))).unwrap();
        let police = terms.iter().find(|term| term.id() == Some(police)).unwrap();
        assert_eq!(police.forms().iter().map(|form| form.text()).collect::<Vec<_>>(), ["PPF"], "аббревиатура легла формой по слову модели");
        assert!(character.forms().is_empty(), "«Mr. Lee» не стал формой");
        let order = terms.iter().find(|term| term.name() == "Order of the Dawn").unwrap();
        assert_eq!(order.forms().iter().map(|form| form.text()).collect::<Vec<_>>(), ["the Order"], "ссылка на находку того же ответа");
        assert_eq!(terms.len(), 2, "PPF и the Order не стали отдельными терминами");

        // Аббревиатура с точками это то же слово, известная форма пропущена.
        let again = open.project.write(|tx| save_found(tx, &[found("P.P.F.", org, None), found("PPF", org, None)])).unwrap();
        assert_eq!(again, ["P.P.F. → People's Police Force"]);

        // Без слова модели аббревиатура к названию сама не ложится: это отдельный термин, пару решит вопрос.
        assert_eq!(open.project.write(|tx| save_found(tx, &[found("OD", org, None)])).unwrap(), ["OD"]);
        assert_eq!(open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap().len(), 3);
    }

    #[test]
    fn known_entries_named_as_one_merge_only_when_the_script_agrees() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let glossary = GlossaryService { state: &state };
        let open = state.open_project(id).unwrap();
        let org = FoundKind::Term(TermCategory::Organization);
        let save = |findings: &[Finding]| open.project.write(|tx| save_scene_found(tx, findings, None)).unwrap();
        let terms = || open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap();
        let forms = |term: &crate::glossary::Term| term.forms().iter().map(|form| (form.text().to_string(), form.translation().map(str::to_string))).collect::<Vec<_>>();

        // Полное название к термину-аббревиатуре: оно становится названием, аббревиатура формой со своим переводом.
        let ppf = glossary.take_term(id, "PPF", TermCategory::Organization).unwrap();
        let mut term = open.project.read(|tx| TermRepo { conn: tx }.get(ppf)).unwrap();
        term.set_translation("НПС");
        glossary.save_term(id, &term).unwrap();
        assert_eq!(save(&[found("People's Police Force", org, Some("PPF"))]).added, ["PPF → People's Police Force"]);
        let list = terms();
        assert_eq!((list.len(), list[0].name(), list[0].translation()), (1, "People's Police Force", None));
        assert_eq!(forms(&list[0]), [("PPF".to_string(), Some("НПС".to_string()))]);

        // Два известных термина, модель назвала их одним, и скрипт видит аббревиатуру: слияние сразу.
        let order = glossary.take_term(id, "Order of the Dawn", TermCategory::Organization).unwrap();
        let od = glossary.take_term(id, "OD", TermCategory::Organization).unwrap();
        let saved = save(&[found("Order of the Dawn", org, Some("OD"))]);
        assert_eq!(saved.added, ["OD → Order of the Dawn (объединено)"]);
        assert!(saved.claims.is_empty());
        let list = terms();
        assert!(list.iter().all(|term| term.id() != Some(od)), "аббревиатура слита");
        assert_eq!(forms(list.iter().find(|term| term.id() == Some(order)).unwrap()), [("OD".to_string(), None)]);

        // Два известных персонажа, короткая и полная форма имени: остаётся запись, записанная раньше, а в игре
        // имён нет, поэтому главным остаётся её имя.
        let eileen = glossary.create_character(id, "Eileen").unwrap();
        let mut character = open.project.read(|tx| CharacterRepo { conn: tx }.get(eileen)).unwrap();
        character.set_last_name("Lee");
        glossary.save_character(id, &character).unwrap();
        let stu = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let mut character = open.project.read(|tx| CharacterRepo { conn: tx }.get(stu)).unwrap();
        character.set_first_name("Stu");
        character.set_translated_names("Стю", "");
        character.set_description("Учитель.");
        glossary.save_character(id, &character).unwrap();
        let stuart = glossary.create_character(id, "Stuart").unwrap();
        assert_eq!(save(&[found("Stuart", FoundKind::Character, Some("Stu"))]).added, ["Stuart → Stu (объединено)"]);
        let (characters, links) = open.project.read(|tx| Ok((CharacterRepo { conn: tx }.list()?, SpeakerRepo { conn: tx }.links_of(stu)?))).unwrap();
        assert_eq!(characters.len(), 2, "Eileen Lee и Stu");
        assert!(characters.iter().all(|character| character.id() != Some(stuart)), "вторая запись слита");
        let merged = characters.iter().find(|character| character.id() == Some(stu)).unwrap();
        let forms_of = |character: &crate::glossary::Character| character.forms().iter().map(|form| (form.text().to_string(), form.translation().map(str::to_string))).collect::<Vec<_>>();
        assert_eq!((merged.first_name(), merged.translated_first_name()), (Some("Stu"), Some("Стю")));
        assert_eq!(forms_of(merged), [("Stuart".to_string(), None)], "второе написание имени стало формой");
        assert_eq!(merged.forms()[0].kind(), FormKind::First);
        assert_eq!(merged.description(), Some("Учитель."));
        assert_eq!(links.iter().map(|link| link.speaker.as_str()).collect::<Vec<_>>(), ["code:e"]);

        // Скрипт связи не видит: известные записи по одному слову модели не сливаются, пара идёт вопросом.
        let leeway = glossary.take_term(id, "Leeway", TermCategory::General).unwrap();
        let saved = save(&[found("Leeway", FoundKind::Character, Some("Eileen Lee"))]);
        assert!(saved.added.is_empty());
        let pair = Pair::new(ExcludeOwner::Term(leeway), ExcludeOwner::Character(eileen)).unwrap();
        assert_eq!(saved.claims, [Claim { pair, same: true }]);
        assert!(terms().iter().any(|term| term.name() == "Leeway"));

        // Ответ «да» на вопрос отдаёт термин персонажу формой, ответ «нет» запоминается, и пара больше не идёт.
        assert_eq!(open.project.write(|tx| merge_pair(tx, pair)).unwrap().as_deref(), Some("Leeway → Eileen Lee (объединено)"));
        assert!(terms().iter().all(|term| term.name() != "Leeway"));
        let mara = glossary.take_term(id, "Mara", TermCategory::General).unwrap();
        open.project.write(|tx| PairCheckRepo { conn: tx }.mark(ExcludeOwner::Term(mara), ExcludeOwner::Character(eileen), false, Timestamp::now())).unwrap();
        assert!(save(&[found("Mara", FoundKind::Character, Some("Eileen Lee"))]).claims.is_empty(), "отвеченная пара не спрашивается");
    }

    #[test]
    fn names_titles_and_name_parts_from_a_real_game() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let glossary = GlossaryService { state: &state };
        let open = state.open_project(id).unwrap();
        let karanovic = glossary.create_character(id, "Karanovic").unwrap();
        let stu = glossary.create_character(id, "Stu").unwrap();
        let dimitria = glossary.create_character(id, "Dimitria").unwrap();
        let save = |findings: &[Finding]| open.project.write(|tx| save_scene_found(tx, findings, None)).unwrap();
        let get = |id: CharacterId| open.project.read(|tx| CharacterRepo { conn: tx }.get(id)).unwrap();

        let saved = save(&[
            person("Vera Karanovic", Some("Vera"), Some("Karanovic"), None),
            person("Detective-Colonel Karanovic", None, None, None),
            person("Stuart", None, None, Some("Stu")),
            person("Vellnikov", None, Some("Vellnikov"), Some("Dimitria")),
            person("Dimi", Some("Dimi"), None, Some("Dimitria")),
        ]);
        assert_eq!(saved.added, ["Vera Karanovic", "Detective-Colonel", "Stuart → Stu", "Vellnikov → Dimitria Vellnikov", "Dimi → Dimitria Vellnikov"]);
        assert!(saved.claims.is_empty());

        let (characters, terms) = open.project.read(|tx| Ok((CharacterRepo { conn: tx }.list()?, TermRepo { conn: tx }.list()?))).unwrap();
        let vera = characters.iter().find(|character| character.first_name() == Some("Vera")).unwrap();
        assert_eq!(vera.last_name(), Some("Karanovic"), "имя и фамилия по отдельности");
        assert!(vera.forms().is_empty(), "должность с фамилией не форма");
        assert_eq!(get(karanovic).full_name().as_deref(), Some("Karanovic"), "одиночная фамилия сама с однофамилицей не сливается");
        assert_eq!(get(stu).first_name(), Some("Stu"), "в игре имён нет: главным остаётся прежнее");
        assert_eq!(get(stu).forms().iter().map(|form| (form.text(), form.kind())).collect::<Vec<_>>(), [("Stuart", FormKind::First)]);
        assert_eq!((get(dimitria).last_name(), get(dimitria).forms().iter().map(|form| form.text()).collect::<Vec<_>>()), (Some("Vellnikov"), vec!["Dimi"]));
        assert_eq!(get(dimitria).forms()[0].kind(), FormKind::First, "короткое имя это форма-имя");
        assert_eq!(terms.iter().map(|term| (term.name(), term.category())).collect::<Vec<_>>(), [("Detective-Colonel", TermCategory::Title)]);

        // Модель назвала одиночную фамилию Верой, и скрипт видит общую фамилию: слияние сразу.
        let merged = save(&[person("Karanovic", None, Some("Karanovic"), Some("Vera Karanovic"))]);
        assert_eq!(merged.added, ["Karanovic → Vera Karanovic (объединено)"]);
        assert!(open.project.read(|tx| CharacterRepo { conn: tx }.get(karanovic)).is_err());

        // Полное имя к известному имени без фамилии: фамилия в пустое поле.
        let hilde = glossary.create_character(id, "Hilde").unwrap();
        assert_eq!(save(&[person("Hilde Tominskova", Some("Hilde"), Some("Tominskova"), Some("Hilde"))]).added, ["Hilde → Hilde Tominskova"]);
        assert_eq!(get(hilde).full_name().as_deref(), Some("Hilde Tominskova"));
    }

    #[test]
    fn an_occupied_name_field_makes_a_separate_entry_and_a_question() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let save = |findings: &[Finding]| open.project.write(|tx| save_scene_found(tx, findings, None)).unwrap();
        let list = || open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        save(&[person("Vera Karanovic", Some("Vera"), Some("Karanovic"), None)]);
        let vera = list()[0].id().unwrap();

        // Вторая фамилия: поле занято, модель одна, скрипт связи не видит. Отдельная запись и пара для вопроса.
        let saved = save(&[person("Petrova", None, Some("Petrova"), Some("Vera Karanovic"))]);
        assert_eq!(saved.added, ["Petrova"]);
        let petrova = list().iter().find(|character| character.last_name() == Some("Petrova")).and_then(Character::id).unwrap();
        assert_eq!(saved.claims, [Claim { pair: Pair::people(vera, petrova).unwrap(), same: true }]);
        // «Да» кладёт вторую фамилию формой.
        let text = open.project.write(|tx| merge_pair(tx, Pair::people(vera, petrova).unwrap())).unwrap();
        assert_eq!(text.as_deref(), Some("Petrova → Vera Karanovic (объединено)"));
        assert_eq!(list()[0].forms().iter().map(|form| form.text()).collect::<Vec<_>>(), ["Petrova"]);

        // Опечатка в фамилии: поле занято, но скрипт видит опечатку. Два довода, написание ложится формой-фамилией.
        let saved = save(&[person("Karanovich", None, Some("Karanovich"), Some("Vera Karanovic"))]);
        assert_eq!(saved.added, ["Karanovich → Vera Karanovic"]);
        assert!(saved.claims.is_empty());
        assert_eq!(list().len(), 1);

        // Псевдоним без частей имени ложится формой сразу: другого довода у него не бывает.
        assert_eq!(save(&[found("Nightingale", FoundKind::Character, Some("Vera Karanovic"))]).added, ["Nightingale → Vera Karanovic"]);
        assert_eq!(
            list()[0].forms().iter().map(|form| (form.text(), form.kind())).collect::<Vec<_>>(),
            [("Petrova", FormKind::Last), ("Karanovich", FormKind::Last), ("Nightingale", FormKind::Other)]
        );
        assert_eq!(list()[0].full_name().as_deref(), Some("Vera Karanovic"), "в игре фамилий нет: главной остаётся прежняя");
    }

    /// Реплики с именами: главное написание выбирается по тому, сколько реплик его показывают.
    const NAMES_SCRIPT: &str = r#"label names:
    "Vera came in."
    "Vera smiled."
    "Veraaaa... oh."
    "The fierce Jovankam smiled."
    "Jovanka nodded."
    "Jovanka left."
    "That damned Alexa."
    "Alexandra is here."
    "Alexandra sat down."
    "Name: Margaret Saint-Saens"
    "My friend, Margaret Dvorak."
    "Dvorak called."
    "Her name is Maja Zoric."
    "Maja waved."
    "Claire Kralj signed it."
"#;

    #[test]
    fn the_main_name_is_the_spelling_the_game_shows_most() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project_with(dir.path(), "http://127.0.0.1:9/v1", &[("names.rpy", NAMES_SCRIPT)]);
        let glossary = GlossaryService { state: &state };
        let open = state.open_project(id).unwrap();
        let save = |findings: &[Finding]| open.project.write(|tx| save_scene_found(tx, findings, None)).unwrap();
        let get = |id: CharacterId| open.project.read(|tx| CharacterRepo { conn: tx }.get(id)).unwrap();
        let forms = |id: CharacterId| get(id).forms().iter().map(|form| (form.text().to_string(), form.kind())).collect::<Vec<_>>();

        // Растянутое написание из одной реплики главным не становится, хотя оно длиннее.
        let vera = glossary.create_character(id, "Vera").unwrap();
        assert_eq!(save(&[person("Veraaaa", Some("Veraaaa"), None, Some("Vera"))]).added, ["Veraaaa → Vera"]);
        assert_eq!((get(vera).first_name(), forms(vera)), (Some("Vera"), vec![("Veraaaa".to_string(), FormKind::First)]));

        // Главным записано редкое написание: частое занимает его место, перевод остаётся при своём написании.
        let jovanka = glossary.create_character(id, "Jovankam").unwrap();
        let mut character = get(jovanka);
        character.set_translated_names("Йованкам", "");
        glossary.save_character(id, &character).unwrap();
        save(&[found("Jovanka", FoundKind::Character, Some("Jovankam"))]);
        let character = get(jovanka);
        assert_eq!((character.first_name(), character.translated_first_name()), (Some("Jovanka"), None));
        assert_eq!(
            character.forms().iter().map(|form| (form.text(), form.translation(), form.kind())).collect::<Vec<_>>(),
            [("Jovankam", Some("Йованкам"), FormKind::First)]
        );

        // Полное имя без частей от модели: имя известно, остаток это фамилия.
        let maja = glossary.create_character(id, "Maja").unwrap();
        assert_eq!(save(&[found("Maja Zoric", FoundKind::Character, Some("Maja"))]).added, ["Maja → Maja Zoric"]);
        assert_eq!((get(maja).last_name(), forms(maja)), (Some("Zoric"), Vec::new()));
        // Одно слово без частей, которое в репликах стоит после имени: фамилия.
        let claire = glossary.create_character(id, "Claire").unwrap();
        save(&[found("Kralj", FoundKind::Character, Some("Claire"))]);
        assert_eq!((get(claire).last_name(), forms(claire)), (Some("Kralj"), Vec::new()));

        // «Объединить с…» в окне: остаётся более ранняя запись, имя выбирает частота.
        let alexa = glossary.create_character(id, "Alexa").unwrap();
        let alexandra = glossary.create_character(id, "Alexandra").unwrap();
        let kept = glossary.merge(id, ExcludeOwner::Character(alexandra), ExcludeOwner::Character(alexa)).unwrap();
        assert_eq!(kept, ExcludeOwner::Character(alexa));
        assert_eq!((get(alexa).first_name(), forms(alexa)), (Some("Alexandra"), vec![("Alexa".to_string(), FormKind::First)]));

        // Две фамилии одного человека: вторая ложится формой-фамилией, главная та, что в игре чаще.
        save(&[
            person("Margaret Saint-Saens", Some("Margaret"), Some("Saint-Saens"), None),
            person("Margaret Dvorak", Some("Margaret"), Some("Dvorak"), None),
        ]);
        let list = open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        let record = |last: &str| list.iter().find(|character| character.last_name() == Some(last)).and_then(Character::id).unwrap();
        let (saens, dvorak) = (record("Saint-Saens"), record("Dvorak"));
        let kept = glossary.merge(id, ExcludeOwner::Character(dvorak), ExcludeOwner::Character(saens)).unwrap();
        assert_eq!(kept, ExcludeOwner::Character(saens));
        assert_eq!((get(saens).full_name().as_deref(), forms(saens)), (Some("Margaret Dvorak"), vec![("Saint-Saens".to_string(), FormKind::Last)]));
        assert!(glossary.merge(id, ExcludeOwner::Character(saens), ExcludeOwner::Character(saens)).is_err(), "запись с собой не объединяется");
    }

    #[test]
    fn a_known_name_of_someone_else_becomes_a_namesake() {
        use crate::common::Gender;
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let save = |findings: &[Finding]| open.project.write(|tx| save_scene_found(tx, findings, None)).unwrap();
        let list = || open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        save(&[person("Vera Karanovic", Some("Vera"), Some("Karanovic"), None), found("Hope", FoundKind::Character, None)]);
        let vera = list().iter().find(|character| character.first_name() == Some("Vera")).and_then(Character::id).unwrap();

        // Без слова модели известная фамилия это та же Вера: находка пропущена.
        assert!(save(&[person("Karanovic", None, Some("Karanovic"), None)]).added.is_empty());

        // Модель сказала, что это не Вера: вторая запись с тем же написанием и пара для вопроса.
        let other = Finding { other_than: Some("Vera Karanovic".to_string()), gender: Some(Gender::Male), ..person("Karanovic", None, Some("Karanovic"), None) };
        let saved = save(std::slice::from_ref(&other));
        assert_eq!(saved.added, ["Karanovic (не Vera Karanovic)"]);
        let husband = list().iter().find(|character| character.first_name().is_none()).and_then(Character::id).unwrap();
        assert_eq!(saved.claims, [Claim { pair: Pair::people(vera, husband).unwrap(), same: false }]);
        assert!(save(std::slice::from_ref(&other)).added.is_empty(), "тёзка создаётся один раз");
        assert_eq!(list().len(), 3);

        // Город с именем персонажа: термин с тем же написанием.
        let town = Finding { other_than: Some("Hope".to_string()), ..found("Hope", FoundKind::Term(TermCategory::Location), None) };
        assert_eq!(save(std::slice::from_ref(&town)).added, ["Hope (не Hope)"]);
        assert!(save(std::slice::from_ref(&town)).added.is_empty());
        assert_eq!(open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap().len(), 1);
    }

    #[test]
    fn a_titled_name_is_never_a_term() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let glossary = GlossaryService { state: &state };
        let save = |findings: &[Finding]| open.project.write(|tx| save_found(tx, findings)).unwrap();
        let names = || {
            open.project
                .read(|tx| {
                    let terms = TermRepo { conn: tx }.list()?.iter().map(|term| term.name().to_string()).collect::<Vec<_>>();
                    let characters = CharacterRepo { conn: tx }.list()?.iter().filter_map(|character| character.first_name().map(str::to_string)).collect::<Vec<_>>();
                    Ok((terms, characters))
                })
                .unwrap()
        };

        assert_eq!(save(&[found("General Store", FoundKind::Term(TermCategory::Location), None)]), ["General Store"], "место с «General» не делится");
        assert_eq!(save(&[found("Princess Selena", FoundKind::Character, None)]), ["Selena"], "обычное звание не термин");
        // Составное звание остаётся термином, одиночное от модели не записывается.
        let added = save(&[found("Detective-Colonel Orin", FoundKind::Character, None), found("Princess", FoundKind::Term(TermCategory::Title), None)]);
        assert_eq!(added, ["Detective-Colonel", "Orin"]);
        // Должность перед именем известного персонажа делится и у находки-термина.
        assert!(save(&[found("Queen Selena", FoundKind::Term(TermCategory::General), None)]).is_empty());
        assert_eq!(names(), (vec!["Detective-Colonel".to_string(), "General Store".to_string()], vec!["Orin".to_string(), "Selena".to_string()]));

        // Записанное раньше разбор сам не чинит: термин с должностью и именем остаётся, пару с персонажем решит вопрос.
        let term = glossary.take_term(id, "Princess Mara", TermCategory::General).unwrap();
        glossary.take_term(id, "Sister", TermCategory::Title).unwrap();
        assert_eq!(save(&[found("Mara", FoundKind::Character, None)]), ["Mara"]);
        assert_eq!(names().0, ["Detective-Colonel", "General Store", "Princess Mara", "Sister"]);
        let glossary_data = open.project.read(|tx| GlossaryData::load(tx)).unwrap();
        let mara = glossary_data.characters.iter().find(|character| character.first_name() == Some("Mara")).and_then(Character::id).unwrap();
        let relation = relations::between(&glossary_data, ExcludeOwner::Term(term), ExcludeOwner::Character(mara), false, false);
        assert_eq!(relation, Some(relations::Relation::Titled));
    }

    #[test]
    fn junk_names_titled_people_and_plural_terms_from_a_real_game() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let glossary = GlossaryService { state: &state };
        let save = |findings: &[Finding]| open.project.write(|tx| save_found(tx, findings)).unwrap();
        let general = FoundKind::Term(TermCategory::General);
        let characters = || {
            open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap().iter().filter_map(|character| character.first_name().map(str::to_string)).collect::<Vec<_>>()
        };
        let terms = || open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap();
        open.project
            .write(|tx| {
                tx.execute("UPDATE segments SET source_text = 'Give him a... boobjob.' WHERE source_text = 'Static.'", [])?;
                tx.execute("UPDATE segments SET source_text = 'Boobjob' WHERE source_text = 'Who?'", [])?;
                tx.execute("UPDATE segments SET source_text = 'I saw Merek today.' WHERE source_text = 'Left side.'", [])?;
                Ok(())
            })
            .unwrap();

        // Слово из меню не персонаж, имя посреди фразы персонаж.
        assert_eq!(save(&[found("Boobjob", FoundKind::Character, None), found("Merek", FoundKind::Character, None)]), ["Merek"]);
        // Должность со служебным словом остаётся должностью целиком, обрывок после неё персонажем не становится.
        let title = FoundKind::Term(TermCategory::Title);
        assert_eq!(save(&[found("Director of Engineering", title, None), found("of Engineering", FoundKind::Character, None)]), ["Director of Engineering"]);
        // Должность со служебным словом перед именем отделяется целиком (реплика Take Over).
        assert_eq!(save(&[found("Chief of Engineering Ristovski", FoundKind::Character, None)]), ["Chief of Engineering", "Ristovski"]);
        // Титул как имя лица это персонаж без артикля.
        assert_eq!(save(&[found("The Fairy Queen", general, None)]), ["Fairy Queen"]);
        assert_eq!(characters(), ["Fairy Queen", "Merek", "Ristovski"]);

        // Множественное число это то же слово: ложится формой, а единственное к множественному становится названием.
        glossary.take_term(id, "pixie", TermCategory::General).unwrap();
        glossary.take_term(id, "witches", TermCategory::General).unwrap();
        assert_eq!(save(&[found("pixies", general, None), found("witch", general, None)]), ["pixies → pixie", "witches → witch"]);

        let names: Vec<(String, Vec<String>)> = terms()
            .iter()
            .filter(|term| term.category() != TermCategory::Title)
            .map(|term| (term.name().to_string(), term.forms().iter().map(|form| form.text().to_string()).collect()))
            .collect();
        assert_eq!(names, [("pixie".to_string(), vec!["pixies".to_string()]), ("witch".to_string(), vec!["witches".to_string()])]);
    }

    #[test]
    fn a_shared_name_belongs_to_the_character_of_the_scene() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let glossary = GlossaryService { state: &state };
        let open = state.open_project(id).unwrap();
        let forms = |id: CharacterId| {
            open.project.read(|tx| CharacterRepo { conn: tx }.get(id)).unwrap().forms().iter().map(|form| form.text().to_string()).collect::<Vec<_>>()
        };

        // Короткая форма без слова модели к полному имени сама не ложится: отдельная запись, пару решит вопрос.
        let dimitria = glossary.create_character(id, "Dimitria").unwrap();
        assert_eq!(open.project.write(|tx| save_found(tx, &[found("Dimi", FoundKind::Character, None)])).unwrap(), ["Dimi"]);
        assert!(forms(dimitria).is_empty());

        // Фамилия у двоих: модель назвала её, и выбрать можно только того, кто есть в разобранной сцене.
        let eileen = glossary.speaker_to_character(id, &SpeakerKey::new("code:e").unwrap()).unwrap();
        let tom = glossary.create_character(id, "Tom").unwrap();
        for person in [eileen, tom] {
            let mut character = open.project.read(|tx| CharacterRepo { conn: tx }.get(person)).unwrap();
            character.set_last_name("Grant");
            glossary.save_character(id, &character).unwrap();
        }
        let start = ContainerKey::new("label:start").unwrap();
        let boss = found("Boss", FoundKind::Character, Some("Grant"));
        assert_eq!(open.project.write(|tx| save_found(tx, std::slice::from_ref(&boss))).unwrap(), ["Boss"], "без сцены двух владельцев не различить");
        let created = open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap().into_iter().find(|character| character.first_name() == Some("Boss")).unwrap();
        glossary.delete_character(id, created.id().unwrap()).unwrap();
        assert_eq!(open.project.write(|tx| save_scene_found(tx, std::slice::from_ref(&boss), Some(&start))).unwrap().added, ["Boss → Eileen Grant"]);
        assert_eq!(forms(eileen), ["Boss"]);
        assert!(forms(tom).is_empty());
    }

    #[test]
    fn scene_analysis_checks_doubtful_mentions_and_keeps_the_answers() {
        let (url, seen) = fake_model(|body| {
            if system_text(body).contains("proper names") {
                completion(r#"[{"kind": "check", "no": ["Q1"]}]"#)
            } else {
                completion("Описание.")
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let open = state.open_project(id).unwrap();
        let start = ContainerKey::new("label:start").unwrap();
        let (mind, hash) = open
            .project
            .write(|tx| {
                tx.execute("UPDATE segments SET source_text = 'Never mind, it is fine.' WHERE source_text = 'Static.'", [])?;
                tx.execute("UPDATE segments SET source_text = 'MInD is watching.' WHERE source_text = 'Narration.'", [])?;
                let hash: String = tx.query_row("SELECT source_hash FROM segments WHERE source_text = 'Never mind, it is fine.'", [], |row| row.get(0))?;
                Ok((TermRepo { conn: tx }.save(&Term::new("MInD", TermCategory::Organization).unwrap())?, hash))
            })
            .unwrap();

        let analyzed = tauri::async_runtime::block_on(service.analyze(id, vec![start.clone()], None, silent())).unwrap();
        assert_eq!(analyzed.error, None);
        let request = seen.lock().unwrap().iter().find(|body| system_text(body).contains("proper names")).cloned().unwrap();
        let user = user_text(&request);
        assert!(user.contains("Known names to check."), "{user}");
        assert!(user.contains("Q1 MInD: \"") && user.contains("Never mind, it is fine.\"\n"), "{user}");
        assert!(!user.contains("Q2"), "точное необычное написание не спрашивается: {user}");

        let checks = open.project.read(|tx| MentionCheckRepo { conn: tx }.terms()).unwrap();
        assert_eq!(checks.get(&mind).and_then(|checks| checks.get(&hash)), Some(&false), "ответ записан по отпечатку реплики");
        let lines = GlossaryService { state: &state }.entry_lines(id, ExcludeOwner::Term(mind)).unwrap();
        let texts: Vec<&str> = lines.iter().map(|line| line.placed.line.source_text.as_str()).collect();
        assert_eq!(texts, ["MInD is watching."]);

        // Ответ «да» важнее написания: строчное «mind» становится упоминанием.
        open.project.write(|tx| MentionCheckRepo { conn: tx }.mark(ExcludeOwner::Term(mind), &hash, true, Timestamp::now())).unwrap();
        assert_eq!(GlossaryService { state: &state }.entry_lines(id, ExcludeOwner::Term(mind)).unwrap().len(), 2);

        // Слияние: ответы уходят к оставшейся записи, её собственный ответ важнее.
        open.project
            .write(|tx| {
                let other = TermRepo { conn: tx }.save(&Term::new("MIND Corp", TermCategory::Organization).unwrap())?;
                let checks = MentionCheckRepo { conn: tx };
                checks.mark(ExcludeOwner::Term(other), "elsewhere", true, Timestamp::now())?;
                checks.mark(ExcludeOwner::Term(other), &hash, false, Timestamp::now())?;
                crate::glossary::service::merge_terms(tx, other, mind)
            })
            .unwrap();
        let checks = open.project.read(|tx| MentionCheckRepo { conn: tx }.terms()).unwrap();
        assert_eq!(checks.len(), 1, "у слитой записи ответов не осталось");
        assert_eq!((checks[&mind].get("elsewhere"), checks[&mind].get(&hash)), (Some(&true), Some(&true)));
    }

    #[test]
    fn scene_analysis_asks_whether_two_characters_are_one_person() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let same = Arc::new(AtomicBool::new(false));
        let answer = same.clone();
        let (url, seen) = fake_model(move |body| {
            let system = system_text(body);
            if system.contains("proper names") {
                completion("[]")
            } else if system.contains("same person") {
                completion(if answer.load(Ordering::SeqCst) { r#"{"same": true}"# } else { r#"{"same": false}"# })
            } else {
                completion("Описание.")
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let open = state.open_project(id).unwrap();
        let start = ContainerKey::new("label:start").unwrap();
        let (dimitria, vellnikov) = open
            .project
            .write(|tx| {
                tx.execute("UPDATE segments SET source_text = 'Chief Vellnikov runs the lab.' WHERE source_text = 'Static.'", [])?;
                tx.execute("UPDATE segments SET source_text = 'Dimitria shows the detective around.' WHERE source_text = 'Narration.'", [])?;
                let characters = CharacterRepo { conn: tx };
                let mut first = Character::new("Dimitria");
                first.set_gender(Some(crate::common::Gender::Female));
                let mut last = Character::new("");
                last.set_last_name("Vellnikov");
                Ok((characters.save(&first)?, characters.save(&last)?))
            })
            .unwrap();
        let questions = || seen.lock().unwrap().iter().filter(|body| system_text(body).contains("same person")).cloned().collect::<Vec<_>>();
        let analyze = || tauri::async_runtime::block_on(service.analyze(id, vec![start.clone()], None, silent())).unwrap();

        // Модель видит персонажей сцены с номерами; «нет» запоминается, пара больше не спрашивается.
        let first = analyze();
        assert_eq!(first.error, None);
        let analysis = seen.lock().unwrap().iter().find(|body| system_text(body).contains("proper names")).cloned().unwrap();
        let cast = format!("Characters of this scene:\nC{}: Dimitria (female)\nC{}: Vellnikov\n", dimitria.get(), vellnikov.get());
        assert!(user_text(&analysis).contains(&cast), "{}", user_text(&analysis));
        let asked = questions();
        assert_eq!(asked.len(), 1);
        assert!(user_text(&asked[0]).starts_with("A: Dimitria\nGender: female\nB: Vellnikov\n"), "{}", user_text(&asked[0]));
        assert!(user_text(&asked[0]).contains("Chief Vellnikov runs the lab."));
        assert_eq!(open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap().len(), 2);
        open.project.write(|tx| Ok(tx.execute("DELETE FROM analyzed_parts", [])?)).unwrap();
        analyze();
        assert_eq!(questions().len(), 1, "отвеченная пара не спрашивается снова");

        // «Да»: имя и фамилия соединяются в одного персонажа.
        open.project.write(|tx| PairCheckRepo { conn: tx }.clear()).unwrap();
        open.project.write(|tx| Ok(tx.execute("DELETE FROM analyzed_parts", [])?)).unwrap();
        same.store(true, Ordering::SeqCst);
        let joined = analyze();
        assert_eq!(joined.added, ["Vellnikov → Dimitria Vellnikov (объединено)"]);
        let characters = open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        let names: Vec<_> = characters.iter().filter_map(Character::full_name).collect();
        assert_eq!(names, ["Dimitria Vellnikov"]);
        assert_eq!(characters[0].id(), Some(dimitria));
    }

    #[test]
    fn name_halves_named_together_elsewhere_are_asked_about_and_join() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let list = || open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        let half = |first: &str, last: &str, translation: &str| {
            open.project
                .write(|tx| {
                    let mut character = Character::new(first);
                    if !last.is_empty() {
                        character.set_last_name(last);
                    }
                    character.set_translated_names(if last.is_empty() { translation } else { "" }, if last.is_empty() { "" } else { translation });
                    CharacterRepo { conn: tx }.save(&character)
                })
                .unwrap()
        };
        let start = ContainerKey::new("label:start").unwrap();
        let asked = |claimed: &[Pair]| open.project.read(|tx| same_entry::questions(tx, &start, claimed)).unwrap();

        // Разобранные сцены назвали имя и фамилию порознь. Скрипт их сам не соединяет.
        let dimitria = half("Dimitria", "", "Димитрия");
        let vellnikov = half("", "Vellnikov", "Велльникова");
        open.project.write(|tx| Ok(tx.execute("UPDATE segments SET source_text = 'Dimitria shows the lab.' WHERE source_text = 'Static.'", [])?)).unwrap();
        assert!(open.project.write(|tx| save_found(tx, &[])).unwrap().is_empty());
        assert!(asked(&[]).is_empty(), "в сцене только имя, а общей реплики в игре нет");

        // Полное имя есть в другой сцене игры: пара становится вопросом с этой репликой.
        open.project.write(|tx| Ok(tx.execute("UPDATE segments SET source_text = '{size=24}Dr Dimitria Vellnikov{/size}' WHERE source_text = 'Left side.'", [])?)).unwrap();
        let questions = asked(&[]);
        let [question] = questions.as_slice() else { panic!("один вопрос: {questions:?}") };
        let pair = Pair::people(dimitria, vellnikov).unwrap();
        assert_eq!(question.pair(), Some(pair));
        assert_eq!(question.game_lines, ["{size=24}Dr Dimitria Vellnikov{/size}"]);

        // «Да»: фамилия с переводом становится фамилией персонажа с именем.
        assert_eq!(open.project.write(|tx| merge_pair(tx, pair)).unwrap().as_deref(), Some("Vellnikov → Dimitria Vellnikov (объединено)"));
        let [joined] = list().try_into().unwrap_or_else(|all: Vec<Character>| panic!("один персонаж: {all:?}"));
        assert_eq!(joined.id(), Some(dimitria));
        assert_eq!((joined.last_name(), joined.translated_last_name()), (Some("Vellnikov"), Some("Велльникова")));
        assert!(joined.forms().is_empty(), "фамилия не остаётся формой");

        // «Нет» запоминается: пара не спрашивается и скриптом не сливается.
        let dimi = half("Dimi", "", "");
        let pair = Pair::people(dimitria, dimi).unwrap();
        assert_eq!(asked(&[]).iter().filter_map(same_entry::Question::pair).collect::<Vec<_>>(), [pair], "короткая форма имени персонажа сцены");
        open.project.write(|tx| PairCheckRepo { conn: tx }.mark(pair.a, pair.b, false, Timestamp::now())).unwrap();
        assert!(asked(&[]).is_empty());
        assert!(asked(&[pair]).is_empty(), "и по слову модели тоже");
        assert_eq!(list().len(), 2);

        // Фамилия записана именем (модель назвала её без `last_name`): пару даёт только общая реплика игры,
        // а кто из двоих фамилия, решает порядок в ней.
        let tominskova = half("Tominskova", "", "Томинскова");
        let hilde = half("Hilde", "", "Хильда");
        open.project.write(|tx| Ok(tx.execute("UPDATE segments SET source_text = 'Hilde waits outside.' WHERE source_text = 'Narration.'", [])?)).unwrap();
        assert!(asked(&[]).is_empty(), "два имени без общей реплики не пара");
        open.project.write(|tx| Ok(tx.execute("UPDATE segments SET source_text = 'Officer Hilde Tominskova reporting.' WHERE source_text = 'Who?'", [])?)).unwrap();
        let pair = Pair::people(hilde, tominskova).unwrap();
        assert_eq!(asked(&[]).iter().filter_map(same_entry::Question::pair).collect::<Vec<_>>(), [pair]);
        assert_eq!(open.project.write(|tx| merge_pair(tx, pair)).unwrap().as_deref(), Some("Tominskova → Hilde Tominskova (объединено)"));
        let joined = list().into_iter().find(|character| character.id() == Some(hilde)).unwrap();
        assert_eq!((joined.last_name(), joined.translated_first_name(), joined.translated_last_name()), (Some("Tominskova"), Some("Хильда"), Some("Томинскова")));
        assert!(joined.forms().is_empty() && list().len() == 3);
    }

    #[test]
    fn scene_analysis_asks_about_terms_too() {
        let (url, seen) = fake_model(|body| {
            let system = system_text(body);
            if system.contains("proper names") {
                completion("[]")
            } else if system.contains("same person") {
                completion(r#"{"same": true}"#)
            } else {
                completion("Описание.")
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), &url);
        let service = DescribeService { state: &state };
        let open = state.open_project(id).unwrap();
        let start = ContainerKey::new("label:start").unwrap();
        open.project
            .write(|tx| {
                tx.execute("UPDATE segments SET source_text = 'The PPF is here.' WHERE source_text = 'Static.'", [])?;
                tx.execute("UPDATE segments SET source_text = 'The People''s Police Force arrived.' WHERE source_text = 'Narration.'", [])?;
                let terms = TermRepo { conn: tx };
                terms.save(&Term::new("People's Police Force", TermCategory::Organization).unwrap())?;
                terms.save(&Term::new("PPF", TermCategory::Organization).unwrap())?;
                Ok(())
            })
            .unwrap();

        let (channel, notes) = recorded();
        let analyzed = tauri::async_runtime::block_on(service.analyze(id, vec![start], None, channel)).unwrap();
        assert_eq!(analyzed.error, None);
        assert_eq!(analyzed.added, ["PPF → People's Police Force (объединено)"]);
        let asked: Vec<_> = seen.lock().unwrap().iter().filter(|body| system_text(body).contains("same person")).cloned().collect();
        assert_eq!(asked.len(), 1);
        let user = user_text(&asked[0]);
        assert!(user.starts_with("A (term, organization): People's Police Force\nB (term, organization): PPF\n"), "{user}");
        assert!(user.contains("The PPF is here.") && user.ends_with("Are A and B the same thing?"), "{user}");
        assert!(notes.lock().unwrap().iter().any(|note| note.contains("одно ли это People's Police Force и PPF")), "{:?}", notes.lock().unwrap());
        let terms = open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap();
        let [term] = terms.as_slice() else { panic!("один термин: {terms:?}") };
        assert_eq!((term.name(), term.forms().iter().map(|form| form.text()).collect::<Vec<_>>()), ("People's Police Force", vec!["PPF"]));
    }

    #[test]
    fn speakers_without_a_character_are_linked_to_found_characters() {
        use crate::common::Gender;
        use crate::glossary::SpeakerLink;
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = project(dir.path(), "http://127.0.0.1:9/v1");
        let open = state.open_project(id).unwrap();
        let links = || open.project.read(|tx| SpeakerRepo { conn: tx }.links()).unwrap();

        // Неразобранный говорящий моделью не скрыт: она назовёт его персонажем.
        let known = open.project.read(|tx| known_for_prompt(tx)).unwrap();
        assert!(!known.contains(&"Eileen".to_string()) && !known.contains(&"Radio Chatter".to_string()));

        // Персонаж с подписью говорящего привязывает говорящего, подпись-роль термином не становится.
        let eileen = Finding { gender: Some(Gender::Female), ..person("Eileen Grant", Some("Eileen"), Some("Grant"), None) };
        let added = open.project.write(|tx| save_found(tx, &[eileen, found("Radio Chatter", FoundKind::Term(TermCategory::General), None)])).unwrap();
        assert_eq!(added, ["Eileen Grant", "Eileen (говорящий) → Eileen Grant"]);
        let characters = open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap();
        let [character] = characters.as_slice() else { panic!("один персонаж: {characters:?}") };
        assert_eq!(links(), [SpeakerLink { speaker: SpeakerKey::new("code:e").unwrap(), character: character.id().unwrap(), is_primary: true }]);
        assert!(open.project.read(|tx| TermRepo { conn: tx }.list()).unwrap().is_empty());

        // Привязанный говорящий снова известен, повтор ничего не меняет.
        assert!(open.project.read(|tx| known_for_prompt(tx)).unwrap().contains(&"Eileen".to_string()));
        assert!(open.project.write(|tx| save_found(tx, &[found("Eileen", FoundKind::Character, None)])).unwrap().is_empty());
        assert_eq!(links().len(), 1);

        // Герой с подписью-токеном модели не виден: говорящий с репликами сам становится персонажем.
        open.project.write(|tx| Ok(tx.execute("UPDATE segments SET speaker_key = 'code:mc' WHERE source_text = 'Narration.'", [])?)).unwrap();
        assert_eq!(open.project.write(|tx| save_found(tx, &[])).unwrap(), ["[player] (говорящий, новый персонаж)"]);
        let hero = open.project.read(|tx| CharacterRepo { conn: tx }.list()).unwrap().into_iter().find(|character| character.first_name() == Some("[player]")).unwrap();
        assert!(links().contains(&SpeakerLink { speaker: SpeakerKey::new("code:mc").unwrap(), character: hero.id().unwrap(), is_primary: true }));
        assert!(open.project.write(|tx| save_found(tx, &[])).unwrap().is_empty(), "второй раз не создаётся");
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
        assert_eq!(depth.parts_for(6), 5, "3,6 округляется до 4 и поднимается до минимума");
        assert_eq!(depth.parts_for(10), 6);
        assert_eq!(depth.parts_for(21), 13);
        assert_eq!(depth.parts_for(1000), 50);
    }

    #[test]
    fn depth_rejects_bad_bounds() {
        assert!(ReadingDepth::new(0, Some(10), 30).is_err());
        assert!(ReadingDepth::new(5, Some(4), 30).is_err());
        assert!(ReadingDepth::new(1, Some(4), 0).is_err());
        assert!(ReadingDepth::new(1, Some(4), 101).is_err());
        let depth = ReadingDepth::new(2, Some(4), 100).unwrap();
        assert_eq!(depth.parts_for(10), 4);
        assert_eq!(depth.parts_for(2), 2);
    }

    #[test]
    fn depth_without_max_reads_by_percent_only() {
        let depth = ReadingDepth::new(5, None, 30).unwrap();
        assert_eq!(depth.parts_for(1000), 300);
        assert_eq!(depth.parts_for(10), 5, "минимум держится");
        assert_eq!(ReadingDepth::new(1, None, 100).unwrap().parts_for(400), 400);
    }

    #[test]
    fn app_keeps_one_depth_for_all_projects() {
        let dir = tempfile::tempdir().unwrap();
        let (state, _) = project(dir.path(), "http://127.0.0.1:9");
        let glossary = super::super::GlossaryService { state: &state };
        assert_eq!(glossary.reading_depth().unwrap(), ReadingDepth::default());
        let depth = ReadingDepth::new(1, Some(3), 100).unwrap();
        glossary.save_reading_depth(&depth).unwrap();
        glossary.save_reading_depth(&depth).unwrap();
        assert_eq!(glossary.reading_depth().unwrap(), depth);
        let unlimited = ReadingDepth::new(2, None, 40).unwrap();
        glossary.save_reading_depth(&unlimited).unwrap();
        assert_eq!(ReadingDepth::load(&state).unwrap(), unlimited, "пустой максимум читается как «без предела»");
    }
}
