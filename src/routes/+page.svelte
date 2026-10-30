<script lang="ts">
  import { onMount, tick } from "svelte";
  import SceneGraph from "../lib/SceneGraph.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
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
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Menu as MenuIcon,
    Network,
    Package,
    Archive,
    Trash2,
    FileCode,
    Download,
    Upload,
    X,
    Pencil,
    Copy,
    BookOpen,
    Plus,
    Save,
    BookMarked
  } from "lucide-svelte";

  // Стейт проекта на Svelte 5 (Runes)
  let gameDir = $state("F:\\Playnite\\Games\\0b588a07-1844-467f-8459-e5629642a1bf\\Installed\\game");
  let recentProjects = $state<string[]>([]);
  let projectNames = $state<Record<string, string>>({});
  let editingProjectPath = $state<string | null>(null);
  let editingProjectName = $state("");
  let showRecentProjects = $state(false);
  let showToolsMenu = $state(false);
  let showProjectManager = $state(false);
  let projectManagerSearch = $state("");
  let loadingProject = $state(false);
  let loadingFile = $state(false);
  let errorMsg = $state("");
  let filteredRecentProjects = $derived.by(() => {
    const query = projectManagerSearch.trim().toLocaleLowerCase();
    if (!query) return recentProjects;
    return recentProjects.filter(path =>
      path.toLocaleLowerCase().includes(query) || projectDisplayName(path).toLocaleLowerCase().includes(query)
    );
  });

  const RECENT_PROJECTS_KEY = "translator.recent-projects";
  const PROJECT_NAMES_KEY = "translator.project-names";

  onMount(() => {
    try {
      const saved = JSON.parse(localStorage.getItem(RECENT_PROJECTS_KEY) || "[]");
      if (Array.isArray(saved)) recentProjects = saved.filter((path): path is string => typeof path === "string");
    } catch {
      recentProjects = [];
    }

    try {
      const savedNames = JSON.parse(localStorage.getItem(PROJECT_NAMES_KEY) || "{}");
      if (savedNames && typeof savedNames === "object" && !Array.isArray(savedNames)) {
        projectNames = Object.fromEntries(Object.entries(savedNames).filter((entry): entry is [string, string] => typeof entry[1] === "string"));
      }
    } catch {
      projectNames = {};
    }

    if (recentProjects.length > 0) gameDir = recentProjects[0];
  });

  function rememberProject(path: string) {
    const normalized = path.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();
    recentProjects = [path, ...recentProjects.filter(item =>
      item.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase() !== normalized
    )];
    persistRecentProjects();
  }

  function persistRecentProjects() {
    try {
      localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(recentProjects));
    } catch {
      // Список останется доступен до закрытия приложения, даже если хранилище недоступно.
    }
  }

  function projectKey(path: string) {
    return path.replace(/\\/g, "/").replace(/\/$/, "").toLocaleLowerCase();
  }

  function projectDisplayName(path: string) {
    const customName = projectNames[projectKey(path)]?.trim();
    if (customName) return customName;
    const parts = path.replace(/\\/g, "/").replace(/\/$/, "").split("/");
    const lastPart = parts.at(-1) || path;
    return lastPart.toLowerCase() === "game" ? parts.at(-2) || lastPart : lastPart;
  }

  function beginProjectRename(path: string) {
    editingProjectPath = path;
    editingProjectName = projectDisplayName(path);
  }

  function saveProjectName(path: string) {
    const key = projectKey(path);
    const name = editingProjectName.trim();
    if (name) {
      projectNames = { ...projectNames, [key]: name };
    } else {
      const { [key]: _removed, ...remainingNames } = projectNames;
      projectNames = remainingNames;
    }
    try {
      localStorage.setItem(PROJECT_NAMES_KEY, JSON.stringify(projectNames));
    } catch {
      // Новое имя сохранится в памяти до закрытия приложения.
    }
    editingProjectPath = null;
  }

  function cancelProjectRename() {
    editingProjectPath = null;
    editingProjectName = "";
  }

  function removeRecentProject(path: string) {
    const normalized = projectKey(path);
    recentProjects = recentProjects.filter(item => projectKey(item) !== normalized);
    const { [normalized]: _removed, ...remainingNames } = projectNames;
    projectNames = remainingNames;
    try {
      localStorage.setItem(PROJECT_NAMES_KEY, JSON.stringify(projectNames));
    } catch {
      // Удаление из списка всё равно действует в текущем запуске.
    }
    persistRecentProjects();
  }

  function openProjectManager() {
    showRecentProjects = false;
    projectManagerSearch = "";
    showProjectManager = true;
  }

  async function pickProjectFolder() {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Выберите папку игры или проекта",
        defaultPath: gameDir.trim() || undefined
      });
      if (typeof selected === "string") {
        gameDir = selected;
        await scanProject();
      }
    } catch (e) {
      errorMsg = `Не удалось открыть выбор папки: ${String(e)}`;
    }
  }

  function chooseRecentProject(path: string) {
    gameDir = path;
    showRecentProjects = false;
    scanProject();
  }

  function closeRecentProjects(event: PointerEvent) {
    if (!(event.target instanceof Element) || !event.target.closest(".recent-project-picker")) {
      showRecentProjects = false;
    }
    if (!(event.target instanceof Element) || !event.target.closest(".tools-menu-wrap")) {
      showToolsMenu = false;
    }
    if (!(event.target instanceof Element) || !event.target.closest(".table-scene-picker-wrap")) {
      showScenePicker = false;
    }
  }

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
    speaker_alias?: string | null;
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

  // Интерфейсы глоссария проекта
  interface CharacterGlossaryEntry {
    alias: string;
    original_name: string;
    translated_name: string;
    gender: string; // "male" | "female" | "unknown"
    notes: string;
  }

  interface TermGlossaryEntry {
    id: string;
    source_term: string;
    translated_term: string;
    category: string; // "general" | "location" | "title" | "item"
    notes: string;
  }

  interface ProjectGlossary {
    characters: CharacterGlossaryEntry[];
    terms: TermGlossaryEntry[];
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

  // Состояние навигации по сценам (сайдбар и тулбар)
  let showGraphSidebar = $state(true);
  let showScenePicker = $state(false);
  let scenePickerSearch = $state("");
  let graphSceneSearch = $state("");

  // Сцены, доступные в табличном виде:
  // По умолчанию отображаются только сцены с диалогами (total_lines > 0).
  // Но если пользователь перешел из графа, выбрав сцену-селектор/хаб без диалогов (s.id === selectedSceneId),
  // она также включается в список!
  let tableScenes = $derived.by(() => {
    if (!currentFile) return [];
    return currentFile.scenes.filter(s => s.total_lines > 0 || s.id === selectedSceneId);
  });

  // Текущая выбранная сцена для селектора в таблице
  let currentSceneIndex = $derived.by(() => {
    if (!currentFile || selectedSceneId === "all") return -1;
    return tableScenes.findIndex(s => s.id === selectedSceneId);
  });

  let currentScene = $derived.by(() => {
    if (!currentFile || selectedSceneId === "all") return null;
    return currentFile.scenes.find(s => s.id === selectedSceneId) ?? null;
  });

  let isPrevSceneDisabled = $derived.by(() => {
    if (tableScenes.length === 0) return true;
    if (selectedSceneId === "all") return true;
    return currentSceneIndex <= 0;
  });

  let isNextSceneDisabled = $derived.by(() => {
    if (tableScenes.length === 0) return true;
    if (selectedSceneId === "all") return tableScenes.length === 0;
    return currentSceneIndex >= tableScenes.length - 1;
  });

  function goToPrevScene() {
    if (tableScenes.length === 0) return;
    if (selectedSceneId === "all") return;
    const idx = currentSceneIndex;
    if (idx > 0) {
      selectedSceneId = tableScenes[idx - 1].id;
    } else if (idx === 0) {
      selectedSceneId = "all";
    }
  }

  function goToNextScene() {
    if (tableScenes.length === 0) return;
    if (selectedSceneId === "all") {
      selectedSceneId = tableScenes[0].id;
      return;
    }
    const idx = currentSceneIndex;
    if (idx >= 0 && idx < tableScenes.length - 1) {
      selectedSceneId = tableScenes[idx + 1].id;
    }
  }

  function selectScene(sceneId: string) {
    selectedSceneId = sceneId;
    showScenePicker = false;
  }

  // Фильтрация сцен для селектора в таблице
  let filteredScenesForPicker = $derived.by(() => {
    if (!currentFile) return [];
    const q = scenePickerSearch.trim().toLowerCase();
    if (!q) return tableScenes;
    return currentFile.scenes.filter(s => s.name.toLowerCase().includes(q));
  });

  // Фильтрация сцен для аутлайнера графа в сайдбаре
  let filteredScenesForGraph = $derived.by(() => {
    if (!currentFile) return [];
    const q = graphSceneSearch.trim().toLowerCase();
    if (!q) return currentFile.scenes;
    return currentFile.scenes.filter(s => s.name.toLowerCase().includes(q));
  });

  // Фокус и плавный скролл к узлу в графе (для обоих режимов: flow и grid)
  function focusAndScrollSceneInGraph(scene: Scene) {
    selectedSceneId = scene.id;
    focusedSceneName = scene.name;
    const targetId = graphSubMode === "flow" 
      ? `scene-node-${scene.id}` 
      : `scene-grid-card-${scene.id}`;
    const el = document.getElementById(targetId);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }

  // Статистика графа
  let totalChoicesCount = $derived(
    currentFile ? currentFile.scenes.reduce((acc, s) => acc + (s.choices ? s.choices.length : 0), 0) : 0
  );

  let totalEdgesCount = $derived(
    currentFile ? (currentFile.edges ? currentFile.edges.length : 0) : 0
  );

  // Быстрый кэш маппинга персонажей: alias -> name и name -> alias
  let charAliasToName = $derived.by(() => {
    const map = new Map<string, string>();
    if (project?.characters) {
      for (const [alias, name] of project.characters) {
        map.set(alias.toLowerCase(), name);
      }
    }
    return map;
  });

  let charNameToAlias = $derived.by(() => {
    const map = new Map<string, string>();
    if (project?.characters) {
      for (const [alias, name] of project.characters) {
        map.set(name.toLowerCase(), alias);
      }
    }
    return map;
  });

  // Состояние глоссария проекта
  let showGlossaryModal = $state(false);
  let glossaryTab = $state<'characters' | 'terms'>('characters');
  let glossary = $state<ProjectGlossary>({ characters: [], terms: [] });
  let loadingGlossary = $state(false);
  let savingGlossary = $state(false);
  let glossaryDirty = $state(false);
  let selectedGlossaryChar = $state<CharacterGlossaryEntry | null>(null);
  let selectedGlossaryTerm = $state<TermGlossaryEntry | null>(null);

  // Фильтры глоссария
  let glossarySearch = $state("");
  let glossaryGenderFilter = $state<"all" | "male" | "female" | "neuter" | "unknown">("all");
  let glossaryCategoryFilter = $state<"all" | "general" | "location" | "title" | "item">("all");

  // Добавление термина
  let newTermSource = $state("");
  let newTermTranslation = $state("");
  let newTermCategory = $state("general");
  let newTermNotes = $state("");

  // Добавление персонажа вручную
  let showAddChar = $state(false);
  let newCharAlias = $state("");
  let newCharName = $state("");
  let newCharTranslation = $state("");
  let newCharGender = $state("unknown");

  // Быстрый кэш маппинга персонажей глоссария
  let glossaryCharByAlias = $derived.by(() => {
    const map = new Map<string, CharacterGlossaryEntry>();
    for (const c of glossary.characters) {
      map.set(c.alias.toLowerCase(), c);
    }
    return map;
  });

  let glossaryCharByName = $derived.by(() => {
    const map = new Map<string, CharacterGlossaryEntry>();
    for (const c of glossary.characters) {
      map.set(c.original_name.toLowerCase(), c);
    }
    return map;
  });

  function getSpeakerInfo(speaker: string | null | undefined, speakerAlias?: string | null): { 
    alias: string | null; 
    name: string; 
    translatedName: string | null;
    gender: string;
    notes: string;
    isNarrator: boolean; 
    isUi: boolean;
  } {
    if (!speaker && !speakerAlias) {
      return { alias: null, name: "Автор", translatedName: null, gender: "unknown", notes: "", isNarrator: true, isUi: false };
    }

    if (speaker === "UI / Choice" || speakerAlias === "UI") {
      return { alias: "UI", name: "Интерфейс", translatedName: null, gender: "unknown", notes: "", isNarrator: false, isUi: true };
    }

    let alias: string | null = null;
    let name = (speaker || speakerAlias || "").trim();

    // 1. Если бэкенд передал оба поля и они различаются
    if (speakerAlias && speaker && speakerAlias !== speaker) {
      alias = speakerAlias;
      name = speaker;
    } else {
      const raw = name;
      const lower = raw.toLowerCase();

      // 2. Если raw — это известный алиас (например, "lis" -> alias="lis", name="Lisa")
      if (charAliasToName.has(lower)) {
        alias = raw;
        name = charAliasToName.get(lower)!;
      }
      // 3. Если raw — это известное имя (например, "Lisa" -> alias="lis", name="Lisa")
      else if (charNameToAlias.has(lower)) {
        alias = charNameToAlias.get(lower)!;
        name = raw;
      } else if (speakerAlias) {
        alias = speakerAlias;
        name = raw;
      } else {
        alias = raw;
        name = raw;
      }
    }

    // Сопоставление с глоссарием проекта
    let glEntry: CharacterGlossaryEntry | undefined;
    if (alias && glossaryCharByAlias.has(alias.toLowerCase())) {
      glEntry = glossaryCharByAlias.get(alias.toLowerCase());
    } else if (name && glossaryCharByName.has(name.toLowerCase())) {
      glEntry = glossaryCharByName.get(name.toLowerCase());
    }

    const translatedName = glEntry?.translated_name?.trim() ? glEntry.translated_name.trim() : null;
    const gender = glEntry?.gender || "unknown";
    const notes = glEntry?.notes || "";

    return { 
      alias, 
      name, 
      translatedName,
      gender,
      notes,
      isNarrator: false, 
      isUi: false 
    };
  }

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
    return currentFile.scenes.some(s => s.name === targetName);
  }

  // Сфокусироваться на сцене или перейти в неё
  function focusScene(sceneName: string) {
    if (!currentFile) return;
    let targetScene = currentFile.scenes.find(s => s.name === sceneName);
    if (targetScene) {
      selectedSceneId = targetScene.id;
      focusedSceneName = targetScene.name;
      const targetId = graphSubMode === 'flow' 
        ? `scene-node-${targetScene.id}` 
        : `scene-grid-card-${targetScene.id}`;
      const el = document.getElementById(targetId);
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
    if (!targetName || graphNavigating) return;

    // 1. Если сцена есть в текущем файле — моментальный фокус
    if (isInternalScene(targetName)) {
      focusScene(targetName);
      showToast(`Сцена "${targetName}" в фокусе (текущий файл)`, "info");
      return;
    }

    graphNavigating = true;
    const origin = selectedFile ? { file: selectedFile, sceneId: selectedSceneId } : null;
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
        showToast(`Метка "${targetName}" не найдена в доступных файлах проекта`, "warning");
        return;
      }

      // Нормализуем пути для надежного сравнения (слэши, подпапки, имя файла)
      const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase();
      const targetPath = norm(found.file_path);

      let targetFile = project?.files.find(f => {
        const fp = norm(f.path);
        return fp === targetPath;
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
        if (!await selectFile(targetFile)) return;
        if (origin) graphHistory = [...graphHistory, origin];
        await tick();
        focusScene(found.scene_name);
        showToast(isInternalScene(found.scene_name)
          ? 'Открыт граф ' + found.short_name + ' → ' + found.scene_name
          : 'Файл открыт, но метка отсутствует среди разобранных сцен: ' + targetName,
          isInternalScene(found.scene_name) ? 'success' : 'warning');
      } else {
        showToast(`Сцена найдена в ${found.short_name}:${found.line_number}, но файл не найден`, "warning");
      }
    } catch (e) {
      showToast(`Ошибка поиска: ${e}`, "error");
    } finally {
      graphNavigating = false;
    }
  }

  let graphNavigating = $state(false);
  let graphHistory = $state<{ file: TranslationFile; sceneId: string }[]>([]);

  async function backToGraph() {
    const previous = graphHistory.at(-1);
    if (!previous || graphNavigating) return;
    graphNavigating = true;
    try {
      if (await selectFile(previous.file)) {
        graphHistory = graphHistory.slice(0, -1);
        selectedSceneId = previous.sceneId;
      }
    } finally { graphNavigating = false; }
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
      const selectedPath = gameDir.trim();
      const scanPath = await invoke<string>("resolve_game_directory", { path: selectedPath });
      project = await invoke<ProjectSummary>("scan_game_project", { gameDir: scanPath });
      graphHistory = [];
      rememberProject(selectedPath);
      loadingProject = false;
      loadGlossary(true);
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
      focusedSceneName = null;
      // По умолчанию выбираем первую реальную сюжетную сцену с диалогами
      if (currentFile.scenes.length > 1) {
        const firstStoryWithLines = currentFile.scenes.find(s => s.name !== "Интерфейс и меню (strings)" && s.total_lines > 0);
        const firstStory = currentFile.scenes.find(s => s.name !== "Интерфейс и меню (strings)");
        selectedSceneId = firstStoryWithLines ? firstStoryWithLines.id : (firstStory ? firstStory.id : currentFile.scenes[0].id);
      } else if (currentFile.scenes.length > 0) {
        selectedSceneId = currentFile.scenes[0].id;
      } else {
        selectedSceneId = "all";
      }

      if (currentFile.scenes.length > 0 && currentFile.scenes[0].entries.length > 0) {
        selectedEntry = currentFile.scenes[0].entries[0];
      }
      return true;
    } catch (e) {
      errorMsg = String(e);
      return false;
    } finally {
      loadingFile = false;
    }
  }

  // Управление студией RPA и RPYC
  let showToolsModal = $state(false);
  let toolsActiveTab = $state<'rpa' | 'rpyc'>('rpa');

  let packingMod = $state(false);
  let cleaningCache = $state(false);
  let unpackingRpa = $state(false);
  let decompilingRpyc = $state(false);

  // Поля вкладки RPA
  let unpackSourceRpa = $state("");
  let unpackTargetDir = $state("");
  let unpackResultMsg = $state("");
  let packModCustomPath = $state("");

  // Поля вкладки RPYC
  let decompileRpycPath = $state("");
  let decompileOutputRpyPath = $state("");
  let decompiledCodePreview = $state("");
  let cleanTlOnly = $state(true);

  // 1. Быстрая сборка RPA мода в один клик
  async function quickPackMod() {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    packingMod = true;
    try {
      const msg = await invoke<string>("pack_mod_rpa", {
        gameDir: gameDir.trim(),
        outputRpaPath: null
      });
      showToast(msg, "success");
    } catch (e) {
      showToast(`Ошибка сборки мода: ${e}`, "error");
    } finally {
      packingMod = false;
    }
  }

  // 2. Быстрая очистка кэша .rpyc
  async function quickCleanCache() {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    cleaningCache = true;
    try {
      const count = await invoke<number>("clean_rpyc_cache", {
        gameDir: gameDir.trim(),
        tlOnly: true
      });
      showToast(`Кэш .rpyc очищен (${count} файлов). Ren'Py соберет свежий байткод!`, "success");
    } catch (e) {
      showToast(`Ошибка очистки кэша: ${e}`, "error");
    } finally {
      cleaningCache = false;
    }
  }

  // 3. Распаковка произвольного RPA архива
  async function executeUnpackRpa() {
    if (!unpackSourceRpa.trim() || !unpackTargetDir.trim()) {
      showToast("Укажите путь к .rpa архиву и целевую папку", "warning");
      return;
    }
    unpackingRpa = true;
    unpackResultMsg = "";
    try {
      const count = await invoke<number>("unpack_rpa", {
        rpaPath: unpackSourceRpa.trim(),
        targetDir: unpackTargetDir.trim()
      });
      unpackResultMsg = `Успешно распаковано ${count} файлов в "${unpackTargetDir.trim()}"`;
      showToast(unpackResultMsg, "success");
    } catch (e) {
      unpackResultMsg = `Ошибка: ${e}`;
      showToast(unpackResultMsg, "error");
    } finally {
      unpackingRpa = false;
    }
  }

  // 4. Сборка RPA мода с кастомным путем
  async function executePackCustomRpa() {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    packingMod = true;
    try {
      const msg = await invoke<string>("pack_mod_rpa", {
        gameDir: gameDir.trim(),
        outputRpaPath: packModCustomPath.trim() || null
      });
      showToast(msg, "success");
    } catch (e) {
      showToast(`Ошибка сборки: ${e}`, "error");
    } finally {
      packingMod = false;
    }
  }

  // 5. Декомпиляция .rpyc в .rpy
  async function executeDecompileRpyc() {
    if (!decompileRpycPath.trim()) {
      showToast("Укажите путь к .rpyc файлу", "warning");
      return;
    }
    decompilingRpyc = true;
    try {
      const code = await invoke<string>("decompile_rpyc_file", {
        rpycPath: decompileRpycPath.trim(),
        outputRpyPath: decompileOutputRpyPath.trim() || null
      });
      decompiledCodePreview = code;
      const lines = code.split("\n").length;
      showToast(`Декомпиляция завершена: ${lines} строк кода!`, "success");
    } catch (e) {
      showToast(`Ошибка декомпиляции: ${e}`, "error");
    } finally {
      decompilingRpyc = false;
    }
  }

  // 6. Копирование декомпилированного кода в буфер
  async function copyDecompiledCode() {
    if (!decompiledCodePreview) return;
    try {
      await navigator.clipboard.writeText(decompiledCodePreview);
      showToast("Код скопирован в буфер обмена!", "success");
    } catch {
      showToast("Не удалось скопировать код", "error");
    }
  }

  // 7. Очистка кэша из модального окна
  async function executeCleanCacheModal() {
    if (!gameDir.trim()) return;
    cleaningCache = true;
    try {
      const count = await invoke<number>("clean_rpyc_cache", {
        gameDir: gameDir.trim(),
        tlOnly: cleanTlOnly
      });
      showToast(`Удалено ${count} кэш-файлов .rpyc (${cleanTlOnly ? 'только tl/russian' : 'вся папка game'})`, "success");
    } catch (e) {
      showToast(`Ошибка очистки: ${e}`, "error");
    } finally {
      cleaningCache = false;
    }
  }

  // --- УПРАВЛЕНИЕ ГЛОССАРИЕМ ПРОЕКТА ---
  let filteredGlossaryCharacters = $derived.by(() => {
    const q = glossarySearch.trim().toLowerCase();
    return glossary.characters.filter(c => {
      if (glossaryGenderFilter !== 'all' && c.gender !== glossaryGenderFilter) {
        return false;
      }
      if (!q) return true;
      return (
        c.alias.toLowerCase().includes(q) ||
        c.original_name.toLowerCase().includes(q) ||
        c.translated_name.toLowerCase().includes(q) ||
        c.notes.toLowerCase().includes(q)
      );
    });
  });

  let filteredGlossaryTerms = $derived.by(() => {
    const q = glossarySearch.trim().toLowerCase();
    return glossary.terms.filter(t => {
      if (glossaryCategoryFilter !== 'all' && t.category !== glossaryCategoryFilter) {
        return false;
      }
      if (!q) return true;
      return (
        t.source_term.toLowerCase().includes(q) ||
        t.translated_term.toLowerCase().includes(q) ||
        t.notes.toLowerCase().includes(q)
      );
    });
  });

  let glossaryStats = $derived.by(() => {
    const totalChars = glossary.characters.length;
    const translatedChars = glossary.characters.filter(c => c.translated_name.trim().length > 0).length;
    const maleChars = glossary.characters.filter(c => c.gender === 'male').length;
    const femaleChars = glossary.characters.filter(c => c.gender === 'female').length;
    const neuterChars = glossary.characters.filter(c => c.gender === 'neuter').length;
    const unknownChars = glossary.characters.filter(c => c.gender === 'unknown' || !c.gender).length;
    const totalTerms = glossary.terms.length;
    return { totalChars, translatedChars, maleChars, femaleChars, neuterChars, unknownChars, totalTerms };
  });

  async function loadGlossary(silent = false) {
    if (!gameDir.trim()) return;
    if (!silent) loadingGlossary = true;
    try {
      const res = await invoke<ProjectGlossary>("get_project_glossary", {
        gameDir: gameDir.trim(),
        scannedCharacters: project?.characters ?? null
      });
      glossary = res;
      glossaryDirty = false;
      if (!selectedGlossaryChar && glossary.characters.length > 0) {
        selectedGlossaryChar = glossary.characters[0];
      }
      if (!selectedGlossaryTerm && glossary.terms.length > 0) {
        selectedGlossaryTerm = glossary.terms[0];
      }
    } catch (e) {
      if (!silent) showToast(`Ошибка загрузки глоссария: ${e}`, "error");
    } finally {
      if (!silent) loadingGlossary = false;
    }
  }

  async function openGlossary(filterAlias?: string) {
    if (!gameDir.trim()) {
      showToast("Сначала выберите или откройте проект для работы с глоссарием", "warning");
      return;
    }
    if (filterAlias) {
      glossarySearch = filterAlias;
    }
    showGlossaryModal = true;
    await loadGlossary();
    if (filterAlias && glossary.characters.length > 0) {
      const match = glossary.characters.find(c => 
        c.alias.toLowerCase() === filterAlias.toLowerCase() || 
        c.original_name.toLowerCase() === filterAlias.toLowerCase()
      );
      if (match) {
        selectedGlossaryChar = match;
      }
    }
  }

  async function saveGlossary() {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    savingGlossary = true;
    try {
      const msg = await invoke<string>("save_project_glossary", {
        gameDir: gameDir.trim(),
        glossary
      });
      glossaryDirty = false;
      showToast(msg, "success");
    } catch (e) {
      showToast(`Ошибка сохранения глоссария: ${e}`, "error");
    } finally {
      savingGlossary = false;
    }
  }

  function updateCharacterGender(char: CharacterGlossaryEntry, gender: "male" | "female" | "neuter" | "unknown") {
    char.gender = gender;
    glossaryDirty = true;
  }

  function addGlossaryTerm() {
    const src = newTermSource.trim();
    const tr = newTermTranslation.trim();
    if (!src) {
      showToast("Введите исходный термин", "warning");
      return;
    }
    const newEntry: TermGlossaryEntry = {
      id: `term_${Date.now()}_${Math.random().toString(36).substring(2, 7)}`,
      source_term: src,
      translated_term: tr,
      category: newTermCategory,
      notes: newTermNotes.trim()
    };
    glossary.terms = [newEntry, ...glossary.terms];
    selectedGlossaryTerm = newEntry;
    newTermSource = "";
    newTermTranslation = "";
    newTermNotes = "";
    glossaryDirty = true;
    showToast(`Термин "${src}" добавлен в глоссарий`, "success");
  }

  function removeGlossaryTerm(id: string) {
    glossary.terms = glossary.terms.filter(t => t.id !== id);
    if (selectedGlossaryTerm?.id === id) {
      selectedGlossaryTerm = glossary.terms[0] || null;
    }
    glossaryDirty = true;
    showToast("Термин удален из глоссария", "info");
  }

  function addCustomCharacter() {
    const alias = newCharAlias.trim().toLowerCase();
    const name = newCharName.trim();
    if (!alias || !name) {
      showToast("Укажите алиас и оригинальное имя персонажа", "warning");
      return;
    }
    if (glossary.characters.some(c => c.alias.toLowerCase() === alias)) {
      showToast(`Персонаж с кодом [${alias}] уже есть в глоссарии`, "warning");
      return;
    }
    const newChar: CharacterGlossaryEntry = {
      alias,
      original_name: name,
      translated_name: newCharTranslation.trim(),
      gender: newCharGender,
      notes: ""
    };
    glossary.characters.push(newChar);
    glossary.characters.sort((a, b) => a.original_name.localeCompare(b.original_name));
    selectedGlossaryChar = newChar;
    newCharAlias = "";
    newCharName = "";
    newCharTranslation = "";
    newCharGender = "unknown";
    showAddChar = false;
    glossaryDirty = true;
    showToast(`Персонаж [${alias}] ${name} добавлен в глоссарий`, "success");
  }
</script>

<!-- Горячая клавиша Ctrl+S для быстрого сохранения глоссария -->
<svelte:window onkeydown={(e) => {
  if (e.key === "Escape") {
    showProjectManager = false;
    showRecentProjects = false;
    showToolsMenu = false;
  }
  if ((e.ctrlKey || e.metaKey) && e.key === 's' && showGlossaryModal) {
    e.preventDefault();
    saveGlossary();
  }
}} onpointerdown={closeRecentProjects} />

<div class="app-shell">
  <!-- ВЕРХНИЙ ТУЛБАР -->
  <header class="topbar">
    <div class="topbar-left">
      <div class="brand">
        <Languages class="icon-brand" size={24} />
        <span class="brand-title">Translator</span>
        <span class="badge-tag">Graph / Scene Edition</span>
      </div>

      <!-- Выбор текущего и ранее открытых проектов -->
      <div class="dir-bar">
        <FolderOpen size={16} class="text-zinc-400" />
        <div class="recent-project-picker">
          <button
            class="current-project-label"
            class:menu-open={showRecentProjects}
            aria-label="Выбрать проект"
            aria-expanded={showRecentProjects}
            onclick={() => showRecentProjects = !showRecentProjects}
          >
            <span>{gameDir.trim() ? projectDisplayName(gameDir) : "Проект не выбран"}</span>
            <ChevronDown size={14} />
          </button>
          {#if showRecentProjects}
            <div class="recent-project-menu">
              <div class="recent-project-menu-title">ПРОЕКТЫ</div>
              {#if recentProjects.length > 0}
                {#each recentProjects as path (path)}
                  <button
                    class="recent-project-option"
                    class:recent-project-current={gameDir === path}
                    title={path}
                    onclick={() => chooseRecentProject(path)}
                  >
                    <FolderOpen size={15} />
                    <span class="recent-project-option-text">
                      <span class="recent-project-name">{projectDisplayName(path)}</span>
                      <span class="recent-project-path">{path}</span>
                    </span>
                  </button>
                {/each}
              {:else}
                <div class="recent-project-empty">Недавно открытые проекты появятся здесь</div>
              {/if}
              <div class="recent-project-menu-footer">
                <button onclick={openProjectManager}>
                  <Folder size={14} />
                  <span>Менеджер проектов</span>
                  <span class="recent-project-menu-count">{recentProjects.length}</span>
                </button>
              </div>
            </div>
          {/if}
        </div>
        <button onclick={pickProjectFolder} disabled={loadingProject} class="btn-browse" title="Выбрать папку проекта">
          <FolderOpen size={15} />
          <span>Обзор</span>
        </button>
        <button onclick={scanProject} disabled={loadingProject} class="btn-primary" title="Сканировать проект">
          {#if loadingProject}
            <Loader2 size={16} class="spin" />
          {:else}
            <Play size={15} />
          {/if}
          <span>Сканировать</span>
        </button>
      </div>
    </div>

    <div class="topbar-right">
      <!-- Проектные элементы: шкала перевода и глоссарий -->
      <div class="project-actions">
        <!-- Глобальная статистика перевода (шкала) -->
        {#if loadingProject}
          <div class="global-stats skeleton-stats-box" title="Сканирование проекта...">
            <div class="skeleton-shimmer skeleton-pill-head"></div>
            <div class="progress-track skeleton-shimmer"></div>
          </div>
        {:else if project}
          <div class="global-stats" title="Прогресс перевода: {project.total_translated} / {project.total_lines} ({totalPercent}%)">
            <div class="stat-text">
              <span class="stat-num">{project.total_translated} / {project.total_lines}</span>
              <span class="stat-pct">({totalPercent}%)</span>
            </div>
            <div class="progress-track">
              <div class="progress-fill" style="width: {totalPercent}%"></div>
            </div>
          </div>
        {/if}

        <!-- Кнопка глоссария -->
        <button
          class="glossary-header-button"
          class:disabled={!gameDir.trim()}
          title={gameDir.trim() ? "Глоссарий проекта: персонажи, термины и пол (М/Ж)" : "Сначала выберите проект для работы с глоссарием"}
          onclick={() => openGlossary()}
        >
          <BookOpen size={15} />
          <span class="glossary-header-label">Глоссарий</span>
          {#if glossary.characters.length > 0 || glossary.terms.length > 0}
            <span class="topbar-badge">{glossary.characters.length + glossary.terms.length}</span>
          {/if}
        </button>
      </div>

      <div class="topbar-divider"></div>

      <!-- Бургер-меню для редко используемых инструментов проекта и управления -->
      <div class="tools-menu-wrap">
        <button
          class="tools-menu-trigger"
          class:menu-open={showToolsMenu}
          aria-label="Инструменты проекта и утилиты"
          aria-expanded={showToolsMenu}
          title="Инструменты проекта"
          onclick={() => showToolsMenu = !showToolsMenu}
        >
          <MenuIcon size={18} />
        </button>
        {#if showToolsMenu}
          <div class="tools-menu">
            <div class="tools-menu-heading">ИНСТРУМЕНТЫ ПРОЕКТА</div>
            <button
              class="tools-menu-item"
              disabled={packingMod || !gameDir}
              onclick={() => {
                showToolsMenu = false;
                quickPackMod();
              }}
            >
              {#if packingMod}<Loader2 size={16} class="spin text-amber-400" />{:else}<Package size={16} class="text-amber-400" />{/if}
              <span><strong>Собрать RPA</strong><small>Упаковать перевод в архив</small></span>
            </button>
            <button
              class="tools-menu-item"
              disabled={cleaningCache || !gameDir}
              onclick={() => {
                showToolsMenu = false;
                quickCleanCache();
              }}
            >
              {#if cleaningCache}<Loader2 size={16} class="spin text-rose-400" />{:else}<Trash2 size={16} class="text-rose-400" />{/if}
              <span><strong>Очистить .rpyc</strong><small>Удалить скомпилированные файлы</small></span>
            </button>
            <button
              class="tools-menu-item"
              onclick={() => {
                showToolsMenu = false;
                showToolsModal = true;
              }}
            >
              <Archive size={16} class="text-sky-400" />
              <span><strong>Студия RPA / RPYC</strong><small>Архивы и декомпиляция</small></span>
            </button>

            <div class="tools-menu-divider"></div>
            <div class="tools-menu-heading">УПРАВЛЕНИЕ</div>

            <button
              class="tools-menu-item"
              onclick={() => {
                showToolsMenu = false;
                openProjectManager();
              }}
            >
              <Folder size={16} class="text-emerald-400" />
              <span><strong>Менеджер проектов</strong><small>Список, переименование ({recentProjects.length})</small></span>
            </button>
          </div>
        {/if}
      </div>
    </div>
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
                  <span class="tree-pct-badge pct-progress">{pct}%</span>
                {/if}
              </div>

              {#if pct > 0}
                <div class="tree-micro-track">
                  <div class="tree-micro-fill" style="width: {pct}%" class:fill-done={pct === 100}></div>
                </div>
              {/if}
            </button>
          {/if}
        {/snippet}

        <!-- Спикеры / Персонажи (компактный аккордеон с глоссарием) -->
        <div class="sidebar-section characters-section">
          <div class="characters-header-wrap">
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
            </button>
            <button 
              class="btn-sidebar-glossary" 
              onclick={() => openGlossary()} 
              title="Открыть глоссарий персонажей и терминов"
            >
              <BookOpen size={11} class="text-emerald-400" />
              <span>Глоссарий</span>
            </button>
          </div>

          {#if charactersExpanded}
            <div class="char-chips">
              {#each project.characters as [alias, name]}
                {@const gl = glossaryCharByAlias.get(alias.toLowerCase())}
                <button 
                  class="char-chip char-chip-interactive" 
                  onclick={() => openGlossary(alias)}
                  title="Открыть [{alias}] в глоссарии"
                >
                  <span class="char-alias">[{alias}]</span>
                  <span class="char-name">{gl?.translated_name ? gl.translated_name : name}</span>
                  {#if gl?.gender === 'male'}
                    <span class="gender-pill gender-male" title="Мужской род">М</span>
                  {:else if gl?.gender === 'female'}
                    <span class="gender-pill gender-female" title="Женский род">Ж</span>
                  {:else if gl?.gender === 'neuter'}
                    <span class="gender-pill gender-neuter" title="Средний род (существо)">Ср</span>
                  {/if}
                </button>
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

          <!-- Компактный навигатор по сценам (для режима таблицы) -->
          {#if viewMode === 'table'}
            <div class="table-scene-nav">
              <button 
                class="btn-scene-step" 
                onclick={goToPrevScene} 
                disabled={isPrevSceneDisabled}
                title="Предыдущая сцена"
                aria-label="Предыдущая сцена"
              >
                <ChevronLeft size={14} />
              </button>

              <div class="table-scene-picker-wrap">
                <button 
                  class="btn-scene-picker" 
                  class:menu-open={showScenePicker}
                  onclick={() => showScenePicker = !showScenePicker}
                  title="Выбрать сцену"
                >
                  <Clapperboard size={13} class="text-sky-400" />
                  <span class="scene-picker-label">
                    {#if selectedSceneId === 'all'}
                      Все сцены ({currentFile.scenes.length})
                    {:else if currentScene}
                      {currentScene.name}
                    {:else}
                      Все сцены
                    {/if}
                  </span>
                  {#if currentScene}
                    {@const pct = currentScene.total_lines > 0 ? Math.round((currentScene.translated_lines / currentScene.total_lines) * 100) : 0}
                    {#if currentScene.total_lines > 0}
                      <span class="scene-picker-badge" class:badge-complete={pct === 100}>
                        {currentScene.translated_lines}/{currentScene.total_lines} ({pct}%)
                      </span>
                    {:else}
                      <span class="scene-picker-badge badge-router">
                        Развилка (0 строк)
                      </span>
                    {/if}
                  {/if}
                  <ChevronDown size={13} class="picker-caret" />
                </button>

                {#if showScenePicker}
                  <div class="scene-picker-dropdown">
                    <div class="scene-picker-search">
                      <Search size={12} class="text-slate-400" />
                      <input 
                        type="text" 
                        bind:value={scenePickerSearch} 
                        placeholder="Поиск сцены..." 
                        class="scene-search-input"
                      />
                      {#if scenePickerSearch}
                        <button class="clear-search-btn" onclick={() => scenePickerSearch = ""}>✕</button>
                      {/if}
                    </div>

                    <div class="scene-picker-list">
                      <button 
                        class="scene-picker-item" 
                        class:selected={selectedSceneId === 'all'}
                        onclick={() => selectScene('all')}
                      >
                        <Clapperboard size={13} class="text-slate-400" />
                        <span class="scene-item-title">Все сцены с диалогами ({tableScenes.filter(s => s.total_lines > 0).length})</span>
                      </button>

                      {#each filteredScenesForPicker as scene, idx}
                        {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                        <button 
                          class="scene-picker-item" 
                          class:selected={selectedSceneId === scene.id}
                          onclick={() => selectScene(scene.id)}
                        >
                          <span class="scene-item-seq">#{idx + 1}</span>
                          <span class="scene-item-title">{scene.name}</span>
                          {#if scene.total_lines > 0}
                            <span class="scene-item-badge" class:badge-complete={pct === 100}>
                              {scene.translated_lines}/{scene.total_lines} ({pct}%)
                            </span>
                          {:else}
                            <span class="scene-item-badge badge-router">
                              Развилка (0 строк)
                            </span>
                          {/if}
                        </button>
                      {/each}
                      {#if filteredScenesForPicker.length === 0}
                        <div class="scene-picker-empty">Сцена не найдена</div>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>

              <button 
                class="btn-scene-step" 
                onclick={goToNextScene} 
                disabled={isNextSceneDisabled}
                title="Следующая сцена"
                aria-label="Следующая сцена"
              >
                <ChevronRight size={14} />
              </button>
            </div>
          {/if}

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

              <div class="graph-topbar-controls">
                {#if graphHistory.length > 0}
                  <button class="graph-mode-btn" disabled={graphNavigating} onclick={backToGraph}>← Назад к графу</button>
                {/if}
                <button 
                  class="graph-sidebar-toggle-btn" 
                  class:active={showGraphSidebar}
                  onclick={() => showGraphSidebar = !showGraphSidebar}
                  title={showGraphSidebar ? "Скрыть панель списка сцен" : "Показать панель списка сцен"}
                >
                  <Clapperboard size={13} />
                  <span>Список сцен</span>
                  {#if showGraphSidebar}
                    <span class="btn-state-pill">Вкл</span>
                  {/if}
                </button>

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
            </div>

            <!-- РАБОЧАЯ ОБЛАСТЬ ГРАФА С ВНУТРЕННИМ САЙДБАРОМ -->
            <div class="graph-workspace">
              <!-- ВНУТРЕННИЙ САЙДБАР ГРАФА ДЛЯ БЫСТРОЙ НАВИГАЦИИ -->
              {#if showGraphSidebar}
                <aside class="graph-inner-sidebar">
                  <div class="graph-sidebar-header">
                    <div class="graph-sidebar-title">
                      <Clapperboard size={13} class="text-sky-400" />
                      <span>Сцены ({currentFile.scenes.length})</span>
                    </div>
                    <div class="graph-sidebar-actions">
                      <button 
                        class="icon-btn-subtle" 
                        class:active={selectedSceneId === 'all'}
                        onclick={() => {
                          selectedSceneId = 'all';
                          focusedSceneName = null;
                        }}
                        title="Показать все сцены (сбросить фокус)"
                      >
                        Все
                      </button>
                      <button 
                        class="icon-btn-subtle"
                        onclick={() => showGraphSidebar = false}
                        title="Скрыть панель сцен"
                      >
                        <X size={13} />
                      </button>
                    </div>
                  </div>

                  <!-- Поиск сцен внутри графа -->
                  <div class="graph-sidebar-search">
                    <Search size={13} class="text-slate-500" />
                    <input 
                      type="text" 
                      placeholder="Поиск сцены..." 
                      bind:value={graphSceneSearch}
                      class="graph-sidebar-input"
                    />
                    {#if graphSceneSearch}
                      <button class="clear-search-pill" onclick={() => graphSceneSearch = ""}>✕</button>
                    {/if}
                  </div>

                  <!-- Список сцен с быстрым скроллом и фокусом -->
                  <div class="graph-sidebar-list">
                    {#if filteredScenesForGraph.length === 0}
                      <div class="graph-sidebar-empty">
                        <span>Сцены не найдены</span>
                      </div>
                    {:else}
                      {#each filteredScenesForGraph as scene, idx}
                        {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                        {@const isSelected = selectedSceneId === scene.id || focusedSceneName === scene.name}
                        <button 
                          class="graph-scene-item" 
                          class:active={isSelected}
                          onclick={() => focusAndScrollSceneInGraph(scene)}
                          title="{scene.name} • {scene.translated_lines}/{scene.total_lines} строк ({pct}%)"
                        >
                          <div class="graph-scene-item-left">
                            <span class="graph-scene-seq">#{idx + 1}</span>
                            <span class="graph-scene-name">{scene.name}</span>
                          </div>
                          <div class="graph-scene-item-right">
                            {#if scene.choices && scene.choices.length > 0}
                              <span class="mini-tag tag-purple" title="Развилок: {scene.choices.length}">
                                <Zap size={10} />
                              </span>
                            {/if}
                            <span class="graph-scene-pct" class:pct-done={pct === 100}>{pct}%</span>
                          </div>
                        </button>
                      {/each}
                    {/if}
                  </div>
                </aside>
              {/if}

              <!-- ОСНОВНАЯ ОБЛАСТЬ ГРАФА / МАТРИЦЫ -->
              <div class="graph-canvas-container">

            {#if graphSubMode === 'flow'}
              {#key currentFile.file_path}
                <SceneGraph scenes={currentFile.scenes} edges={currentFile.edges ?? []}
                  selectedId={selectedSceneId} busy={graphNavigating}
                  onselect={(scene) => { selectedSceneId = scene.id; focusedSceneName = scene.name; }}
                  onnavigate={navigateOrFocusScene} onopen={openSceneInTable} />
              {/key}
            {:else}
              <!-- РЕЖИМ 2: СЕТКА КАРТОЧЕК -->
              <div class="scenes-flow-grid">
                {#each currentFile.scenes as scene, index}
                  {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                  {@const outgoing = getOutgoingEdges(scene.name)}
                  <div 
                    id={`scene-grid-card-${scene.id}`}
                    class="scene-card" 
                    class:card-selected={selectedSceneId === scene.id || focusedSceneName === scene.name}
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
            </div>
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
                          <th style="width: 160px;">Персонаж</th>
                          <th>Оригинал (EN)</th>
                          <th>Перевод (RU)</th>
                          <th style="width: 80px;">Статус</th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each sceneEntries as entry}
                          {@const spk = getSpeakerInfo(entry.speaker, entry.speaker_alias)}
                          <tr 
                            class:selected={selectedEntry?.id === entry.id}
                            onclick={() => selectedEntry = entry}
                          >
                            <td class="col-line">{entry.line_number}</td>
                            <td class="col-speaker">
                              {#if spk.isNarrator}
                                <span class="narrator-badge">Автор</span>
                              {:else if spk.isUi}
                                <span class="char-chip char-chip-ui" title="Интерфейсные строки">
                                  <span class="char-alias">[UI]</span>
                                  <span class="char-name">Интерфейс</span>
                                </span>
                              {:else}
                                <span class="char-chip" title="Код: {spk.alias || '—'}">
                                  {#if spk.alias}
                                    <span class="char-alias">[{spk.alias}]</span>
                                  {/if}
                                  <span class="char-name">{spk.translatedName ? spk.translatedName : spk.name}</span>
                                  {#if spk.translatedName && spk.translatedName !== spk.name}
                                    <span class="char-orig-sub">({spk.name})</span>
                                  {/if}
                                  {#if spk.gender === 'male'}
                                    <span class="gender-pill gender-male" title="Мужской род">М</span>
                                  {:else if spk.gender === 'female'}
                                    <span class="gender-pill gender-female" title="Женский род">Ж</span>
                                  {:else if spk.gender === 'neuter'}
                                    <span class="gender-pill gender-neuter" title="Средний род (существо)">Ср</span>
                                  {/if}
                                </span>
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
                  {:else if selectedSceneId === scene.id}
                    <div class="empty-scene-card">
                      <div class="empty-scene-icon-wrap">
                        <GitBranch size={30} class="text-sky-400" />
                      </div>
                      <div class="empty-scene-info">
                        <h3>Сцена «{scene.name}» не содержит диалоговых строк</h3>
                        <p>
                          Эта метка используется в сценарии игры как меню выбора или маршрутизатор сюжета
                          {#if (scene.choices?.length ?? 0) > 0}
                            (вариантов меню: {scene.choices.length})
                          {/if}
                          {#if (scene.jumps?.length ?? 0) > 0}
                            (переходов jump/call: {scene.jumps.length})
                          {/if}.
                        </p>
                        <button type="button" class="btn-return-graph" onclick={() => viewMode = 'scenes'}>
                          <GitBranch size={14} />
                          <span>Открыть переходы этой сцены в графе →</span>
                        </button>
                      </div>
                    </div>
                  {/if}
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        <!-- НИЖНИЙ ИНСПЕКТОР СТРОКИ -->
        {#if selectedEntry}
          {@const spk = getSpeakerInfo(selectedEntry.speaker, selectedEntry.speaker_alias)}
          <div class="inspector-panel">
            <div class="inspector-header">
              <div class="inspector-breadcrumb">
                <Clapperboard size={13} class="text-sky-400" />
                <span class="inspect-scene">{selectedEntry.scene_name}</span>
                <span class="breadcrumb-sep">/</span>
                <span class="inspect-line">Строка #{selectedEntry.line_number}</span>
                <span class="breadcrumb-sep">/</span>
                {#if spk.isNarrator}
                  <span class="inspect-narrator">Авторский текст</span>
                {:else if spk.isUi}
                  <span class="char-chip char-chip-ui char-chip-inspect">
                    <span class="char-alias">[UI]</span>
                    <strong class="char-name">Интерфейс</strong>
                  </span>
                {:else}
                  <div class="inspect-speaker-group">
                    <span class="char-chip char-chip-inspect" title="Код: {spk.alias || '—'}">
                      {#if spk.alias}<span class="char-alias">[{spk.alias}]</span>{/if}
                      <strong class="char-name">{spk.translatedName ? spk.translatedName : spk.name}</strong>
                      {#if spk.translatedName && spk.translatedName !== spk.name}
                        <span class="char-orig-sub">({spk.name})</span>
                      {/if}
                      {#if spk.gender === 'male'}
                        <span class="gender-pill gender-male" title="Мужской род (он)">М</span>
                      {:else if spk.gender === 'female'}
                        <span class="gender-pill gender-female" title="Женский род (она)">Ж</span>
                      {:else if spk.gender === 'neuter'}
                        <span class="gender-pill gender-neuter" title="Средний род (оно / существо)">Ср</span>
                      {/if}
                    </span>
                    {#if spk.notes}
                      <span class="inspect-char-notes" title="Заметки персонажа">💡 {spk.notes}</span>
                    {/if}
                    <button 
                      class="btn-micro-glossary" 
                      onclick={() => openGlossary(spk.alias || spk.name)}
                      title="Редактировать в глоссарии"
                    >
                      <BookOpen size={11} />
                    </button>
                  </div>
                {/if}
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

  <!-- МОДАЛЬНОЕ ОКНО RPA & RPYC STUDIO -->
  {#if showToolsModal}
    <div 
      class="modal-backdrop" 
      onclick={(e) => { if (e.target === e.currentTarget) showToolsModal = false; }}
      role="dialog"
      aria-modal="true"
    >
      <div class="tools-modal">
        <div class="tools-modal-header">
          <div class="modal-title-wrap">
            <Archive size={18} class="text-sky-400" />
            <h3>Студия Ren'Py: Архивы RPA и Байткод RPYC</h3>
          </div>
          <button class="btn-close-modal" onclick={() => showToolsModal = false} aria-label="Закрыть">
            <X size={16} />
          </button>
        </div>

        <div class="tools-tab-bar">
          <button 
            class="tools-tab" 
            class:tab-active={toolsActiveTab === 'rpa'}
            onclick={() => toolsActiveTab = 'rpa'}
          >
            <Package size={15} />
            <span>Архивы RPA (Сборка & Распаковка)</span>
          </button>
          <button 
            class="tools-tab" 
            class:tab-active={toolsActiveTab === 'rpyc'}
            onclick={() => toolsActiveTab = 'rpyc'}
          >
            <FileCode size={15} />
            <span>Байткод RPYC (Декомпилятор & Кэш)</span>
          </button>
        </div>

        <div class="tools-modal-body">
          {#if toolsActiveTab === 'rpa'}
            <!-- ВКЛАДКА RPA -->
            <div class="tools-cards-grid">
              <!-- Карточка 1: Сборка RPA мода -->
              <div class="tool-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <Package size={16} class="text-amber-400" />
                    <h4>Экспорт мода перевода (RPA-3.0)</h4>
                  </div>
                  <span class="tool-badge">Для игроков</span>
                </div>
                <p class="tool-desc">
                  Упаковывает папку <code>tl/russian</code> со всеми скриптами, шрифтами и графикой в единый <code>.rpa</code> файл. Игрокам нужно лишь скопировать его в папку <code>game/</code>.
                </p>

                <div class="tool-form-group">
                  <span class="tool-field-label">Папка источника (tl/russian)</span>
                  <div class="tool-readonly-path">{gameDir ? `${gameDir}\\tl\\russian` : 'Папка игры не выбрана'}</div>
                </div>

                <div class="tool-form-group">
                  <span class="tool-field-label">Путь к итоговому файлу RPA (по умолчанию в game/)</span>
                  <input 
                    type="text" 
                    bind:value={packModCustomPath} 
                    placeholder={gameDir ? `${gameDir}\\russian_mod.rpa` : "Куда сохранить .rpa..."} 
                    class="tool-input"
                  />
                </div>

                <button 
                  class="btn-tool-action btn-pack-action" 
                  onclick={executePackCustomRpa} 
                  disabled={packingMod || !gameDir}
                >
                  {#if packingMod}
                    <Loader2 size={15} class="spin" />
                    <span>Упаковка файлов в RPA-3.0...</span>
                  {:else}
                    <Download size={15} />
                    <span>Собрать и экспортировать RPA мод</span>
                  {/if}
                </button>
              </div>

              <!-- Карточка 2: Распаковка RPA архива -->
              <div class="tool-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <Archive size={16} class="text-sky-400" />
                    <h4>Распаковка RPA архива</h4>
                  </div>
                  <span class="tool-badge tool-badge-cyan">Экстрактор</span>
                </div>
                <p class="tool-desc">
                  Извлекает скрипты, текстуры, фоны и музыку из запакованных архивов игры (<code>archive.rpa</code>, <code>scripts.rpa</code>, <code>images.rpa</code>).
                </p>

                <div class="tool-form-group">
                  <span class="tool-field-label">Путь к .rpa архиву</span>
                  <input 
                    type="text" 
                    bind:value={unpackSourceRpa} 
                    placeholder="Например: C:\\Game\\game\\archive.rpa" 
                    class="tool-input"
                  />
                </div>

                <div class="tool-form-group">
                  <span class="tool-field-label">Куда распаковать (целевая папка)</span>
                  <input 
                    type="text" 
                    bind:value={unpackTargetDir} 
                    placeholder="Например: C:\\Game\\game\\extracted" 
                    class="tool-input"
                  />
                </div>

                <button 
                  class="btn-tool-action btn-unpack-action" 
                  onclick={executeUnpackRpa} 
                  disabled={unpackingRpa}
                >
                  {#if unpackingRpa}
                    <Loader2 size={15} class="spin" />
                    <span>Извлечение файлов из архива...</span>
                  {:else}
                    <Upload size={15} />
                    <span>Распаковать архив</span>
                  {/if}
                </button>

                {#if unpackResultMsg}
                  <div class="tool-result-box">{unpackResultMsg}</div>
                {/if}
              </div>
            </div>

          {:else}
            <!-- ВКЛАДКА RPYC -->
            <div class="tools-cards-grid">
              <!-- Карточка 1: Декомпилятор -->
              <div class="tool-card full-width-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <FileCode size={16} class="text-emerald-400" />
                    <h4>Декомпилятор байткода .rpyc в исходный код .rpy</h4>
                  </div>
                  <span class="tool-badge tool-badge-green">Pure Rust AST</span>
                </div>
                <p class="tool-desc">
                  Восстанавливает человекочитаемые диалоги, сцены (label), ветвления (menu) и переходы (jump) прямо из скомпилированного байткода Ren'Py без необходимости в Python!
                </p>

                <div class="tool-inline-form">
                  <div class="tool-form-group flex-1">
                    <span class="tool-field-label">Путь к скомпилированному файлу .rpyc</span>
                    <input 
                      type="text" 
                      bind:value={decompileRpycPath} 
                      placeholder={gameDir ? `${gameDir}\\celia.rpyc` : "Путь к .rpyc..."} 
                      class="tool-input"
                    />
                  </div>

                  <div class="tool-form-group flex-1">
                    <span class="tool-field-label">Сохранить как .rpy (опционально)</span>
                    <input 
                      type="text" 
                      bind:value={decompileOutputRpyPath} 
                      placeholder="Например: D:\\output\\decompiled.rpy" 
                      class="tool-input"
                    />
                  </div>

                  <div class="tool-form-action-wrap">
                    <button 
                      class="btn-tool-action btn-decompile-action" 
                      onclick={executeDecompileRpyc} 
                      disabled={decompilingRpyc}
                    >
                      {#if decompilingRpyc}
                        <Loader2 size={15} class="spin" />
                        <span>Декомпиляция...</span>
                      {:else}
                        <Zap size={15} />
                        <span>Декомпилировать</span>
                      {/if}
                    </button>
                  </div>
                </div>

                {#if decompiledCodePreview}
                  <div class="decompile-preview-wrap">
                    <div class="preview-header">
                      <span>Предпросмотр декомпилированного кода:</span>
                      <button class="btn-copy-code" onclick={copyDecompiledCode}>
                        <Copy size={13} />
                        <span>Копировать код</span>
                      </button>
                    </div>
                    <textarea readonly class="decompile-code-area" value={decompiledCodePreview}></textarea>
                  </div>
                {/if}
              </div>

              <!-- Карточка 2: Очистка кэша .rpyc -->
              <div class="tool-card full-width-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <Trash2 size={16} class="text-rose-400" />
                    <h4>Очистка устаревшего кэша скомпилированных скриптов .rpyc</h4>
                  </div>
                  <span class="tool-badge tool-badge-rose">Cache Flush</span>
                </div>
                <p class="tool-desc">
                  Устраняет частую проблему Ren'Py, когда движок запускает старый скомпилированный байткод вместо свежепереведенных файлов .rpy.
                </p>

                <div class="cache-options-row">
                  <label class="radio-label">
                    <input type="radio" bind:group={cleanTlOnly} value={true} />
                    <span>Только файлы перевода в <code>tl/russian</code> (Рекомендуется, безопасно)</span>
                  </label>
                  <label class="radio-label">
                    <input type="radio" bind:group={cleanTlOnly} value={false} />
                    <span>Все файлы .rpyc во всей папке <code>game/</code></span>
                  </label>
                </div>

                <button 
                  class="btn-tool-action btn-clean-action" 
                  onclick={executeCleanCacheModal} 
                  disabled={cleaningCache || !gameDir}
                >
                  {#if cleaningCache}
                    <Loader2 size={15} class="spin" />
                    <span>Удаление кэша...</span>
                  {:else}
                    <Trash2 size={15} />
                    <span>Очистить кэш .rpyc</span>
                  {/if}
                </button>
              </div>
            </div>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <!-- МОДАЛЬНОЕ ОКНО ГЛОССАРИЯ (ПЕРСОНАЖИ И ТЕРМИНЫ) -->
  {#if showGlossaryModal}
    <div 
      class="modal-backdrop" 
      onclick={(e) => { if (e.target === e.currentTarget) showGlossaryModal = false; }}
      role="dialog"
      aria-modal="true"
    >
      <div class="tools-modal glossary-modal">
        <!-- Заголовок модального окна глоссария -->
        <div class="tools-modal-header">
          <div class="modal-title-wrap">
            <BookOpen size={20} class="text-emerald-400" />
            <div>
              <h3>Глоссарий проекта: Персонажи и Терминология</h3>
              <p class="modal-subtitle">Имена, грамматический род (М/Ж) для согласования окончаний и словарь терминов</p>
            </div>
          </div>
          <div class="modal-header-actions">
            <button 
              class="btn-modal-save" 
              class:dirty={glossaryDirty}
              onclick={saveGlossary} 
              disabled={savingGlossary}
            >
              {#if savingGlossary}
                <Loader2 size={14} class="spin" />
                <span>Сохранение...</span>
              {:else}
                <Save size={14} />
                <span>{glossaryDirty ? 'Сохранить изменения *' : 'Сохранено'}</span>
              {/if}
            </button>
            <button class="btn-close-modal" onclick={() => showGlossaryModal = false} aria-label="Закрыть">
              <X size={16} />
            </button>
          </div>
        </div>

        <!-- Переключатель вкладок: Персонажи / Термины -->
        <div class="tools-tab-bar">
          <button 
            class="tools-tab" 
            class:tab-active={glossaryTab === 'characters'}
            onclick={() => glossaryTab = 'characters'}
          >
            <Users size={15} />
            <span>Персонажи ({glossaryStats.totalChars})</span>
            {#if glossaryStats.translatedChars > 0}
              <span class="tab-badge-accent">{glossaryStats.translatedChars} переведено</span>
            {/if}
          </button>
          <button 
            class="tools-tab" 
            class:tab-active={glossaryTab === 'terms'}
            onclick={() => glossaryTab = 'terms'}
          >
            <BookMarked size={15} />
            <span>Термины и локации ({glossaryStats.totalTerms})</span>
          </button>
        </div>

        <!-- Тулбар фильтрации и поиска внутри глоссария -->
        <div class="glossary-toolbar">
          <div class="glossary-search-box">
            <Search size={14} class="search-box-icon text-slate-400" />
            <input 
              type="text" 
              bind:value={glossarySearch} 
              placeholder={glossaryTab === 'characters' ? "Поиск по коду, имени, переводу..." : "Поиск по термину, переводу..."}
              class="glossary-search-input"
            />
            {#if glossarySearch}
              <button class="btn-clear-search" onclick={() => glossarySearch = ""}>
                <X size={12} />
              </button>
            {/if}
          </div>

          {#if glossaryTab === 'characters'}
            <!-- Фильтр по полу (М, Ж, Ср, —) -->
            <div class="gender-filter-group">
              <span class="filter-label">Род:</span>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryGenderFilter === 'all'} 
                onclick={() => glossaryGenderFilter = 'all'}
              >
                Все ({glossaryStats.totalChars})
              </button>
              <button 
                class="btn-filter-pill filter-male" 
                class:active={glossaryGenderFilter === 'male'} 
                onclick={() => glossaryGenderFilter = 'male'}
              >
                М ({glossaryStats.maleChars})
              </button>
              <button 
                class="btn-filter-pill filter-female" 
                class:active={glossaryGenderFilter === 'female'} 
                onclick={() => glossaryGenderFilter = 'female'}
              >
                Ж ({glossaryStats.femaleChars})
              </button>
              <button 
                class="btn-filter-pill filter-neuter" 
                class:active={glossaryGenderFilter === 'neuter'} 
                onclick={() => glossaryGenderFilter = 'neuter'}
              >
                Ср ({glossaryStats.neuterChars})
              </button>
              <button 
                class="btn-filter-pill filter-unknown" 
                class:active={glossaryGenderFilter === 'unknown'} 
                onclick={() => glossaryGenderFilter = 'unknown'}
              >
                — ({glossaryStats.unknownChars})
              </button>
            </div>

            <button 
              class="btn-toggle-add-char" 
              onclick={() => showAddChar = !showAddChar}
              title="Добавить вручную нового персонажа"
            >
              <Plus size={14} />
              <span>{showAddChar ? 'Отмена' : '+ Персонаж'}</span>
            </button>
          {:else}
            <!-- Фильтр по категории терминов -->
            <div class="category-filter-group">
              <span class="filter-label">Категория:</span>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryCategoryFilter === 'all'} 
                onclick={() => glossaryCategoryFilter = 'all'}
              >
                Все
              </button>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryCategoryFilter === 'general'} 
                onclick={() => glossaryCategoryFilter = 'general'}
              >
                Общие
              </button>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryCategoryFilter === 'location'} 
                onclick={() => glossaryCategoryFilter = 'location'}
              >
                Локации
              </button>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryCategoryFilter === 'title'} 
                onclick={() => glossaryCategoryFilter = 'title'}
              >
                Титулы
              </button>
              <button 
                class="btn-filter-pill" 
                class:active={glossaryCategoryFilter === 'item'} 
                onclick={() => glossaryCategoryFilter = 'item'}
              >
                Предметы
              </button>
            </div>
          {/if}
        </div>

        <!-- Форма быстрого добавления нового персонажа (если раскрыта) -->
        {#if glossaryTab === 'characters' && showAddChar}
          <div class="add-char-panel">
            <div class="add-char-row">
              <div class="field-item">
                <span class="field-label">Алиас (код в script.rpy):</span>
                <input type="text" bind:value={newCharAlias} placeholder="напр. cr" class="glossary-input" />
              </div>
              <div class="field-item">
                <span class="field-label">Имя в игре (EN):</span>
                <input type="text" bind:value={newCharName} placeholder="Shadow Creature" class="glossary-input" />
              </div>
              <div class="field-item">
                <span class="field-label">Перевод имени (RU):</span>
                <input type="text" bind:value={newCharTranslation} placeholder="Теневое существо" class="glossary-input" />
              </div>
              <div class="field-item">
                <span class="field-label">Грамматический род:</span>
                <div class="gender-toggle-group">
                  <button 
                    type="button"
                    class="btn-gender-toggle gender-male" 
                    class:active={newCharGender === 'male'} 
                    onclick={() => newCharGender = 'male'}
                  >М</button>
                  <button 
                    type="button"
                    class="btn-gender-toggle gender-female" 
                    class:active={newCharGender === 'female'} 
                    onclick={() => newCharGender = 'female'}
                  >Ж</button>
                  <button 
                    type="button"
                    class="btn-gender-toggle gender-neuter" 
                    class:active={newCharGender === 'neuter'} 
                    onclick={() => newCharGender = 'neuter'}
                  >Ср</button>
                  <button 
                    type="button"
                    class="btn-gender-toggle gender-unknown" 
                    class:active={newCharGender === 'unknown'} 
                    onclick={() => newCharGender = 'unknown'}
                  >—</button>
                </div>
              </div>
              <button class="btn-confirm-add" onclick={addCustomCharacter}>
                <Plus size={14} />
                <span>Добавить</span>
              </button>
            </div>
          </div>
        {/if}

        <!-- Форма добавления нового термина (всегда доступна во вкладке терминов) -->
        {#if glossaryTab === 'terms'}
          <div class="add-term-panel">
            <div class="add-term-row">
              <div class="field-item flex-2">
                <span class="field-label">Исходный термин (EN):</span>
                <input 
                  type="text" 
                  bind:value={newTermSource} 
                  placeholder="напр. Demon Blade, Ancient Ruins" 
                  class="glossary-input" 
                  onkeydown={(e) => { if (e.key === 'Enter') addGlossaryTerm(); }}
                />
              </div>
              <div class="field-item flex-2">
                <span class="field-label">Русский перевод (RU):</span>
                <input 
                  type="text" 
                  bind:value={newTermTranslation} 
                  placeholder="Клинок демона, Древние руины" 
                  class="glossary-input" 
                  onkeydown={(e) => { if (e.key === 'Enter') addGlossaryTerm(); }}
                />
              </div>
              <div class="field-item flex-1">
                <span class="field-label">Категория:</span>
                <select bind:value={newTermCategory} class="glossary-select">
                  <option value="general">Общий термин</option>
                  <option value="location">Локация / Место</option>
                  <option value="title">Титул / Звание</option>
                  <option value="item">Предмет / Артефакт</option>
                </select>
              </div>
              <div class="field-item flex-2">
                <span class="field-label">Заметки / Контекст:</span>
                <input 
                  type="text" 
                  bind:value={newTermNotes} 
                  placeholder="напр. магический артефакт" 
                  class="glossary-input" 
                  onkeydown={(e) => { if (e.key === 'Enter') addGlossaryTerm(); }}
                />
              </div>
              <button class="btn-confirm-add" onclick={addGlossaryTerm}>
                <Plus size={14} />
                <span>Добавить</span>
              </button>
            </div>
          </div>
        {/if}

        <!-- РАЗДЕЛЕННЫЙ РЕЖИМ (MASTER-DETAIL): СПИСОК СЛЕВА, САЙДБАР РЕДАКТИРОВАНИЯ СПРАВА -->
        <div class="glossary-split-container">
          <!-- ЛЕВАЯ КОЛОНКА: ЧИСТЫЙ СПИСОК / ТАБЛИЦА БЕЗ ИНЛАЙН-ИНПУТОВ -->
          <div class="glossary-list-pane">
            {#if loadingGlossary}
              <div class="glossary-loading">
                <Loader2 size={28} class="spin text-emerald-400" />
                <span>Загрузка глоссария проекта...</span>
              </div>
            {:else if glossaryTab === 'characters'}
              {#if filteredGlossaryCharacters.length === 0}
                <div class="glossary-empty-state">
                  <Users size={32} class="text-slate-600" />
                  <p>Персонажи не найдены по запросу «{glossarySearch}»</p>
                </div>
              {:else}
                <table class="glossary-list-table">
                  <thead>
                    <tr>
                      <th style="width: 80px;">Алиас</th>
                      <th style="width: 140px;">Оригинал (EN)</th>
                      <th style="width: 150px;">Перевод (RU)</th>
                      <th style="width: 60px; text-align: center;">Род</th>
                      <th>Заметки</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each filteredGlossaryCharacters as char}
                      {@const isSel = selectedGlossaryChar?.alias === char.alias}
                      <tr 
                        class="glossary-selectable-row"
                        class:selected-row={isSel}
                        onclick={() => selectedGlossaryChar = char}
                      >
                        <td class="col-alias">
                          <span class="char-alias-badge">[{char.alias}]</span>
                        </td>
                        <td class="col-orig-name">
                          <span class="char-name-en">{char.original_name}</span>
                        </td>
                        <td class="col-trans-name">
                          {#if char.translated_name}
                            <span class="char-name-ru">{char.translated_name}</span>
                          {:else}
                            <span class="char-name-empty">—</span>
                          {/if}
                        </td>
                        <td class="col-gender-cell" style="text-align: center;">
                          {#if char.gender === 'male'}
                            <span class="gender-pill gender-male" title="Мужской род (он)">М</span>
                          {:else if char.gender === 'female'}
                            <span class="gender-pill gender-female" title="Женский род (она)">Ж</span>
                          {:else if char.gender === 'neuter'}
                            <span class="gender-pill gender-neuter" title="Средний род (оно / существо)">Ср</span>
                          {:else}
                            <span class="gender-pill gender-unknown" title="Не указан">—</span>
                          {/if}
                        </td>
                        <td class="col-notes-snippet">
                          {#if char.notes}
                            <span class="notes-snippet" title={char.notes}>{char.notes}</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              {/if}
            {:else}
              <!-- Вкладка ТЕРМИНЫ -->
              {#if filteredGlossaryTerms.length === 0}
                <div class="glossary-empty-state">
                  <BookMarked size={32} class="text-slate-600" />
                  <p>В глоссарии пока нет терминов. Добавьте термин выше!</p>
                </div>
              {:else}
                <table class="glossary-list-table">
                  <thead>
                    <tr>
                      <th style="width: 170px;">Термин (EN)</th>
                      <th style="width: 170px;">Перевод (RU)</th>
                      <th style="width: 100px;">Категория</th>
                      <th>Заметки</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each filteredGlossaryTerms as term}
                      {@const isSel = selectedGlossaryTerm?.id === term.id}
                      <tr 
                        class="glossary-selectable-row"
                        class:selected-row={isSel}
                        onclick={() => selectedGlossaryTerm = term}
                      >
                        <td class="col-orig-name">
                          <span class="char-name-en">{term.source_term}</span>
                        </td>
                        <td class="col-trans-name">
                          {#if term.translated_term}
                            <span class="char-name-ru">{term.translated_term}</span>
                          {:else}
                            <span class="char-name-empty">—</span>
                          {/if}
                        </td>
                        <td class="col-category">
                          <span class="category-badge category-{term.category}">
                            {term.category === 'location' ? 'Локация' : term.category === 'title' ? 'Титул' : term.category === 'item' ? 'Предмет' : 'Общий'}
                          </span>
                        </td>
                        <td class="col-notes-snippet">
                          {#if term.notes}
                            <span class="notes-snippet" title={term.notes}>{term.notes}</span>
                          {/if}
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              {/if}
            {/if}
          </div>

          <!-- ПРАВАЯ КОЛОНКА: САЙДБАР РЕДАКТИРОВАНИЯ В ОКНЕ -->
          <aside class="glossary-edit-sidebar">
            {#if glossaryTab === 'characters'}
              {#if selectedGlossaryChar}
                <div class="sidebar-edit-header">
                  <div class="sidebar-char-badge-row">
                    <span class="char-alias-badge-lg">[{selectedGlossaryChar.alias}]</span>
                    <div class="sidebar-char-headings">
                      <h4>{selectedGlossaryChar.original_name}</h4>
                      <span class="char-sub-label">Код и оригинальное имя</span>
                    </div>
                  </div>
                </div>

                <div class="sidebar-edit-content">
                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Русский перевод имени:</span>
                    </label>
                    <input 
                      type="text" 
                      bind:value={selectedGlossaryChar.translated_name} 
                      oninput={() => glossaryDirty = true}
                      placeholder={selectedGlossaryChar.original_name}
                      class="sidebar-input sidebar-input-primary"
                    />
                    <span class="edit-hint">Отображается в диалогах, сюжетном графе и инспекторе</span>
                  </div>

                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Грамматический род (для согласования окончаний):</span>
                    </label>
                    <div class="gender-cards-grid">
                      <button 
                        type="button" 
                        class="gender-card gender-card-male"
                        class:active={selectedGlossaryChar.gender === 'male'}
                        onclick={() => { if (selectedGlossaryChar) updateCharacterGender(selectedGlossaryChar, 'male'); }}
                      >
                        <div class="gender-card-top">
                          <span class="gender-letter">М</span>
                          <span class="gender-title">Мужской</span>
                        </div>
                        <span class="gender-example">он сказал, красивый</span>
                      </button>

                      <button 
                        type="button" 
                        class="gender-card gender-card-female"
                        class:active={selectedGlossaryChar.gender === 'female'}
                        onclick={() => { if (selectedGlossaryChar) updateCharacterGender(selectedGlossaryChar, 'female'); }}
                      >
                        <div class="gender-card-top">
                          <span class="gender-letter">Ж</span>
                          <span class="gender-title">Женский</span>
                        </div>
                        <span class="gender-example">она сказала, красивая</span>
                      </button>

                      <button 
                        type="button" 
                        class="gender-card gender-card-neuter"
                        class:active={selectedGlossaryChar.gender === 'neuter'}
                        onclick={() => { if (selectedGlossaryChar) updateCharacterGender(selectedGlossaryChar, 'neuter'); }}
                      >
                        <div class="gender-card-top">
                          <span class="gender-letter">Ср</span>
                          <span class="gender-title">Средний</span>
                        </div>
                        <span class="gender-example">оно / существо, чудовище</span>
                      </button>

                      <button 
                        type="button" 
                        class="gender-card gender-card-unknown"
                        class:active={selectedGlossaryChar.gender === 'unknown' || !selectedGlossaryChar.gender}
                        onclick={() => { if (selectedGlossaryChar) updateCharacterGender(selectedGlossaryChar, 'unknown'); }}
                      >
                        <div class="gender-card-top">
                          <span class="gender-letter">—</span>
                          <span class="gender-title">Не указан</span>
                        </div>
                        <span class="gender-example">нейтрально</span>
                      </button>
                    </div>
                  </div>

                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Контекст персонажа / Стиль речи:</span>
                    </label>
                    <textarea 
                      bind:value={selectedGlossaryChar.notes} 
                      oninput={() => glossaryDirty = true}
                      rows="4" 
                      placeholder="напр. Обращение на 'ты', дерзкая студентка, младшая сестра героя, использует неформальный сленг..."
                      class="sidebar-textarea"
                    ></textarea>
                    <div class="ai-prompt-callout">
                      <Sparkles size={13} class="text-sky-400 shrink-0" />
                      <span>Передается в промпт LM Studio для точного согласования окончаний и тона реплик</span>
                    </div>
                  </div>
                </div>
              {:else}
                <div class="sidebar-edit-empty">
                  <Users size={36} class="text-slate-600" />
                  <p>Выберите персонажа из списка слева для настройки перевода и параметров</p>
                </div>
              {/if}
            {:else}
              <!-- Редактирование термина -->
              {#if selectedGlossaryTerm}
                <div class="sidebar-edit-header">
                  <div class="sidebar-char-headings">
                    <h4>Редактирование термина</h4>
                    <span class="char-sub-label">Словарь терминологии проекта</span>
                  </div>
                </div>

                <div class="sidebar-edit-content">
                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Исходный термин (EN):</span>
                    </label>
                    <input 
                      type="text" 
                      bind:value={selectedGlossaryTerm.source_term} 
                      oninput={() => glossaryDirty = true}
                      class="sidebar-input"
                    />
                  </div>

                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Русский перевод (RU):</span>
                    </label>
                    <input 
                      type="text" 
                      bind:value={selectedGlossaryTerm.translated_term} 
                      oninput={() => glossaryDirty = true}
                      placeholder="Перевод на русский..."
                      class="sidebar-input sidebar-input-primary"
                    />
                  </div>

                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Категория термина:</span>
                    </label>
                    <select 
                      bind:value={selectedGlossaryTerm.category} 
                      onchange={() => glossaryDirty = true}
                      class="sidebar-select"
                    >
                      <option value="general">Общий термин</option>
                      <option value="location">Локация / Место</option>
                      <option value="title">Титул / Звание</option>
                      <option value="item">Предмет / Артефакт</option>
                    </select>
                  </div>

                  <div class="edit-group">
                    <label class="edit-label">
                      <span>Заметки / Контекст:</span>
                    </label>
                    <textarea 
                      bind:value={selectedGlossaryTerm.notes} 
                      oninput={() => glossaryDirty = true}
                      rows="3" 
                      placeholder="Контекст употребления термина..."
                      class="sidebar-textarea"
                    ></textarea>
                  </div>

                  <div class="edit-sidebar-footer-actions">
                    <button 
                      class="btn-delete-term" 
                      onclick={() => { if (selectedGlossaryTerm) removeGlossaryTerm(selectedGlossaryTerm.id); }}
                    >
                      <Trash2 size={13} />
                      <span>Удалить термин</span>
                    </button>
                  </div>
                </div>
              {:else}
                <div class="sidebar-edit-empty">
                  <BookMarked size={36} class="text-slate-600" />
                  <p>Выберите термин из списка слева для редактирования</p>
                </div>
              {/if}
            {/if}
          </aside>
        </div>

        <!-- Подвал модального окна глоссария -->
        <div class="glossary-modal-footer">
          <div class="footer-path-info">
            <span class="info-dot" class:dirty={glossaryDirty}></span>
            {#if glossaryDirty}
              <span class="text-amber-400">Есть несохраненные правки в глоссарии</span>
            {:else}
              <span class="text-slate-400">Все изменения сохранены в кэш проекта и translator_glossary.json</span>
            {/if}
          </div>
          <div class="footer-actions">
            <button class="btn-secondary" onclick={() => showGlossaryModal = false}>Закрыть</button>
            <button class="btn-primary-save" onclick={saveGlossary} disabled={savingGlossary}>
              {#if savingGlossary}
                <Loader2 size={14} class="spin" />
                <span>Сохранение...</span>
              {:else}
                <Save size={14} />
                <span>Сохранить глоссарий</span>
              {/if}
            </button>
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if showProjectManager}
    <div
      class="project-manager-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="project-manager-title"
      tabindex="-1"
      onclick={(event) => {
        if (event.target === event.currentTarget) showProjectManager = false;
      }}
      onkeydown={(event) => {
        if (event.key === "Escape") showProjectManager = false;
      }}
    >
      <section class="project-manager-modal">
        <header class="project-manager-header">
          <div class="project-manager-heading">
            <div class="project-manager-icon"><Folder size={19} /></div>
            <div>
              <h2 id="project-manager-title">Менеджер проектов</h2>
              <p>Недавно открытые проекты · {recentProjects.length}</p>
            </div>
          </div>
          <button class="project-manager-close" aria-label="Закрыть" onclick={() => showProjectManager = false}>
            <X size={17} />
          </button>
        </header>

        <div class="project-manager-toolbar">
          <div class="project-manager-search">
            <Search size={15} />
            <input
              bind:value={projectManagerSearch}
              placeholder="Найти проект по имени или пути..."
              aria-label="Поиск проектов"
            />
            {#if projectManagerSearch}
              <button aria-label="Очистить поиск" onclick={() => projectManagerSearch = ""}>
                <X size={14} />
              </button>
            {/if}
          </div>
          <button
            class="project-manager-browse"
            disabled={loadingProject}
            onclick={() => {
              showProjectManager = false;
              pickProjectFolder();
            }}
          >
            <FolderOpen size={15} />
            <span>Открыть папку...</span>
          </button>
        </div>

        <div class="project-manager-list">
          {#if filteredRecentProjects.length === 0}
            <div class="project-manager-empty">
              <FolderOpen size={30} />
              <strong>{recentProjects.length === 0 ? "Пока нет открытых проектов" : "Ничего не найдено"}</strong>
              <span>{recentProjects.length === 0 ? "Выберите папку игры, чтобы добавить первый проект." : "Измените запрос и попробуйте ещё раз."}</span>
            </div>
          {:else}
            {#each filteredRecentProjects as path (path)}
              <div class="project-manager-row" class:project-current={gameDir === path}>
                {#if editingProjectPath === path}
                  <div class="project-manager-rename">
                    <span class="project-manager-row-icon"><FolderOpen size={17} /></span>
                    <input
                      class="project-manager-name-input"
                      bind:value={editingProjectName}
                      aria-label="Новое название проекта"
                      onkeydown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          saveProjectName(path);
                        } else if (event.key === "Escape") {
                          event.stopPropagation();
                          cancelProjectRename();
                        }
                      }}
                    />
                    <button class="project-manager-save-name" title="Сохранить название" onclick={() => saveProjectName(path)}>
                      <Save size={14} />
                    </button>
                    <button class="project-manager-remove" title="Отменить переименование" onclick={cancelProjectRename}>
                      <X size={14} />
                    </button>
                  </div>
                {:else}
                  <button
                    class="project-manager-open"
                    title={`Открыть ${path}`}
                    onclick={() => {
                      showProjectManager = false;
                      chooseRecentProject(path);
                    }}
                  >
                    <span class="project-manager-row-icon"><FolderOpen size={17} /></span>
                    <span class="project-manager-row-info">
                      <span class="project-manager-name">{projectDisplayName(path)}</span>
                      <span class="project-manager-path">{path}</span>
                    </span>
                    <span class="project-manager-open-label">Открыть</span>
                  </button>
                  <button
                    class="project-manager-rename-trigger"
                    title="Переименовать проект"
                    aria-label={`Переименовать ${projectDisplayName(path)}`}
                    onclick={() => beginProjectRename(path)}
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    class="project-manager-remove"
                    title="Убрать из списка проектов"
                    aria-label={`Убрать ${projectDisplayName(path)} из списка`}
                    onclick={() => removeRecentProject(path)}
                  >
                    <Trash2 size={15} />
                  </button>
                {/if}
              </div>
            {/each}
          {/if}
        </div>

        <footer class="project-manager-footer">
          <span>Удаление убирает проект только из этого списка.</span>
          <button onclick={() => showProjectManager = false}>Готово</button>
        </footer>
      </section>
    </div>
  {/if}

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
    width: 100%;
    min-width: 0;
  }

  /* TOPBAR */
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 18px;
    background: #14171d;
    border-bottom: 1px solid #232832;
    min-height: 52px;
    box-sizing: border-box;
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    flex: 1;
    max-width: 780px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 0 0 auto;
    min-width: 0;
  }

  .brand-title {
    font-weight: 700;
    font-size: 16px;
    color: #f8fafc;
    letter-spacing: -0.3px;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .badge-tag {
    font-size: 11px;
    background: #1e293b;
    color: #38bdf8;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid #0284c733;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .dir-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #0d0f12;
    border: 1px solid #262c38;
    border-radius: 8px;
    padding: 4px 10px;
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
  }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
    margin-left: auto;
  }

  .project-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .topbar-divider {
    width: 1px;
    height: 22px;
    background: #252c38;
    flex-shrink: 0;
  }

  .tools-menu-wrap {
    position: relative;
    flex-shrink: 0;
  }

  .glossary-header-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    min-height: 34px;
    padding: 6px 11px;
    border: 1px solid #1e5c4a;
    border-radius: 7px;
    background: #10271f;
    color: #a7f3d0;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    flex-shrink: 0;
    transition: all 0.15s ease;
  }

  .glossary-header-button:hover:not(.disabled) {
    border-color: #34d399;
    background: #17372c;
    color: #ecfdf5;
  }

  .glossary-header-button.disabled {
    opacity: 0.5;
    cursor: not-allowed;
    border-color: #23352f;
    background: #121915;
    color: #637f75;
  }

  .tools-menu-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    min-height: 34px;
    padding: 6px;
    border: 1px solid #30394a;
    border-radius: 7px;
    background: #151922;
    color: #c7d0df;
    font: inherit;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .tools-menu-trigger:hover,
  .tools-menu-trigger.menu-open {
    border-color: #3b82f6;
    background: #202738;
    color: #f1f5f9;
  }

  .tools-menu {
    position: absolute;
    z-index: 200;
    top: calc(100% + 8px);
    right: 0;
    width: min(300px, calc(100vw - 20px));
    padding: 7px;
    border: 1px solid #30394a;
    border-radius: 10px;
    background: #11151d;
    box-shadow: 0 16px 38px #0009, 0 2px 8px #0006;
  }

  .tools-menu-heading {
    padding: 7px 9px 9px;
    color: #74829a;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.09em;
  }

  .tools-menu-divider {
    height: 1px;
    background: #212937;
    margin: 6px 0;
  }

  .tools-menu-item {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    padding: 9px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: #9aa8bd;
    text-align: left;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .tools-menu-item:hover:not(:disabled) {
    background: #1d2a40;
    color: #e2e8f0;
  }

  .tools-menu-item:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .tools-menu-item > span:first-of-type {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .tools-menu-item strong {
    color: #dbe4f0;
    font-size: 12px;
    font-weight: 600;
  }

  .tools-menu-item small {
    color: #77849a;
    font-size: 10px;
  }

  .btn-browse,
  .btn-primary {
    flex: 0 0 auto;
  }

  .btn-browse {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #151922;
    border: 1px solid #30394a;
    border-radius: 6px;
    color: #c7d0df;
    padding: 6px 9px;
    font: inherit;
    font-size: 12px;
    white-space: nowrap;
    transition: background 0.15s, border-color 0.15s, color 0.15s;
  }

  .btn-browse {
    cursor: pointer;
  }

  .btn-browse:hover:not(:disabled) {
    background: #222834;
    color: #fff;
  }

  .btn-browse:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  .recent-project-picker {
    position: relative;
    flex: 1 1 auto;
    min-width: 100px;
  }

  .current-project-label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    min-width: 0;
    padding: 6px 4px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: #e2e8f0;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    text-align: left;
    transition: background 0.15s, border-color 0.15s, color 0.15s;
  }

  .current-project-label > span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .current-project-label > :global(svg) {
    flex: 0 0 auto;
    color: #8b9ab2;
    transition: transform 0.15s;
  }

  .current-project-label:hover,
  .current-project-label.menu-open {
    background: #202738;
    border-color: #3b82f6;
    color: #f1f5f9;
  }

  .current-project-label.menu-open > :global(svg) {
    transform: rotate(180deg);
  }

  .recent-project-menu {
    position: absolute;
    z-index: 100;
    top: calc(100% + 8px);
    right: 0;
    width: min(360px, calc(100vw - 32px));
    max-height: 340px;
    overflow-y: auto;
    padding: 6px;
    background: #11151d;
    border: 1px solid #30394a;
    border-radius: 10px;
    box-shadow: 0 16px 38px #0009, 0 2px 8px #0006;
  }

  .recent-project-menu-title {
    padding: 7px 9px 8px;
    color: #74829a;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.09em;
  }

  .recent-project-empty {
    padding: 12px 10px;
    color: #8290a5;
    font-size: 11px;
  }

  .recent-project-menu-footer {
    margin-top: 5px;
    padding-top: 5px;
    border-top: 1px solid #252e3d;
  }

  .recent-project-menu-footer button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 9px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: #aebbd0;
    text-align: left;
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }

  .recent-project-menu-footer button:hover {
    background: #1d2a40;
    color: #93c5fd;
  }

  .recent-project-menu-count {
    margin-left: auto;
    color: #7888a1;
    font-size: 10px;
  }

  .recent-project-option {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 9px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: #9aa8bd;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }

  .recent-project-option:hover,
  .recent-project-option.recent-project-current {
    background: #1d2a40;
    color: #60a5fa;
  }

  .recent-project-option-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .recent-project-name,
  .recent-project-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .recent-project-name {
    color: #e2e8f0;
    font-size: 12px;
    font-weight: 600;
  }

  .recent-project-path {
    max-width: 300px;
    color: #77849a;
    font-size: 10px;
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
    width: 160px;
    flex-shrink: 0;
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

  @media (max-width: 1280px) {
    .topbar {
      gap: 12px;
      padding: 8px 14px;
    }

    .badge-tag {
      display: none;
    }

    .global-stats {
      width: 130px;
    }
  }

  @media (max-width: 1024px) {
    .dir-bar .btn-browse span,
    .dir-bar .btn-primary span {
      display: none;
    }

    .btn-browse,
    .dir-bar .btn-primary {
      justify-content: center;
      min-width: 34px;
      padding: 7px;
    }
  }

  @media (max-width: 768px) {
    .topbar {
      gap: 8px;
      padding: 6px 10px;
    }

    .topbar-left {
      gap: 8px;
    }

    .global-stats {
      display: none;
    }

    .glossary-header-label {
      display: none;
    }

    .glossary-header-button {
      min-width: 34px;
      padding: 6px 8px;
    }
  }

  @media (max-width: 520px) {
    .brand-title {
      display: none;
    }

    .topbar-divider {
      display: none;
    }

    .recent-project-picker {
      min-width: 60px;
    }

    .dir-bar > :global(svg:first-child) {
      display: none;
    }
  }

  /* MODAL WINDOW FOR RPA & RPYC STUDIO */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(4, 7, 12, 0.75);
    backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    animation: modalFadeIn 0.18s ease-out;
  }

  @keyframes modalFadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .tools-modal {
    background: #0f121a;
    border: 1px solid #1e2638;
    border-radius: 12px;
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.8), 0 0 0 1px rgba(56, 189, 248, 0.1);
    width: 100%;
    max-width: 860px;
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: modalScaleUp 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes modalScaleUp {
    from { transform: scale(0.96) translateY(8px); }
    to { transform: scale(1) translateY(0); }
  }

  .tools-modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    background: #141824;
    border-bottom: 1px solid #1e2638;
  }

  .modal-title-wrap {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .modal-title-wrap h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: #f1f5f9;
  }

  .btn-close-modal {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 6px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s;
  }

  .btn-close-modal:hover {
    background: #1e2638;
    color: #f1f5f9;
  }

  .tools-tab-bar {
    display: flex;
    background: #0d0f16;
    border-bottom: 1px solid #1a2232;
    padding: 0 16px;
  }

  .tools-tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: #94a3b8;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .tools-tab:hover {
    color: #f1f5f9;
  }

  .tools-tab.tab-active {
    color: #38bdf8;
    border-bottom-color: #38bdf8;
    background: rgba(56, 189, 248, 0.05);
  }

  .tools-modal-body {
    padding: 20px;
    overflow-y: auto;
    max-height: calc(88vh - 120px);
  }

  .tools-cards-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }

  .tool-card {
    background: #131722;
    border: 1px solid #1e2638;
    border-radius: 10px;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .tool-card.full-width-card {
    grid-column: 1 / -1;
  }

  .tool-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .tool-head-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .tool-head-title h4 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: #f8fafc;
  }

  .tool-badge {
    font-size: 10px;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.3);
    font-weight: 600;
  }

  .tool-badge-cyan {
    background: rgba(56, 189, 248, 0.15);
    color: #38bdf8;
    border-color: rgba(56, 189, 248, 0.3);
  }

  .tool-badge-green {
    background: rgba(52, 211, 153, 0.15);
    color: #34d399;
    border-color: rgba(52, 211, 153, 0.3);
  }

  .tool-badge-rose {
    background: rgba(244, 63, 94, 0.15);
    color: #f43f5e;
    border-color: rgba(244, 63, 94, 0.3);
  }

  .tool-desc {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: #94a3b8;
  }

  .tool-desc code {
    background: #090c12;
    padding: 2px 5px;
    border-radius: 4px;
    color: #38bdf8;
    font-size: 11px;
    border: 1px solid #1a2232;
  }

  .tool-form-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .tool-field-label {
    font-size: 11px;
    font-weight: 500;
    color: #cbd5e1;
  }

  .tool-readonly-path {
    background: #0b0e14;
    border: 1px solid #1a2232;
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 12px;
    color: #64748b;
    font-family: monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tool-input {
    background: #0b0e14;
    border: 1px solid #232c3e;
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 12px;
    color: #f1f5f9;
    font-family: monospace;
    outline: none;
    transition: border-color 0.15s;
  }

  .tool-input:focus {
    border-color: #38bdf8;
  }

  .btn-tool-action {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 9px 16px;
    border-radius: 7px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    border: none;
    transition: all 0.15s ease;
  }

  .btn-tool-action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-pack-action {
    background: #d97706;
    color: #ffffff;
  }

  .btn-pack-action:hover:not(:disabled) {
    background: #b45309;
    box-shadow: 0 0 16px rgba(217, 119, 6, 0.4);
  }

  .btn-unpack-action {
    background: #0284c7;
    color: #ffffff;
  }

  .btn-unpack-action:hover:not(:disabled) {
    background: #0369a1;
    box-shadow: 0 0 16px rgba(2, 132, 199, 0.4);
  }

  .btn-decompile-action {
    background: #059669;
    color: #ffffff;
    height: 34px;
    white-space: nowrap;
  }

  .btn-decompile-action:hover:not(:disabled) {
    background: #047857;
    box-shadow: 0 0 16px rgba(5, 150, 105, 0.4);
  }

  .btn-clean-action {
    background: #e11d48;
    color: #ffffff;
  }

  .btn-clean-action:hover:not(:disabled) {
    background: #be123c;
    box-shadow: 0 0 16px rgba(225, 29, 72, 0.4);
  }

  .tool-result-box {
    background: #0c121d;
    border: 1px solid #1a2b42;
    color: #7dd3fc;
    font-size: 12px;
    padding: 8px 12px;
    border-radius: 6px;
    font-family: monospace;
  }

  .tool-inline-form {
    display: flex;
    align-items: flex-end;
    gap: 12px;
  }

  .flex-1 {
    flex: 1;
  }

  .tool-form-action-wrap {
    margin-bottom: 0;
  }

  .decompile-preview-wrap {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .preview-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    color: #94a3b8;
  }

  .btn-copy-code {
    display: flex;
    align-items: center;
    gap: 5px;
    background: #182030;
    border: 1px solid #233048;
    border-radius: 5px;
    padding: 4px 10px;
    font-size: 11px;
    color: #7dd3fc;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-copy-code:hover {
    background: #202c44;
    border-color: #38bdf8;
    color: #f0f9ff;
  }

  .decompile-code-area {
    width: 100%;
    height: 240px;
    background: #080a0f;
    border: 1px solid #1a2232;
    border-radius: 6px;
    padding: 12px;
    font-family: 'JetBrains Mono', 'Fira Code', Consolas, monospace;
    font-size: 12px;
    line-height: 1.5;
    color: #e2e8f0;
    resize: vertical;
    outline: none;
    box-sizing: border-box;
  }

  .cache-options-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: #0a0d14;
    padding: 10px 14px;
    border-radius: 6px;
    border: 1px solid #1a2232;
  }

  .radio-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: #cbd5e1;
    cursor: pointer;
  }

  .radio-label code {
    background: #131824;
    padding: 1px 4px;
    border-radius: 3px;
    color: #f43f5e;
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

  /* Единый стиль строки дерева (28px, гармоничные отступы, шрифт 12px) */
  .tree-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    min-height: 28px;
    padding: 0 8px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 5px;
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
    background: #162438 !important;
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

  /* Тонкая микрополоска прогресса: зазор снизу 2.5px, чтобы НЕ перекрывать нижнюю рамку */
  .tree-micro-track {
    position: absolute;
    bottom: 2.5px;
    left: 28px;
    right: 8px;
    height: 2px;
    background: #1a2230;
    border-radius: 999px;
    overflow: hidden;
    pointer-events: none;
    z-index: 1;
  }

  .tree-row:hover .tree-micro-track {
    background: #202b3c;
  }

  .tree-file-row.active .tree-micro-track {
    bottom: 2.5px;
    background: #1b2e47;
  }

  .tree-micro-fill {
    height: 100%;
    background: #0284c7;
    border-radius: 999px;
    transition: width 0.2s ease;
  }

  .tree-micro-fill.fill-done {
    background: #10b981;
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
      background-color: #162438;
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
    gap: 5px;
    background: #141822;
    border: 1px solid #202838;
    padding: 2px 7px;
    border-radius: 4px;
    font-size: 11px;
    white-space: nowrap;
    max-width: 100%;
    box-sizing: border-box;
  }

  .char-chip.char-chip-ui {
    background: rgba(168, 85, 247, 0.08);
    border-color: rgba(168, 85, 247, 0.28);
  }

  .char-chip.char-chip-ui .char-alias {
    color: #c084fc;
  }

  .char-chip.char-chip-ui .char-name {
    color: #e9d5ff;
  }

  .char-chip-snippet {
    background: transparent;
    border: none;
    padding: 0;
    margin-right: 4px;
  }

  .char-chip-inspect {
    background: #162030;
    border-color: #243550;
    padding: 2px 8px;
    font-size: 12px;
  }

  .inspect-narrator {
    color: #64748b;
    font-style: italic;
    font-size: 12px;
  }

  .char-alias {
    color: #38bdf8;
    font-weight: 600;
    letter-spacing: 0.3px;
    flex-shrink: 0;
  }

  .char-name {
    color: #cbd5e1;
    overflow: hidden;
    text-overflow: ellipsis;
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
    padding: 0;
    font-family: inherit;
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

  /* GRAPH WORKSPACE & INNER SIDEBAR */
  .graph-workspace {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }

  .graph-topbar-controls {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .graph-sidebar-toggle-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    background: #141923;
    border: 1px solid #1e2636;
    border-radius: 6px;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .graph-sidebar-toggle-btn:hover {
    background: #1a2232;
    border-color: #2b394e;
    color: #e2e8f0;
  }

  .graph-sidebar-toggle-btn.active {
    background: #152236;
    border-color: #0284c7;
    color: #38bdf8;
  }

  .btn-state-pill {
    font-size: 10px;
    font-weight: 600;
    background: rgba(2, 132, 199, 0.2);
    color: #38bdf8;
    padding: 1px 5px;
    border-radius: 4px;
  }

  .graph-inner-sidebar {
    width: 250px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: #0d1017;
    border-right: 1px solid #1a202c;
    min-height: 0;
    overflow: hidden;
    z-index: 10;
  }

  .graph-sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid #171d28;
    background: #0f131c;
  }

  .graph-sidebar-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #94a3b8;
    text-transform: uppercase;
  }

  .graph-sidebar-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .graph-sidebar-search {
    position: relative;
    display: flex;
    align-items: center;
    padding: 8px 10px;
    border-bottom: 1px solid #171d28;
    background: #0b0e14;
    gap: 6px;
  }

  .graph-sidebar-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font: inherit;
    font-size: 12px;
  }

  .graph-sidebar-input::placeholder {
    color: #4b5563;
  }

  .graph-sidebar-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .graph-sidebar-empty {
    padding: 24px 12px;
    text-align: center;
    color: #4b5563;
    font-size: 12px;
  }

  .graph-scene-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 7px 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: #94a3b8;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .graph-scene-item:hover {
    background: #141a26;
    border-color: #202b3d;
    color: #f1f5f9;
  }

  .graph-scene-item.active {
    background: #14243b;
    border-color: #0284c7;
    color: #38bdf8;
  }

  .graph-scene-item-left {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    flex: 1;
  }

  .graph-scene-seq {
    color: #55657e;
    font-size: 10px;
    font-family: monospace;
    flex-shrink: 0;
  }

  .graph-scene-name {
    font-size: 12px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .graph-scene-item-right {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
  }

  .graph-scene-pct {
    font-size: 10px;
    font-weight: 600;
    color: #64748b;
    padding: 1px 4px;
    background: #090c12;
    border-radius: 4px;
  }

  .graph-scene-pct.pct-done {
    background: rgba(16, 185, 129, 0.15);
    color: #34d399;
  }

  .graph-canvas-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  /* TABLE SCENE NAV (DROPDOWN + STEPPERS IN TOOLBAR) */
  .table-scene-nav {
    display: flex;
    align-items: center;
    gap: 4px;
    position: relative;
  }

  .btn-scene-step {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: #141820;
    border: 1px solid #242c3b;
    border-radius: 6px;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-scene-step:hover:not(:disabled) {
    background: #202838;
    color: #f8fafc;
    border-color: #38bdf8;
  }

  .btn-scene-step:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .table-scene-picker-wrap {
    position: relative;
  }

  .btn-scene-picker {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    min-height: 28px;
    padding: 4px 9px;
    background: #141820;
    border: 1px solid #242c3b;
    border-radius: 6px;
    color: #e2e8f0;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    transition: all 0.15s ease;
    max-width: 260px;
  }

  .btn-scene-picker:hover,
  .btn-scene-picker.menu-open {
    border-color: #38bdf8;
    background: #1a2230;
    color: #fff;
  }

  .scene-picker-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }

  .scene-picker-badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 4px;
    background: #1e2636;
    color: #7dd3fc;
    font-weight: 600;
    flex-shrink: 0;
  }

  .scene-picker-badge.badge-complete {
    background: rgba(16, 185, 129, 0.15);
    color: #34d399;
  }

  .scene-picker-badge.badge-router {
    background: rgba(99, 102, 241, 0.2);
    border: 1px solid rgba(99, 102, 241, 0.4);
    color: #a5b4fc;
  }

  .empty-scene-card {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 32px 28px;
    margin: 20px;
    background: #0f172a;
    border: 1px dashed #334155;
    border-radius: 12px;
  }

  .empty-scene-icon-wrap {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    border-radius: 12px;
    background: #1e293b;
    border: 1px solid #38bdf840;
    flex-shrink: 0;
  }

  .empty-scene-info h3 {
    margin: 0 0 6px 0;
    font-size: 15px;
    font-weight: 600;
    color: #f1f5f9;
  }

  .empty-scene-info p {
    margin: 0 0 12px 0;
    font-size: 13px;
    color: #94a3b8;
    line-height: 1.5;
  }

  .btn-return-graph {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 6px 12px;
    background: #0284c7;
    border: 1px solid #38bdf8;
    border-radius: 6px;
    color: #ffffff;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s;
  }

  .btn-return-graph:hover {
    background: #0369a1;
  }

  .picker-caret {
    color: #64748b;
    flex-shrink: 0;
  }

  .scene-picker-dropdown {
    position: absolute;
    z-index: 300;
    top: calc(100% + 5px);
    left: 0;
    width: min(320px, calc(100vw - 24px));
    max-height: 380px;
    display: flex;
    flex-direction: column;
    background: #11151d;
    border: 1px solid #2c3647;
    border-radius: 9px;
    box-shadow: 0 16px 36px rgba(0, 0, 0, 0.65), 0 2px 8px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }

  .scene-picker-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 9px;
    background: #0d1017;
    border-bottom: 1px solid #1e2634;
  }

  .scene-search-input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font: inherit;
    font-size: 11px;
  }

  .clear-search-btn {
    border: none;
    background: transparent;
    color: #64748b;
    cursor: pointer;
    font-size: 11px;
    padding: 2px 4px;
  }

  .clear-search-btn:hover {
    color: #e2e8f0;
  }

  .scene-picker-list {
    flex: 1;
    overflow-y: auto;
    padding: 5px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .scene-picker-item {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 6px 8px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: #cbd5e1;
    font: inherit;
    font-size: 11px;
    text-align: left;
    cursor: pointer;
    transition: all 0.1s ease;
  }

  .scene-picker-item:hover {
    background: #1a2332;
    color: #fff;
  }

  .scene-picker-item.selected {
    background: #192a40;
    border-color: #0284c7;
    color: #38bdf8;
    font-weight: 600;
  }

  .scene-item-seq {
    color: #55657e;
    font-size: 10px;
    font-family: monospace;
    flex-shrink: 0;
  }

  .scene-item-title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scene-item-badge {
    font-size: 10px;
    padding: 1px 4px;
    border-radius: 3px;
    background: #0d1219;
    color: #64748b;
    flex-shrink: 0;
  }

  .scene-item-badge.badge-complete {
    background: rgba(16, 185, 129, 0.15);
    color: #34d399;
  }

  .scene-picker-empty {
    padding: 16px;
    text-align: center;
    color: #64748b;
    font-size: 11px;
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

  /* GLOSSARY STYLES */
  .char-chip-interactive {
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .char-chip-interactive:hover {
    background: #1b2232;
    border-color: #38bdf8;
    transform: translateY(-1px);
  }

  .char-orig-sub {
    font-size: 10px;
    color: #64748b;
    margin-left: 2px;
  }

  .gender-pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    font-weight: 700;
    line-height: 1;
    padding: 1px 4px;
    border-radius: 3px;
    margin-left: 3px;
    flex-shrink: 0;
  }

  .gender-pill.gender-male {
    background: rgba(56, 189, 248, 0.2);
    color: #38bdf8;
    border: 1px solid rgba(56, 189, 248, 0.4);
  }

  .gender-pill.gender-female {
    background: rgba(244, 63, 94, 0.2);
    color: #fb7185;
    border: 1px solid rgba(244, 63, 94, 0.4);
  }

  .gender-pill.gender-unknown {
    background: rgba(148, 163, 184, 0.15);
    color: #94a3b8;
    border: 1px solid rgba(148, 163, 184, 0.3);
  }

  .inspect-speaker-group {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .inspect-char-notes {
    font-size: 11px;
    color: #38bdf8;
    background: rgba(56, 189, 248, 0.08);
    border: 1px dashed rgba(56, 189, 248, 0.25);
    padding: 2px 8px;
    border-radius: 4px;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .btn-micro-glossary {
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    color: #34d399;
    padding: 3px 6px;
    border-radius: 4px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
  }

  .btn-micro-glossary:hover {
    background: rgba(16, 185, 129, 0.25);
    color: #ffffff;
  }

  .characters-header-wrap {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-right: 8px;
  }

  .btn-sidebar-glossary {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    color: #34d399;
    padding: 2px 7px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-sidebar-glossary:hover {
    background: rgba(16, 185, 129, 0.25);
    border-color: #34d399;
    color: #ffffff;
  }

  .topbar-badge {
    background: rgba(16, 185, 129, 0.3);
    color: #6ee7b7;
    font-size: 10px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 999px;
    margin-left: 2px;
  }

  /* GLOSSARY MODAL */
  .glossary-modal {
    max-width: 1140px;
    height: 86vh;
    max-height: 86vh;
  }

  .modal-subtitle {
    margin: 2px 0 0 0;
    font-size: 11px;
    color: #64748b;
    font-weight: 400;
  }

  .modal-header-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .btn-modal-save {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #142820;
    border: 1px solid #1e4a38;
    color: #34d399;
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-modal-save:hover:not(:disabled) {
    background: #10b981;
    border-color: #10b981;
    color: #ffffff;
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.3);
  }

  .btn-modal-save.dirty {
    background: #3c2415;
    border-color: #f59e0b;
    color: #fcd34d;
    animation: pulseDirty 2s infinite ease-in-out;
  }

  @keyframes pulseDirty {
    0%, 100% { border-color: #f59e0b; }
    50% { border-color: #fbbf24; box-shadow: 0 0 10px rgba(245, 158, 11, 0.3); }
  }

  .tab-badge-accent {
    background: rgba(16, 185, 129, 0.2);
    color: #34d399;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    font-weight: 600;
  }

  .glossary-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 18px;
    background: #11151f;
    border-bottom: 1px solid #1a2232;
    flex-wrap: wrap;
  }

  /* ИСПРАВЛЕННЫЙ КОНТЕЙНЕР ПОИСКА (БЕЗ НАЕЗДА ИКОНКИ) */
  .glossary-search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #0a0d14;
    border: 1px solid #232c3f;
    border-radius: 6px;
    padding: 6px 10px;
    flex: 1;
    min-width: 240px;
    max-width: 380px;
    transition: all 0.15s ease;
  }

  .glossary-search-box:focus-within {
    border-color: #10b981;
    box-shadow: 0 0 0 1px #10b981;
    background: #0d121c;
  }

  .glossary-search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font-size: 12px;
    padding: 0;
  }

  .search-box-icon {
    flex-shrink: 0;
  }

  .btn-clear-search {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
    border-radius: 3px;
  }

  .btn-clear-search:hover {
    color: #f1f5f9;
  }

  .gender-filter-group,
  .category-filter-group {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .filter-label {
    font-size: 11px;
    color: #64748b;
    margin-right: 4px;
  }

  .btn-filter-pill {
    background: #141924;
    border: 1px solid #20293b;
    color: #94a3b8;
    font-size: 11px;
    font-weight: 500;
    padding: 3px 9px;
    border-radius: 999px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-filter-pill:hover {
    color: #f1f5f9;
    border-color: #334155;
  }

  .btn-filter-pill.active {
    background: #1e293b;
    border-color: #475569;
    color: #f8fafc;
  }

  .btn-filter-pill.filter-male.active {
    background: rgba(56, 189, 248, 0.2);
    border-color: #38bdf8;
    color: #7dd3fc;
  }

  .btn-filter-pill.filter-female.active {
    background: rgba(244, 63, 94, 0.2);
    border-color: #fb7185;
    color: #fda4af;
  }

  .btn-filter-pill.filter-neuter.active {
    background: rgba(168, 85, 247, 0.2);
    border-color: #a855f7;
    color: #d8b4fe;
  }

  .btn-filter-pill.filter-unknown.active {
    background: rgba(148, 163, 184, 0.2);
    border-color: #94a3b8;
    color: #cbd5e1;
  }

  .btn-toggle-add-char {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: #141f2e;
    border: 1px solid #23354f;
    color: #38bdf8;
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-toggle-add-char:hover {
    background: #1c2b40;
    border-color: #38bdf8;
  }

  .add-char-panel,
  .add-term-panel {
    background: #0d111a;
    border-bottom: 1px solid #1a2232;
    padding: 10px 18px;
  }

  .add-char-row,
  .add-term-row {
    display: flex;
    align-items: flex-end;
    gap: 12px;
    flex-wrap: wrap;
  }

  .field-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 130px;
  }

  .field-item.flex-2 {
    flex: 2;
    min-width: 180px;
  }

  .field-label {
    font-size: 11px;
    font-weight: 500;
    color: #64748b;
  }

  .btn-confirm-add {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: #064e3b;
    border: 1px solid #10b981;
    color: #6ee7b7;
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    height: 31px;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .btn-confirm-add:hover {
    background: #10b981;
    color: #ffffff;
  }

  /* РАЗДЕЛЕННЫЙ РЕЖИМ (MASTER-DETAIL): СПИСОК СЛЕВА, САЙДБАР РЕДАКТИРОВАНИЯ СПРАВА */
  .glossary-split-container {
    flex: 1;
    display: flex;
    overflow: hidden;
    background: #0a0d14;
    min-height: 0;
  }

  /* ЛЕВАЯ ПАНЕЛЬ: СПИСОК */
  .glossary-list-pane {
    flex: 1;
    overflow-y: auto;
    border-right: 1px solid #1a2232;
    background: #0a0d14;
  }

  .glossary-list-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .glossary-list-table th {
    position: sticky;
    top: 0;
    background: #111520;
    padding: 8px 12px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: #94a3b8;
    border-bottom: 1px solid #1c2436;
    z-index: 10;
  }

  .glossary-selectable-row {
    cursor: pointer;
    transition: background 0.1s ease;
  }

  .glossary-selectable-row td {
    padding: 8px 12px;
    border-bottom: 1px solid #131722;
    vertical-align: middle;
  }

  .glossary-selectable-row:hover {
    background: #121622;
  }

  .glossary-selectable-row.selected-row {
    background: #141c2c;
    border-left: 3px solid #10b981;
  }

  .glossary-selectable-row.selected-row td {
    border-bottom-color: #1e283c;
  }

  .char-name-en {
    color: #f1f5f9;
    font-weight: 600;
    font-size: 13px;
  }

  .char-name-ru {
    color: #34d399;
    font-weight: 500;
  }

  .char-name-empty {
    color: #475569;
    font-style: italic;
    font-size: 11px;
  }

  .notes-snippet {
    display: block;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #94a3b8;
    font-size: 11px;
  }

  .category-badge {
    display: inline-block;
    padding: 2px 7px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 600;
  }

  .category-general { background: rgba(148, 163, 184, 0.15); color: #94a3b8; }
  .category-location { background: rgba(56, 189, 248, 0.15); color: #38bdf8; }
  .category-title { background: rgba(245, 158, 11, 0.15); color: #fbbf24; }
  .category-item { background: rgba(168, 85, 247, 0.15); color: #c084fc; }

  /* ПРАВАЯ ПАНЕЛЬ: САЙДБАР РЕДАКТИРОВАНИЯ */
  .glossary-edit-sidebar {
    width: 380px;
    background: #0f131d;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    flex-shrink: 0;
  }

  .sidebar-edit-header {
    padding: 16px 18px 12px;
    border-bottom: 1px solid #1a2232;
    background: #111624;
  }

  .sidebar-char-badge-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .char-alias-badge-lg {
    display: inline-block;
    font-family: monospace;
    font-size: 13px;
    font-weight: 700;
    color: #38bdf8;
    background: rgba(56, 189, 248, 0.12);
    border: 1px solid rgba(56, 189, 248, 0.3);
    padding: 3px 8px;
    border-radius: 6px;
  }

  .sidebar-char-headings h4 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: #f8fafc;
  }

  .char-sub-label {
    font-size: 11px;
    color: #64748b;
  }

  .sidebar-edit-content {
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    flex: 1;
  }

  .sidebar-edit-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: #64748b;
    gap: 12px;
    padding: 24px;
    text-align: center;
  }

  .edit-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .edit-label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    font-weight: 600;
    color: #cbd5e1;
  }

  .sidebar-input {
    width: 100%;
    background: #090c12;
    border: 1px solid #222c3f;
    border-radius: 6px;
    padding: 7px 11px;
    color: #f8fafc;
    font-size: 13px;
    outline: none;
    box-sizing: border-box;
    transition: all 0.15s;
  }

  .sidebar-input:focus {
    border-color: #10b981;
    background: #0d121c;
    box-shadow: 0 0 0 1px #10b981;
  }

  .sidebar-input-primary {
    font-weight: 600;
    font-size: 14px;
    color: #34d399;
    border-color: #1e3a2e;
  }

  .sidebar-select {
    width: 100%;
    background: #090c12;
    border: 1px solid #222c3f;
    border-radius: 6px;
    padding: 7px 10px;
    color: #f8fafc;
    font-size: 13px;
    outline: none;
    box-sizing: border-box;
    cursor: pointer;
  }

  .sidebar-select:focus {
    border-color: #10b981;
  }

  .sidebar-textarea {
    width: 100%;
    background: #090c12;
    border: 1px solid #222c3f;
    border-radius: 6px;
    padding: 8px 11px;
    color: #f8fafc;
    font-size: 12px;
    line-height: 1.5;
    outline: none;
    box-sizing: border-box;
    resize: vertical;
    font-family: inherit;
    transition: all 0.15s;
  }

  .sidebar-textarea:focus {
    border-color: #10b981;
    background: #0d121c;
    box-shadow: 0 0 0 1px #10b981;
  }

  .edit-hint {
    font-size: 11px;
    color: #64748b;
  }

  .ai-prompt-callout {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    background: rgba(56, 189, 248, 0.08);
    border: 1px dashed rgba(56, 189, 248, 0.25);
    padding: 8px 10px;
    border-radius: 6px;
    font-size: 11px;
    color: #7dd3fc;
    line-height: 1.4;
  }

  /* КАРТОЧКИ ВЫБОРА ПОЛА (М, Ж, Ср, —) В САЙДБАРЕ */
  .gender-cards-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .gender-card {
    background: #0a0d14;
    border: 1px solid #1e2638;
    border-radius: 8px;
    padding: 8px 10px;
    cursor: pointer;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 3px;
    transition: all 0.15s ease;
  }

  .gender-card:hover {
    background: #121824;
    border-color: #334155;
  }

  .gender-card-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .gender-letter {
    font-size: 12px;
    font-weight: 800;
    width: 22px;
    height: 22px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #151b27;
    color: #94a3b8;
  }

  .gender-title {
    font-size: 12px;
    font-weight: 600;
    color: #e2e8f0;
  }

  .gender-example {
    font-size: 10px;
    color: #64748b;
  }

  /* Активные состояния карточек пола */
  .gender-card-male.active {
    background: #0c2d48;
    border-color: #0284c7;
    box-shadow: 0 0 10px rgba(56, 189, 248, 0.25);
  }
  .gender-card-male.active .gender-letter {
    background: #0284c7;
    color: #ffffff;
  }
  .gender-card-male.active .gender-title {
    color: #38bdf8;
  }

  .gender-card-female.active {
    background: #471223;
    border-color: #e11d48;
    box-shadow: 0 0 10px rgba(244, 63, 94, 0.25);
  }
  .gender-card-female.active .gender-letter {
    background: #e11d48;
    color: #ffffff;
  }
  .gender-card-female.active .gender-title {
    color: #fb7185;
  }

  .gender-card-neuter.active {
    background: #3b185f;
    border-color: #a855f7;
    box-shadow: 0 0 10px rgba(168, 85, 247, 0.25);
  }
  .gender-card-neuter.active .gender-letter {
    background: #a855f7;
    color: #ffffff;
  }
  .gender-card-neuter.active .gender-title {
    color: #d8b4fe;
  }

  .gender-card-unknown.active {
    background: #1e293b;
    border-color: #475569;
  }
  .gender-card-unknown.active .gender-letter {
    background: #475569;
    color: #ffffff;
  }
  .gender-card-unknown.active .gender-title {
    color: #f1f5f9;
  }

  .edit-sidebar-footer-actions {
    margin-top: auto;
    padding-top: 12px;
    border-top: 1px solid #1a2232;
  }

  .btn-delete-term {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: rgba(244, 63, 94, 0.1);
    border: 1px solid rgba(244, 63, 94, 0.3);
    color: #fb7185;
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-delete-term:hover {
    background: rgba(244, 63, 94, 0.2);
    border-color: #f43f5e;
    color: #ffffff;
  }

  /* БЕЙДЖИ ПОЛА */
  .gender-pill.gender-neuter {
    background: rgba(168, 85, 247, 0.2);
    color: #c084fc;
    border: 1px solid rgba(168, 85, 247, 0.4);
  }

  /* КНОПКИ ТОГГЛА В ДОБАВЛЕНИИ */
  .btn-gender-toggle.gender-neuter.active {
    background: #3b185f;
    border-color: #a855f7;
    color: #d8b4fe;
    box-shadow: 0 0 8px rgba(168, 85, 247, 0.35);
  }

  .glossary-loading,
  .glossary-empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 60px 20px;
    color: #64748b;
    text-align: center;
  }

  .glossary-modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 18px;
    background: #11141e;
    border-top: 1px solid #1a2232;
  }

  .footer-path-info {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
  }

  .info-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #10b981;
  }

  .info-dot.dirty {
    background: #f59e0b;
    box-shadow: 0 0 6px #f59e0b;
  }

  .footer-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .btn-secondary {
    background: #161c28;
    border: 1px solid #232d40;
    color: #cbd5e1;
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-secondary:hover {
    background: #1e2638;
    color: #f8fafc;
  }

  .btn-primary-save {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #059669;
    border: 1px solid #10b981;
    color: #ffffff;
    padding: 6px 16px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-primary-save:hover:not(:disabled) {
    background: #10b981;
    box-shadow: 0 0 12px rgba(16, 185, 129, 0.4);
  }

  .btn-primary-save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  @media (max-width: 700px) {
    .sidebar {
      width: 230px;
      flex: 0 0 230px;
    }
  }

  @media (max-width: 480px) {
    .sidebar {
      width: 190px;
      flex-basis: 190px;
    }
  }

  .project-manager-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 22px;
    background: rgba(5, 8, 13, 0.76);
    backdrop-filter: blur(7px);
  }

  .project-manager-modal {
    display: flex;
    flex-direction: column;
    width: min(760px, 100%);
    max-height: min(780px, 90vh);
    overflow: hidden;
    border: 1px solid #2a3548;
    border-radius: 14px;
    background: #11151d;
    box-shadow: 0 28px 80px #000b, 0 0 0 1px #60a5fa12;
  }

  .project-manager-header,
  .project-manager-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 17px 20px;
  }

  .project-manager-header {
    border-bottom: 1px solid #252e3d;
  }

  .project-manager-heading {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .project-manager-icon,
  .project-manager-row-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    color: #60a5fa;
  }

  .project-manager-icon {
    width: 38px;
    height: 38px;
    border: 1px solid #2d4770;
    border-radius: 10px;
    background: #1a2a43;
  }

  .project-manager-heading h2 {
    margin: 0;
    color: #f1f5f9;
    font-size: 17px;
    font-weight: 650;
  }

  .project-manager-heading p {
    margin: 4px 0 0;
    color: #8492a8;
    font-size: 11px;
  }

  .project-manager-close,
  .project-manager-search button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #8290a5;
    cursor: pointer;
  }

  .project-manager-close {
    width: 32px;
    height: 32px;
  }

  .project-manager-close:hover,
  .project-manager-search button:hover {
    background: #252e3d;
    color: #f1f5f9;
  }

  .project-manager-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 20px;
    border-bottom: 1px solid #202937;
  }

  .project-manager-search {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    flex: 1;
    padding: 8px 10px;
    border: 1px solid #2a3547;
    border-radius: 8px;
    background: #0c1017;
    color: #74839b;
  }

  .project-manager-search:focus-within {
    border-color: #3b82f6;
    box-shadow: 0 0 0 2px #3b82f622;
  }

  .project-manager-search input {
    min-width: 0;
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    color: #e2e8f0;
    font: inherit;
    font-size: 12px;
  }

  .project-manager-search input::placeholder {
    color: #617087;
  }

  .project-manager-search button {
    width: 22px;
    height: 22px;
  }

  .project-manager-browse,
  .project-manager-footer button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    flex: 0 0 auto;
    border: 1px solid #34445d;
    border-radius: 8px;
    background: #1a2433;
    color: #d4deeb;
    padding: 8px 11px;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }

  .project-manager-browse:hover:not(:disabled),
  .project-manager-footer button:hover {
    border-color: #3b82f6;
    background: #20334f;
    color: #f8fafc;
  }

  .project-manager-browse:disabled {
    opacity: 0.55;
    cursor: wait;
  }

  .project-manager-list {
    min-height: 120px;
    overflow-y: auto;
    padding: 8px 12px;
  }

  .project-manager-row {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 2px 0;
    padding-right: 5px;
    border: 1px solid transparent;
    border-radius: 9px;
  }

  .project-manager-row:hover,
  .project-manager-row.project-current {
    border-color: #273b58;
    background: #182131;
  }

  .project-manager-open {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    flex: 1;
    padding: 10px 9px;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .project-manager-row-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .project-manager-name,
  .project-manager-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .project-manager-name {
    color: #e2e8f0;
    font-size: 12px;
    font-weight: 600;
  }

  .project-manager-path {
    color: #7f8da3;
    font-size: 10px;
  }

  .project-manager-open-label {
    flex: 0 0 auto;
    color: #7f9bc1;
    font-size: 10px;
    opacity: 0;
    transition: opacity 0.12s;
  }

  .project-manager-row:hover .project-manager-open-label {
    opacity: 1;
  }

  .project-manager-rename {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 5px 7px 9px;
  }

  .project-manager-name-input {
    min-width: 0;
    flex: 1;
    padding: 7px 9px;
    border: 1px solid #3b82f6;
    border-radius: 6px;
    outline: none;
    background: #0c111a;
    color: #f1f5f9;
    font: inherit;
    font-size: 12px;
  }

  .project-manager-rename-trigger,
  .project-manager-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 32px;
    height: 32px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: #738198;
    cursor: pointer;
  }

  .project-manager-rename-trigger {
    opacity: 0;
    transition: color 0.12s, background 0.12s, opacity 0.12s;
  }

  .project-manager-row:hover .project-manager-rename-trigger,
  .project-manager-row:focus-within .project-manager-rename-trigger {
    opacity: 1;
  }

  .project-manager-rename-trigger:hover {
    border-color: #355078;
    background: #1c2b42;
    color: #93c5fd;
  }

  .project-manager-remove:hover {
    border-color: #713340;
    background: #3a1d27;
    color: #fb7185;
  }

  .project-manager-save-name {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 32px;
    height: 32px;
    border: 1px solid #16634c;
    border-radius: 7px;
    background: #123327;
    color: #6ee7b7;
    cursor: pointer;
  }

  .project-manager-save-name:hover {
    background: #174734;
    color: #a7f3d0;
  }

  .project-manager-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 9px;
    min-height: 220px;
    color: #617087;
    text-align: center;
  }

  .project-manager-empty strong {
    color: #cbd5e1;
    font-size: 13px;
  }

  .project-manager-empty span {
    font-size: 11px;
  }

  .project-manager-footer {
    border-top: 1px solid #252e3d;
    color: #75839a;
    font-size: 10px;
  }

  .project-manager-footer button {
    min-width: 84px;
  }

  @media (max-width: 560px) {
    .project-manager-backdrop {
      padding: 10px;
    }

    .project-manager-modal {
      max-height: 94vh;
      border-radius: 11px;
    }

    .project-manager-header,
    .project-manager-footer {
      padding: 13px;
    }

    .project-manager-toolbar {
      align-items: stretch;
      flex-direction: column;
      padding: 11px 13px;
    }

    .project-manager-browse {
      min-height: 34px;
    }

    .project-manager-list {
      padding: 6px 8px;
    }

    .project-manager-open {
      gap: 8px;
      padding: 9px 6px;
    }

    .project-manager-open-label {
      display: none;
    }

    .project-manager-rename-trigger {
      opacity: 1;
    }

    .project-manager-footer span {
      max-width: 200px;
      line-height: 1.4;
    }
  }
</style>
