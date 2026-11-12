//! Повторное считывание и пересборка снимка.

use super::{EdgeRepo, FileId, MatchSegment, Matcher, MissingSegment, ScanReport, ScanRepo, SegmentId};
use crate::common::{AppError, ValidationError};
use crate::engines::{self, GameEngine, GameSnapshot, GameSource, SourceText, TextVariable};
use crate::projects::{GameRoot, ProjectId, ProjectInfoRepo, ProjectRepo};
use crate::state::{AppState, OpenProject};
use crate::storage::DbError;

pub struct ScanService<'a> {
    pub state: &'a AppState,
}

impl ScanService<'_> {
    /// Считывает игру из привязанной папки и переносит перевод на новую версию.
    /// Под `snapshot_lock`: чтение вне транзакции, запись снимка и переносов одной транзакцией, потом уборка `files/`.
    pub fn rescan(&self, project: ProjectId) -> Result<ScanReport, AppError> {
        let stored = self.state.app.read(|tx| ProjectRepo { conn: tx }.get(project))?;
        let root = stored.game_root().cloned().ok_or(AppError::Unbound)?;
        if !root.as_path().is_dir() {
            return Err(AppError::GameMissing);
        }
        let (engine, info) = engines::detect(&root).ok_or(AppError::UnknownGame)?;
        if engine.id() != stored.engine() {
            return Err(ValidationError::OtherEngine.into());
        }
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        // Долго и вне транзакции: файлы пишутся в files/ по мере чтения.
        let snapshot = engine.read(&GameSource::Folder(root.clone()), &info, &open.files)?;
        Self::apply(&open, engine, &snapshot, Some(&root))
    }

    /// Текст файла текущего снимка для просмотра исходника. Папка игры не нужна.
    pub fn source(&self, project: ProjectId, file: FileId) -> Result<(String, SourceText), AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let (info, path, files) = open.project.read(|tx| {
            let repo = ScanRepo { conn: tx };
            Ok((ProjectInfoRepo { conn: tx }.get()?, repo.current_path(file)?, repo.current_files()?))
        })?;
        let path = path.ok_or(AppError::Db(DbError::NotFound))?;
        let current = files.iter().find(|item| item.path == path).ok_or(AppError::Db(DbError::NotFound))?;
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        // Долго для .rpyc перевода: декомпилируются все скрипты. Вне транзакции.
        Ok((path, engine.source_text(current, &files, &open.files)?))
    }

    /// Что скрипты текущего снимка говорят о переменных игры `names`: запись на имя, в том же порядке. Папка
    /// игры не нужна. Считается при каждом вызове и долго: читаются все скрипты, `.rpyc` декомпилируются.
    /// Принимает открытый проект, а не `self`: шаг токенов зовёт это из потока для блокирующей работы.
    pub fn text_variables(open: &OpenProject, names: &[String]) -> Result<Vec<TextVariable>, AppError> {
        let _lock = open.snapshot_lock.lock();
        let (info, files) = open.project.read(|tx| Ok((ProjectInfoRepo { conn: tx }.get()?, ScanRepo { conn: tx }.current_files()?)))?;
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        Ok(engine.text_variables(names, &files, &open.files)?)
    }

    /// Пересобирает снимок из `files/` последнего считывания, без папки игры. `GameInfo` берётся из того же считывания.
    pub fn rebuild_from_files(&self, project: ProjectId) -> Result<ScanReport, AppError> {
        self.rebuild(project, true)?.ok_or(AppError::Db(DbError::NotFound))
    }

    /// Пересобирает снимок, если его разобрала прошлая версия движка (`parser_version`). `None`: снимок свежий.
    pub fn rebuild_if_stale(&self, project: ProjectId) -> Result<Option<ScanReport>, AppError> {
        self.rebuild(project, false)
    }

    /// Пересобирает отставшие производные таблицы. Это `edges`; cache.db собирает шаг поиска.
    pub fn refresh_derived(&self, project: ProjectId) -> Result<(), AppError> {
        self.refresh_edges(project)?;
        Ok(())
    }

    /// Пересобирает `edges`, если они собраны по прошлому считыванию или старой версией движка.
    /// Нужны только файлы снимка, папка игры не нужна. Возвращает, была ли пересборка.
    pub fn refresh_edges(&self, project: ProjectId) -> Result<bool, AppError> {
        let open = self.state.open_project(project)?;
        // Уборка files/ не удалит файлы, которые сейчас читаются.
        let _lock = open.snapshot_lock.lock();
        let (info, scan, state, files) = open.project.read(|tx| {
            let scans = ScanRepo { conn: tx };
            Ok((ProjectInfoRepo { conn: tx }.get()?, scans.latest()?, EdgeRepo { conn: tx }.state()?, scans.current_files()?))
        })?;
        let Some(scan) = scan else { return Ok(false) };
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        if state == Some((engine.edges_version(), scan.id)) {
            return Ok(false);
        }
        let edges = engine.read_edges(&files, &open.files)?;
        open.project.write(|tx| EdgeRepo { conn: tx }.replace(&edges, engine.edges_version(), scan.id))?;
        Ok(true)
    }

    pub fn missing(&self, project: ProjectId) -> Result<Vec<MissingSegment>, AppError> {
        let open = self.state.open_project(project)?;
        Ok(open.project.read(|tx| ScanRepo { conn: tx }.missing())?)
    }

    /// Переносит перевод пропавшей реплики на текущую руками. Пишет строку `transfer`.
    pub fn attach_missing(&self, project: ProjectId, missing: SegmentId, target: SegmentId) -> Result<(), AppError> {
        let open = self.state.open_project(project)?;
        open.project.write(|tx| ScanRepo { conn: tx }.attach_missing(missing, target)).map_err(|error| match error {
            DbError::NotFound => AppError::NothingToAttach,
            other => other.into(),
        })
    }

    /// Сопоставляет считанный снимок с текущим и пишет его. Вызывающий держит `snapshot_lock`.
    /// Чистка заметок и уборка `files/` идут после фиксации: их сбой не отменяет считывание.
    pub(crate) fn apply(
        open: &OpenProject,
        engine: &dyn GameEngine,
        snapshot: &GameSnapshot,
        root: Option<&GameRoot>,
    ) -> Result<ScanReport, AppError> {
        let old = open.project.read(|tx| ScanRepo { conn: tx }.current_segments())?;
        let new: Vec<MatchSegment<usize>> = snapshot
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| MatchSegment {
                id: index,
                key: segment.key.clone(),
                container: segment.container.clone(),
                file: segment.file.clone(),
                source_hash: segment.source_hash.clone(),
                source_text: segment.source_text.clone(),
                story_order: segment.story_order,
            })
            .collect();
        let plan = Matcher::plan(&old, &new);
        let report = open.project.write(|tx| {
            ScanRepo { conn: tx }.apply(snapshot, &plan, root, engine.parser_version(), engine.edges_version())
        })?;
        // Части сцен зависят от реплик: заметки к частям, которых больше нет, не нужны.
        if let Err(error) = open.project.write(|tx| crate::glossary::prune_notes(tx)) {
            tracing::warn!("устаревшие заметки проекта {} не удалены: {error}", open.id);
        }
        Self::clean_files(open);
        Ok(report)
    }

    /// Удаляет из `files/` всё, на что не ссылается проект. Вызывающий держит `snapshot_lock`.
    pub(crate) fn clean_files(open: &OpenProject) {
        let cleaned = open
            .project
            .read(|tx| ScanRepo { conn: tx }.referenced_hashes())
            .map_err(AppError::from)
            .and_then(|keep| Ok(open.files.retain(&keep)?));
        if let Err(error) = cleaned {
            tracing::warn!("уборка files/ проекта {} не удалась: {error}", open.id);
        }
    }

    fn rebuild(&self, project: ProjectId, always: bool) -> Result<Option<ScanReport>, AppError> {
        let open = self.state.open_project(project)?;
        let _lock = open.snapshot_lock.lock();
        let (info, scan, files) = open.project.read(|tx| {
            let scans = ScanRepo { conn: tx };
            Ok((ProjectInfoRepo { conn: tx }.get()?, scans.latest()?, scans.current_files()?))
        })?;
        let Some(scan) = scan else { return Ok(None) };
        let engine = engines::by_id(info.engine).ok_or(AppError::UnsupportedEngine(info.engine))?;
        if !always && scan.parser_version >= engine.parser_version() {
            return Ok(None);
        }
        let snapshot = engine.read(&GameSource::Stored(files), &scan.game_info, &open.files)?;
        Self::apply(&open, engine, &snapshot, None).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use parking_lot::Mutex;
    use rusqlite::OptionalExtension;

    use super::*;
    use crate::projects::{ProjectService, RebindCheck};
    use crate::secrets::SecretStore;
    use crate::translation::TranslationService;

    const SCRIPT: &str = r#"define e = Character("Eileen")

label start:
    e "Hello."
    "Narration."
    "Radio Chatter" "Static."
    e "Hello."
    jump left

label left:
    e "Left side."
    return
"#;

    /// Опечатку исправили, «Static.» убрали, вставили реплику, метку `left` переименовали.
    const SCRIPT_V2: &str = r#"define e = Character("Eileen")

label start:
    e "Hello."
    "Narration!"
    e "Brand new line."
    e "Hello."
    jump west

label west:
    e "Left side."
    return
"#;

    fn state(dir: &Path) -> AppState {
        AppState::start(dir.to_path_buf(), SecretStore::Memory(Mutex::new(HashMap::new()))).unwrap()
    }

    fn game(root: &Path, script: &str) -> GameRoot {
        let root = crate::engines::renpy::snapshot::tests::sample_game(root);
        std::fs::write(root.as_path().join("game").join("script.rpy"), script).unwrap();
        root
    }

    /// Проект по игре, открытый.
    fn project(state: &AppState, root: &GameRoot) -> ProjectId {
        let projects = ProjectService { state };
        let id = projects.create(root.clone()).unwrap().id();
        projects.open(id).unwrap();
        id
    }

    /// Строка текущей реплики с таким текстом. У повторов первая по сюжету.
    fn line(state: &AppState, project: ProjectId, text: &str) -> SegmentId {
        let open = state.open_project(project).unwrap();
        open.project
            .read(|tx| {
                Ok(SegmentId::from_row(tx.query_row(
                    "SELECT id FROM segments WHERE missing = 0 AND source_text = ?1 ORDER BY story_order, id LIMIT 1",
                    [text],
                    |row| row.get(0),
                )?))
            })
            .unwrap()
    }

    /// Перевод реплики: текст и черновик.
    fn translation(state: &AppState, project: ProjectId, line: SegmentId) -> Option<(String, bool)> {
        let open = state.open_project(project).unwrap();
        open.project
            .read(|tx| {
                Ok(tx
                    .query_row("SELECT text, draft FROM translations WHERE segment_id = ?1", [line.get()], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .optional()?)
            })
            .unwrap()
    }

    fn scalar(state: &AppState, project: ProjectId, sql: &str) -> i64 {
        let open = state.open_project(project).unwrap();
        open.project.read(|tx| Ok(tx.query_row(sql, [], |row| row.get(0))?)).unwrap()
    }

    #[test]
    fn rescan_carries_translations_to_the_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let root = game(&dir.path().join("Game"), SCRIPT);
        let id = project(&state, &root);
        let translations = TranslationService { state: &state };
        for (text, translated) in [("Hello.", "Привет."), ("Narration.", "Рассказ."), ("Static.", "Помехи."), ("Left side.", "Левая сторона.")] {
            translations.save_text(id, line(&state, id, text), translated).unwrap();
        }
        let open = state.open_project(id).unwrap();
        open.project
            .write(|tx| {
                tx.execute(
                    "INSERT INTO scene_descriptions (container_key, title, updated_at) VALUES ('label:left', 'Налево', 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        std::fs::write(root.as_path().join("game").join("script.rpy"), SCRIPT_V2).unwrap();
        let scans = ScanService { state: &state };
        let report = scans.rescan(id).unwrap();
        assert_eq!(report.lines, 5);
        assert_eq!(report.new_lines, 1);
        assert_eq!(report.missing, 1);
        assert!(report.drafts >= 1);

        // Повтор «Hello.» потерял соседа «Narration.»: его перевод переносится черновиком.
        assert_eq!(translation(&state, id, line(&state, id, "Hello.")), Some(("Привет.".into(), true)));
        assert_eq!(translation(&state, id, line(&state, id, "Narration!")), Some(("Рассказ.".into(), true)));
        assert_eq!(translation(&state, id, line(&state, id, "Left side.")), Some(("Левая сторона.".into(), false)));
        assert_eq!(translation(&state, id, line(&state, id, "Brand new line.")), None);
        assert_eq!(
            scalar(&state, id, "SELECT count(*) FROM scene_descriptions WHERE container_key = 'label:west' AND title = 'Налево'"),
            1
        );
        // История идёт за переводом.
        let left = line(&state, id, "Left side.");
        let history = scalar(&state, id, &format!("SELECT count(*) FROM revisions WHERE segment_id = {}", left.get()));
        assert!(history >= 1);

        // «Static.» пропала вместе с переводом, его можно привязать руками.
        let missing = scans.missing(id).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].source_text, "Static.");
        assert_eq!(missing[0].translation.as_deref(), Some("Помехи."));
        let target = line(&state, id, "Brand new line.");
        scans.attach_missing(id, missing[0].id, target).unwrap();
        assert_eq!(translation(&state, id, target), Some(("Помехи.".into(), false)));
        assert!(scans.missing(id).unwrap().is_empty());
        assert!(matches!(scans.attach_missing(id, missing[0].id, target), Err(AppError::NothingToAttach)));

        // Повторное считывание той же версии ничего не меняет.
        let again = scans.rescan(id).unwrap();
        assert_eq!((again.new_lines, again.transferred, again.drafts, again.missing), (0, 0, 0, 0));
        assert_eq!(scalar(&state, id, "SELECT count(*) FROM segments"), 5);
        assert_eq!(scalar(&state, id, "SELECT count(*) FROM containers"), 2);
        assert_eq!(scalar(&state, id, "SELECT count(*) FROM files WHERE path = 'game/script.rpy'"), 1);
        let referenced = open.project.read(|tx| ScanRepo { conn: tx }.referenced_hashes()).unwrap();
        let stored = std::fs::read_dir(open.dir.join("files")).unwrap().count();
        assert_eq!(stored, referenced.len());
    }

    #[test]
    fn rebuild_from_files_works_without_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let root = game(&dir.path().join("Game"), SCRIPT);
        let id = project(&state, &root);
        TranslationService { state: &state }.save_text(id, line(&state, id, "Narration."), "Рассказ.").unwrap();
        std::fs::remove_dir_all(root.as_path()).unwrap();

        let scans = ScanService { state: &state };
        assert_eq!(scans.rebuild_if_stale(id).unwrap(), None);
        let report = scans.rebuild_from_files(id).unwrap();
        assert_eq!((report.lines, report.new_lines, report.missing), (5, 0, 0));
        assert_eq!(translation(&state, id, line(&state, id, "Narration.")), Some(("Рассказ.".into(), false)));
        assert_eq!(scalar(&state, id, "SELECT count(*) FROM scans WHERE game_root IS NULL"), 1);
        assert!(matches!(scans.rescan(id), Err(AppError::GameMissing)));
    }

    #[test]
    fn source_shows_the_file_lines_point_to_without_the_game() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let root = game(&dir.path().join("Game"), SCRIPT);
        let id = project(&state, &root);
        std::fs::remove_dir_all(root.as_path()).unwrap();

        let (file, location) = state
            .open_project(id)
            .unwrap()
            .project
            .read(|tx| {
                Ok(tx.query_row("SELECT file_id, location FROM segments WHERE source_text = 'Narration.'", [], |row| {
                    Ok((FileId::from_row(row.get(0)?), row.get::<_, String>(1)?))
                })?)
            })
            .unwrap();
        let scans = ScanService { state: &state };
        let (path, source) = scans.source(id, file).unwrap();
        assert_eq!(path, "game/script.rpy");
        assert!(!source.decompiled);
        let line: usize = location.parse().unwrap();
        assert_eq!(source.text.lines().nth(line - 1).map(str::trim), Some("\"Narration.\""), "номер строки реплики указывает на неё");
        assert!(matches!(scans.source(id, FileId::from_row(9999)), Err(AppError::Db(DbError::NotFound))));
    }

    #[test]
    fn rebind_warns_about_another_game_and_rescans_on_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let state = state(&dir.path().join("data"));
        let root = game(&dir.path().join("Game"), SCRIPT);
        let id = project(&state, &root);
        let projects = ProjectService { state: &state };
        TranslationService { state: &state }.save_text(id, line(&state, id, "Left side."), "Левая сторона.").unwrap();

        let other = game(&dir.path().join("Other"), "label start:\n    \"Something else.\"\n    return\n");
        let check = projects.rebind(id, other.clone(), false).unwrap();
        assert!(matches!(check, RebindCheck::LooksLikeOtherGame { shared: 0, total: 5 }), "{check:?}");
        assert_eq!(projects.list().unwrap()[0].game_root(), Some(&root));

        let moved = game(&dir.path().join("Moved"), SCRIPT);
        let RebindCheck::Done { project, report } = projects.rebind(id, moved.clone(), false).unwrap() else {
            panic!("та же игра в другой папке");
        };
        assert_eq!(project.game_root(), Some(&moved));
        assert_eq!((report.new_lines, report.missing), (0, 0));
        assert_eq!(translation(&state, id, line(&state, id, "Left side.")), Some(("Левая сторона.".into(), false)));

        let RebindCheck::Done { report, .. } = projects.rebind(id, other, true).unwrap() else { panic!("подтверждено") };
        assert_eq!((report.lines, report.missing), (1, 1));
    }

    /// Реальная игра из `TRANSLATOR_TEST_RESCAN_GAME` (корень игры): перевод каждой реплики переживает
    /// пересборку из `files/` и повторное считывание той же версии без потерь и черновиков.
    #[test]
    #[ignore]
    fn real_game_source_lines_point_to_their_segments() {
        let Some(path) = std::env::var_os("TRANSLATOR_TEST_RESCAN_GAME") else { return };
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let root = crate::projects::ProjectService::resolve_root(Path::new(&path)).unwrap();
        let id = project(&state, &root);
        let open = state.open_project(id).unwrap();
        let segments: Vec<(i64, String, String, Option<String>)> = open
            .project
            .read(|tx| {
                let mut statement =
                    tx.prepare("SELECT file_id, location, source_text, game_text FROM segments WHERE missing = 0 AND location IS NOT NULL")?;
                let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .unwrap();
        let mut by_file: HashMap<i64, Vec<(String, String, Option<String>)>> = HashMap::new();
        for (file, location, source, game) in segments {
            by_file.entry(file).or_default().push((location, source, game));
        }
        // Текст без экранирования и лишних пробелов: в файле кавычки и переводы строк записаны через `\`,
        // а движок сворачивает пробелы в тексте реплики.
        let plain = |text: &str| text.replace("\\n", " ").replace(['\\', '"', '\n'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
        let head = |text: &str| plain(text).chars().take(20).collect::<String>();
        let scans = ScanService { state: &state };
        let (mut checked, mut wrong) = (0, Vec::new());
        let mut files: Vec<_> = by_file.into_iter().collect();
        files.sort_by_key(|(file, _)| *file);
        for (file, lines) in files {
            let started = std::time::Instant::now();
            let (path, source) = scans.source(id, FileId::from_row(file)).unwrap();
            let elapsed = started.elapsed();
            if elapsed.as_millis() > 500 {
                eprintln!("{path}: {:?}, декомпилирован: {}", elapsed, source.decompiled);
            }
            let text: Vec<String> = source.text.lines().map(plain).collect();
            for (location, original, game) in lines {
                checked += 1;
                let line = location.parse::<usize>().ok().and_then(|number| text.get(number.wrapping_sub(1)));
                let found = line.is_some_and(|line| {
                    line.contains(&head(&original)) || game.as_deref().is_some_and(|game| line.contains(&head(game)))
                });
                if !found {
                    wrong.push(format!("{path}:{location} «{}» → {:?}", head(&original), line));
                }
            }
        }
        eprintln!("строк проверено: {checked}, не совпало: {}", wrong.len());
        for item in wrong.iter().take(20) {
            eprintln!("  {item}");
        }
        assert!(checked > 0);
        assert!(wrong.is_empty(), "номер строки реплики не указывает на неё");
    }

    #[test]
    #[ignore]
    fn real_game_keeps_every_translation_across_scans() {
        let Some(path) = std::env::var_os("TRANSLATOR_TEST_RESCAN_GAME") else { return };
        let dir = tempfile::tempdir().unwrap();
        let state = state(dir.path());
        let root = crate::projects::ProjectService::resolve_root(Path::new(&path)).unwrap();
        let id = project(&state, &root);
        let open = state.open_project(id).unwrap();
        let total = open
            .project
            .write(|tx| {
                Ok(tx.execute(
                    "INSERT INTO translations (segment_id, text, source_hash, updated_at)
                     SELECT id, 'ru:' || id, source_hash, 0 FROM segments WHERE missing = 0",
                    [],
                )?)
            })
            .unwrap() as i64;
        let scans = ScanService { state: &state };
        for name in ["пересборка", "считывание"] {
            let started = std::time::Instant::now();
            let report = if name == "пересборка" { scans.rebuild_from_files(id) } else { scans.rescan(id) }.unwrap();
            eprintln!("{name}: {report:?} за {:?}", started.elapsed());
            assert_eq!((report.new_lines, report.drafts, report.missing), (0, 0, 0), "{name}");
            assert_eq!(report.lines as i64, total, "{name}");
        }
        let kept = scalar(
            &state,
            id,
            "SELECT count(*) FROM translations t JOIN segments s ON s.id = t.segment_id
             WHERE s.missing = 0 AND t.draft = 0 AND t.source_hash = s.source_hash",
        );
        assert_eq!(kept, total);
    }
}
