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
    History,
    Languages,
    LoaderCircle,
    Package,
    PanelLeft,
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
  import ReadingDepthSettings from "./ReadingDepthSettings.svelte";
  import ScanPanel from "./ScanPanel.svelte";
  import SceneCard from "./SceneCard.svelte";
  import SceneGraph from "./SceneGraph.svelte";
  import SourceViewer from "./SourceViewer.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { WriteRun } from "./suggest-run.svelte";
  import { progressLabel, progressPercent } from "./progress";
  import { fileName, filterScenes, pickId, sceneItemName, sceneList, type ScenePick } from "./scene-tree";
  import { sceneLabel, type GraphScene } from "$lib/scene-graph";
  import {
    commands,
    type EntryLineDto_Serialize as EntryLine,
    type FileScenesDto_Serialize as FileScenes,
    type LineDto_Serialize as Line,
    type LineRunDto_Serialize as LineRun,
    type ProjectDto_Serialize as Project,
    type RunProgress_Deserialize as RunProgress,
    type SceneDto_Serialize as Scene,
    type SceneItemDto_Serialize as SceneItem,
    type ScopeDto,
  } from "$lib/bindings";

  let {
    project,
    onBack,
    onSettings,
    onProject,
  }: { project: Project; onBack: () => void; onSettings: () => void; onProject: (next: Project) => void } = $props();

  type Filter = "all" | "empty" | "draft";

  /** Файлы с прогрессом и их сцены. */
  let tree = $state<FileScenes[]>([]);
  let loadedTree = $state(false);
  /** Открытая сцена или «Без сцены» файла. */
  let selected = $state<ScenePick | null>(null);
  /** Реплики открытой сцены: одна группа, у «Без сцены» по группе на контейнер. */
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
  /** Реплика, к которой перешли из глоссария: строка отмечена, пока не выбрана другая сцена. */
  let highlighted = $state<number | null>(null);
  let query = $state("");
  let depthOpen = $state(false);
  /** К чему относятся «Перевести», «Вычитать» и «Очистить»: к открытой сцене или ко всему её файлу. */
  let scope = $state<"scene" | "file">("scene");
  /** Панель с описанием сцены слева от реплик: открыта, пока её не скрыли. Помнится до закрытия окна. */
  let sideOpen = $state(true);
  const SIDE_KEY = "translator.scene-side";

  function setSide(open: boolean) {
    sideOpen = open;
    try {
      sessionStorage.setItem(SIDE_KEY, open ? "open" : "closed");
    } catch {
      // Без хранилища выбор просто не запомнится.
    }
  }
  /** Описание сцен моделью: открытая сцена или все сцены её файла. */
  const describe = new WriteRun("Описания");

  type RunMode = "translate" | "review";
  /** Запуск перевода или редактуры из этого окна. */
  let run = $state<{ mode: RunMode; progress: RunProgress; stopping: boolean } | null>(null);
  /** Итог последнего запуска. */
  let notice = $state("");
  /** Запуск из итога: его можно откатить кнопкой в баннере. */
  let lastRun = $state<number | null>(null);
  /** Меню «Запуски»: `null`, пока закрыто. */
  let runs = $state<LineRun[] | null>(null);
  /** Что ждёт подтверждения: откат запуска или очистка файла или сцены. */
  let confirming = $state<{ kind: "rollback"; run: number } | { kind: "reset"; scope: ScopeDto; what: string; count: number } | null>(null);
  let working = $state(false);

  /** Модель занята: перевод, редактура или описание. Второй запуск Rust всё равно не даст. */
  const busy = $derived(run !== null || describe.active);
  const group = $derived.by(() => {
    const pick = selected;
    if (!pick) return null;
    const file = fileOf(pick);
    return tree.find((item) => item.file.id === file) ?? null;
  });
  const item = $derived(selected && group ? (group.scenes.find((scene) => pickId(asPick(scene, group.file.id)) === pickId(selected!)) ?? null) : null);
  const sceneLines = $derived(scenes.flatMap((scene) => scene.lines));
  const sceneEmpty = $derived(sceneLines.filter((line) => line.state === "empty").length);
  const sceneReady = $derived(sceneLines.filter((line) => line.state !== "empty").length);
  const sceneDrafts = $derived(sceneLines.filter((line) => line.state === "draft"));
  const visible = (line: Line) => filter === "all" || (filter === "empty" ? line.state === "empty" : line.state === "draft");
  const shown = $derived(
    scenes.map((scene) => ({ ...scene, lines: scene.lines.filter(visible) })).filter((scene) => scene.lines.length > 0),
  );
  const order = $derived(shown.flatMap((scene) => scene.lines.map((line) => line.id)));
  const list = $derived(sceneList(tree));
  const found = $derived(filterScenes(list, query));
  /** Сцены файла открытой сцены: их описывает «Описать сцены файла». */
  const fileKeys = $derived(group ? group.scenes.flatMap((scene) => (scene.key ? [scene.key] : [])) : []);
  /** Счётчики действий по выбранной области: у сцены по загруженным репликам, у файла по прогрессу. */
  const actionEmpty = $derived(scope === "file" ? (group ? group.file.total - group.file.translated : 0) : sceneEmpty);
  const actionReady = $derived(scope === "file" ? (group?.file.translated ?? 0) : sceneReady);
  const actionScope = $derived<ScopeDto | null>(
    !selected ? null : scope === "file" ? (group ? { kind: "file", file: group.file.id } : null) : scopeOf(selected),
  );
  const scopeWord = $derived(scope === "file" ? "файла" : "сцены");
  /** Карточка описания есть только у настоящей сцены. */
  const described = $derived(selected?.kind === "scene" ? (scenes[0] ?? null) : null);

  /** Последняя открытая сцена проекта: после перезагрузки страница вернётся к ней. */
  const pickKey = $derived(`translator.scene.${project.id}`);

  function fileOf(pick: ScenePick) {
    if (pick.kind === "loose") return pick.file;
    return tree.find((item) => item.scenes.some((scene) => scene.key === pick.key))?.file.id ?? null;
  }

  function asPick(scene: SceneItem, file: number): ScenePick {
    return scene.key ? { kind: "scene", key: scene.key } : { kind: "loose", file };
  }

  function scopeOf(pick: ScenePick): ScopeDto {
    return pick.kind === "scene" ? { kind: "scenes", keys: [pick.key] } : { kind: "loose", file: pick.file };
  }

  onMount(async () => {
    try {
      sideOpen = sessionStorage.getItem(SIDE_KEY) !== "closed";
    } catch {
      sideOpen = true;
    }
    await loadTree();
    let saved: ScenePick | null = null;
    try {
      saved = JSON.parse(sessionStorage.getItem(pickKey) ?? "null");
    } catch {
      saved = null;
    }
    const next = (saved && exists(saved) ? saved : null) ?? firstPick();
    if (next) await select(next);
  });

  async function loadTree() {
    try {
      tree = await commands.translationScenes(project.id);
    } catch (e) {
      error = String(e);
    } finally {
      loadedTree = true;
    }
  }

  function exists(pick: ScenePick) {
    if (pick.kind === "loose") return tree.some((item) => item.file.id === pick.file && item.scenes.some((scene) => scene.key === null));
    return fileOf(pick) !== null;
  }

  function firstPick(): ScenePick | null {
    const first = tree.find((item) => item.scenes.length > 0);
    return first ? asPick(first.scenes[0], first.file.id) : null;
  }

  /** После считывания: сцены и реплики могли смениться, открытая сцена могла исчезнуть. */
  async function reloadSnapshot() {
    editing = null;
    await loadTree();
    const next = (selected && exists(selected) ? selected : null) ?? firstPick();
    if (next) await select(next);
    else {
      selected = null;
      scenes = [];
    }
  }

  /** Номер загрузки реплик: ответ для сцены, которую уже сменили, не показывается. */
  let loadToken = 0;

  async function select(pick: ScenePick) {
    if (editing) await save();
    const token = ++loadToken;
    selected = pick;
    editing = null;
    highlighted = null;
    try {
      sessionStorage.setItem(pickKey, JSON.stringify(pick));
    } catch {
      // Без хранилища сцена просто не запомнится.
    }
    loadingLines = true;
    try {
      const found = await commands.sceneLines(project.id, pick);
      // Пока шла загрузка, могли открыть другую сцену.
      if (token === loadToken) scenes = found;
      error = "";
    } catch (e) {
      error = String(e);
      scenes = [];
    } finally {
      if (token === loadToken) loadingLines = false;
    }
    // Сцену могли открыть из графа или глоссария: список показывает её.
    await tick();
    document.querySelector(`[data-pick="${pickId(pick)}"]`)?.scrollIntoView({ block: "nearest" });
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
      await loadTree();
    } catch (e) {
      error = String(e);
      return;
    } finally {
      saving = false;
    }
    const nextLine = nextId === undefined ? null : sceneLines.find((line) => line.id === nextId);
    if (nextLine) await edit(nextLine);
  }

  async function confirm(lines: Line[]) {
    if (lines.length === 0) return;
    try {
      await commands.confirmLines(project.id, lines.map((line) => line.id));
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  let refreshing = false;
  let refreshAgain = false;

  /** Перечитывает реплики сцены и прогресс без крутилки: во время запуска черновики появляются по пакетам. */
  async function refresh() {
    if (refreshing) {
      refreshAgain = true;
      return;
    }
    refreshing = true;
    try {
      do {
        refreshAgain = false;
        const token = loadToken;
        const pick = selected ? $state.snapshot(selected) : null;
        if (pick) {
          const found = await commands.sceneLines(project.id, pick);
          if (token === loadToken) scenes = found;
        }
        await loadTree();
      } while (refreshAgain);
    } catch (e) {
      error = String(e);
    } finally {
      refreshing = false;
    }
  }

  /** Запускает перевод или редактуру. Черновики пишутся по пакетам, итог показывается баннером. */
  async function startRun(mode: RunMode, scope: ScopeDto) {
    if (busy) return;
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

  /** Из графа в таблицу: та же сцена. Сцены без реплик в таблице нет. */
  async function openScene(scene: GraphScene) {
    view = "table";
    filter = "all";
    graphFocus = scene.key;
    const pick: ScenePick = { kind: "scene", key: scene.key };
    if (exists(pick)) await select(pick);
    else error = `У сцены ${sceneLabel(scene.key)} нет реплик`;
  }

  /** Реплика из глоссария: её сцена с отметкой строки или исходник на её строке. */
  async function openLine(found: EntryLine, where: "table" | "source") {
    if (where === "source") {
      const number = Number.parseInt(found.line.location ?? "", 10);
      viewing = { file: found.file, line: Number.isFinite(number) && number > 0 ? number : null };
      return;
    }
    view = "table";
    filter = "all";
    const scene: ScenePick | null = found.scene ? { kind: "scene", key: found.scene } : null;
    await select(scene && exists(scene) ? scene : { kind: "loose", file: found.file });
    highlighted = found.line.id;
    await tick();
    document.querySelector<HTMLElement>(`[data-row="${found.line.id}"]`)?.scrollIntoView({ block: "center" });
  }

  /** Исходник файла на строке реплики. */
  function viewSource(file: number | null, line: string | null | undefined) {
    if (file === null) return;
    const number = Number.parseInt(line ?? "", 10);
    viewing = { file, line: Number.isFinite(number) && number > 0 ? number : null };
  }

  /** Из таблицы в граф: сцена в центре. */
  function showOnGraph(key: string) {
    graphFocus = key;
    view = "graph";
  }

  function describedNotice(written: number, stopped: boolean) {
    const text = `Описано сцен: ${written}`;
    return stopped ? `Остановлено. ${text}` : text;
  }

  /** Все сцены файла с глубиной чтения проекта. Описанные сцены без новых частей пропускаются. */
  async function describeFile() {
    const keys = [...fileKeys];
    if (keys.length === 0) return;
    await describe.start((channel) => commands.describeScenes(project.id, keys, null, channel), (message) => (error = message), describedNotice);
    await refresh();
  }

  async function describeOne(key: string) {
    await describe.start((channel) => commands.describeScene(project.id, key, null, channel), (message) => (error = message), describedNotice);
    await refresh();
  }

  /** Правка названия и описания из карточки: список и карточка показывают новое без перечитывания. */
  function patchScene(key: string, title: string | null, description: string | null) {
    for (const scene of scenes) {
      if (scene.key === key) {
        scene.title = title;
        scene.description = description;
      }
    }
    for (const item of tree) {
      for (const scene of item.scenes) {
        if (scene.key === key) {
          scene.title = title;
          scene.described = description !== null;
        }
      }
    }
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
    <nav class="scenes" aria-label="Сцены">
      <div class="scenes-head">
        <input class="input" placeholder="Поиск сцены или файла" bind:value={query} aria-label="Поиск сцен" />
      </div>
      <div class="scene-list">
        {#each found as entry (entry.id)}
          <button
            class="scene"
            class:active={selected !== null && pickId(selected) === entry.id}
            class:loose={entry.pick.kind === "loose"}
            data-pick={entry.id}
            title={entry.pick.kind === "loose" ? `${entry.file.path}: строки интерфейса, экраны, код вне меток` : `${entry.scene.key} · ${entry.file.path}`}
            onclick={() => select(entry.pick)}
          >
            <span class="scene-name">{entry.name}</span>
            <span class="scene-percent" class:done={entry.scene.total > 0 && entry.scene.translated >= entry.scene.total}
              >{progressLabel(entry.scene.translated, entry.scene.total)}</span
            >
          </button>
        {:else}
          <p class="muted">{!loadedTree ? "Загрузка" : tree.length ? "Ничего не нашлось" : "В снимке нет реплик"}</p>
        {/each}
      </div>
    </nav>

    <section class="lines">
      {#if selected && group}
        <div class="toolbar">
          <div class="toolbar-title">
            <strong>{item ? sceneItemName(item) : selected.kind === "scene" ? sceneLabel(selected.key) : "Без сцены"}</strong>
            <span>
              {fileName(group.file.path)}{#if item} · {item.translated} из {item.total} · {progressLabel(item.translated, item.total)}{/if}{#if item?.stale}
                · <span class="stale">описание устарело</span>{/if}
            </span>
          </div>
          {#if selected.kind === "scene"}
            {@const key = selected.key}
            <button
              class="icon-button"
              class:active={sideOpen}
              aria-pressed={sideOpen}
              aria-label="Описание сцены"
              title={sideOpen ? "Скрыть описание сцены" : "Показать описание сцены"}
              onclick={() => setSide(!sideOpen)}><PanelLeft size={15} /></button
            >
            <button class="icon-button" aria-label="Показать на графе" title="Показать на графе" onclick={() => showOnGraph(key)}><Waypoints size={15} /></button>
          {/if}
          <button class="icon-button" aria-label="Сцена в исходнике" title="Сцена в исходнике" onclick={() => viewSource(group?.file.id ?? null, sceneLines[0]?.location)}
            ><FileCode size={15} /></button
          >
          <button
            class="icon-button"
            class:active={runs !== null}
            aria-label="Запуски"
            title="Последние запуски перевода и редактуры: их можно откатить"
            onclick={toggleRuns}><History size={15} /></button
          >
        </div>

        <div class="actions" role="toolbar" aria-label="Действия с репликами">
          <div class="segmented" role="radiogroup" aria-label="Область действий">
            <button role="radio" aria-checked={scope === "scene"} class:on={scope === "scene"} onclick={() => (scope = "scene")}>Сцена</button>
            <button role="radio" aria-checked={scope === "file"} class:on={scope === "file"} title="Действия для всех реплик файла {group.file.path}" onclick={() => (scope = "file")}
              >Файл</button
            >
          </div>
          <div class="segmented" role="tablist" aria-label="Какие реплики показать">
            {#each [["all", "Все"], ["empty", "Пустые"], ["draft", "Модель"]] as [value, label] (value)}
              <button role="tab" aria-selected={filter === value} class:on={filter === value} onclick={() => (filter = value as Filter)}>
                {label}
              </button>
            {/each}
          </div>
          <span class="actions-gap"></span>
          {#if sceneDrafts.length > 0}
            <button class="btn small warning" title="Подтвердить все черновики модели в сцене" onclick={() => confirm(sceneDrafts)}>
              <CheckCheck size={14} />
              <span>Подтвердить · {sceneDrafts.length}</span>
            </button>
          {/if}
          <button
            class="btn small"
            disabled={busy || working || actionReady === 0 || !actionScope}
            title={actionReady === 0 ? `В ${scope === "file" ? "файле" : "сцене"} нет переведённых реплик` : `Очистить перевод ${scopeWord}: модель переведёт его заново`}
            onclick={() => actionScope && (confirming = { kind: "reset", scope: actionScope, what: scopeWord, count: actionReady })}
          >
            <Eraser size={14} />
            <span>Очистить</span>
          </button>
          <button
            class="btn small"
            disabled={busy || actionReady === 0 || !actionScope}
            title={busy ? "Модель уже работает" : actionReady === 0 ? `В ${scope === "file" ? "файле" : "сцене"} нет переведённых реплик` : `Модель вычитает переведённые реплики ${scopeWord}`}
            onclick={() => actionScope && startRun("review", actionScope)}
          >
            {#if run?.mode === "review"}<LoaderCircle size={14} class="spin" />{:else}<SpellCheck size={14} />{/if}
            <span>Вычитать</span>
          </button>
          <button
            class="btn small primary"
            disabled={busy || actionEmpty === 0 || !actionScope}
            title={busy ? "Модель уже работает" : actionEmpty === 0 ? `В ${scope === "file" ? "файле" : "сцене"} нет пустых реплик` : `Модель переведёт пустые реплики ${scopeWord}`}
            onclick={() => actionScope && startRun("translate", actionScope)}
          >
            {#if run?.mode === "translate"}<LoaderCircle size={14} class="spin" />{:else}<Languages size={14} />{/if}
            <span>Перевести{actionEmpty ? ` · ${actionEmpty}` : ""}</span>
          </button>
        </div>
      {/if}

      {#if depthOpen}<div class="page-error"><ReadingDepthSettings projectId={project.id} onClose={() => (depthOpen = false)} /></div>{/if}
      <SuggestStatus run={describe} onError={(message) => (error = message)} />

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
            {:else}
              Очистить перевод {confirming.what}? Реплик с переводом: {confirming.count}. Модель переведёт их заново, история правок останется
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
                else reset(confirming.scope);
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
          {#each runs as entry (entry.id)}
            <div class="run-row">
              <span class="run-when">{runWhen(entry.startedAt)}</span>
              <span class="run-what">{runActions[entry.action] ?? entry.action} · {entry.model}</span>
              <span class="run-state" class:failed={entry.status === "failed"} title={entry.error ?? ""}>{runStatuses[entry.status]} · реплик: {entry.lines}</span>
              <button
                class="btn small"
                disabled={working || busy || entry.status === "running" || entry.lines === 0}
                title={entry.lines === 0 ? "Запуск ничего не записал" : "Вернуть реплики к виду до запуска"}
                onclick={() => (confirming = { kind: "rollback", run: entry.id })}
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

      {#if !selected}
        <div class="empty">{loadedTree ? "Выберите сцену в списке слева" : ""}</div>
      {:else if loadingLines}
        <div class="empty"><LoaderCircle size={20} class="spin" /></div>
      {:else}
        <div class="content">
          {#if described?.key && sideOpen}
            <aside class="scene-side" aria-label="Описание сцены">
              <SceneCard
                projectId={project.id}
                scene={described}
                sceneKey={described.key}
                stale={item?.stale ?? false}
                {busy}
                describing={describe.active}
                fileScenes={fileKeys.length}
                {depthOpen}
                onDescribe={describeOne}
                onDescribeFile={describeFile}
                onDepth={() => (depthOpen = !depthOpen)}
                onSaved={patchScene}
                onClose={() => setSide(false)}
                onError={(message) => (error = message)}
              />
            </aside>
          {/if}
        <div class="table">
          {#if shown.length === 0}
            <div class="no-lines">{filter === "all" ? "В сцене нет реплик" : "Таких реплик нет"}</div>
          {/if}
          {#each shown as scene, index (scene.key ?? `outside-${index}`)}
            {#if selected.kind === "loose" && shown.length > 1}
              <h3 class="subgroup">{scene.key ?? "Вне контейнеров"}</h3>
            {/if}
            {#each scene.lines as line (line.id)}
              <div class="row" class:editing={editing?.id === line.id} class:highlighted={highlighted === line.id} data-row={line.id}>
                {#if line.location}
                  <button class="where" title="Строка {line.location} в исходнике" onclick={() => viewSource(group?.file.id ?? null, line.location)}>{line.location}</button>
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
        </div>
      {/if}
    </section>
  </div>
  {/if}
</main>

{#if viewing}
  <SourceViewer projectId={project.id} files={tree.map((entry) => ({ id: entry.file.id, path: entry.file.path }))} file={viewing.file} line={viewing.line} onClose={() => (viewing = null)} />
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

  .scenes {
    display: flex;
    flex-direction: column;
    width: 260px;
    flex: 0 0 auto;
    min-height: 0;
    border-right: 1px solid var(--divider);
    background: var(--surface-side);
  }

  .scenes-head {
    display: flex;
    padding: 10px 8px 8px;
  }

  .scenes-head .input {
    flex: 1;
    min-width: 0;
  }

  .scene-list {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    overflow-y: auto;
    padding: 0 8px 12px;
  }

  .scene {
    display: flex;
    flex: 0 0 auto;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--text-soft);
    font-size: 12px;
    text-align: left;
  }

  .scene:hover {
    background: var(--hover);
  }

  .scene.active {
    border-color: var(--border-selected);
    background: var(--selected);
    color: var(--text-strong);
  }

  .scene.loose {
    color: var(--text-muted);
    font-style: italic;
  }

  .scene-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scene-percent {
    flex: 0 0 auto;
    color: var(--text-ghost);
    font-size: 10px;
    font-style: normal;
  }

  .scene-percent.done {
    color: var(--success);
  }

  .lines {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .content {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .scene-side {
    width: 320px;
    flex: 0 0 auto;
    overflow-y: auto;
    padding: 12px 14px;
    border-right: 1px solid var(--divider);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--divider);
    background: var(--surface-side);
  }

  .actions-gap {
    flex: 1;
  }

  .stale {
    color: var(--warning);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
  }

  .toolbar-title {
    display: flex;
    flex: 1 1 120px;
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toolbar-title span,
  .muted {
    color: var(--text-faint);
    font-size: 11px;
  }

  /* Баннеры таблицы, и баннер описаний из `SuggestStatus` тоже. */
  .lines :global(.page-error) {
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

  .no-lines {
    padding: 24px 0;
    color: var(--text-faint);
    font-size: 12px;
    text-align: center;
  }

  .table {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px 24px;
  }

  .subgroup {
    position: sticky;
    top: -12px;
    z-index: 1;
    margin: 0;
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

  /* На узком окне три колонки делят место: список и панель сужаются, реплики остаются читаемыми. */
  @media (max-width: 1180px) {
    .scenes {
      width: 220px;
    }

    .scene-side {
      width: 260px;
    }

    .row {
      grid-template-columns: 36px 84px minmax(0, 1fr) minmax(0, 1fr) 76px;
      gap: 8px;
    }
  }
</style>
