<script lang="ts">
  import { onMount, tick } from "svelte";
  import { Channel } from "@tauri-apps/api/core";
  import {
    ArrowLeft,
    Bot,
    Check,
    CheckCheck,
    Eraser,
    FileCode,
    FileText,
    History,
    Languages,
    LoaderCircle,
    Package,
    RefreshCw,
    SpellCheck,
    Square,
    TriangleAlert,
    Undo2,
    Waypoints,
    X,
  } from "lucide-svelte";
  import Glossary from "./Glossary.svelte";
  import ModPanel from "./ModPanel.svelte";
  import ScanPanel from "./ScanPanel.svelte";
  import SceneGraph from "./SceneGraph.svelte";
  import SourceViewer from "./SourceViewer.svelte";
  import { progressLabel, progressPercent } from "./progress";
  import type { GraphScene } from "$lib/scene-graph";
  import {
    commands,
    type EntryLineDto_Serialize as EntryLine,
    type FileDto,
    type LineDto_Serialize as Line,
    type LineRunDto_Serialize as LineRun,
    type ProjectDto_Serialize as Project,
    type RunProgress_Deserialize as RunProgress,
    type SceneDto_Serialize as Scene,
    type ScopeDto,
  } from "$lib/bindings";

  let {
    project,
    onBack,
    onSettings,
    onProject,
  }: { project: Project; onBack: () => void; onSettings: () => void; onProject: (next: Project) => void } = $props();

  type Filter = "all" | "empty" | "draft";

  let files = $state<FileDto[]>([]);
  let fileId = $state<number | null>(null);
  let scenes = $state<Scene[]>([]);
  let loadingLines = $state(false);
  let error = $state("");
  let filter = $state<Filter>("all");
  let editing = $state<{ id: number; text: string } | null>(null);
  let saving = $state(false);
  let view = $state<"table" | "graph" | "glossary">("table");
  /** Карточка мода под шапкой. */
  let modOpen = $state(false);
  /** Карточка версии игры: считывание, смена папки, «Пропавшие». */
  let scanOpen = $state(false);
  /** Сцена в центре графа: переживает переключение вида. */
  let graphFocus = $state<string | null>(null);
  /** Открытый исходник: файл снимка и строка. */
  let viewing = $state<{ file: number; line: number | null } | null>(null);
  /** Реплика, к которой перешли из глоссария: строка отмечена, пока не выбран другой файл. */
  let highlighted = $state<number | null>(null);

  type RunMode = "translate" | "review";
  /** Запуск модели из этого окна: перевод или редактура. */
  let run = $state<{ mode: RunMode; progress: RunProgress; stopping: boolean } | null>(null);
  /** Итог последнего запуска. */
  let notice = $state("");
  /** Запуск из итога: его можно откатить кнопкой в баннере. */
  let lastRun = $state<number | null>(null);
  /** Меню «Запуски»: `null`, пока закрыто. */
  let runs = $state<LineRun[] | null>(null);
  /** Что ждёт подтверждения: откат запуска, очистка файла или сцены. */
  let confirming = $state<{ kind: "rollback"; run: number } | { kind: "file" } | { kind: "scene"; key: string } | null>(null);
  let working = $state(false);

  const file = $derived(files.find((item) => item.id === fileId) ?? null);
  const fileLines = $derived(scenes.flatMap((scene) => scene.lines));
  const fileEmpty = $derived(fileLines.filter((line) => line.state === "empty").length);
  const fileReady = $derived(fileLines.filter((line) => line.state !== "empty").length);
  const visible = (line: Line) =>
    filter === "all" || (filter === "empty" ? line.state === "empty" : line.state === "draft");
  const shown = $derived(
    scenes.map((scene) => ({ ...scene, lines: scene.lines.filter(visible) })).filter((scene) => scene.lines.length > 0),
  );
  const order = $derived(shown.flatMap((scene) => scene.lines.map((line) => line.id)));
  const fileDrafts = $derived(scenes.flatMap((scene) => scene.lines).filter((line) => line.state === "draft"));

  /** Последний открытый файл проекта: после перезагрузки страница вернётся к нему. */
  const fileKey = $derived(`translator.file.${project.id}`);

  onMount(async () => {
    await loadFiles();
    let saved: number | null = null;
    try {
      saved = Number(sessionStorage.getItem(fileKey)) || null;
    } catch {
      saved = null;
    }
    const first = files.find((item) => item.id === saved) ?? files[0];
    if (first) await selectFile(first.id);
  });

  async function loadFiles() {
    try {
      files = await commands.translationFiles(project.id);
    } catch (e) {
      error = String(e);
    }
  }

  /** После считывания: файлы и реплики могли смениться, открытый файл мог исчезнуть. */
  async function reloadSnapshot() {
    editing = null;
    // Изменённый файл получает новую строку: его находят по пути.
    const path = file?.path;
    await loadFiles();
    const next = files.find((item) => item.id === fileId) ?? files.find((item) => item.path === path) ?? files[0];
    if (next) await selectFile(next.id);
    else {
      fileId = null;
      scenes = [];
    }
  }

  async function selectFile(id: number) {
    if (editing) await save();
    fileId = id;
    editing = null;
    highlighted = null;
    loadingLines = true;
    try {
      sessionStorage.setItem(fileKey, String(id));
    } catch {
      // Без хранилища файл просто не запомнится.
    }
    try {
      scenes = await commands.fileLines(project.id, id);
      error = "";
    } catch (e) {
      error = String(e);
      scenes = [];
    } finally {
      loadingLines = false;
    }
  }

  function replace(line: Line) {
    for (const scene of scenes) {
      const index = scene.lines.findIndex((item) => item.id === line.id);
      if (index >= 0) scene.lines[index] = line;
    }
  }

  async function edit(line: Line) {
    if (editing?.id === line.id) return;
    if (editing) await save();
    editing = { id: line.id, text: line.text };
    await tick();
    const area = document.querySelector<HTMLTextAreaElement>(`textarea[data-line="${line.id}"]`);
    area?.focus();
    area?.select();
  }

  /** Сохраняет правку. `next`: открыть следующую видимую реплику. */
  async function save(next = false) {
    if (!editing || saving) return;
    const { id, text } = editing;
    const nextId = next ? order[order.indexOf(id) + 1] : undefined;
    saving = true;
    try {
      replace(await commands.saveLine(project.id, id, text));
      // Пока шло сохранение по уходу фокуса, пользователь мог открыть другую реплику: её не закрываем.
      if (editing?.id === id) editing = null;
      error = "";
      await loadFiles();
    } catch (e) {
      error = String(e);
      return;
    } finally {
      saving = false;
    }
    const nextLine = nextId === undefined ? null : scenes.flatMap((scene) => scene.lines).find((line) => line.id === nextId);
    if (nextLine) await edit(nextLine);
  }

  async function confirm(lines: Line[]) {
    if (lines.length === 0 || fileId === null) return;
    try {
      await commands.confirmLines(project.id, lines.map((line) => line.id));
      scenes = await commands.fileLines(project.id, fileId);
      await loadFiles();
    } catch (e) {
      error = String(e);
    }
  }

  let refreshing = false;
  let refreshAgain = false;

  /** Перечитывает реплики файла и прогресс без крутилки: во время запуска черновики появляются по пакетам. */
  async function refresh() {
    if (refreshing) {
      refreshAgain = true;
      return;
    }
    refreshing = true;
    try {
      do {
        refreshAgain = false;
        if (fileId !== null) scenes = await commands.fileLines(project.id, fileId);
        await loadFiles();
      } while (refreshAgain);
    } catch (e) {
      error = String(e);
    } finally {
      refreshing = false;
    }
  }

  /** Запускает перевод или редактуру. Черновики пишутся по пакетам, итог показывается баннером. */
  async function startRun(mode: RunMode, scope: ScopeDto) {
    if (run) return;
    if (editing) await save();
    notice = "";
    lastRun = null;
    confirming = null;
    error = "";
    run = { mode, progress: { done: 0, total: 0, written: 0, message: null }, stopping: false };
    const channel = new Channel<RunProgress>();
    let written = 0;
    channel.onmessage = (progress) => {
      if (!run) return;
      run.progress = progress;
      if (progress.written > written) {
        written = progress.written;
        refresh();
      }
    };
    try {
      const result =
        mode === "translate"
          ? await commands.translateLines(project.id, scope, null, channel)
          : await commands.reviewLines(project.id, scope, null, channel);
      notice = resultText(mode, result.written, run?.stopping ?? false);
      lastRun = result.written > 0 ? result.run : null;
    } catch (e) {
      error = String(e);
    } finally {
      run = null;
      await refresh();
      if (runs) await loadRuns();
    }
  }

  async function loadRuns() {
    try {
      runs = await commands.lineRuns(project.id);
    } catch (e) {
      error = String(e);
    }
  }

  function toggleRuns() {
    if (runs) runs = null;
    else loadRuns();
  }

  /** Откат запуска после подтверждения. */
  async function rollback(id: number) {
    working = true;
    try {
      const { rolled, left } = await commands.rollbackRun(project.id, id);
      notice = left > 0 ? `Откачено реплик: ${rolled}. Не тронуто: ${left}, их правили после запуска` : `Откачено реплик: ${rolled}`;
      if (lastRun === id) lastRun = null;
      confirming = null;
      await refresh();
      if (runs) await loadRuns();
    } catch (e) {
      error = String(e);
    } finally {
      working = false;
    }
  }

  /** Очистка перевода файла или сцены после подтверждения. */
  async function reset(scope: ScopeDto) {
    if (editing) await save();
    working = true;
    try {
      const cleared = await commands.resetLines(project.id, scope);
      notice = `Очищено реплик: ${cleared}`;
      lastRun = null;
      confirming = null;
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      working = false;
    }
  }

  const runActions: Record<string, string> = { lines: "Перевод", review: "Редактура" };
  const runStatuses: Record<LineRun["status"], string> = { running: "идёт", done: "готово", cancelled: "остановлен", failed: "ошибка" };

  function runWhen(seconds: number) {
    return new Date(seconds * 1000).toLocaleString("ru", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  }

  /** Непустые реплики сцены: их очистит очистка сцены. */
  function sceneReady(key: string) {
    return scenes.find((item) => item.key === key)?.lines.filter((line) => line.state !== "empty").length ?? 0;
  }

  function resultText(mode: RunMode, count: number, stopped: boolean) {
    const prefix = stopped ? "Остановлено. " : "";
    if (mode === "translate") {
      return count > 0 ? `${prefix}Переведено реплик: ${count}. Черновики ждут проверки` : `${prefix}Модель не перевела ни одной реплики`;
    }
    return count > 0 ? `${prefix}Исправлено реплик: ${count}. Исправления ждут проверки` : `${prefix}Модель не нашла ошибок`;
  }

  async function stopRun() {
    if (!run) return;
    run.stopping = true;
    try {
      await commands.cancelLlmRun();
    } catch (e) {
      error = String(e);
    }
  }

  function runText(current: NonNullable<typeof run>) {
    const { done, total, written, message } = current.progress;
    const action = current.mode === "translate" ? "Перевод" : "Редактура";
    if (current.stopping) return `${action}: останавливается после текущего запроса`;
    if (total === 0) return `${action}: подготовка`;
    const parts = [`${action}: запрос ${Math.min(done + 1, total)} из ${total}`, `записано: ${written}`];
    if (message) parts.push(message);
    return parts.join(" · ");
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      save(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      editing = null;
    }
  }

  /** Из графа в таблицу: файл сцены, все реплики, прокрутка к сцене. */
  async function openScene(scene: GraphScene) {
    view = "table";
    filter = "all";
    graphFocus = scene.key;
    if (scene.file === null) return;
    if (scene.file !== fileId) await selectFile(scene.file);
    await tick();
    const header = [...document.querySelectorAll<HTMLElement>("h3.scene")].find((item) => item.dataset.scene === scene.key);
    header?.scrollIntoView({ block: "start" });
  }

  /** Реплика из глоссария: строка таблицы с отметкой или исходник на её строке. */
  async function openLine(found: EntryLine, where: "table" | "source") {
    if (where === "source") {
      const number = Number.parseInt(found.line.location ?? "", 10);
      viewing = { file: found.file, line: Number.isFinite(number) && number > 0 ? number : null };
      return;
    }
    view = "table";
    filter = "all";
    if (found.file !== fileId) await selectFile(found.file);
    highlighted = found.line.id;
    await tick();
    document.querySelector<HTMLElement>(`[data-row="${found.line.id}"]`)?.scrollIntoView({ block: "center" });
  }

  /** Исходник файла таблицы на строке реплики. */
  function viewSource(line: string | null | undefined) {
    if (fileId === null) return;
    const number = Number.parseInt(line ?? "", 10);
    viewing = { file: fileId, line: Number.isFinite(number) && number > 0 ? number : null };
  }

  /** Из таблицы в граф: сцена в центре. */
  function showOnGraph(key: string | null) {
    if (!key) return;
    graphFocus = key;
    view = "graph";
  }

  function sceneName(scene: Scene) {
    if (!scene.key) return "Вне сцен";
    const name = scene.key.replace(/^[a-z]+:/, "");
    return scene.title ? `${scene.title} · ${name}` : name;
  }

  const stateLabel: Record<Line["state"], string> = {
    empty: "Пусто",
    draft: "Модель",
    done: "Готово",
    game: "Из игры",
  };
</script>

<main class="workspace">
  <header class="bar">
    <button class="btn small" onclick={onBack}>
      <ArrowLeft size={15} />
      <span>Проекты</span>
    </button>
    <div class="title">
      <strong>{project.title}</strong>
      <span>
        {#if !project.gameRoot}
          Папка игры не привязана
        {:else if project.gameMissing}
          <TriangleAlert size={11} /> Папки нет на диске: {project.gameRoot}
        {:else}
          {project.gameRoot}
        {/if}
      </span>
    </div>
    <div class="segmented view-switch" role="tablist" aria-label="Вид">
      <button role="tab" aria-selected={view === "table"} class:on={view === "table"} onclick={() => (view = "table")}>Таблица</button>
      <button role="tab" aria-selected={view === "graph"} class:on={view === "graph"} onclick={() => (view = "graph")}>Граф</button>
      <button role="tab" aria-selected={view === "glossary"} class:on={view === "glossary"} onclick={() => (view = "glossary")}>Глоссарий</button>
    </div>
    <button class="btn small" class:active={scanOpen} onclick={() => (scanOpen = !scanOpen)} title="Считать новую версию игры, сменить папку, пропавшие реплики">
      <RefreshCw size={15} />
      <span>Версия игры</span>
    </button>
    <button class="btn small" class:active={modOpen} onclick={() => (modOpen = !modOpen)} title="Собрать мод перевода и поставить его в игру">
      <Package size={15} />
      <span>Мод</span>
    </button>
    <button class="btn small" onclick={onSettings} title="Провайдеры, модели и настройки действий">
      <Bot size={15} />
      <span>Модели</span>
    </button>
  </header>

  {#if scanOpen}
    <ScanPanel {project} {onProject} onClose={() => (scanOpen = false)} onChanged={reloadSnapshot} />
  {/if}
  {#if modOpen}
    <ModPanel {project} onClose={() => (modOpen = false)} />
  {/if}

  {#if view === "graph"}
    <div class="body">
      <SceneGraph projectId={project.id} focus={graphFocus} onFocus={(key) => (graphFocus = key)} onOpen={openScene} />
    </div>
  {:else if view === "glossary"}
    <div class="body">
      <Glossary projectId={project.id} onOpenLine={openLine} />
    </div>
  {:else}
  <div class="body">
    <nav class="files" aria-label="Файлы">
      {#each files as item (item.id)}
        <button class="file" class:active={item.id === fileId} onclick={() => selectFile(item.id)} title={item.path}>
          <span class="file-name"><FileText size={13} /> {item.path.replace(/^game\//, "")}</span>
          <span class="file-progress">
            <span class="bar-track"><span class="bar-fill" style:width="{progressPercent(item.translated, item.total)}%"></span></span>
            <span class="file-count">{item.translated}/{item.total}{item.drafts ? ` · модель: ${item.drafts}` : ""}</span>
          </span>
        </button>
      {:else}
        <p class="muted">В снимке нет реплик</p>
      {/each}
    </nav>

    <section class="lines">
      {#if file}
        <div class="toolbar">
          <div class="toolbar-title">
            <strong>{file.path}</strong>
            <span>{file.translated} из {file.total} · {progressLabel(file.translated, file.total)}</span>
          </div>
          <div class="segmented" role="tablist" aria-label="Какие реплики показать">
            {#each [["all", "Все"], ["empty", "Пустые"], ["draft", "Модель"]] as [value, label] (value)}
              <button role="tab" aria-selected={filter === value} class:on={filter === value} onclick={() => (filter = value as Filter)}>
                {label}
              </button>
            {/each}
          </div>
          {#if fileDrafts.length > 0}
            <button class="btn small warning" onclick={() => confirm(fileDrafts)}>
              <CheckCheck size={14} />
              <span>Подтвердить модель · {fileDrafts.length}</span>
            </button>
          {/if}
          <button class="btn small" title="Текст файла с подсветкой и поиском" onclick={() => viewSource(null)}>
            <FileCode size={14} />
            <span>Исходник</span>
          </button>
          <button class="btn small" title="Последние запуски перевода и редактуры: их можно откатить" onclick={toggleRuns}>
            <History size={14} />
            <span>Запуски</span>
          </button>
          <button
            class="btn small"
            disabled={run !== null || working || fileReady === 0}
            title={fileReady === 0 ? "В файле нет переведённых реплик" : "Очистить перевод всех реплик файла: модель переведёт их заново"}
            onclick={() => (confirming = { kind: "file" })}
          >
            <Eraser size={14} />
            <span>Очистить</span>
          </button>
          <button
            class="btn small"
            disabled={run !== null || fileReady === 0}
            title={run ? "Модель уже работает" : fileReady === 0 ? "В файле нет переведённых реплик" : "Модель вычитает переведённые реплики файла"}
            onclick={() => fileId !== null && startRun("review", { kind: "file", file: fileId })}
          >
            {#if run?.mode === "review"}<LoaderCircle size={14} class="spin" />{:else}<SpellCheck size={14} />{/if}
            <span>Вычитать</span>
          </button>
          <button
            class="btn small primary"
            disabled={run !== null || fileEmpty === 0}
            title={run ? "Модель уже работает" : fileEmpty === 0 ? "В файле нет пустых реплик" : "Модель переведёт пустые реплики файла"}
            onclick={() => fileId !== null && startRun("translate", { kind: "file", file: fileId })}
          >
            {#if run?.mode === "translate"}<LoaderCircle size={14} class="spin" />{:else}<Languages size={14} />{/if}
            <span>Перевести{fileEmpty ? ` · ${fileEmpty}` : ""}</span>
          </button>
        </div>
      {/if}

      {#if run}
        <div class="banner progress page-error" role="status">
          <LoaderCircle size={14} class="spin" />
          <span>{runText(run)}</span>
          <button class="btn small" disabled={run.stopping} onclick={stopRun}>
            <Square size={13} />
            <span>Остановить</span>
          </button>
        </div>
      {:else if notice}
        <div class="banner page-error" role="status">
          <Check size={14} />
          <span>{notice}</span>
          {#if lastRun !== null}
            <button class="btn small" disabled={working} title="Вернуть реплики к виду до запуска" onclick={() => lastRun !== null && (confirming = { kind: "rollback", run: lastRun })}>
              <Undo2 size={13} />
              <span>Откатить</span>
            </button>
          {/if}
          <button class="icon-button" aria-label="Скрыть итог" title="Скрыть итог" onclick={() => ((notice = ""), (lastRun = null))}><X size={14} /></button>
        </div>
      {/if}

      {#if confirming}
        <div class="banner warn choice page-error" role="alertdialog">
          <TriangleAlert size={14} />
          <span>
            {#if confirming.kind === "rollback"}
              Откатить запуск? Реплики, которые не правили после него, вернутся к виду до запуска
            {:else if confirming.kind === "file"}
              Очистить перевод файла? Реплик с переводом: {fileReady}. Модель переведёт их заново, история правок останется
            {:else}
              Очистить перевод сцены? Реплик с переводом: {sceneReady(confirming.key)}. Модель переведёт их заново, история правок останется
            {/if}
          </span>
          <div class="choice-actions">
            <button class="btn small" disabled={working} onclick={() => (confirming = null)}>Отмена</button>
            <button
              class="btn small danger"
              disabled={working}
              onclick={() => {
                if (!confirming) return;
                if (confirming.kind === "rollback") rollback(confirming.run);
                else if (confirming.kind === "file") fileId !== null && reset({ kind: "file", file: fileId });
                else reset({ kind: "scenes", keys: [confirming.key] });
              }}
            >
              {#if working}<LoaderCircle size={13} class="spin" />{/if}
              <span>{confirming.kind === "rollback" ? "Откатить" : "Очистить"}</span>
            </button>
          </div>
        </div>
      {/if}

      {#if runs}
        <div class="card runs page-error">
          <div class="runs-head">
            <strong>Запуски перевода и редактуры</strong>
            <button class="icon-button" aria-label="Закрыть запуски" title="Закрыть" onclick={() => (runs = null)}><X size={14} /></button>
          </div>
          {#each runs as item (item.id)}
            <div class="run-row">
              <span class="run-when">{runWhen(item.startedAt)}</span>
              <span class="run-what">{runActions[item.action] ?? item.action} · {item.model}</span>
              <span class="run-state" class:failed={item.status === "failed"} title={item.error ?? ""}>{runStatuses[item.status]} · реплик: {item.lines}</span>
              <button
                class="btn small"
                disabled={working || run !== null || item.status === "running" || item.lines === 0}
                title={item.lines === 0 ? "Запуск ничего не записал" : "Вернуть реплики к виду до запуска"}
                onclick={() => (confirming = { kind: "rollback", run: item.id })}
              >
                <Undo2 size={13} />
                <span>Откатить</span>
              </button>
            </div>
          {:else}
            <p class="muted">Запусков ещё не было</p>
          {/each}
        </div>
      {/if}

      {#if error}
        <div class="banner error page-error" role="alert">
          <TriangleAlert size={14} />
          <span>{error}</span>
          <button class="icon-button" aria-label="Скрыть ошибку" onclick={() => (error = "")}><X size={14} /></button>
        </div>
      {/if}

      {#if loadingLines}
        <div class="empty"><LoaderCircle size={20} class="spin" /></div>
      {:else if file && shown.length === 0}
        <div class="empty">{filter === "all" ? "В файле нет реплик" : "Таких реплик нет"}</div>
      {:else}
        <div class="table">
          {#each shown as scene, index (scene.key ?? `outside-${index}`)}
            <h3 class="scene" data-scene={scene.key}>
              <span>{sceneName(scene)}</span>
              {#if scene.key}
                <button class="icon-button" aria-label="Показать на графе" title="Показать на графе" onclick={() => showOnGraph(scene.key)}>
                  <Waypoints size={13} />
                </button>
                <button class="icon-button" aria-label="Сцена в исходнике" title="Сцена в исходнике" onclick={() => viewSource(scene.lines[0]?.location)}>
                  <FileCode size={13} />
                </button>
                {#if run === null && scenes.find((item) => item.key === scene.key)?.lines.some((line) => line.state === "empty")}
                  <button
                    class="icon-button"
                    aria-label="Перевести пустые реплики сцены"
                    title="Перевести пустые реплики сцены"
                    onclick={() => scene.key && startRun("translate", { kind: "scenes", keys: [scene.key] })}
                  >
                    <Languages size={13} />
                  </button>
                {/if}
                {#if run === null && !working && sceneReady(scene.key) > 0}
                  <button
                    class="icon-button"
                    aria-label="Очистить перевод сцены"
                    title="Очистить перевод сцены: модель переведёт её заново"
                    onclick={() => scene.key && (confirming = { kind: "scene", key: scene.key })}
                  >
                    <Eraser size={13} />
                  </button>
                {/if}
              {/if}
            </h3>
            {#each scene.lines as line (line.id)}
              <div class="row" class:editing={editing?.id === line.id} class:highlighted={highlighted === line.id} data-row={line.id}>
                {#if line.location}
                  <button class="where" title="Строка {line.location} в исходнике" onclick={() => viewSource(line.location)}>{line.location}</button>
                {:else}
                  <span class="where"></span>
                {/if}
                <span class="speaker" title={line.speaker ?? "Рассказчик"}>{line.speaker ?? ""}</span>
                <span class="source">{line.sourceText}</span>
                <div class="target">
                  {#if editing?.id === line.id}
                    <textarea
                      data-line={line.id}
                      rows="2"
                      bind:value={editing.text}
                      disabled={saving}
                      onkeydown={onKey}
                      onblur={() => save()}
                      aria-label="Перевод"
                    ></textarea>
                  {:else}
                    <button class="text" class:placeholder={!line.text} onclick={() => edit(line)}>
                      {line.text || "Нет перевода"}
                    </button>
                  {/if}
                </div>
                <span class="state {line.state}">
                  {stateLabel[line.state]}
                  {#if line.sourceChanged}
                    <span class="changed" title="Игра изменила оригинал после перевода"><TriangleAlert size={11} /></span>
                  {/if}
                  {#if line.state === "draft"}
                    <button class="confirm" aria-label="Подтвердить перевод модели" title="Подтвердить перевод модели" onclick={() => confirm([line])}>
                      <Check size={12} />
                    </button>
                  {/if}
                </span>
              </div>
            {/each}
          {/each}
        </div>
      {/if}
    </section>
  </div>
  {/if}
</main>

{#if viewing}
  <SourceViewer projectId={project.id} files={files.map((item) => ({ id: item.id, path: item.path }))} file={viewing.file} line={viewing.line} onClose={() => (viewing = null)} />
{/if}

<style>
  .workspace {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
    background: var(--surface);
  }

  .view-switch {
    margin-left: auto;
  }

  .title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title strong {
    color: var(--text-strong);
    font-size: 14px;
    font-weight: 600;
  }

  .title span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
    color: var(--text-faint);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .files {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 260px;
    flex: 0 0 auto;
    overflow-y: auto;
    padding: 10px 8px;
    border-right: 1px solid var(--divider);
    background: var(--surface-side);
  }

  .file {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: var(--text-soft);
    text-align: left;
  }

  .file:hover,
  .file.active {
    border-color: var(--border-selected);
    background: var(--hover);
  }

  .file-name {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-progress {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .bar-track {
    flex: 1;
    height: 4px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--track);
  }

  .bar-fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }

  .file-count {
    color: var(--text-faint);
    font-size: 10px;
    white-space: nowrap;
  }

  .lines {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
  }

  .toolbar-title {
    display: flex;
    flex: 1 1 160px;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .toolbar-title strong {
    overflow: hidden;
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toolbar-title span {
    white-space: nowrap;
  }

  .toolbar-title span,
  .muted {
    color: var(--text-faint);
    font-size: 11px;
  }

  .page-error {
    margin: 10px 16px 0;
  }

  .empty {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    color: var(--text-faint);
    font-size: 12px;
  }

  .table {
    flex: 1;
    overflow-y: auto;
    padding: 0 16px 24px;
  }

  .scene {
    position: sticky;
    top: 0;
    z-index: 1;
    margin: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 10px 0 4px;
    background: var(--bg);
    color: var(--accent-text);
    font-size: 12px;
    font-weight: 600;
  }

  .row {
    display: grid;
    grid-template-columns: 44px 120px minmax(0, 1fr) minmax(0, 1fr) 96px;
    gap: 10px;
    align-items: start;
    padding: 6px;
    border-bottom: 1px solid var(--divider-row);
    font-size: 12px;
    line-height: 1.45;
  }

  .row:hover,
  .row.editing {
    background: var(--hover-row);
  }

  .row.highlighted {
    background: var(--selected);
  }

  .where {
    padding: 2px 0 0;
    border: 0;
    background: none;
    color: var(--text-ghost);
    font: inherit;
    font-size: 10px;
    text-align: right;
  }

  button.where {
    align-self: start;
    cursor: pointer;
  }

  button.where:hover {
    color: var(--accent-text);
    text-decoration: underline;
  }

  .speaker {
    overflow: hidden;
    color: var(--text-soft);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source {
    color: var(--text-soft);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .target {
    min-width: 0;
  }

  .text {
    display: block;
    width: 100%;
    min-height: 20px;
    margin: -3px 0;
    padding: 3px 6px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    text-align: left;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .text:hover {
    border-color: var(--border);
  }

  .text.placeholder {
    color: var(--text-ghost);
    font-style: italic;
  }

  textarea {
    box-sizing: border-box;
    width: 100%;
    margin: -3px 0;
    padding: 3px 6px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    outline: 0;
    background: var(--bg-sunken);
    color: var(--text-strong);
    font: inherit;
    line-height: 1.45;
    resize: vertical;
    field-sizing: content;
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    justify-self: start;
    color: var(--text-faint);
    font-size: 11px;
  }

  .state.done {
    color: var(--success);
  }

  .state.draft {
    color: var(--warning);
  }

  .state.game {
    color: var(--accent-text);
  }

  .changed {
    display: inline-flex;
    color: var(--warning-strong);
  }

  .confirm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--warning-border);
    border-radius: 6px;
    background: transparent;
    color: var(--warning);
  }

  .confirm:hover {
    background: var(--warning-bg);
  }

  .choice-actions {
    display: flex;
    gap: 6px;
  }

  .runs {
    gap: 4px;
    max-height: 280px;
    overflow: auto;
  }

  .runs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 4px;
    font-size: 12px;
  }

  .run-row {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr) auto auto;
    gap: 12px;
    align-items: center;
    padding: 4px 0;
    border-top: 1px solid var(--divider-row);
    font-size: 12px;
  }

  .run-when,
  .run-state {
    color: var(--text-faint);
    font-size: 11px;
  }

  .run-what {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .run-state.failed {
    color: var(--danger-banner);
  }
</style>
