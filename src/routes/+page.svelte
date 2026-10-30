<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { 
    Folder,
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
    Loader2,
    CornerDownRight,
    Zap,
    Layers,
    ArrowUpRight,
    ChevronRight,
    ChevronDown,
    Network
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

  interface ChoiceOption {
    text: string;
    target_jump: string | null;
  }

  interface ChoiceBlock {
    prompt: string | null;
    options: ChoiceOption[];
  }

  interface SceneEdge {
    from: string;
    to: string;
    kind: "jump" | "branch" | "choice" | "linear" | "call";
    condition: string | null;
  }

  interface Scene {
    id: string;
    name: string;
    total_lines: number;
    translated_lines: number;
    choices: ChoiceBlock[];
    jumps: string[];
    entries: DialogueEntry[];
  }

  interface ParsedFile {
    file_path: string;
    file_name: string;
    total_lines: number;
    translated_lines: number;
    scenes: Scene[];
    edges: SceneEdge[];
  }

  let project = $state<ProjectSummary | null>(null);
  let selectedFile = $state<TranslationFile | null>(null);
  let currentFile = $state<ParsedFile | null>(null);

  // Структура узла дерева файлов
  interface FileTreeNode {
    id: string;
    name: string;
    path: string;
    isFolder: boolean;
    file?: TranslationFile;
    children: FileTreeNode[];
    totalLines: number;
    translatedLines: number;
    level: number;
  }

  let fileSearchFilter = $state("");
  let charactersExpanded = $state(false);
  let expandedFolders = $state<Record<string, boolean>>({
    "script": true,
    "script/locations": true
  });

  function toggleFolder(path: string) {
    expandedFolders[path] = !expandedFolders[path];
  }

  function isFolderExpanded(path: string): boolean {
    return !!expandedFolders[path];
  }

  function expandAllFolders() {
    if (!project) return;
    for (const f of project.files) {
      const parts = f.name.replace(/\\/g, '/').split('/');
      let cur = '';
      for (let i = 0; i < parts.length - 1; i++) {
        cur = cur ? `${cur}/${parts[i]}` : parts[i];
        expandedFolders[cur] = true;
      }
    }
  }

  function collapseAllFolders() {
    expandedFolders = {};
  }

  function expandParentsForFile(fileNameOrPath: string) {
    const parts = fileNameOrPath.replace(/\\/g, '/').split('/');
    let cur = '';
    for (let i = 0; i < parts.length - 1; i++) {
      cur = cur ? `${cur}/${parts[i]}` : parts[i];
      expandedFolders[cur] = true;
    }
  }

  function scrollToActiveFile(filePath: string) {
    const domId = `sidebar-file-${encodeURIComponent(filePath).replace(/%/g, '_')}`;
    setTimeout(() => {
      const el = document.getElementById(domId);
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
        el.classList.remove('file-highlight-pulse');
        void el.offsetWidth;
        el.classList.add('file-highlight-pulse');
      }
    }, 70);
  }

  function buildFileTree(files: TranslationFile[], filter: string): FileTreeNode[] {
    const rootNodes: FileTreeNode[] = [];
    const folderMap = new Map<string, FileTreeNode>();

    const normalizedFilter = filter.trim().toLowerCase();
    const filteredFiles = normalizedFilter
      ? files.filter(f => f.name.toLowerCase().includes(normalizedFilter) || f.path.toLowerCase().includes(normalizedFilter))
      : files;

    for (const file of filteredFiles) {
      const parts = file.name.replace(/\\/g, '/').split('/');
      let currentPath = '';

      for (let i = 0; i < parts.length; i++) {
        const part = parts[i];
        const isLast = i === parts.length - 1;
        const prevPath = currentPath;
        currentPath = currentPath ? `${currentPath}/${part}` : part;

        if (!isLast) {
          if (!folderMap.has(currentPath)) {
            const folderNode: FileTreeNode = {
              id: currentPath,
              name: part,
              path: currentPath,
              isFolder: true,
              children: [],
              totalLines: 0,
              translatedLines: 0,
              level: i
            };
            folderMap.set(currentPath, folderNode);

            if (prevPath) {
              const parent = folderMap.get(prevPath);
              if (parent && !parent.children.some(c => c.id === currentPath)) {
                parent.children.push(folderNode);
              }
            } else {
              rootNodes.push(folderNode);
            }
          }
          const folder = folderMap.get(currentPath)!;
          folder.totalLines += file.total_lines;
          folder.translatedLines += file.translated_lines;
        } else {
          const fileNode: FileTreeNode = {
            id: file.path,
            name: part,
            path: file.name,
            isFolder: false,
            file,
            children: [],
            totalLines: file.total_lines,
            translatedLines: file.translated_lines,
            level: i
          };

          if (prevPath) {
            const parent = folderMap.get(prevPath);
            if (parent) {
              parent.children.push(fileNode);
            }
          } else {
            rootNodes.push(fileNode);
          }
        }
      }
    }

    function sortNodes(nodes: FileTreeNode[]) {
      nodes.sort((a, b) => {
        if (a.isFolder !== b.isFolder) {
          return a.isFolder ? -1 : 1;
        }
        return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
      });
      for (const n of nodes) {
        if (n.isFolder) {
          sortNodes(n.children);
        }
      }
    }

    sortNodes(rootNodes);
    return rootNodes;
  }

  let fileTree = $derived(
    project ? buildFileTree(project.files, fileSearchFilter) : []
  );

  // Навигация и фильтрация
  let selectedSceneId = $state<string>("all");
  let searchQuery = $state("");
  let filterStatus = $state<"all" | "untranslated" | "translated">("all");
  let selectedEntry = $state<DialogueEntry | null>(null);
  let viewMode = $state<"table" | "scenes">("table");
  let graphSubMode = $state<"flow" | "grid">("flow");
  let focusedSceneName = $state<string | null>(null);

  // Статистика графа
  let totalChoicesCount = $derived(
    currentFile ? currentFile.scenes.reduce((acc, s) => acc + (s.choices ? s.choices.length : 0), 0) : 0
  );

  let totalEdgesCount = $derived(
    currentFile ? (currentFile.edges ? currentFile.edges.length : 0) : 0
  );

  // Исходящие связи для сцены
  function getOutgoingEdges(sceneName: string): SceneEdge[] {
    if (!currentFile || !currentFile.edges) return [];
    return currentFile.edges.filter(e => e.from === sceneName);
  }

  // Входящие связи для сцены
  function getIncomingEdges(sceneName: string): SceneEdge[] {
    if (!currentFile || !currentFile.edges) return [];
    return currentFile.edges.filter(e => e.to === sceneName);
  }

  // Проверяет, ведет ли переход на сцену в текущем файле
  function isInternalScene(targetName: string): boolean {
    if (!currentFile) return false;
    return currentFile.scenes.some(s => s.name === targetName || s.name.startsWith(targetName));
  }

  // Сфокусироваться на сцене или перейти в неё
  function focusScene(sceneName: string) {
    if (!currentFile) return;
    let targetScene = currentFile.scenes.find(s => s.name === sceneName);
    if (!targetScene) {
      targetScene = currentFile.scenes.find(s => s.name.startsWith(sceneName) || sceneName.startsWith(s.name));
    }
    if (targetScene) {
      selectedSceneId = targetScene.id;
      focusedSceneName = targetScene.name;
      const el = document.getElementById(`scene-node-${targetScene.id}`);
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    }
  }

  // Всплывающее уведомление о переходах и событиях
  let toast = $state<{ text: string; type: 'info' | 'success' | 'warning' | 'error' } | null>(null);
  let toastTimer: any = null;

  function showToast(text: string, type: 'info' | 'success' | 'warning' | 'error' = 'info') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { text, type };
    toastTimer = setTimeout(() => {
      toast = null;
    }, 4500);
  }

  // Интерактивный переход к сцене: внутри текущего файла или между файлами проекта
  async function navigateOrFocusScene(targetName: string) {
    if (!targetName) return;

    // 1. Если сцена есть в текущем файле — моментальный фокус
    if (isInternalScene(targetName)) {
      focusScene(targetName);
      showToast(`Сцена "${targetName}" в фокусе (текущий файл)`, "info");
      return;
    }

    // 2. Иначе ищем в проекте через Rust бэкенд
    showToast(`Поиск перехода к "${targetName}" по файлам проекта...`, "info");
    try {
      const found = await invoke<{
        file_name: string;
        short_name: string;
        file_path: string;
        original_path: string;
        line_number: number;
        scene_name: string;
      } | null>("find_scene_target", {
        gameDir,
        targetName
      });

      if (!found) {
        showToast(`Метка "${targetName}" — внешняя локация / системный вызов Ren'Py`, "warning");
        return;
      }

      // Нормализуем пути для надежного сравнения (слэши, подпапки, имя файла)
      const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase();
      const targetPath = norm(found.file_path);
      const targetNameNormalized = norm(found.file_name);
      const targetShort = norm(found.short_name);

      let targetFile = project?.files.find(f => {
        const fp = norm(f.path);
        const fn = norm(f.name);
        return fp === targetPath ||
               fn === targetNameNormalized ||
               fn === targetShort ||
               fn.endsWith('/' + targetShort) ||
               fp.endsWith('/' + targetShort);
      });

      // Если файл есть на диске, но не был в списке tl/, добавляем его динамически
      if (!targetFile && project) {
        targetFile = {
          name: found.file_name,
          path: found.file_path,
          size_kb: 0,
          total_lines: 0,
          translated_lines: 0
        };
        project.files.push(targetFile);
      }

      if (targetFile) {
        showToast(`Переход в ${found.short_name} ➔ сцена "${targetName}" (строка ${found.line_number})`, "success");
        await selectFile(targetFile);
        // Небольшая задержка на отрисовку DOM после парсинга файла
        setTimeout(() => {
          focusScene(targetName);
        }, 150);
      } else {
        showToast(`Сцена найдена в ${found.short_name}:${found.line_number}, но файл не найден`, "warning");
      }
    } catch (e) {
      showToast(`Ошибка поиска: ${e}`, "error");
    }
  }

  // Открыть сцену в режиме таблицы диалогов
  function openSceneInTable(sceneId: string) {
    selectedSceneId = sceneId;
    viewMode = 'table';
  }

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
      loadingProject = false;
      if (project.files.length > 0) {
        // Микропауза для отрисовки сайдбара и статов перед загрузкой первого файла
        await new Promise(r => setTimeout(r, 20));
        selectFile(project.files[0]);
      }
    } catch (e) {
      errorMsg = String(e);
      loadingProject = false;
    }
  }

  // Загрузить выбранный файл
  async function selectFile(file: TranslationFile) {
    selectedFile = file;
    expandParentsForFile(file.name);
    scrollToActiveFile(file.path);
    loadingFile = true;
    errorMsg = "";
    try {
      currentFile = await invoke<ParsedFile>("parse_file_entries", { filePath: file.path });
      // По умолчанию выбираем первую реальную сюжетную сцену (или все, если сцена одна)
      if (currentFile.scenes.length > 1) {
        const firstStory = currentFile.scenes.find(s => s.name !== "Интерфейс и меню (strings)");
        selectedSceneId = firstStory ? firstStory.id : currentFile.scenes[0].id;
      } else if (currentFile.scenes.length > 0) {
        selectedSceneId = currentFile.scenes[0].id;
      } else {
        selectedSceneId = "all";
      }

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
    {#if loadingProject}
      <div class="global-stats skeleton-stats-box">
        <div class="skeleton-shimmer skeleton-pill-head"></div>
        <div class="progress-track skeleton-shimmer"></div>
      </div>
    {:else if project}
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
      {#if loadingProject}
        <!-- Скелетоны при сканировании файлов -->
        <div class="sidebar-section">
          <div class="sidebar-header">
            <Loader2 size={14} class="spin text-sky-400" />
            <span>СКАНИРОВАНИЕ ПРОЕКТА...</span>
          </div>
          <div class="skeleton-list">
            {#each Array(9) as _}
              <div class="skeleton-file-item">
                <div class="skeleton-shimmer skeleton-line w-75"></div>
                <div class="skeleton-shimmer skeleton-line w-45"></div>
              </div>
            {/each}
          </div>
        </div>
        <div class="sidebar-section characters-section">
          <div class="sidebar-header">
            <Users size={14} />
            <span>ПЕРСОНАЖИ...</span>
          </div>
          <div class="skeleton-chips">
            {#each Array(6) as _}
              <div class="skeleton-shimmer skeleton-chip-item"></div>
            {/each}
          </div>
        </div>
      {:else if project}
        <!-- ДЕРЕВО ФАЙЛОВ ПРОЕКТА -->
        <div class="sidebar-section file-tree-section">
          <div class="sidebar-header">
            <div class="header-left">
              <FolderOpen size={14} class="text-sky-400" />
              <span>ФАЙЛЫ ({project.files.length})</span>
            </div>
            <div class="header-actions">
              <button class="icon-btn-subtle" onclick={expandAllFolders} title="Развернуть все папки">⊞</button>
              <button class="icon-btn-subtle" onclick={collapseAllFolders} title="Свернуть все папки">⊟</button>
            </div>
          </div>

          <!-- Быстрый поиск файлов по дереву -->
          <div class="sidebar-search-box">
            <Search size={13} class="search-box-icon text-slate-500" />
            <input 
              type="text" 
              placeholder="Поиск файла..." 
              bind:value={fileSearchFilter}
              class="sidebar-search-input"
            />
            {#if fileSearchFilter}
              <button class="clear-search-pill" onclick={() => fileSearchFilter = ""}>✕</button>
            {/if}
          </div>

          <!-- Контейнер дерева файлов -->
          <div class="file-tree-container">
            {#if fileTree.length === 0}
              <div class="empty-tree-hint">
                <span>Ничего не найдено</span>
              </div>
            {:else}
              {#each fileTree as node}
                {@render renderTreeNode(node)}
              {/each}
            {/if}
          </div>
        </div>

        <!-- Сниппет компактного узла дерева в стиле современных IDE -->
        {#snippet renderTreeNode(node: FileTreeNode)}
          {#if node.isFolder}
            {@const isExp = isFolderExpanded(node.path) || fileSearchFilter.trim() !== ''}
            {@const pct = node.totalLines > 0 ? Math.round((node.translatedLines / node.totalLines) * 100) : 0}
            <div class="tree-node-group">
              <button 
                class="tree-row tree-folder-row"
                onclick={() => toggleFolder(node.path)}
                title={node.path}
              >
                <span class="tree-caret">
                  {#if isExp}
                    <ChevronDown size={12} />
                  {:else}
                    <ChevronRight size={12} />
                  {/if}
                </span>
                <span class="tree-icon">
                  {#if isExp}
                    <FolderOpen size={14} class="folder-svg text-amber-400" />
                  {:else}
                    <Folder size={14} class="folder-svg text-amber-500/80" />
                  {/if}
                </span>
                <span class="tree-label folder-label">{node.name}</span>
                
                <div class="tree-meta">
                  <span class="tree-count">{node.children.length}</span>
                  {#if node.totalLines > 0}
                    <span class="tree-pct-badge" class:pct-complete={pct === 100}>{pct}%</span>
                  {/if}
                </div>
              </button>

              {#if isExp}
                <div class="tree-branch-children">
                  {#each node.children as child}
                    {@render renderTreeNode(child)}
                  {/each}
                </div>
              {/if}
            </div>
          {:else if node.file}
            {@const f = node.file}
            {@const pct = f.total_lines > 0 ? Math.round((f.translated_lines / f.total_lines) * 100) : 0}
            {@const isSel = selectedFile?.path === f.path}
            <button 
              id={`sidebar-file-${encodeURIComponent(f.path).replace(/%/g, '_')}`}
              class="tree-row tree-file-row"
              class:active={isSel}
              onclick={() => selectFile(f)}
              title="{f.name} • {f.translated_lines}/{f.total_lines} переведено ({pct}%) • {f.size_kb.toFixed(0)} KB"
            >
              <span class="tree-caret-spacer"></span>
              <span class="tree-icon">
                <FileText size={13} class={isSel ? "file-svg text-sky-400" : "file-svg text-slate-500"} />
              </span>
              <span class="tree-label file-label">{node.name}</span>

              <div class="tree-meta">
                {#if pct === 100}
                  <span class="tree-pct-badge pct-complete">100%</span>
                {:else if pct > 0}
                  <div class="tree-progress-capsule" title="{pct}% переведено">
                    <div class="tree-capsule-track">
                      <div class="tree-capsule-fill" style="width: {pct}%"></div>
                    </div>
                    <span class="tree-capsule-val">{pct}%</span>
                  </div>
                {/if}
              </div>
            </button>
          {/if}
        {/snippet}

        <!-- Спикеры / Персонажи (компактный аккордеон) -->
        <div class="sidebar-section characters-section">
          <button 
            class="characters-toggle-header" 
            onclick={() => charactersExpanded = !charactersExpanded}
            title={charactersExpanded ? "Свернуть список персонажей" : "Развернуть список персонажей"}
          >
            <div class="header-left">
              <span class="tree-caret">
                {#if charactersExpanded}
                  <ChevronDown size={12} />
                {:else}
                  <ChevronRight size={12} />
                {/if}
              </span>
              <Users size={13} class="text-sky-400" />
              <span>ПЕРСОНАЖИ ({project.characters.length})</span>
            </div>
            <span class="characters-state-hint">{charactersExpanded ? "свернуть" : "раскрыть"}</span>
          </button>

          {#if charactersExpanded}
            <div class="char-chips">
              {#each project.characters as [alias, name]}
                <div class="char-chip" title="Код: {alias}">
                  <span class="char-alias">[{alias}]</span>
                  <span class="char-name">{name}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <div class="empty-sidebar">
          <p>Введите путь к папке игры и нажмите «Сканировать»</p>
        </div>
      {/if}
    </aside>

    <!-- ОСНОВНАЯ ОБЛАСТЬ (Сцены, Таблица реплик и Инспектор) -->
    <main class="content-area">
      {#if loadingProject}
        <div class="loading-project-workspace">
          <div class="workspace-shimmer-card">
            <Loader2 size={36} class="spin text-sky-400" />
            <h3>Сканирование и анализ сценариев...</h3>
            <p>Параллельная обработка скриптов через Rust Rayon</p>
            <div class="skeleton-shimmer skeleton-bar-wide"></div>
          </div>
        </div>
      {:else if currentFile}
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
        <div 
          class="scenes-nav-bar"
          onwheel={(e) => {
            if (e.deltaY !== 0) {
              e.currentTarget.scrollLeft += e.deltaY;
              e.preventDefault();
            }
          }}
        >
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
          <!-- СЮЖЕТНЫЙ ГРАФ И СХЕМА ВЕТВЛЕНИЙ -->
          <div class="story-graph-wrapper">
            <!-- ВЕРХНЯЯ ПАНЕЛЬ ГРАФА -->
            <div class="graph-topbar">
              <div class="graph-title-row">
                <Network size={16} class="text-sky-400" />
                <span class="graph-heading">Сюжетная структура и развилки сценария</span>
                <span class="graph-pill">{currentFile.scenes.length} сцен</span>
                <span class="graph-pill pill-purple">{totalChoicesCount} выборов (menu)</span>
                <span class="graph-pill pill-amber">{totalEdgesCount} переходов (jump/branch)</span>
              </div>

              <div class="graph-submode-toggles">
                <button 
                  class="graph-mode-btn" 
                  class:active={graphSubMode === 'flow'}
                  onclick={() => graphSubMode = 'flow'}
                  title="Потоковая диаграмма с ветвлениями"
                >
                  <GitBranch size={14} />
                  <span>Поток сюжета</span>
                </button>
                <button 
                  class="graph-mode-btn" 
                  class:active={graphSubMode === 'grid'}
                  onclick={() => graphSubMode = 'grid'}
                  title="Компактная матрица сцен"
                >
                  <Layers size={14} />
                  <span>Матрица</span>
                </button>
              </div>
            </div>

            {#if graphSubMode === 'flow'}
              <!-- РЕЖИМ 1: ИНТЕРАКТИВНЫЙ ПОТОК СЮЖЕТА (DAG FLOW WITH BRANCHES) -->
              <div class="story-flow-canvas">
                <div class="story-nodes-track">
                  {#each currentFile.scenes as scene, index}
                    {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                    {@const incoming = getIncomingEdges(scene.name)}
                    {@const outgoing = getOutgoingEdges(scene.name)}
                    {@const isFocused = focusedSceneName === scene.name}

                    <div 
                      id={`scene-node-${scene.id}`}
                      class="flow-scene-node"
                      class:node-focused={isFocused}
                      class:node-selected={selectedSceneId === scene.id}
                    >
                      <!-- ЗАГОЛОВОК СЦЕНЫ -->
                      <div class="node-header">
                        <div class="node-title-group">
                          <span class="node-seq">#{index + 1}</span>
                          <Clapperboard size={16} class="node-icon text-sky-400" />
                          <h4 class="node-name">{scene.name}</h4>
                          {#if scene.name === "Интерфейс и меню (strings)"}
                            <span class="node-tag tag-ui">Интерфейс</span>
                          {:else if scene.choices && scene.choices.length > 0}
                            <span class="node-tag tag-choice">Развилка</span>
                          {/if}
                        </div>

                        <div class="node-progress-badge" class:badge-complete={pct === 100}>
                          <span>{scene.translated_lines}/{scene.total_lines} строк</span>
                          <strong>{pct}%</strong>
                        </div>
                      </div>

                      <div class="node-progress-line">
                        <div class="node-fill" style="width: {pct}%"></div>
                      </div>

                      <!-- ВХОДЯЩИЕ ПЕРЕХОДЫ (ОТКУДА СЮДА ПРИШЛИ) -->
                      {#if incoming.length > 0}
                        <div class="node-incoming-box">
                          <span class="box-label">⬅ Вход из сцен:</span>
                          <div class="edge-chips">
                            {#each incoming as edge}
                              <button 
                                class="edge-chip chip-incoming" 
                                onclick={() => navigateOrFocusScene(edge.from)}
                                title="Перейти к сцене {edge.from}"
                              >
                                <span>{edge.from}</span>
                                {#if edge.condition}
                                  <span class="edge-condition-sub">({edge.condition})</span>
                                {/if}
                              </button>
                            {/each}
                          </div>
                        </div>
                      {/if}

                      <!-- ПРЕВЬЮ ДИАЛОГА -->
                      {#if scene.entries.length > 0}
                        <div class="node-dialogue-snippet">
                          <span class="snippet-speaker">{scene.entries[0].speaker ?? "Автор"}:</span>
                          <span class="snippet-text">"{scene.entries[0].original_text}"</span>
                        </div>
                      {/if}

                      <!-- СЕКЦИЯ 1: ВЫБОРЫ В СЦЕНЕ (MENU:) -->
                      {#if scene.choices && scene.choices.length > 0}
                        <div class="node-choices-section">
                          <div class="choices-section-header">
                            <Zap size={14} class="text-purple-400" />
                            <strong>Выбор игрока (Меню развилки):</strong>
                          </div>
                          {#each scene.choices as choiceBlock}
                            <div class="choice-options-list">
                              {#each choiceBlock.options as opt}
                                <div class="choice-option-pill">
                                  <span class="opt-bullet">◆</span>
                                  <span class="opt-title">"{opt.text}"</span>
                                  {#if opt.target_jump}
                                    <span class="opt-arrow">➔</span>
                                    {@const isInt = isInternalScene(opt.target_jump)}
                                    <button 
                                      class="target-link"
                                      class:link-internal={isInt}
                                      class:link-external={!isInt}
                                      onclick={() => navigateOrFocusScene(opt.target_jump!)}
                                      title={isInt ? `Перейти к сцене ${opt.target_jump} в этом файле` : `Перейти к ${opt.target_jump} (межфайловый переход)`}
                                    >
                                      <span>{opt.target_jump}</span>
                                      {#if isInt}
                                        <ChevronRight size={11} />
                                      {:else}
                                        <ArrowUpRight size={11} />
                                      {/if}
                                    </button>
                                  {/if}
                                </div>
                              {/each}
                            </div>
                          {/each}
                        </div>
                      {/if}

                      <!-- СЕКЦИЯ 2: ИСХОДЯЩИЕ ВЕТВЛЕНИЯ И ПЕРЕХОДЫ (JUMP/IF) -->
                      {#if outgoing.length > 0}
                        <div class="node-jumps-section">
                          <div class="jumps-section-header">
                            <CornerDownRight size={14} class="text-amber-400" />
                            <strong>Переходы и ветвления:</strong>
                          </div>
                          <div class="outgoing-chips-grid">
                            {#each outgoing as edge}
                              {@const isInternal = isInternalScene(edge.to)}
                              <div class="outgoing-edge-card" class:edge-choice={edge.kind === 'choice'} class:edge-branch={edge.kind === 'branch'}>
                                {#if edge.condition}
                                  <div class="edge-cond-badge">
                                    <span class="cond-icon">🔀</span>
                                    <span class="cond-text">{edge.condition}</span>
                                  </div>
                                {/if}
                                <div class="edge-dest-row">
                                  <span class="edge-arrow">➔</span>
                                  <button 
                                    class="dest-btn"
                                    class:internal={isInternal}
                                    class:external={!isInternal}
                                    onclick={() => navigateOrFocusScene(edge.to)}
                                    title={isInternal ? `Фокус на сцене ${edge.to} (в текущем файле)` : `Перейти к сцене ${edge.to} в проекте (межфайловый переход)`}
                                  >
                                    <span>{edge.to}</span>
                                    {#if isInternal}
                                      <ChevronRight size={13} />
                                    {:else}
                                      <ArrowUpRight size={12} />
                                    {/if}
                                  </button>
                                </div>
                              </div>
                            {/each}
                          </div>
                        </div>
                      {/if}

                      <!-- КНОПКА ДЕЙСТВИЯ: ПЕРЕЙТИ В ТАБЛИЦУ -->
                      <div class="node-footer-bar">
                        <button 
                          class="btn-open-scene-table"
                          onclick={() => openSceneInTable(scene.id)}
                        >
                          <Table size={13} />
                          <span>Открыть диалоги ({scene.total_lines} строк)</span>
                          <ArrowRight size={13} />
                        </button>
                      </div>
                    </div>

                    <!-- ВИЗУАЛЬНЫЙ КОННЕКТОР МЕЖДУ СЦЕНАМИ -->
                    {#if index < currentFile.scenes.length - 1}
                      <div class="flow-step-connector">
                        <div class="connector-line"></div>
                        <div class="connector-dot">
                          <ArrowRight size={12} class="rotate-90 text-sky-400" />
                        </div>
                        <div class="connector-line"></div>
                      </div>
                    {/if}
                  {/each}
                </div>
              </div>
            {:else}
              <!-- РЕЖИМ 2: СЕТКА КАРТОЧЕК -->
              <div class="scenes-flow-grid">
                {#each currentFile.scenes as scene, index}
                  {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                  {@const outgoing = getOutgoingEdges(scene.name)}
                  <div 
                    class="scene-card" 
                    class:card-selected={selectedSceneId === scene.id}
                    onclick={() => openSceneInTable(scene.id)}
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

                    <!-- Значки выборов и переходов на карточке -->
                    <div class="scene-card-badges-row">
                      {#if scene.choices && scene.choices.length > 0}
                        <span class="mini-pill pill-purple">
                          <Zap size={11} />
                          <span>{scene.choices.length} выбор</span>
                        </span>
                      {/if}
                      {#if outgoing.length > 0}
                        <span class="mini-pill pill-amber">
                          <CornerDownRight size={11} />
                          <span>{outgoing.length} переход(ов)</span>
                        </span>
                      {/if}
                    </div>

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
            {/if}
          </div>
        {:else}
          <!-- РЕЖИМ ТАБЛИЦЫ ДИАЛОГОВ -->
          <div class="table-container">
            {#if loadingFile}
              <div class="skeleton-table-wrapper">
                <div class="skeleton-scene-divider">
                  <div class="skeleton-shimmer skeleton-line w-30"></div>
                  <div class="skeleton-shimmer skeleton-line w-15"></div>
                </div>
                <div class="skeleton-rows-list">
                  {#each Array(10) as _, idx}
                    <div class="skeleton-table-row">
                      <div class="skeleton-shimmer skeleton-cell w-5"></div>
                      <div class="skeleton-shimmer skeleton-cell w-15 skeleton-badge"></div>
                      <div class="skeleton-shimmer skeleton-cell" style="width: {40 + (idx % 4) * 10}%;"></div>
                      <div class="skeleton-shimmer skeleton-cell" style="width: {30 + (idx % 3) * 10}%;"></div>
                      <div class="skeleton-shimmer skeleton-cell w-10 skeleton-badge"></div>
                    </div>
                  {/each}
                </div>
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

  <!-- ВСПЛЫВАЮЩИЙ ТОАСТ НАВИГАЦИИ -->
  {#if toast}
    <div class="toast-notification toast-{toast.type}">
      {#if toast.type === 'success'}
        <CheckCircle2 size={16} class="toast-icon text-emerald-400" />
      {:else if toast.type === 'warning'}
        <AlertCircle size={16} class="toast-icon text-amber-400" />
      {:else if toast.type === 'error'}
        <AlertCircle size={16} class="toast-icon text-rose-400" />
      {:else}
        <Sparkles size={16} class="toast-icon text-sky-400" />
      {/if}
      <span class="toast-text">{toast.text}</span>
    </div>
  {/if}
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

  /* КАСТОМНЫЕ МИНИМАЛИСТИЧНЫЕ СКРОЛЛБАРЫ */
  :global(*),
  :global(*::before),
  :global(*::after) {
    scrollbar-width: thin;
    scrollbar-color: #1e2638 transparent;
  }

  :global(*::-webkit-scrollbar),
  :global(::-webkit-scrollbar) {
    width: 6px;
    height: 6px;
  }

  :global(*::-webkit-scrollbar-track),
  :global(::-webkit-scrollbar-track) {
    background: transparent;
  }

  :global(*::-webkit-scrollbar-thumb),
  :global(::-webkit-scrollbar-thumb) {
    background: #1e2638;
    border-radius: 999px;
    border: 1px solid transparent;
    transition: background 0.2s ease;
  }

  :global(*::-webkit-scrollbar-thumb:hover),
  :global(::-webkit-scrollbar-thumb:hover) {
    background: #334155;
  }

  :global(*::-webkit-scrollbar-thumb:active),
  :global(::-webkit-scrollbar-thumb:active) {
    background: #0284c7;
  }

  :global(*::-webkit-scrollbar-corner),
  :global(::-webkit-scrollbar-corner) {
    background: transparent;
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
    width: 300px;
    background: #111419;
    border-right: 1px solid #1e232d;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .file-tree-section {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    padding: 10px 10px 6px 10px;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .icon-btn-subtle {
    background: transparent;
    border: 1px solid #202735;
    color: #94a3b8;
    border-radius: 4px;
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .icon-btn-subtle:hover {
    background: #1e2638;
    color: #38bdf8;
    border-color: #0284c7;
  }

  .sidebar-search-box {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #0d1016;
    border: 1px solid #232c3d;
    border-radius: 6px;
    padding: 5px 8px;
    margin-bottom: 6px;
  }

  .sidebar-search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-size: 11px;
    color: #e2e8f0;
    font-family: inherit;
  }

  .clear-search-pill {
    background: transparent;
    border: none;
    color: #64748b;
    font-size: 10px;
    cursor: pointer;
    padding: 0 2px;
  }

  .clear-search-pill:hover {
    color: #e2e8f0;
  }

  /* SIDEBAR TREE STYLES (IDE STYLE: COMPACT, PROPORTIONAL, CLEAN) */
  .file-tree-container {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 2px 4px 8px 4px;
    user-select: none;
  }

  .empty-tree-hint {
    padding: 24px 16px;
    text-align: center;
    color: #64748b;
    font-size: 12px;
  }

  .tree-node-group {
    display: flex;
    flex-direction: column;
  }

  /* Единый стиль строки дерева (26px, гармоничные отступы, шрифт 12px) */
  .tree-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    min-height: 26px;
    padding: 0 8px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #cbd5e1;
    font-size: 12px;
    font-weight: 400;
    cursor: pointer;
    text-align: left;
    width: 100%;
    box-sizing: border-box;
    transition: background 0.1s ease, color 0.1s ease, border-color 0.1s ease;
  }

  .tree-row:hover {
    background: #171d27;
    color: #f8fafc;
  }

  .tree-file-row.active {
    background: #152336 !important;
    border: 1px solid #233e62 !important;
    color: #ffffff !important;
  }

  .tree-file-row.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    width: 2.5px;
    background: #38bdf8;
    border-radius: 999px;
  }

  .tree-caret {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    color: #64748b;
    flex-shrink: 0;
  }

  .tree-caret-spacer {
    display: inline-flex;
    width: 14px;
    flex-shrink: 0;
  }

  .tree-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    flex-shrink: 0;
  }

  .tree-label {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 12px;
  }

  .folder-label {
    font-weight: 500;
    color: #e2e8f0;
  }

  .file-label {
    color: #cbd5e1;
  }

  .tree-meta {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .tree-count {
    color: #475569;
    font-size: 10px;
  }

  .tree-pct-badge {
    font-size: 10px;
    padding: 1px 4px;
    border-radius: 3px;
    font-weight: 500;
  }

  .tree-pct-badge.pct-complete {
    color: #34d399;
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(52, 211, 153, 0.25);
  }

  .tree-pct-badge.pct-progress {
    color: #94a3b8;
    background: #151b24;
  }

  /* Мини-капсула прогресса в правой колонке метаданных строки */
  .tree-progress-capsule {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #131922;
    padding: 1px 4px;
    border-radius: 3px;
    border: 1px solid #1f2735;
  }

  .tree-capsule-track {
    width: 20px;
    height: 3px;
    background: #252f40;
    border-radius: 999px;
    overflow: hidden;
  }

  .tree-capsule-fill {
    height: 100%;
    background: #0284c7;
    border-radius: 999px;
  }

  .tree-capsule-val {
    font-size: 10px;
    color: #94a3b8;
    font-weight: 500;
    min-width: 20px;
    text-align: right;
  }

  .tree-row.active .tree-progress-capsule {
    background: #0f1c2d;
    border-color: #213c5f;
  }

  .tree-row.active .tree-capsule-val {
    color: #bae6fd;
  }

  /* Иерархическая ветка: тонкая направляющая линия */
  .tree-branch-children {
    position: relative;
    margin-left: 14px;
    padding-left: 4px;
    border-left: 1px solid #1a2230;
    display: flex;
    flex-direction: column;
  }

  /* АНИМАЦИЯ ПОДСВЕТКИ ПРИ НАВИГАЦИИ */
  @keyframes fileGlowPulse {
    0% {
      background-color: #1e3a5f;
      border-color: #38bdf8 !important;
      box-shadow: 0 0 12px rgba(56, 189, 248, 0.4);
    }
    50% {
      background-color: #1a2d47;
      border-color: #0284c7 !important;
    }
    100% {
      background-color: #152336;
      border-color: #233e62 !important;
      box-shadow: none;
    }
  }

  .file-highlight-pulse {
    animation: fileGlowPulse 1.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  /* СЕКЦИЯ ПЕРСОНАЖЕЙ (КОМПАКТНЫЙ АККОРДЕОН) */
  .characters-section {
    border-top: 1px solid #1a202c;
    padding: 4px 6px;
    flex-shrink: 0;
    background: #0f1217;
  }

  .characters-toggle-header {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px;
    background: transparent;
    border: none;
    color: #94a3b8;
    cursor: pointer;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.5px;
    border-radius: 4px;
    transition: background 0.1s, color 0.1s;
  }

  .characters-toggle-header:hover {
    background: #161c24;
    color: #e2e8f0;
  }

  .characters-state-hint {
    font-size: 10px;
    color: #475569;
    text-transform: lowercase;
  }

  .char-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 8px 8px 8px;
    max-height: 140px;
    overflow-y: auto;
  }

  .char-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #141822;
    border: 1px solid #202838;
    padding: 2px 6px;
    border-radius: 4px;
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
    scrollbar-width: thin;
    scrollbar-color: #1e2638 transparent;
  }

  .scenes-nav-bar::-webkit-scrollbar {
    height: 4px;
  }

  .scenes-nav-bar::-webkit-scrollbar-track {
    background: transparent;
  }

  .scenes-nav-bar::-webkit-scrollbar-thumb {
    background: #1e2638;
    border-radius: 999px;
    transition: background 0.15s ease;
  }

  .scenes-nav-bar::-webkit-scrollbar-thumb:hover {
    background: #38bdf8;
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

  /* STORY GRAPH & FLOW VIEW */
  .story-graph-wrapper {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: #0b0d12;
    background-image: radial-gradient(#181f2c 1px, transparent 1px);
    background-size: 20px 20px;
    overflow: hidden;
  }

  .graph-topbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 18px;
    background: #0f131a;
    border-bottom: 1px solid #1a202c;
  }

  .graph-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .graph-heading {
    font-size: 13px;
    font-weight: 600;
    color: #e2e8f0;
  }

  .graph-pill {
    font-size: 11px;
    background: #18202d;
    border: 1px solid #232d3f;
    color: #94a3b8;
    padding: 2px 7px;
    border-radius: 999px;
  }

  .graph-pill.pill-purple {
    background: rgba(168, 85, 247, 0.12);
    border-color: rgba(168, 85, 247, 0.3);
    color: #c084fc;
  }

  .graph-pill.pill-amber {
    background: rgba(245, 158, 11, 0.12);
    border-color: rgba(245, 158, 11, 0.3);
    color: #fbbf24;
  }

  .graph-submode-toggles {
    display: flex;
    gap: 4px;
    background: #141923;
    padding: 3px;
    border-radius: 6px;
    border: 1px solid #1e2636;
  }

  .graph-mode-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    background: transparent;
    border: none;
    border-radius: 4px;
    font-size: 12px;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s;
  }

  .graph-mode-btn.active {
    background: #0284c7;
    color: #ffffff;
  }

  /* FLOW CANVAS */
  .story-flow-canvas {
    flex: 1;
    overflow-y: auto;
    padding: 28px 20px;
  }

  .story-nodes-track {
    max-width: 820px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .flow-scene-node {
    width: 100%;
    background: #121620;
    border: 1px solid #202838;
    border-radius: 10px;
    padding: 16px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    transition: all 0.2s;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .flow-scene-node:hover {
    border-color: #2e3a52;
    background: #151a26;
  }

  .flow-scene-node.node-focused {
    border-color: #38bdf8;
    box-shadow: 0 0 0 2px rgba(56, 189, 248, 0.3);
  }

  .flow-scene-node.node-selected {
    border-color: #0284c7;
  }

  .node-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .node-title-group {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .node-seq {
    font-size: 11px;
    font-weight: 700;
    color: #64748b;
    font-family: monospace;
    background: #0b0e14;
    padding: 2px 6px;
    border-radius: 4px;
  }

  .node-name {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: #f8fafc;
  }

  .node-tag {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 4px;
    text-transform: uppercase;
  }

  .node-tag.tag-ui {
    background: #1e293b;
    color: #94a3b8;
  }

  .node-tag.tag-choice {
    background: rgba(168, 85, 247, 0.2);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.4);
  }

  .node-progress-badge {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    background: #18202d;
    padding: 3px 8px;
    border-radius: 6px;
    color: #94a3b8;
  }

  .node-progress-badge.badge-complete {
    background: rgba(16, 185, 129, 0.15);
    color: #34d399;
  }

  .node-progress-line {
    height: 3px;
    background: #1b2230;
    border-radius: 999px;
    overflow: hidden;
  }

  .node-fill {
    height: 100%;
    background: #38bdf8;
    transition: width 0.3s;
  }

  /* Входящие связи */
  .node-incoming-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #0f131a;
    padding: 6px 10px;
    border-radius: 6px;
    border-left: 3px solid #38bdf8;
  }

  .box-label {
    font-size: 11px;
    color: #64748b;
    white-space: nowrap;
  }

  .edge-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .edge-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #192130;
    border: 1px solid #27344a;
    color: #cbd5e1;
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 4px;
    cursor: pointer;
  }

  .edge-chip:hover {
    background: #232e42;
    border-color: #38bdf8;
    color: #38bdf8;
  }

  .edge-condition-sub {
    color: #fbbf24;
    font-size: 10px;
  }

  /* Превью диалога */
  .node-dialogue-snippet {
    background: #0d1016;
    border: 1px solid #1a202c;
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 12px;
    line-height: 1.4;
    color: #94a3b8;
  }

  .snippet-speaker {
    font-weight: 600;
    color: #38bdf8;
    margin-right: 4px;
  }

  .snippet-text {
    font-style: italic;
  }

  /* ВЫБОРЫ В СЦЕНЕ (MENU:) */
  .node-choices-section {
    background: rgba(168, 85, 247, 0.06);
    border: 1px solid rgba(168, 85, 247, 0.25);
    border-radius: 8px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .choices-section-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #c084fc;
  }

  .choice-options-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-left: 8px;
  }

  .choice-option-pill {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #141724;
    border: 1px solid rgba(168, 85, 247, 0.3);
    border-radius: 6px;
    padding: 5px 10px;
    font-size: 12px;
  }

  .opt-bullet {
    color: #c084fc;
    font-size: 10px;
  }

  .opt-title {
    color: #f1f5f9;
    font-weight: 500;
  }

  .opt-arrow {
    color: #64748b;
    font-size: 11px;
  }

  .target-link {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 4px;
  }

  .target-link.link-internal {
    background: #0284c7;
    color: #ffffff;
    border: none;
    cursor: pointer;
    transition: background 0.15s;
  }

  .target-link.link-internal:hover {
    background: #0369a1;
  }

  .target-link.link-external {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #182232;
    border: 1px solid #273750;
    color: #93c5fd;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .target-link.link-external:hover {
    background: #202d44;
    border-color: #38bdf8;
    color: #ffffff;
  }

  /* ИСХОДЯЩИЕ ПЕРЕХОДЫ И ВЕТВЛЕНИЯ */
  .node-jumps-section {
    background: rgba(245, 158, 11, 0.05);
    border: 1px solid rgba(245, 158, 11, 0.22);
    border-radius: 8px;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .jumps-section-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #fbbf24;
  }

  .outgoing-chips-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .outgoing-edge-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: #141920;
    border: 1px solid #232c3d;
    border-radius: 6px;
    padding: 6px 10px;
  }

  .edge-cond-badge {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: #fbbf24;
    font-weight: 500;
  }

  .edge-dest-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .edge-arrow {
    color: #64748b;
    font-size: 11px;
  }

  .dest-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .dest-btn.internal {
    background: #0369a1;
    color: #ffffff;
    border: 1px solid #0284c7;
  }

  .dest-btn.internal:hover {
    background: #0284c7;
    box-shadow: 0 0 10px rgba(56, 189, 248, 0.3);
  }

  .dest-btn.external {
    background: #182232;
    border: 1px solid #273750;
    color: #93c5fd;
  }

  .dest-btn.external:hover {
    background: #202d44;
    border-color: #38bdf8;
    color: #ffffff;
    box-shadow: 0 0 12px rgba(56, 189, 248, 0.25);
  }

  /* ВСПЛЫВАЮЩИЙ ТОАСТ НАВИГАЦИИ */
  .toast-notification {
    position: fixed;
    bottom: 24px;
    right: 24px;
    z-index: 9999;
    display: flex;
    align-items: center;
    gap: 10px;
    background: #121620;
    border: 1px solid #243044;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.7);
    padding: 10px 16px;
    border-radius: 8px;
    font-size: 13px;
    color: #f1f5f9;
    animation: toastSlideUp 0.22s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .toast-notification.toast-success {
    background: #0a1b16;
    border-color: rgba(52, 211, 153, 0.4);
    color: #ecfdf5;
  }

  .toast-notification.toast-warning {
    background: #1c180e;
    border-color: rgba(251, 191, 36, 0.4);
    color: #fef3c7;
  }

  .toast-notification.toast-error {
    background: #1f1013;
    border-color: rgba(244, 63, 94, 0.4);
    color: #ffe4e6;
  }

  @keyframes toastSlideUp {
    from {
      opacity: 0;
      transform: translateY(12px) scale(0.97);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  /* ФУТЕР УЗЛА */
  .node-footer-bar {
    display: flex;
    justify-content: flex-end;
    padding-top: 4px;
    border-top: 1px solid #1a2230;
  }

  .btn-open-scene-table {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #182234;
    border: 1px solid #263550;
    color: #38bdf8;
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-open-scene-table:hover {
    background: #0284c7;
    color: #ffffff;
    border-color: #0284c7;
  }

  /* КОННЕКТОР МЕЖДУ УЗЛАМИ */
  .flow-step-connector {
    display: flex;
    flex-direction: column;
    align-items: center;
    height: 36px;
  }

  .connector-line {
    width: 2px;
    flex: 1;
    background: #232d3f;
  }

  .connector-dot {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: #141a24;
    border: 2px solid #28354c;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* СЕТКА КАРТОЧЕК */
  .scenes-flow-grid {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
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

  .scene-card-badges-row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .mini-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 4px;
  }

  .mini-pill.pill-purple {
    background: rgba(168, 85, 247, 0.15);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.3);
  }

  .mini-pill.pill-amber {
    background: rgba(245, 158, 11, 0.15);
    color: #fbbf24;
    border: 1px solid rgba(245, 158, 11, 0.3);
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

  /* GLOBAL SPIN ANIMATION */
  :global(.spin) {
    animation: spin 0.8s linear infinite !important;
    display: inline-block;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  /* SKELETON SHIMMER PLACEHOLDERS */
  @keyframes shimmer {
    0% {
      background-position: -200% 0;
    }
    100% {
      background-position: 200% 0;
    }
  }

  .skeleton-shimmer {
    background: linear-gradient(
      90deg,
      #131822 0%,
      #1e2636 30%,
      #2c394e 50%,
      #1e2636 70%,
      #131822 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
    border-radius: 4px;
  }

  .skeleton-line {
    height: 12px;
  }

  .skeleton-stats-box {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 130px;
  }

  .skeleton-pill-head {
    height: 14px;
    width: 90px;
  }

  .w-5 { width: 5%; }
  .w-10 { width: 10%; }
  .w-15 { width: 15%; }
  .w-20 { width: 20%; }
  .w-30 { width: 30%; }
  .w-40 { width: 40%; }
  .w-45 { width: 45%; }
  .w-50 { width: 50%; }
  .w-60 { width: 60%; }
  .w-75 { width: 75%; }
  .w-80 { width: 80%; }
  .w-100 { width: 100%; }

  .skeleton-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
  }

  .skeleton-file-item {
    padding: 10px;
    background: #11141c;
    border: 1px solid #1a202c;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .skeleton-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px;
  }

  .skeleton-chip-item {
    height: 22px;
    width: 65px;
    border-radius: 4px;
  }

  .skeleton-table-wrapper {
    display: flex;
    flex-direction: column;
    padding: 14px;
    gap: 12px;
  }

  .skeleton-scene-divider {
    display: flex;
    justify-content: space-between;
    padding: 10px 14px;
    background: #131722;
    border-radius: 6px;
    border: 1px solid #1e2638;
  }

  .skeleton-rows-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .skeleton-table-row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 14px;
    background: #0f121a;
    border: 1px solid #181f2c;
    border-radius: 6px;
  }

  .skeleton-cell {
    height: 14px;
  }

  .skeleton-badge {
    height: 20px;
    border-radius: 4px;
  }

  .loading-project-workspace {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #0b0e14;
  }

  .workspace-shimmer-card {
    background: #121622;
    border: 1px solid #1e2638;
    border-radius: 12px;
    padding: 36px 48px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    text-align: center;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4);
    max-width: 420px;
    width: 100%;
  }

  .workspace-shimmer-card h3 {
    margin: 0;
    font-size: 16px;
    color: #f1f5f9;
  }

  .workspace-shimmer-card p {
    margin: 0;
    font-size: 13px;
    color: #64748b;
  }

  .skeleton-bar-wide {
    height: 6px;
    width: 100%;
    margin-top: 8px;
    border-radius: 999px;
  }
</style>
