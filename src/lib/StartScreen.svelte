<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    Archive,
    Bot,
    ArchiveRestore,
    ChevronDown,
    ChevronRight,
    FolderOpen,
    FolderPlus,
    LoaderCircle,
    Pencil,
    Search,
    Trash2,
    TriangleAlert,
    X,
  } from "lucide-svelte";
  import { commands, type ProjectDto_Serialize as Project } from "$lib/bindings";

  let { onOpen, onSettings }: { onOpen: (project: Project) => void; onSettings: () => void } = $props();

  let projects = $state<Project[]>([]);
  let loading = $state(true);
  let error = $state("");
  let search = $state("");
  let showArchive = $state(false);
  /** Папка, которую сейчас считываем: пока она есть, остальные действия недоступны. */
  let creatingRoot = $state<string | null>(null);
  /** Выбранная папка, к которой уже привязаны проекты: окно спрашивает, открыть их или создать ещё один. */
  let existing = $state<{ root: string; projects: Project[] } | null>(null);
  let renaming = $state<{ id: string; title: string } | null>(null);
  let deleting = $state<Project | null>(null);
  let busy = $state(false);

  const query = $derived(search.trim().toLowerCase());
  const matches = (project: Project) =>
    !query ||
    project.title.toLowerCase().includes(query) ||
    (project.gameRoot ?? "").toLowerCase().includes(query);
  const active = $derived(projects.filter((project) => !project.archived && matches(project)));
  const archived = $derived(projects.filter((project) => project.archived && matches(project)));

  onMount(load);

  async function load() {
    loading = true;
    try {
      projects = await commands.listProjects();
      error = "";
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  /** Обёртка действия: ошибка показывается сверху, список после действия перечитывается. */
  async function run(action: () => Promise<unknown>) {
    busy = true;
    try {
      await action();
      error = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
    await load();
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, title: "Корневая папка игры" });
    if (typeof picked !== "string") return;
    error = "";
    try {
      const found = await commands.findProjects(picked);
      if (found.length > 0) {
        existing = { root: picked, projects: found };
        return;
      }
    } catch (e) {
      error = String(e);
      return;
    }
    await create(picked);
  }

  async function create(root: string) {
    existing = null;
    creatingRoot = root;
    try {
      const project = await commands.createProject(root);
      error = "";
      onOpen(await commands.openProject(project.id));
    } catch (e) {
      error = String(e);
      await load();
    } finally {
      creatingRoot = null;
    }
  }

  async function openProject(project: Project) {
    busy = true;
    error = "";
    try {
      onOpen(await commands.openProject(project.id));
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function saveTitle() {
    if (!renaming) return;
    const { id, title } = renaming;
    renaming = null;
    await run(() => commands.renameProject(id, title));
  }

  function place(project: Project): string {
    if (!project.gameRoot) return "Папка игры не привязана";
    return project.gameMissing ? `Папки нет на диске: ${project.gameRoot}` : project.gameRoot;
  }

  function when(project: Project): string {
    const seconds = project.openedAt ?? project.createdAt;
    const date = new Date(seconds * 1000).toLocaleDateString("ru-RU", { day: "numeric", month: "short", year: "numeric" });
    return project.openedAt ? `открыт ${date}` : `создан ${date}`;
  }
</script>

{#snippet row(project: Project)}
  <li class="row" class:missing={project.gameMissing || !project.gameRoot}>
    {#if renaming?.id === project.id}
      <form class="rename" onsubmit={(event) => { event.preventDefault(); saveTitle(); }}>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          bind:value={renaming.title}
          aria-label="Название проекта"
          autofocus
          onkeydown={(event) => { if (event.key === "Escape") renaming = null; }}
        />
        <button type="submit" class="btn small primary" disabled={!renaming.title.trim()}>Сохранить</button>
        <button type="button" class="btn small" onclick={() => (renaming = null)}>Отмена</button>
      </form>
    {:else}
      <button class="open" disabled={busy || creatingRoot !== null} onclick={() => openProject(project)}>
        <span class="icon"><FolderOpen size={17} /></span>
        <span class="info">
          <span class="title">{project.title}</span>
          <span class="path">
            {#if project.gameMissing || !project.gameRoot}<TriangleAlert size={11} />{/if}
            {place(project)}
          </span>
        </span>
        <span class="when">{when(project)}</span>
      </button>
      <div class="actions">
        <button class="icon-button" aria-label="Переименовать" title="Переименовать" onclick={() => (renaming = { id: project.id, title: project.title })}>
          <Pencil size={14} />
        </button>
        {#if project.archived}
          <button class="icon-button" aria-label="Вернуть из архива" title="Вернуть из архива" onclick={() => run(() => commands.unarchiveProject(project.id))}>
            <ArchiveRestore size={14} />
          </button>
        {:else}
          <button class="icon-button" aria-label="В архив" title="В архив" onclick={() => run(() => commands.archiveProject(project.id))}>
            <Archive size={14} />
          </button>
        {/if}
        <button class="icon-button danger" aria-label="Удалить проект" title="Удалить проект" onclick={() => (deleting = project)}>
          <Trash2 size={14} />
        </button>
      </div>
    {/if}
  </li>
{/snippet}

<main class="start">
  <header class="head">
    <div>
      <h1>Проекты</h1>
      <p>Проект один раз считывает игру и дальше работает со своей копией</p>
    </div>
    <div class="head-actions">
      <button class="btn" onclick={onSettings} title="Провайдеры, модели и настройки действий">
        <Bot size={15} />
        <span>Модели</span>
      </button>
      <button class="btn primary" disabled={creatingRoot !== null || busy} onclick={chooseFolder}>
        <FolderPlus size={16} />
        <span>Новый проект</span>
      </button>
    </div>
  </header>

  {#if error}
    <div class="banner error" role="alert">
      <TriangleAlert size={15} />
      <span>{error}</span>
      <button class="icon-button" aria-label="Скрыть ошибку" onclick={() => (error = "")}><X size={14} /></button>
    </div>
  {/if}

  {#if creatingRoot}
    <div class="banner progress" role="status">
      <LoaderCircle size={15} class="spin" />
      <span>Считываю игру: <strong>{creatingRoot}</strong>. Большие игры читаются до минуты.</span>
    </div>
  {/if}

  {#if existing}
    <div class="banner choice">
      <div class="choice-text">
        <strong>К этой папке уже привязан проект</strong>
        <span>{existing.root}</span>
      </div>
      <div class="choice-actions">
        {#each existing.projects as project (project.id)}
          <button class="btn small primary" onclick={() => { existing = null; openProject(project); }}>Открыть «{project.title}»</button>
        {/each}
        <button class="btn small" onclick={() => existing && create(existing.root)}>Создать ещё один</button>
        <button class="btn small" onclick={() => (existing = null)}>Отмена</button>
      </div>
    </div>
  {/if}

  {#if deleting}
    <div class="banner choice danger-zone">
      <div class="choice-text">
        <strong>Удалить проект «{deleting.title}»?</strong>
        <span>Удалятся перевод, глоссарий и копия файлов игры. Папку самой игры это не затронет.</span>
      </div>
      <div class="choice-actions">
        <button
          class="btn small danger"
          onclick={() => {
            const id = deleting?.id;
            deleting = null;
            if (id) run(() => commands.deleteProject(id));
          }}
        >Удалить</button>
        <button class="btn small" onclick={() => (deleting = null)}>Отмена</button>
      </div>
    </div>
  {/if}

  <div class="search">
    <Search size={15} />
    <input bind:value={search} placeholder="Найти проект по названию или папке" aria-label="Поиск проектов" />
    {#if search}
      <button class="icon-button" aria-label="Очистить поиск" onclick={() => (search = "")}><X size={14} /></button>
    {/if}
  </div>

  {#if loading && projects.length === 0}
    <div class="empty"><LoaderCircle size={22} class="spin" /></div>
  {:else if projects.length === 0}
    <div class="empty">
      <FolderOpen size={30} />
      <strong>Проектов пока нет</strong>
      <span>Выберите корневую папку игры: ту, где лежат <code>game</code> и <code>renpy</code></span>
    </div>
  {:else}
    {#if active.length > 0}
      <ul class="list">
        {#each active as project (project.id)}{@render row(project)}{/each}
      </ul>
    {:else if query}
      <div class="empty small-empty"><span>Ничего не найдено</span></div>
    {/if}

    {#if archived.length > 0}
      <button class="archive-toggle" onclick={() => (showArchive = !showArchive)}>
        {#if showArchive}<ChevronDown size={14} />{:else}<ChevronRight size={14} />{/if}
        <span>Архив · {archived.length}</span>
      </button>
      {#if showArchive}
        <ul class="list">
          {#each archived as project (project.id)}{@render row(project)}{/each}
        </ul>
      {/if}
    {/if}
  {/if}
</main>

<style>
  .start {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: min(820px, 100%);
    margin: 0 auto;
    padding: 40px 24px;
    box-sizing: border-box;
  }

  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
  }

  h1 {
    margin: 0;
    color: var(--text-strong);
    font-size: 22px;
    font-weight: 650;
  }

  .head p {
    margin: 6px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .choice-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .choice-text span {
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .choice-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-sunken);
    color: var(--text-faint);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-ring);
  }

  .search input {
    min-width: 0;
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text-strong);
    font: inherit;
    font-size: 12px;
  }

  .search input::placeholder {
    color: var(--text-ghost);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-right: 6px;
    border: 1px solid transparent;
    border-radius: 8px;
  }

  .row:hover {
    border-color: var(--border-selected);
    background: var(--hover);
  }

  .open {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    flex: 1;
    padding: 10px;
    border: 0;
    background: transparent;
    color: inherit;
    text-align: left;
  }

  .icon {
    display: inline-flex;
    color: var(--accent-icon);
  }

  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .title,
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
  }

  .path {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-faint);
    font-size: 11px;
  }

  .row.missing .path {
    color: var(--warning-strong);
  }

  .when {
    flex: 0 0 auto;
    color: var(--text-faint);
    font-size: 11px;
  }

  .actions {
    display: flex;
    gap: 2px;
    opacity: 0;
  }

  .row:hover .actions,
  .row:focus-within .actions {
    opacity: 1;
  }

  .rename {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    padding: 8px 10px;
  }

  .rename input {
    min-width: 0;
    flex: 1;
    padding: 6px 8px;
    border: 1px solid var(--accent);
    border-radius: 6px;
    outline: 0;
    background: var(--bg-sunken);
    color: var(--text-strong);
    font: inherit;
    font-size: 12px;
  }

  .archive-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    margin-top: 8px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
  }

  .archive-toggle:hover {
    color: var(--text);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 16px;
    color: var(--text-faint);
    font-size: 12px;
    text-align: center;
  }

  .empty strong {
    color: var(--text-soft);
    font-size: 14px;
    font-weight: 600;
  }

  .small-empty {
    padding: 24px;
  }
</style>
