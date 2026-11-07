<script lang="ts">
  import { onMount, tick } from "svelte";
  import { ArrowLeft, Bot, Check, CheckCheck, FileText, LoaderCircle, TriangleAlert, Waypoints, X } from "lucide-svelte";
  import SceneGraph from "./SceneGraph.svelte";
  import type { GraphScene } from "$lib/scene-graph";
  import {
    commands,
    type FileDto,
    type LineDto_Serialize as Line,
    type ProjectDto_Serialize as Project,
    type SceneDto_Serialize as Scene,
  } from "$lib/bindings";

  let { project, onBack, onSettings }: { project: Project; onBack: () => void; onSettings: () => void } = $props();

  type Filter = "all" | "empty" | "draft";

  let files = $state<FileDto[]>([]);
  let fileId = $state<number | null>(null);
  let scenes = $state<Scene[]>([]);
  let loadingLines = $state(false);
  let error = $state("");
  let filter = $state<Filter>("all");
  let editing = $state<{ id: number; text: string } | null>(null);
  let saving = $state(false);
  let view = $state<"table" | "graph">("table");
  /** Сцена в центре графа: переживает переключение вида. */
  let graphFocus = $state<string | null>(null);

  const file = $derived(files.find((item) => item.id === fileId) ?? null);
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

  async function selectFile(id: number) {
    if (editing) await save();
    fileId = id;
    editing = null;
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

  /** Из таблицы в граф: сцена в центре. */
  function showOnGraph(key: string | null) {
    if (!key) return;
    graphFocus = key;
    view = "graph";
  }

  function percent(item: FileDto) {
    return item.total === 0 ? 0 : Math.round((item.translated / item.total) * 100);
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
    </div>
    <button class="btn small" onclick={onSettings} title="Провайдеры, модели и настройки действий">
      <Bot size={15} />
      <span>Модели</span>
    </button>
  </header>

  {#if view === "graph"}
    <div class="body">
      <SceneGraph projectId={project.id} focus={graphFocus} onFocus={(key) => (graphFocus = key)} onOpen={openScene} />
    </div>
  {:else}
  <div class="body">
    <nav class="files" aria-label="Файлы">
      {#each files as item (item.id)}
        <button class="file" class:active={item.id === fileId} onclick={() => selectFile(item.id)} title={item.path}>
          <span class="file-name"><FileText size={13} /> {item.path.replace(/^game\//, "")}</span>
          <span class="file-progress">
            <span class="bar-track"><span class="bar-fill" style:width="{percent(item)}%"></span></span>
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
            <span>{file.translated} из {file.total} · {percent(file)}%</span>
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
              {/if}
            </h3>
            {#each scene.lines as line (line.id)}
              <div class="row" class:editing={editing?.id === line.id}>
                <span class="where">{line.location ?? ""}</span>
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
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
  }

  .toolbar-title {
    display: flex;
    flex: 1;
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

  .where {
    padding-top: 2px;
    color: var(--text-ghost);
    font-size: 10px;
    text-align: right;
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
</style>
