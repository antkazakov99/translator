<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { 
    FolderOpen, 
    FileText, 
    CheckCircle2, 
    AlertCircle, 
    Sparkles, 
    Play, 
    Search, 
    Users, 
    Languages, 
    Clapperboard,
    GitBranch,
    Table,
    ArrowRight,
    Loader2
  } from "lucide-svelte";

  // Стейт проекта на Svelte 5 (Runes)
  let gameDir = $state("F:\\Playnite\\Games\\0b588a07-1844-467f-8459-e5629642a1bf\\Installed\\game");
  let loadingProject = $state(false);
  let loadingFile = $state(false);
  let errorMsg = $state("");

  interface TranslationFile {
    name: string;
    path: string;
    size_kb: number;
    total_lines: number;
    translated_lines: number;
  }

  interface ProjectSummary {
    game_name: string;
    characters: [string, string][];
    files: TranslationFile[];
    total_lines: number;
    total_translated: number;
  }

  interface DialogueEntry {
    id: string;
    line_number: number;
    speaker: string | null;
    original_text: string;
    translated_text: string | null;
    status: string;
    scene_name: string;
  }

  interface Scene {
    id: string;
    name: string;
    total_lines: number;
    translated_lines: number;
    entries: DialogueEntry[];
  }

  interface ParsedFile {
    file_path: string;
    file_name: string;
    total_lines: number;
    translated_lines: number;
    scenes: Scene[];
  }

  let project = $state<ProjectSummary | null>(null);
  let selectedFile = $state<TranslationFile | null>(null);
  let currentFile = $state<ParsedFile | null>(null);

  // Навигация и фильтрация
  let selectedSceneId = $state<string>("all");
  let searchQuery = $state("");
  let filterStatus = $state<"all" | "untranslated" | "translated">("all");
  let selectedEntry = $state<DialogueEntry | null>(null);
  let viewMode = $state<"table" | "scenes">("table");

  // Все реплики текущего файла плоским списком (для глобального поиска)
  let allEntries = $derived(
    currentFile ? currentFile.scenes.flatMap(s => s.entries) : []
  );

  // Реплики выбранной сцены (или всех сцен)
  let visibleScenes = $derived(
    currentFile 
      ? (selectedSceneId === "all" ? currentFile.scenes : currentFile.scenes.filter(s => s.id === selectedSceneId))
      : []
  );

  // Отфильтрованные реплики с учетом поиска и статуса
  function filterSceneEntries(entries: DialogueEntry[]) {
    return entries.filter(e => {
      const matchSearch = searchQuery === "" || 
        e.original_text.toLowerCase().includes(searchQuery.toLowerCase()) ||
        (e.translated_text && e.translated_text.toLowerCase().includes(searchQuery.toLowerCase())) ||
        (e.speaker && e.speaker.toLowerCase().includes(searchQuery.toLowerCase()));

      const isTranslated = e.translated_text !== null && e.translated_text.trim() !== "";
      const matchFilter = 
        filterStatus === "all" ||
        (filterStatus === "untranslated" && !isTranslated) ||
        (filterStatus === "translated" && isTranslated);

      return matchSearch && matchFilter;
    });
  }

  let totalPercent = $derived(
    project && project.total_lines > 0
      ? Math.round((project.total_translated / project.total_lines) * 100)
      : 0
  );

  // Сканировать проект
  async function scanProject() {
    if (!gameDir.trim()) return;
    loadingProject = true;
    errorMsg = "";
    try {
      project = await invoke<ProjectSummary>("scan_game_project", { gameDir });
      if (project.files.length > 0) {
        selectFile(project.files[0]);
      }
    } catch (e) {
      errorMsg = String(e);
    } finally {
      loadingProject = false;
    }
  }

  // Загрузить выбранный файл
  async function selectFile(file: TranslationFile) {
    selectedFile = file;
    loadingFile = true;
    errorMsg = "";
    selectedSceneId = "all";
    try {
      currentFile = await invoke<ParsedFile>("parse_file_entries", { filePath: file.path });
      if (currentFile.scenes.length > 0 && currentFile.scenes[0].entries.length > 0) {
        selectedEntry = currentFile.scenes[0].entries[0];
      }
    } catch (e) {
      errorMsg = String(e);
    } finally {
      loadingFile = false;
    }
  }
</script>

<div class="app-shell">
  <!-- ВЕРХНИЙ ТУЛБАР -->
  <header class="topbar">
    <div class="brand">
      <Languages class="icon-brand" size={24} />
      <span class="brand-title">Translator</span>
      <span class="badge-tag">Graph / Scene Edition</span>
    </div>

    <!-- Поле ввода папки игры -->
    <div class="dir-bar">
      <FolderOpen size={16} class="text-zinc-400" />
      <input 
        type="text" 
        bind:value={gameDir} 
        placeholder="Путь к папке игры (game)..." 
        class="input-dir" 
      />
      <button onclick={scanProject} disabled={loadingProject} class="btn-primary">
        {#if loadingProject}
          <Loader2 size={16} class="spin" />
        {:else}
          <Play size={15} />
        {/if}
        <span>Сканировать</span>
      </button>
    </div>

    <!-- Глобальная статистика перевода -->
    {#if project}
      <div class="global-stats">
        <div class="stat-text">
          <span class="stat-num">{project.total_translated} / {project.total_lines}</span>
          <span class="stat-pct">({totalPercent}%)</span>
        </div>
        <div class="progress-track">
          <div class="progress-fill" style="width: {totalPercent}%"></div>
        </div>
      </div>
    {/if}
  </header>

  {#if errorMsg}
    <div class="error-banner">
      <AlertCircle size={16} />
      <span>{errorMsg}</span>
    </div>
  {/if}

  <div class="main-layout">
    <!-- ЛЕВАЯ КОЛОНКА (Сайдбар файлов и персонажей) -->
    <aside class="sidebar">
      {#if project}
        <div class="sidebar-section">
          <div class="sidebar-header">
            <FileText size={14} />
            <span>ФАЙЛЫ ({project.files.length})</span>
          </div>
          <div class="file-list">
            {#each project.files as file}
              <button 
                class="file-item" 
                class:active={selectedFile?.path === file.path}
                onclick={() => selectFile(file)}
              >
                <div class="file-title-row">
                  <span class="file-name">{file.name}</span>
                  <span class="file-size">{file.size_kb.toFixed(0)} KB</span>
                </div>
                <div class="file-progress-row">
                  <div class="file-track">
                    <div 
                      class="file-fill" 
                      style="width: {file.total_lines > 0 ? (file.translated_lines / file.total_lines) * 100 : 0}%"
                    ></div>
                  </div>
                  <span class="file-stats">{file.translated_lines}/{file.total_lines}</span>
                </div>
              </button>
            {/each}
          </div>
        </div>

        <!-- Спикеры -->
        <div class="sidebar-section characters-section">
          <div class="sidebar-header">
            <Users size={14} />
            <span>ПЕРСОНАЖИ ({project.characters.length})</span>
          </div>
          <div class="char-chips">
            {#each project.characters as [alias, name]}
              <div class="char-chip" title="Код: {alias}">
                <span class="char-alias">[{alias}]</span>
                <span class="char-name">{name}</span>
              </div>
            {/each}
          </div>
        </div>
      {:else}
        <div class="empty-sidebar">
          <p>Введите путь к папке игры и нажмите «Сканировать»</p>
        </div>
      {/if}
    </aside>

    <!-- ОСНОВНАЯ ОБЛАСТЬ (Сцены, Таблица реплик и Инспектор) -->
    <main class="content-area">
      {#if currentFile}
        <!-- ТУЛБАР ФАЙЛА И ПЕРЕКЛЮЧАТЕЛЬ РЕЖИМОВ -->
        <div class="file-toolbar">
          <div class="file-info-badge">
            <FileText size={16} />
            <strong>{currentFile.file_name}</strong>
            <span class="badge-lines">{currentFile.scenes.length} сцен(ы)</span>
            <span class="badge-lines">{currentFile.total_lines} строк</span>
          </div>

          <!-- Переключатель: Таблица / Сцены -->
          <div class="view-switcher">
            <button 
              class:active={viewMode === 'table'} 
              onclick={() => viewMode = 'table'}
              title="Режим таблицы диалогов"
            >
              <Table size={14} />
              <span>Таблица</span>
            </button>
            <button 
              class:active={viewMode === 'scenes'} 
              onclick={() => viewMode = 'scenes'}
              title="Режим графа и структуры сцен"
            >
              <GitBranch size={14} />
              <span>Сцены & Граф</span>
            </button>
          </div>

          <!-- Поиск -->
          <div class="search-box">
            <Search size={14} />
            <input 
              type="search" 
              bind:value={searchQuery} 
              placeholder="Поиск по тексту, переводу или спикеру..." 
            />
          </div>

          <!-- Фильтры статуса -->
          <div class="filter-pills">
            <button 
              class:active={filterStatus === 'all'} 
              onclick={() => filterStatus = 'all'}
            >Все</button>
            <button 
              class:active={filterStatus === 'untranslated'} 
              onclick={() => filterStatus = 'untranslated'}
            >Без перевода</button>
            <button 
              class:active={filterStatus === 'translated'} 
              onclick={() => filterStatus = 'translated'}
            >Переведенные</button>
          </div>
        </div>

        <!-- ЛЕНТА ПЕРЕКЛЮЧЕНИЯ СЦЕН (Сверху над таблицей) -->
        <div class="scenes-nav-bar">
          <button 
            class="scene-nav-chip" 
            class:active={selectedSceneId === 'all'}
            onclick={() => selectedSceneId = 'all'}
          >
            <span>Все сцены ({currentFile.scenes.length})</span>
          </button>

          {#each currentFile.scenes as scene}
            {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
            <button 
              class="scene-nav-chip" 
              class:active={selectedSceneId === scene.id}
              onclick={() => selectedSceneId = scene.id}
            >
              <Clapperboard size={12} class="scene-chip-icon" />
              <span class="scene-chip-title">{scene.name}</span>
              <span class="scene-chip-badge" class:badge-complete={pct === 100}>
                {scene.translated_lines}/{scene.total_lines} ({pct}%)
              </span>
            </button>
          {/each}
        </div>

        <!-- ОСНОВНОЙ ВИД -->
        {#if viewMode === 'scenes'}
          <!-- РЕЖИМ ГРАФА И КАРТОЧЕК СЦЕН (Story Flow) -->
          <div class="scenes-flow-container">
            <div class="scenes-flow-grid">
              {#each currentFile.scenes as scene, index}
                {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                <div 
                  class="scene-card" 
                  class:card-selected={selectedSceneId === scene.id}
                  onclick={() => { selectedSceneId = scene.id; viewMode = 'table'; }}
                >
                  <div class="scene-card-header">
                    <div class="scene-card-title-row">
                      <Clapperboard size={16} class="text-sky-400" />
                      <span class="scene-card-name">{scene.name}</span>
                    </div>
                    <span class="scene-card-seq">#{index + 1}</span>
                  </div>

                  <p class="scene-card-preview">
                    {scene.entries.length > 0 ? scene.entries[0].original_text : "Пустая сцена"}
                  </p>

                  <div class="scene-card-footer">
                    <div class="scene-card-progress">
                      <div class="scene-card-fill" style="width: {pct}%"></div>
                    </div>
                    <div class="scene-card-stats">
                      <span>{scene.translated_lines} из {scene.total_lines} строк</span>
                      <span class="pct-text">{pct}%</span>
                    </div>
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {:else}
          <!-- РЕЖИМ ТАБЛИЦЫ ДИАЛОГОВ -->
          <div class="table-container">
            {#if loadingFile}
              <div class="loading-state">
                <Loader2 size={32} class="spin" />
                <p>Парсинг файла сценария через Rust...</p>
              </div>
            {:else}
              <div class="scenes-grouped-view">
                {#each visibleScenes as scene}
                  {@const sceneEntries = filterSceneEntries(scene.entries)}
                  {#if sceneEntries.length > 0}
                    <!-- РАЗДЕЛИТЕЛЬ СЦЕНЫ -->
                    <div class="scene-section-divider">
                      <div class="scene-section-left">
                        <Clapperboard size={15} />
                        <span class="scene-section-title">Сцена: <strong>{scene.name}</strong></span>
                        <span class="badge-lines">{sceneEntries.length} строк</span>
                      </div>
                      <div class="scene-section-right">
                        <span>{scene.translated_lines}/{scene.total_lines} переведено</span>
                        <div class="scene-mini-track">
                          <div 
                            class="scene-mini-fill" 
                            style="width: {scene.total_lines > 0 ? (scene.translated_lines / scene.total_lines) * 100 : 0}%"
                          ></div>
                        </div>
                      </div>
                    </div>

                    <table class="grid-table">
                      <thead>
                        <tr>
                          <th style="width: 50px;">#</th>
                          <th style="width: 140px;">Персонаж</th>
                          <th>Оригинал (EN)</th>
                          <th>Перевод (RU)</th>
                          <th style="width: 80px;">Статус</th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each sceneEntries as entry}
                          <tr 
                            class:selected={selectedEntry?.id === entry.id}
                            onclick={() => selectedEntry = entry}
                          >
                            <td class="col-line">{entry.line_number}</td>
                            <td class="col-speaker">
                              {#if entry.speaker}
                                <span class="speaker-badge">{entry.speaker}</span>
                              {:else}
                                <span class="narrator-badge">Автор</span>
                              {/if}
                            </td>
                            <td class="col-text en-text">{entry.original_text}</td>
                            <td class="col-text ru-text">
                              {#if entry.translated_text}
                                <span>{entry.translated_text}</span>
                              {:else}
                                <span class="placeholder-text">Не переведено</span>
                              {/if}
                            </td>
                            <td class="col-status">
                              {#if entry.translated_text}
                                <span class="status-pill status-ok">Готово</span>
                              {:else}
                                <span class="status-pill status-empty">Пусто</span>
                              {/if}
                            </td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  {/if}
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        <!-- НИЖНИЙ ИНСПЕКТОР СТРОКИ -->
        {#if selectedEntry}
          <div class="inspector-panel">
            <div class="inspector-header">
              <div class="inspector-breadcrumb">
                <Clapperboard size={13} class="text-sky-400" />
                <span class="inspect-scene">{selectedEntry.scene_name}</span>
                <span class="breadcrumb-sep">/</span>
                <span class="inspect-line">Строка #{selectedEntry.line_number}</span>
                <span class="breadcrumb-sep">/</span>
                <strong class="inspect-speaker">{selectedEntry.speaker ?? "Авторский текст"}</strong>
              </div>
              <button class="btn-ai">
                <Sparkles size={14} />
                <span>Перевести через LM Studio</span>
              </button>
            </div>
            <div class="inspector-body">
              <div class="inspect-col">
                <label>Оригинальный текст (EN)</label>
                <div class="text-box-en">{selectedEntry.original_text}</div>
              </div>
              <div class="inspect-col">
                <label>Русский перевод (RU)</label>
                <textarea 
                  class="text-box-ru" 
                  bind:value={selectedEntry.translated_text} 
                  placeholder="Введите перевод..."
                ></textarea>
              </div>
            </div>
          </div>
        {/if}
      {:else}
        <div class="empty-workspace">
          <Languages size={48} class="empty-icon" />
          <h2>Проект не загружен</h2>
          <p>Укажите папку с установленной новеллой сверху и нажмите «Сканировать»</p>
        </div>
      {/if}
    </main>
  </div>
</div>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    background: #0d0f12;
    color: #e2e8f0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
    user-select: none;
    overflow: hidden;
  }

  .app-shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
  }

  /* TOPBAR */
  .topbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 18px;
    background: #14171d;
    border-bottom: 1px solid #232832;
    min-height: 48px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .brand-title {
    font-weight: 700;
    font-size: 16px;
    color: #f8fafc;
    letter-spacing: -0.3px;
  }

  .badge-tag {
    font-size: 11px;
    background: #1e293b;
    color: #38bdf8;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid #0284c733;
  }

  .dir-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #0d0f12;
    border: 1px solid #262c38;
    border-radius: 8px;
    padding: 4px 10px;
    flex: 1;
    max-width: 720px;
  }

  .input-dir {
    background: transparent;
    border: none;
    color: #e2e8f0;
    font-size: 13px;
    font-family: inherit;
    flex: 1;
    outline: none;
  }

  .btn-primary {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #2563eb;
    color: #fff;
    border: none;
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-primary:hover {
    background: #1d4ed8;
  }

  .global-stats {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 150px;
  }

  .stat-text {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
  }

  .stat-num {
    color: #e2e8f0;
    font-weight: 600;
  }

  .stat-pct {
    color: #38bdf8;
    font-weight: 600;
  }

  .progress-track {
    height: 6px;
    background: #1e293b;
    border-radius: 999px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: #38bdf8;
    transition: width 0.3s;
  }

  .error-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #7f1d1d;
    color: #fecaca;
    padding: 8px 16px;
    font-size: 13px;
  }

  /* LAYOUT */
  .main-layout {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  /* SIDEBAR */
  .sidebar {
    width: 280px;
    background: #111419;
    border-right: 1px solid #1e232d;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .sidebar-section {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
    letter-spacing: 0.5px;
  }

  .file-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .file-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 10px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: pointer;
    text-align: left;
    color: #cbd5e1;
    transition: all 0.15s;
  }

  .file-item:hover {
    background: #171b22;
  }

  .file-item.active {
    background: #1c222c;
    border-color: #2e3846;
    color: #f8fafc;
  }

  .file-title-row {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 500;
  }

  .file-size {
    font-size: 11px;
    color: #64748b;
  }

  .file-progress-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .file-track {
    flex: 1;
    height: 4px;
    background: #242c38;
    border-radius: 999px;
    overflow: hidden;
  }

  .file-fill {
    height: 100%;
    background: #22c55e;
  }

  .file-stats {
    font-size: 10px;
    color: #64748b;
  }

  .characters-section {
    border-top: 1px solid #1a1f27;
    flex: 1;
  }

  .char-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    max-height: 160px;
    overflow-y: auto;
  }

  .char-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #181c24;
    border: 1px solid #28303d;
    padding: 3px 6px;
    border-radius: 5px;
    font-size: 11px;
  }

  .char-alias {
    color: #38bdf8;
    font-weight: 600;
  }

  .char-name {
    color: #cbd5e1;
  }

  /* WORKSPACE */
  .content-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: #0d0f12;
  }

  .file-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 16px;
    background: #12151b;
    border-bottom: 1px solid #1e232d;
    flex-wrap: wrap;
  }

  .file-info-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }

  .badge-lines {
    font-size: 11px;
    background: #1e293b;
    color: #94a3b8;
    padding: 2px 7px;
    border-radius: 999px;
  }

  .view-switcher {
    display: flex;
    background: #0d0f12;
    border: 1px solid #232935;
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }

  .view-switcher button {
    display: flex;
    align-items: center;
    gap: 5px;
    background: transparent;
    border: none;
    color: #94a3b8;
    padding: 4px 10px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }

  .view-switcher button.active {
    background: #1e293b;
    color: #38bdf8;
    font-weight: 500;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #0d0f12;
    border: 1px solid #232935;
    border-radius: 6px;
    padding: 5px 10px;
    flex: 1;
    min-width: 200px;
    max-width: 320px;
  }

  .search-box input {
    background: transparent;
    border: none;
    color: #e2e8f0;
    font-size: 12px;
    flex: 1;
    outline: none;
  }

  .filter-pills {
    display: flex;
    gap: 4px;
  }

  .filter-pills button {
    background: transparent;
    border: 1px solid #232935;
    color: #94a3b8;
    padding: 4px 10px;
    border-radius: 5px;
    font-size: 12px;
    cursor: pointer;
  }

  .filter-pills button.active {
    background: #1e293b;
    color: #38bdf8;
    border-color: #0284c7;
  }

  /* SCENES NAV BAR */
  .scenes-nav-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 16px;
    background: #0f1217;
    border-bottom: 1px solid #1a1f28;
    overflow-x: auto;
    white-space: nowrap;
  }

  .scene-nav-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #141820;
    border: 1px solid #202734;
    border-radius: 6px;
    padding: 4px 9px;
    font-size: 12px;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s;
  }

  .scene-nav-chip:hover {
    background: #19202c;
    color: #cbd5e1;
  }

  .scene-nav-chip.active {
    background: #1d2535;
    border-color: #0284c7;
    color: #38bdf8;
  }

  .scene-chip-title {
    font-weight: 500;
  }

  .scene-chip-badge {
    font-size: 10px;
    background: #0d0f12;
    padding: 1px 5px;
    border-radius: 4px;
    color: #64748b;
  }

  .scene-chip-badge.badge-complete {
    background: #064e3b;
    color: #34d399;
  }

  /* FLOW VIEW */
  .scenes-flow-container {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    background: #0b0d10;
  }

  .scenes-flow-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 16px;
  }

  .scene-card {
    background: #13171f;
    border: 1px solid #202734;
    border-radius: 8px;
    padding: 14px;
    cursor: pointer;
    transition: all 0.15s;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .scene-card:hover {
    background: #181d27;
    border-color: #2b3648;
    transform: translateY(-2px);
  }

  .scene-card.card-selected {
    border-color: #0284c7;
    box-shadow: 0 0 0 1px #0284c7;
  }

  .scene-card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .scene-card-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
    font-size: 13px;
    color: #f1f5f9;
  }

  .scene-card-seq {
    font-size: 11px;
    color: #64748b;
    font-family: monospace;
  }

  .scene-card-preview {
    font-size: 12px;
    color: #94a3b8;
    margin: 0;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.4;
  }

  .scene-card-footer {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: auto;
  }

  .scene-card-progress {
    height: 4px;
    background: #1e2430;
    border-radius: 999px;
    overflow: hidden;
  }

  .scene-card-fill {
    height: 100%;
    background: #38bdf8;
  }

  .scene-card-stats {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: #64748b;
  }

  .pct-text {
    color: #38bdf8;
    font-weight: 600;
  }

  /* TABLE */
  .table-container {
    flex: 1;
    overflow: auto;
  }

  .scene-section-divider {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: #141820;
    border-top: 1px solid #1e2532;
    border-bottom: 1px solid #1e2532;
    padding: 6px 14px;
    font-size: 12px;
    color: #94a3b8;
    position: sticky;
    top: 0;
    z-index: 2;
  }

  .scene-section-left {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #38bdf8;
  }

  .scene-section-title strong {
    color: #f1f5f9;
  }

  .scene-section-right {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: #64748b;
  }

  .scene-mini-track {
    width: 60px;
    height: 4px;
    background: #242c38;
    border-radius: 999px;
    overflow: hidden;
  }

  .scene-mini-fill {
    height: 100%;
    background: #22c55e;
  }

  .grid-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .grid-table th {
    background: #111419;
    padding: 7px 12px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
    border-bottom: 1px solid #1c222c;
  }

  .grid-table td {
    padding: 9px 12px;
    border-bottom: 1px solid #14171e;
    vertical-align: top;
    user-select: text;
  }

  .grid-table tr {
    cursor: pointer;
    transition: background 0.1s;
  }

  .grid-table tr:hover {
    background: #14171f;
  }

  .grid-table tr.selected {
    background: #1a2230;
  }

  .col-line {
    font-family: monospace;
    font-size: 11px;
    color: #64748b;
  }

  .speaker-badge {
    background: #1e2634;
    color: #38bdf8;
    padding: 2px 7px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
  }

  .narrator-badge {
    color: #64748b;
    font-size: 11px;
    font-style: italic;
  }

  .en-text {
    color: #cbd5e1;
    line-height: 1.45;
  }

  .ru-text {
    color: #f1f5f9;
    line-height: 1.45;
  }

  .placeholder-text {
    color: #475569;
    font-style: italic;
  }

  .status-pill {
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 4px;
    font-weight: 600;
  }

  .status-ok {
    background: #064e3b;
    color: #34d399;
  }

  .status-empty {
    background: #27272a;
    color: #71717a;
  }

  /* INSPECTOR */
  .inspector-panel {
    background: #14171e;
    border-top: 1px solid #232935;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 220px;
  }

  .inspector-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .inspector-breadcrumb {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }

  .inspect-scene {
    color: #38bdf8;
    font-weight: 600;
  }

  .breadcrumb-sep {
    color: #475569;
  }

  .inspect-line {
    color: #94a3b8;
  }

  .inspect-speaker {
    color: #f8fafc;
  }

  .btn-ai {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #4f46e5;
    color: #fff;
    border: none;
    border-radius: 5px;
    padding: 5px 12px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
  }

  .btn-ai:hover {
    background: #4338ca;
  }

  .inspector-body {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  .inspect-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .inspect-col label {
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
  }

  .text-box-en {
    background: #0d0f12;
    border: 1px solid #232935;
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 13px;
    color: #cbd5e1;
    min-height: 50px;
    user-select: text;
  }

  .text-box-ru {
    background: #0d0f12;
    border: 1px solid #334155;
    border-radius: 6px;
    padding: 8px 10px;
    font-size: 13px;
    color: #f8fafc;
    min-height: 50px;
    resize: none;
    outline: none;
    font-family: inherit;
    user-select: text;
  }

  .text-box-ru:focus {
    border-color: #38bdf8;
  }

  .empty-workspace, .empty-sidebar, .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #64748b;
    gap: 10px;
    text-align: center;
  }

  .spin {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
