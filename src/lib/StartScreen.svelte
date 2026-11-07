<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import {
    Archive,
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

  let { onOpen }: { onOpen: (project: Project) => void } = $props();

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
        <button type="submit" class="small primary" disabled={!renaming.title.trim()}>Сохранить</button>
        <button type="button" class="small" onclick={() => (renaming = null)}>Отмена</button>
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
        <button class="icon-button" title="Переименовать" onclick={() => (renaming = { id: project.id, title: project.title })}>
          <Pencil size={14} />
        </button>
        {#if project.archived}
          <button class="icon-button" title="Вернуть из архива" onclick={() => run(() => commands.unarchiveProject(project.id))}>
            <ArchiveRestore size={14} />
          </button>
        {:else}
          <button class="icon-button" title="В архив" onclick={() => run(() => commands.archiveProject(project.id))}>
            <Archive size={14} />
          </button>
        {/if}
        <button class="icon-button danger" title="Удалить проект" onclick={() => (deleting = project)}>
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
      <p>Проект один раз считывает игру и дальше работает со своей копией.</p>
    </div>
    <button class="primary" disabled={creatingRoot !== null || busy} onclick={chooseFolder}>
      <FolderPlus size={16} />
      <span>Новый проект</span>
    </button>
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
        <strong>К этой папке уже привязан проект.</strong>
        <span>{existing.root}</span>
      </div>
      <div class="choice-actions">
        {#each existing.projects as project (project.id)}
          <button class="small primary" onclick={() => { existing = null; openProject(project); }}>Открыть «{project.title}»</button>
        {/each}
        <button class="small" onclick={() => existing && create(existing.root)}>Создать ещё один</button>
        <button class="small" onclick={() => (existing = null)}>Отмена</button>
      </div>
    </div>
  {/if}

  {#if deleting}
    <div class="banner danger-zone">
      <div class="choice-text">
        <strong>Удалить проект «{deleting.title}»?</strong>
        <span>Удалятся перевод, глоссарий и копия файлов игры. Папку самой игры это не затронет.</span>
      </div>
      <div class="choice-actions">
        <button
          class="small danger-button"
          onclick={() => {
            const id = deleting?.id;
            deleting = null;
            if (id) run(() => commands.deleteProject(id));
          }}
        >Удалить</button>
        <button class="small" onclick={() => (deleting = null)}>Отмена</button>
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
      <span>Выберите корневую папку игры: ту, где лежат <code>game</code> и <code>renpy</code>.</span>
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
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
  }

  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
  }

  h1 {
    margin: 0;
    color: #f1f5f9;
    font-size: 22px;
    font-weight: 650;
  }

  .head p {
    margin: 6px 0 0;
    color: #8492a8;
    font-size: 12px;
  }

  button {
    font: inherit;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .primary,
  .small {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    border: 1px solid #34445d;
    border-radius: 8px;
    background: #1a2433;
    color: #d4deeb;
    padding: 8px 12px;
    font-size: 12px;
    font-weight: 600;
  }

  .primary {
    border-color: #2563eb;
    background: #1d4ed8;
    color: #f8fafc;
  }

  .primary:hover:not(:disabled) {
    background: #2563eb;
  }

  .small {
    padding: 6px 10px;
    font-size: 11px;
  }

  .small:hover:not(:disabled) {
    border-color: #3b82f6;
    background: #20334f;
  }

  .danger-button {
    border-color: #b91c1c;
    background: #7f1d1d;
    color: #fee2e2;
  }

  .danger-button:hover:not(:disabled) {
    border-color: #ef4444;
    background: #991b1b;
  }

  .icon-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #8290a5;
  }

  .icon-button:hover {
    background: #252e3d;
    color: #f1f5f9;
  }

  .icon-button.danger:hover {
    background: #3b1820;
    color: #fca5a5;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid #2a3548;
    border-radius: 10px;
    background: #11151d;
    color: #cbd5e1;
    font-size: 12px;
  }

  .banner.error {
    border-color: #7f1d1d;
    background: #1f1013;
    color: #fecaca;
  }

  .banner.error span {
    flex: 1;
  }

  .banner.progress {
    border-color: #1e3a5f;
    background: #0f1a2a;
    color: #bfdbfe;
  }

  .banner.choice,
  .banner.danger-zone {
    flex-wrap: wrap;
    justify-content: space-between;
  }

  .banner.danger-zone {
    border-color: #7f1d1d;
  }

  .choice-text {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .choice-text span {
    color: #8492a8;
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
    gap: 9px;
    padding: 8px 10px;
    border: 1px solid #2a3547;
    border-radius: 8px;
    background: #0c1017;
    color: #74839b;
  }

  .search:focus-within {
    border-color: #3b82f6;
    box-shadow: 0 0 0 2px #3b82f622;
  }

  .search input,
  .rename input {
    min-width: 0;
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: #e2e8f0;
    font: inherit;
    font-size: 12px;
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
    border-radius: 9px;
  }

  .row:hover {
    border-color: #273b58;
    background: #182131;
  }

  .open {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    flex: 1;
    padding: 10px 9px;
    border: 0;
    background: transparent;
    color: inherit;
    text-align: left;
  }

  .icon {
    display: inline-flex;
    color: #60a5fa;
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
    color: #e2e8f0;
    font-size: 13px;
    font-weight: 600;
  }

  .path {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: #74839b;
    font-size: 11px;
  }

  .row.missing .path {
    color: #d6a35c;
  }

  .when {
    flex: 0 0 auto;
    color: #617087;
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
    padding: 8px 9px;
  }

  .rename input {
    padding: 6px 8px;
    border: 1px solid #3b82f6;
    border-radius: 6px;
    background: #0c1017;
  }

  .archive-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    margin-top: 8px;
    border: 0;
    background: transparent;
    color: #8492a8;
    font-size: 12px;
  }

  .archive-toggle:hover {
    color: #e2e8f0;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 16px;
    color: #74839b;
    font-size: 12px;
    text-align: center;
  }

  .empty strong {
    color: #cbd5e1;
    font-size: 14px;
  }

  .small-empty {
    padding: 20px;
  }

  code {
    color: #93c5fd;
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
