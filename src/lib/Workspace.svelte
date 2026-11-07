<script lang="ts">
  import { onMount, tick } from "svelte";
  import { ArrowLeft, Bot, Check, CheckCheck, FileText, LoaderCircle, TriangleAlert, X } from "lucide-svelte";
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
    <button class="back" onclick={onBack}>
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
    <button class="back settings" onclick={onSettings} title="Провайдеры, модели и настройки действий">
      <Bot size={15} />
      <span>Модели</span>
    </button>
  </header>

  <div class="body">
    <nav class="files" aria-label="Файлы">
      {#each files as item (item.id)}
        <button class="file" class:active={item.id === fileId} onclick={() => selectFile(item.id)} title={item.path}>
          <span class="file-name"><FileText size={13} /> {item.path.replace(/^game\//, "")}</span>
          <span class="file-progress">
            <span class="bar-track"><span class="bar-fill" style:width="{percent(item)}%"></span></span>
            <span class="file-count">{item.translated}/{item.total}{item.drafts ? ` · ${item.drafts} модель` : ""}</span>
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
          <div class="filters" role="tablist">
            {#each [["all", "Все"], ["empty", "Пустые"], ["draft", "Модель"]] as [value, label] (value)}
              <button role="tab" aria-selected={filter === value} class:on={filter === value} onclick={() => (filter = value as Filter)}>
                {label}
              </button>
            {/each}
          </div>
          {#if fileDrafts.length > 0}
            <button class="confirm-all" onclick={() => confirm(fileDrafts)}>
              <CheckCheck size={14} />
              <span>Подтвердить модель · {fileDrafts.length}</span>
            </button>
          {/if}
        </div>
      {/if}

      {#if error}
        <div class="error" role="alert">
          <TriangleAlert size={14} />
          <span>{error}</span>
          <button aria-label="Скрыть ошибку" onclick={() => (error = "")}><X size={13} /></button>
        </div>
      {/if}

      {#if loadingLines}
        <div class="empty"><LoaderCircle size={20} class="spin" /></div>
      {:else if file && shown.length === 0}
        <div class="empty">{filter === "all" ? "В файле нет реплик" : "Таких реплик нет"}</div>
      {:else}
        <div class="table">
          {#each shown as scene, index (scene.key ?? `outside-${index}`)}
            <h3 class="scene">{sceneName(scene)}</h3>
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
                    <button class="confirm" title="Подтвердить перевод модели" onclick={() => confirm([line])}>
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
</main>

<style>
  .workspace {
    display: flex;
    flex-direction: column;
    height: 100vh;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
  }

  button {
    font: inherit;
    cursor: pointer;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border-bottom: 1px solid #202937;
    background: #11151d;
  }

  .back {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid #34445d;
    border-radius: 8px;
    background: #1a2433;
    color: #d4deeb;
    padding: 6px 10px;
    font-size: 12px;
  }

  .back:hover {
    border-color: #3b82f6;
    background: #20334f;
  }

  .settings {
    margin-left: auto;
  }

  .title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title strong {
    color: #f1f5f9;
    font-size: 14px;
  }

  .title span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    color: #74839b;
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
    border-right: 1px solid #202937;
    background: #0f1319;
  }

  .file {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 9px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: #cbd5e1;
    text-align: left;
  }

  .file:hover {
    background: #161d29;
  }

  .file.active {
    border-color: #273b58;
    background: #182131;
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
    background: #1e2638;
  }

  .bar-fill {
    display: block;
    height: 100%;
    background: #3b82f6;
  }

  .file-count {
    color: #74839b;
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
    border-bottom: 1px solid #202937;
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
    color: #e2e8f0;
    font-size: 13px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .toolbar-title span,
  .muted {
    color: #74839b;
    font-size: 11px;
  }

  .filters {
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid #2a3547;
    border-radius: 8px;
    background: #0c1017;
  }

  .filters button {
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #8492a8;
    padding: 5px 10px;
    font-size: 11px;
  }

  .filters button.on {
    background: #1d2c44;
    color: #e2e8f0;
  }

  .confirm-all {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid #6d4f1c;
    border-radius: 8px;
    background: #2a2111;
    color: #fcd58a;
    padding: 6px 10px;
    font-size: 11px;
  }

  .confirm-all:hover {
    border-color: #d69e2e;
  }

  .error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 10px 16px 0;
    padding: 8px 10px;
    border: 1px solid #7f1d1d;
    border-radius: 8px;
    background: #1f1013;
    color: #fecaca;
    font-size: 12px;
  }

  .error span {
    flex: 1;
  }

  .error button {
    border: 0;
    background: transparent;
    color: inherit;
  }

  .empty {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    color: #74839b;
    font-size: 12px;
  }

  .table {
    flex: 1;
    overflow-y: auto;
    padding: 0 16px 40px;
  }

  .scene {
    position: sticky;
    top: 0;
    z-index: 1;
    margin: 0;
    padding: 14px 0 6px;
    background: #0d0f12;
    color: #93c5fd;
    font-size: 12px;
    font-weight: 600;
  }

  .row {
    display: grid;
    grid-template-columns: 44px 120px minmax(0, 1fr) minmax(0, 1fr) 96px;
    gap: 10px;
    align-items: start;
    padding: 7px 6px;
    border-bottom: 1px solid #161c26;
    font-size: 12px;
    line-height: 1.45;
  }

  .row:hover,
  .row.editing {
    background: #11161f;
  }

  .where {
    color: #4b5870;
    font-size: 10px;
    text-align: right;
    padding-top: 2px;
  }

  .speaker {
    overflow: hidden;
    color: #a5b4cb;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source {
    color: #cbd5e1;
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
    color: #e2e8f0;
    text-align: left;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .text:hover {
    border-color: #2a3547;
  }

  .text.placeholder {
    color: #4b5870;
    font-style: italic;
  }

  textarea {
    box-sizing: border-box;
    width: 100%;
    margin: -3px 0;
    padding: 3px 6px;
    border: 1px solid #3b82f6;
    border-radius: 6px;
    outline: 0;
    background: #0c1017;
    color: #f1f5f9;
    font: inherit;
    line-height: 1.45;
    resize: vertical;
    field-sizing: content;
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    justify-self: start;
    color: #74839b;
    font-size: 11px;
  }

  .state.done {
    color: #6ee7b7;
  }

  .state.draft {
    color: #fcd58a;
  }

  .state.game {
    color: #93c5fd;
  }

  .changed {
    display: inline-flex;
    color: #f59e0b;
  }

  .confirm {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: 1px solid #6d4f1c;
    border-radius: 5px;
    background: transparent;
    color: #fcd58a;
  }

  .confirm:hover {
    background: #2a2111;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
