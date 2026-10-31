<script lang="ts">
  import { onMount, tick } from "svelte";
  import SceneGraph from "../lib/SceneGraph.svelte";
  import SceneNodeIcon from "../lib/SceneNodeIcon.svelte";
  import SourceViewerModal from "../lib/SourceViewerModal.svelte";
  import GlossaryAiModal from "../lib/GlossaryAiModal.svelte";
  import { invoke, isTauri } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { 
    Folder,
    FolderOpen, 
    FileText, 
    CheckCircle2, 
    AlertCircle, 
    Sparkles, 
    Search, 
    Users, 
    Languages, 
    Clapperboard,
    GitBranch,
    Split,
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
    Import,
    BookOpen,
    Plus,
    Save,
    BookMarked,
    ExternalLink
  } from "lucide-svelte";

  // Стейт проекта на Svelte 5 (Runes)
  let gameDir = $state("");
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
  let showFileSkeleton = $state(false);
  let fileSkeletonTimer: ReturnType<typeof setTimeout> | null = null;
  let fileSkeletonShownAt = 0;
  let fileSelectSeq = 0;
  let projectScanSeq = 0;
  let glossaryLoadSeq = 0;
  let tableContainerEl = $state<HTMLElement | null>(null);
  let errorMsg = $state("");

  // Модальное окно просмотра исходного кода
  let showSourceViewer = $state(false);
  let sourceViewerPath = $state<string | null>(null);
  let sourceViewerTargetScene = $state<string | null>(null);
  let sourceViewerTargetLine = $state<number | null>(null);

  function openSourceViewer(path?: string | null, targetScene?: string | null, targetLine?: number | null) {
    sourceViewerPath = path || currentFile?.file_path || selectedFile?.path || null;
    sourceViewerTargetScene = targetScene || null;
    sourceViewerTargetLine = targetLine || null;
    showSourceViewer = true;
  }

  async function openCurrentInEditor(path?: string | null) {
    const target = path || currentFile?.file_path || selectedFile?.path;
    if (!target) return;
    try {
      await openPath(target);
    } catch {
      try {
        await invoke("open_in_system_editor", { filePath: target });
      } catch (e) {
        showToast("Не удалось открыть в редакторе: " + e, "error");
      }
    }
  }

  async function revealCurrentInFolder(path?: string | null) {
    const target = path || currentFile?.file_path || selectedFile?.path;
    if (!target) return;
    try {
      await revealItemInDir(target);
    } catch {
      try {
        await invoke("reveal_in_system_explorer", { filePath: target });
      } catch (e) {
        showToast("Не удалось открыть проводник: " + e, "error");
      }
    }
  }
  let filteredRecentProjects = $derived.by(() => {
    const query = projectManagerSearch.trim().toLocaleLowerCase();
    if (!query) return recentProjects;
    return recentProjects.filter(path =>
      path.toLocaleLowerCase().includes(query) || projectDisplayName(path).toLocaleLowerCase().includes(query)
    );
  });

  const RECENT_PROJECTS_KEY = "translator.recent-projects";
  const PROJECT_NAMES_KEY = "translator.project-names";
  const OPENING_PROJECT_KEY = "translator.opening-project";

  onMount(() => {
    restoreRecentProjects();
    // Проект, уронивший приложение, не должен ронять его при каждом запуске
    if (takeInterruptedProject() === gameDir) {
      errorMsg = `Прошлое открытие проекта «${projectDisplayName(gameDir)}» не завершилось, поэтому он не загружен автоматически. Чтобы попробовать снова, выберите его в списке проектов.`;
      gameDir = "";
    } else {
      void scanProject();
    }
    return watchWindowClose();
  });

  function takeInterruptedProject() {
    try {
      const path = localStorage.getItem(OPENING_PROJECT_KEY);
      localStorage.removeItem(OPENING_PROJECT_KEY);
      return path;
    } catch {
      return null;
    }
  }

  function markProjectOpening(path: string | null) {
    try {
      if (path) localStorage.setItem(OPENING_PROJECT_KEY, path);
      else localStorage.removeItem(OPENING_PROJECT_KEY);
    } catch {
      // Без отметки приложение просто загрузит этот проект при следующем запуске.
    }
  }

  function restoreRecentProjects() {
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
  }

  function watchWindowClose() {
    if (!isTauri()) return;
    const window = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let disposed = false;
    let closing = false;
    void window.onCloseRequested(async (event) => {
      if (!glossaryDirty && !translationDirty) return;
      event.preventDefault();
      if (closing) return;
      closing = true;
      try {
        const translationsSaved = await saveTranslations();
        const glossarySaved = !glossaryDirty || await saveGlossary();
        // Ошибка сохранения не должна запирать пользователя в приложении
        const canClose = (translationsSaved && glossarySaved) || await ask(
          "Не удалось сохранить изменения. Закрыть приложение без них?",
          { title: "Translator", kind: "warning", okLabel: "Закрыть", cancelLabel: "Остаться" }
        );
        if (canClose) await window.destroy();
      } finally {
        closing = false;
      }
    }).then((fn) => {
      // Компонент мог размонтироваться раньше, чем Tauri вернул функцию отписки.
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }

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

  // Папка данных зависит от пути к игре: после переезда или в новой версии её надо перенести
  async function transferProjectData(path: string) {
    if (!activeProjectPath) return;
    const confirmed = await ask(
      `Скопировать правки и глоссарий из «${projectDisplayName(path)}» в открытый проект? Правки, уже сделанные в нём, останутся.`,
      { title: "Перенос данных проекта", kind: "info" }
    );
    if (!confirmed) return;
    if (!(await saveTranslations())) return;
    if (glossaryDirty && !(await saveGlossary())) return;
    try {
      const message = await invoke<string>("transfer_project_data", {
        sourceDir: path,
        gameDir: activeProjectPath
      });
      showProjectManager = false;
      await scanProject();
      showToast(message, "success");
    } catch (e) {
      showToast(`Ошибка переноса данных: ${e}`, "error");
    }
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
    translation_key: string;
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
  let translationEdits = $state<Record<string, string>>({});
  let savingTranslations = $state(false);
  let translationSavePromise: Promise<boolean> | null = null;
  let translationDirty = $derived(Object.keys(translationEdits).length > 0);
  let activeProjectPath = $state("");

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
  let focusedSceneName = $state<string | null>(null);

  // Состояние навигации по сценам (единый сайдбар)
  let showSceneSidebar = $state(true);
  let sceneSearchQuery = $state("");

  // Сцены, доступные в табличном виде
  let tableScenes = $derived.by(() => {
    if (!currentFile) return [];
    return currentFile.scenes;
  });

  let currentScene = $derived.by(() => {
    if (!currentFile || selectedSceneId === "all") return null;
    return currentFile.scenes.find(s => s.id === selectedSceneId) ?? null;
  });

  // Фильтрация сцен для сайдбара
  let filteredScenes = $derived.by(() => {
    if (!currentFile) return [];
    const q = sceneSearchQuery.trim().toLowerCase();
    if (!q) return currentFile.scenes;
    return currentFile.scenes.filter(s => s.name.toLowerCase().includes(q));
  });

  function selectScene(scene: Scene) {
    selectedSceneId = scene.id;
    focusedSceneName = scene.name;
    if (viewMode === 'table') {
      if (tableContainerEl) {
        tableContainerEl.scrollTop = 0;
      }
    } else {
      const targetId = `scene-node-${scene.id}`;
      const el = document.getElementById(targetId);
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
      }
    }
  }

  function resetSceneSelection() {
    selectedSceneId = "all";
    focusedSceneName = null;
    if (tableContainerEl) {
      tableContainerEl.scrollTop = 0;
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
  let showGlossaryAiModal = $state(false);
  let glossaryTab = $state<'characters' | 'terms'>('characters');
  let glossary = $state<ProjectGlossary>({ characters: [], terms: [] });
  let loadingGlossary = $state(false);
  let savingGlossary = $state(false);
  let glossaryDirty = $state(false);
  let glossaryProjectPath = $state<string | null>(null);
  let selectedGlossaryChar = $state<CharacterGlossaryEntry | null>(null);
  let selectedGlossaryTerm = $state<TermGlossaryEntry | null>(null);

  function handleGlossaryAiApply(updatedChars: CharacterGlossaryEntry[], updatedTerms: TermGlossaryEntry[]) {
    const selectedAlias = selectedGlossaryChar?.alias;
    const selectedTermId = selectedGlossaryTerm?.id;
    glossary.characters = updatedChars;
    glossary.terms = updatedTerms;
    // Инспектор должен править записи нового списка, а не заменённые
    selectedGlossaryChar = glossary.characters.find(c => c.alias === selectedAlias) ?? glossary.characters[0] ?? null;
    selectedGlossaryTerm = glossary.terms.find(t => t.id === selectedTermId) ?? glossary.terms[0] ?? null;
    glossaryDirty = true;
    showToast("Глоссарий обновлён через нейросеть!", "info");
  }

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
      const targetId = `scene-node-${targetScene.id}`;
      const el = document.getElementById(targetId);
      if (el) {
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    }
  }

  // Всплывающее уведомление о переходах и событиях
  let toast = $state<{ text: string; type: 'info' | 'success' | 'warning' | 'error' } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

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
    graphNavigatingTarget = targetName;
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
      graphNavigatingTarget = null;
    }
  }

  let graphNavigating = $state(false);
  let graphNavigatingTarget = $state<string | null>(null);
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

  let allFilteredEntriesCount = $derived(
    visibleScenes.reduce((acc, s) => acc + filterSceneEntries(s.entries).length, 0)
  );

  let activeSceneStats = $derived.by(() => {
    if (!currentFile) return { total: 0, trans: 0, pct: 0 };
    if (selectedSceneId === 'all') {
      const total = currentFile.total_lines || 0;
      const trans = currentFile.scenes.reduce((acc, s) => acc + s.translated_lines, 0);
      const pct = total > 0 ? Math.round((trans / total) * 100) : 0;
      return { total, trans, pct };
    }
    if (currentScene) {
      const total = currentScene.total_lines || currentScene.entries.length;
      const trans = currentScene.translated_lines;
      const pct = total > 0 ? Math.round((trans / total) * 100) : 0;
      return { total, trans, pct };
    }
    return { total: 0, trans: 0, pct: 0 };
  });

  let totalPercent = $derived(
    project && project.total_lines > 0
      ? Math.round((project.total_translated / project.total_lines) * 100)
      : 0
  );

  function updateTranslationStats() {
    if (!currentFile) return;
    let translated = 0;
    for (const scene of currentFile.scenes) {
      scene.translated_lines = scene.entries.filter(entry => entry.translated_text?.trim()).length;
      translated += scene.translated_lines;
    }
    currentFile.translated_lines = translated;
    const summaryFile = project?.files.find(file => file.path === currentFile?.file_path);
    if (project && summaryFile) {
      project.total_translated += translated - summaryFile.translated_lines;
      summaryFile.translated_lines = translated;
    }
  }

  function recordTranslationEdit(entry: DialogueEntry, text: string) {
    entry.translated_text = text;
    translationEdits[entry.translation_key] = text;
    updateTranslationStats();
  }

  async function saveTranslations(): Promise<boolean> {
    if (!currentFile || !translationDirty) return true;
    if (translationSavePromise) {
      if (!(await translationSavePromise)) return false;
      return translationDirty ? saveTranslations() : true;
    }
    const filePath = currentFile.file_path;
    const snapshot = { ...translationEdits };
    savingTranslations = true;
    const task = invoke<void>("save_file_translations", {
      filePath,
      changes: Object.entries(snapshot).map(([key, text]) => ({ key, text }))
    }).then(() => {
      if (currentFile?.file_path === filePath) {
        for (const [key, text] of Object.entries(snapshot)) {
          if (translationEdits[key] === text) delete translationEdits[key];
        }
      }
      showToast("Перевод сохранён в данных приложения", "success");
      return !translationDirty;
    }).catch((e) => {
      showToast(`Ошибка сохранения перевода: ${e}`, "error");
      return false;
    }).finally(() => {
      savingTranslations = false;
      translationSavePromise = null;
    });
    translationSavePromise = task;
    return task;
  }

  // Сканировать проект
  async function scanProject() {
    if (!gameDir.trim()) return;
    if (!(await saveTranslations())) {
      if (activeProjectPath) gameDir = activeProjectPath;
      return;
    }
    if (glossaryDirty && !(await saveGlossary())) {
      if (glossaryProjectPath) gameDir = glossaryProjectPath;
      return;
    }
    const selectedPath = gameDir.trim();
    const seq = ++projectScanSeq;
    ++fileSelectSeq;
    ++glossaryLoadSeq;
    if (fileSkeletonTimer) clearTimeout(fileSkeletonTimer);
    fileSkeletonTimer = null;
    loadingFile = false;
    showFileSkeleton = false;
    loadingProject = true;
    errorMsg = "";
    project = null;
    selectedFile = null;
    currentFile = null;
    translationEdits = {};
    selectedEntry = null;
    selectedSceneId = "all";
    glossary = { characters: [], terms: [] };
    glossaryProjectPath = null;
    glossaryDirty = false;
    selectedGlossaryChar = null;
    selectedGlossaryTerm = null;
    markProjectOpening(selectedPath);
    try {
      const scanPath = await invoke<string>("resolve_game_directory", { path: selectedPath });
      if (seq !== projectScanSeq || gameDir.trim() !== selectedPath) return;
      const summary = await invoke<ProjectSummary>("scan_game_project", { gameDir: scanPath });
      if (seq !== projectScanSeq || gameDir.trim() !== selectedPath) return;
      project = summary;
      activeProjectPath = selectedPath;
      graphHistory = [];
      rememberProject(selectedPath);
      void loadGlossary(true);
      if (summary.files.length > 0) {
        // Микропауза для отрисовки сайдбара и статов перед загрузкой первого файла
        await new Promise(r => setTimeout(r, 20));
        if (seq !== projectScanSeq || gameDir.trim() !== selectedPath) return;
        void selectFile(summary.files[0]).finally(() => {
          if (seq === projectScanSeq) markProjectOpening(null);
        });
      } else {
        markProjectOpening(null);
      }
    } catch (e) {
      if (seq === projectScanSeq) {
        errorMsg = String(e);
        markProjectOpening(null);
      }
    } finally {
      if (seq === projectScanSeq) loadingProject = false;
    }
  }

  // Загрузить выбранный файл
  async function selectFile(file: TranslationFile) {
    if (!(await saveTranslations())) return false;
    const seq = ++fileSelectSeq;
    selectedFile = file;
    expandParentsForFile(file.name);
    scrollToActiveFile(file.path);
    loadingFile = true;
    errorMsg = "";

    // Сбрасываем предыдущий таймер скелетона
    if (fileSkeletonTimer) {
      clearTimeout(fileSkeletonTimer);
      fileSkeletonTimer = null;
    }
    showFileSkeleton = false;
    fileSkeletonShownAt = 0;

    // Скелетон отображаем ТОЛЬКО если загрузка затянулась (> 180 мс),
    // чтобы исключить раздражающее промигивание при быстром переключении
    fileSkeletonTimer = setTimeout(() => {
      if (seq === fileSelectSeq && loadingFile) {
        showFileSkeleton = true;
        fileSkeletonShownAt = Date.now();
      }
    }, 180);

    try {
      const parsed = await invoke<ParsedFile>("parse_file_entries", { filePath: file.path });
      if (seq !== fileSelectSeq) return false;

      // Если скелетон уже показан, задерживаем его минимум на 200 мс для плавности
      if (showFileSkeleton && fileSkeletonShownAt > 0) {
        const elapsed = Date.now() - fileSkeletonShownAt;
        const minDisplayTime = 200;
        if (elapsed < minDisplayTime) {
          await new Promise(r => setTimeout(r, minDisplayTime - elapsed));
        }
      }
      if (seq !== fileSelectSeq) return false;

      currentFile = parsed;
      translationEdits = {};
      updateTranslationStats();
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

      selectedEntry = currentFile.scenes.find(s => s.id === selectedSceneId)?.entries[0] ?? null;

      if (tableContainerEl) {
        tableContainerEl.scrollTop = 0;
      }

      return true;
    } catch (e) {
      if (seq === fileSelectSeq) {
        errorMsg = String(e);
      }
      return false;
    } finally {
      if (seq === fileSelectSeq) {
        if (fileSkeletonTimer) {
          clearTimeout(fileSkeletonTimer);
          fileSkeletonTimer = null;
        }
        showFileSkeleton = false;
        fileSkeletonShownAt = 0;
        loadingFile = false;
      }
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

  // 1. Сборка архива мода, который игрок кладёт в папку game/
  async function exportMod(outputPath: string | null) {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    if (!(await saveTranslations())) return;
    packingMod = true;
    try {
      const result = await invoke<{ message: string; output_path: string }>("export_mod", {
        gameDir: gameDir.trim(),
        outputPath
      });
      showToast(result.message, "success");
      await revealCurrentInFolder(result.output_path);
    } catch (e) {
      showToast(`Ошибка экспорта мода: ${e}`, "error");
    } finally {
      packingMod = false;
    }
  }

  // Обновить распакованные скрипты в кэше приложения, не меняя файлы игры.
  async function refreshScriptCache() {
    if (!gameDir.trim()) {
      showToast("Укажите путь к папке игры", "warning");
      return;
    }
    if (!(await saveTranslations())) return;
    if (glossaryDirty) {
      if (!(await saveGlossary())) return;
    }
    cleaningCache = true;
    try {
      await invoke<void>("clear_project_script_cache", { gameDir: gameDir.trim() });
      await scanProject();
      if (!project) throw new Error(errorMsg || "Повторное сканирование не удалось");
      showToast("Временный кэш скриптов обновлён. Файлы игры не изменены.", "success");
    } catch (e) {
      showToast(`Ошибка обновления кэша: ${e}`, "error");
    } finally {
      cleaningCache = false;
    }
  }

  // 3. Распаковка произвольного RPA архива
  async function executeUnpackRpa() {
    if (!unpackSourceRpa.trim()) {
      showToast("Укажите путь к .rpa архиву", "warning");
      return;
    }
    unpackingRpa = true;
    unpackResultMsg = "";
    try {
      const result = await invoke<{ count: number; target_dir: string }>("unpack_rpa", {
        rpaPath: unpackSourceRpa.trim(),
        targetDir: unpackTargetDir.trim(),
        gameDir: gameDir.trim() || null
      });
      unpackResultMsg = `Распаковано ${result.count} файлов в "${result.target_dir}"`;
      showToast(unpackResultMsg, "success");
    } catch (e) {
      unpackResultMsg = `Ошибка: ${e}`;
      showToast(unpackResultMsg, "error");
    } finally {
      unpackingRpa = false;
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
        outputRpyPath: decompileOutputRpyPath.trim() || null,
        gameDir: gameDir.trim() || null
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

  async function loadGlossary(silent = false): Promise<boolean> {
    if (!gameDir.trim()) return false;
    const path = gameDir.trim();
    if (glossaryDirty) {
      if (glossaryProjectPath === path) return true;
      if (!(await saveGlossary())) return false;
    }
    const seq = ++glossaryLoadSeq;
    const characters = project?.characters ?? null;
    if (!silent) loadingGlossary = true;
    try {
      const res = await invoke<ProjectGlossary>("get_project_glossary", {
        gameDir: path,
        scannedCharacters: characters
      });
      if (seq !== glossaryLoadSeq || gameDir.trim() !== path) return true;
      glossary = res;
      glossaryDirty = false;
      glossaryProjectPath = path;
      selectedGlossaryChar = glossary.characters[0] ?? null;
      selectedGlossaryTerm = glossary.terms[0] ?? null;
      return true;
    } catch (e) {
      if (seq === glossaryLoadSeq) showToast(`Ошибка загрузки глоссария: ${e}`, "error");
      return false;
    } finally {
      if (!silent && seq === glossaryLoadSeq) loadingGlossary = false;
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
    // Без загруженного глоссария правки в пустом окне заменили бы данные на диске
    if (!(await loadGlossary())) {
      showGlossaryModal = false;
      return;
    }
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

  async function saveGlossary(): Promise<boolean> {
    const path = glossaryProjectPath || gameDir.trim();
    if (!path) {
      showToast("Укажите путь к папке игры", "warning");
      return false;
    }
    if (savingGlossary) return false;
    const snapshot = JSON.stringify(glossary);
    savingGlossary = true;
    try {
      const msg = await invoke<string>("save_project_glossary", {
        gameDir: path,
        glossary: JSON.parse(snapshot) as ProjectGlossary
      });
      glossaryProjectPath = path;
      if (JSON.stringify(glossary) === snapshot) glossaryDirty = false;
      showToast(msg, "success");
      return !glossaryDirty;
    } catch (e) {
      showToast(`Ошибка сохранения глоссария: ${e}`, "error");
      return false;
    } finally {
      savingGlossary = false;
    }
  }

  async function closeGlossary() {
    if (glossaryDirty && !(await saveGlossary())) return;
    showGlossaryModal = false;
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
    selectedGlossaryTerm = glossary.terms[0];
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
    selectedGlossaryChar = glossary.characters.find(c => c.alias === alias) ?? null;
    newCharAlias = "";
    newCharName = "";
    newCharTranslation = "";
    newCharGender = "unknown";
    showAddChar = false;
    glossaryDirty = true;
    showToast(`Персонаж [${alias}] ${name} добавлен в глоссарий`, "success");
  }
</script>

<!-- Горячая клавиша Ctrl+S для сохранения текущих правок -->
<svelte:window onkeydown={(e) => {
  if (e.key === "Escape") {
    showProjectManager = false;
    showRecentProjects = false;
    showToolsMenu = false;
  }
  if ((e.ctrlKey || e.metaKey) && e.key === 's' && showGlossaryModal) {
    e.preventDefault();
    void saveGlossary();
  } else if ((e.ctrlKey || e.metaKey) && e.key === 's' && currentFile) {
    e.preventDefault();
    void saveTranslations();
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
                exportMod(null);
              }}
            >
              {#if packingMod}<Loader2 size={16} class="spin text-amber-400" />{:else}<Package size={16} class="text-amber-400" />{/if}
              <span><strong>Собрать мод</strong><small>Архив ru_lang.rpa для папки game</small></span>
            </button>
            <button
              class="tools-menu-item"
              disabled={cleaningCache || !gameDir}
              onclick={() => {
                showToolsMenu = false;
                refreshScriptCache();
              }}
            >
              {#if cleaningCache}<Loader2 size={16} class="spin text-rose-400" />{:else}<Trash2 size={16} class="text-rose-400" />{/if}
              <span><strong>Обновить кэш</strong><small>Перечитать архивы игры</small></span>
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
                <span
                  role="button"
                  tabindex="0"
                  class="btn-tree-code"
                  onclick={(e) => { e.stopPropagation(); openSourceViewer(f.path); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); openSourceViewer(f.path); } }}
                  title="Посмотреть исходный код файла"
                >
                  <FileCode size={11} />
                </span>
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
          <p>Выберите папку игры кнопкой «Обзор»</p>
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

          <!-- Действия с исходным кодом файла -->
          <div class="file-code-actions">
            <button 
              type="button"
              class="btn-file-code"
              onclick={() => openSourceViewer(currentFile?.file_path, focusedSceneName)}
              title="Открыть просмотрщик исходного кода файла .rpy (посмотреть реализацию сцен и переходов)"
            >
              <FileCode size={13} class="text-sky-400" />
              <span>Код файла</span>
            </button>

            <button 
              type="button"
              class="btn-file-action-quick"
              onclick={() => openCurrentInEditor(currentFile?.file_path)}
              title="Открыть файл во внешнем редакторе (VS Code, Notepad++ и т.д.)"
            >
              <ExternalLink size={12} />
              <span>Редактор</span>
            </button>

            <button 
              type="button"
              class="btn-file-action-quick"
              onclick={() => revealCurrentInFolder(currentFile?.file_path)}
              title="Показать файл в проводнике Windows"
            >
              <FolderOpen size={12} />
              <span>В папке</span>
            </button>
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

          <div class="graph-inline-pills">
            {#if totalChoicesCount > 0}
              <span class="graph-pill pill-purple">{totalChoicesCount} выборов (menu)</span>
            {/if}
            {#if totalEdgesCount > 0}
              <span class="graph-pill pill-amber">{totalEdgesCount} переходов (jump/branch)</span>
            {/if}
          </div>
        </div>

        {#snippet sceneSidebarPanel()}
          <aside class="scene-inner-sidebar">
            <div class="scene-sidebar-header">
              <button 
                type="button" 
                class="btn-sidebar-burger active"
                onclick={() => showSceneSidebar = false}
                title="Скрыть список сцен"
              >
                <Clapperboard size={14} />
              </button>
              <div class="scene-sidebar-title">
                <span>Сцены ({currentFile?.scenes.length ?? 0})</span>
              </div>
            </div>

            <!-- Поиск сцен внутри сайдбара -->
            <div class="scene-sidebar-search">
              <Search size={13} class="text-slate-500" />
              <input 
                type="text" 
                placeholder="Поиск сцены..." 
                bind:value={sceneSearchQuery}
                class="scene-sidebar-input"
              />
              {#if sceneSearchQuery}
                <button type="button" class="clear-search-pill" onclick={() => sceneSearchQuery = ""}>✕</button>
              {/if}
            </div>

            <!-- Список сцен -->
            <div class="scene-sidebar-list">
              {#if filteredScenes.length === 0}
                <div class="scene-sidebar-empty">
                  <span>Сцены не найдены</span>
                </div>
              {:else}
                {#each filteredScenes as scene, idx}
                  {@const pct = scene.total_lines > 0 ? Math.round((scene.translated_lines / scene.total_lines) * 100) : 0}
                  {@const isSelected = selectedSceneId === scene.id || (viewMode === 'scenes' && focusedSceneName === scene.name)}
                  <button 
                    type="button"
                    class="scene-sidebar-item" 
                    class:active={isSelected}
                    onclick={() => selectScene(scene)}
                    title="{scene.name} • {scene.translated_lines}/{scene.total_lines} строк ({pct}%)"
                  >
                    <div class="scene-sidebar-item-left">
                      <span class="scene-sidebar-seq">#{idx + 1}</span>
                      <SceneNodeIcon {scene} size={12} />
                      <span class="scene-sidebar-name">{scene.name}</span>
                    </div>
                    <div class="scene-sidebar-item-right">
                      {#if scene.choices && scene.choices.length > 0}
                        <span class="mini-tag tag-purple" title="Развилок: {scene.choices.length}">
                          <Zap size={10} />
                        </span>
                      {/if}
                      <span class="scene-sidebar-pct" class:pct-done={pct === 100}>{pct}%</span>
                    </div>
                  </button>
                {/each}
              {/if}
            </div>
          </aside>
        {/snippet}

        <!-- ОСНОВНОЙ ВИД -->
        {#if viewMode === 'scenes'}
          <!-- СЮЖЕТНЫЙ ГРАФ И СХЕМА ВЕТВЛЕНИЙ -->
          <div class="story-graph-wrapper">
            <!-- РАБОЧАЯ ОБЛАСТЬ ГРАФА С ВНУТРЕННИМ САЙДБАРОМ -->
            <div class="graph-workspace">
              {#if showSceneSidebar}
                {@render sceneSidebarPanel()}
              {/if}

              <!-- ОСНОВНАЯ ОБЛАСТЬ ГРАФА -->
              <div class="graph-canvas-container">
                {#key currentFile.file_path}
                  <SceneGraph scenes={currentFile.scenes} edges={currentFile.edges ?? []}
                    selectedId={selectedSceneId} busy={graphNavigating} busyTarget={graphNavigatingTarget}
                    showSidebar={showSceneSidebar} ontogglesidebar={() => showSceneSidebar = !showSceneSidebar}
                    onselect={(scene) => { selectedSceneId = scene.id; focusedSceneName = scene.name; }}
                    onnavigate={navigateOrFocusScene} onopen={openSceneInTable}
                    onviewcode={(scene) => openSourceViewer(currentFile?.file_path, scene.name)}
                    onback={graphHistory.length > 0 ? backToGraph : null} />
                {/key}
              </div>
            </div>
          </div>
        {:else}
          <!-- РЕЖИМ ТАБЛИЦЫ ДИАЛОГОВ -->
          <div class="table-workspace">
            {#if showSceneSidebar}
              {@render sceneSidebarPanel()}
            {/if}

            <!-- ОСНОВНАЯ ОБЛАСТЬ ТАБЛИЦЫ С ТУЛБАРОМ -->
            <div class="table-main-area">
              <div class="table-subtoolbar">
                <div class="table-subtoolbar-left">
                  {#if !showSceneSidebar}
                    <button
                      type="button"
                      class="btn-sidebar-burger"
                      onclick={() => showSceneSidebar = true}
                      title="Показать список сцен"
                    >
                      <Clapperboard size={14} />
                    </button>
                    <div class="subtoolbar-divider"></div>
                  {/if}

                  {#if currentScene}
                    <div class="table-scene-heading">
                      <SceneNodeIcon scene={currentScene} size={13} />
                      <span class="focus-title-badge">Сцена:</span>
                      <strong class="focus-scene-name" title={currentScene.name}>{currentScene.name}</strong>
                      <button
                        type="button"
                        class="btn-toolbar-code"
                        onclick={() => openSourceViewer(currentFile?.file_path, currentScene?.name)}
                        title="Посмотреть исходный код сцены в файле (.rpy)"
                      >
                        <FileCode size={11} />
                        <span>Код</span>
                      </button>
                    </div>
                  {:else}
                    <div class="table-scene-heading">
                      <span class="focus-title-badge">Все сцены:</span>
                      <strong class="focus-scene-name">({currentFile.scenes.length})</strong>
                      <button
                        type="button"
                        class="btn-toolbar-code"
                        onclick={() => openSourceViewer(currentFile?.file_path)}
                        title="Посмотреть исходный код файла (.rpy)"
                      >
                        <FileCode size={11} />
                        <span>Код</span>
                      </button>
                    </div>
                  {/if}

                  <div class="subtoolbar-divider"></div>

                  <div class="table-subtoolbar-progress">
                    <span class="scene-trans-stat">{activeSceneStats.trans}/{activeSceneStats.total} переведено ({activeSceneStats.pct}%)</span>
                    <div class="scene-mini-track" title="Переведено {activeSceneStats.pct}% ({activeSceneStats.trans} из {activeSceneStats.total})">
                      <div class="scene-mini-fill" style="width: {activeSceneStats.pct}%"></div>
                    </div>
                  </div>
                </div>

                <div class="table-subtoolbar-right">
                  <button
                    type="button"
                    class="btn-toolbar-code"
                    onclick={() => void saveTranslations()}
                    disabled={!translationDirty || savingTranslations}
                    title="Сохранить правки перевода в данных приложения (Ctrl+S)"
                  >
                    {#if savingTranslations}<Loader2 size={12} class="spin" />{:else}<Save size={12} />{/if}
                    <span>{translationDirty ? 'Сохранить перевод *' : 'Перевод сохранён'}</span>
                  </button>
                  <div class="search-box">
                    <Search size={13} class="text-slate-400" />
                    <input 
                      type="search" 
                      bind:value={searchQuery} 
                      placeholder="Поиск строк..." 
                    />
                    {#if searchQuery}
                      <button type="button" class="clear-search-pill" onclick={() => searchQuery = ""}>✕</button>
                    {/if}
                  </div>

                  <div class="filter-pills">
                    <button 
                      type="button"
                      class:active={filterStatus === 'all'} 
                      onclick={() => filterStatus = 'all'}
                    >Все</button>
                    <button 
                      type="button"
                      class:active={filterStatus === 'untranslated'} 
                      onclick={() => filterStatus = 'untranslated'}
                    >Без перевода</button>
                    <button 
                      type="button"
                      class:active={filterStatus === 'translated'} 
                      onclick={() => filterStatus = 'translated'}
                    >Переведенные</button>
                  </div>
                </div>
              </div>

              <div class="table-container" bind:this={tableContainerEl}>
            {#if showFileSkeleton}
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
                    {#if selectedSceneId === 'all'}
                      <!-- РАЗДЕЛИТЕЛЬ СЦЕНЫ (ТОЛЬКО ПРИ ПРОСМОТРЕ ВСЕХ СЦЕН) -->
                      <div class="scene-section-divider">
                        <div class="scene-section-left">
                          <SceneNodeIcon {scene} size={13} />
                          <span class="focus-title-badge">Сцена:</span>
                          <strong class="focus-scene-name">{scene.name}</strong>
                          <button
                            type="button"
                            class="btn-toolbar-code"
                            onclick={(e) => { e.stopPropagation(); openSourceViewer(currentFile?.file_path, scene.name); }}
                            title="Посмотреть исходный код сцены в файле (.rpy)"
                          >
                            <FileCode size={11} />
                            <span>Код</span>
                          </button>
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
                    {/if}

                    <table class="grid-table">
                      <colgroup>
                        <col class="col-w-line" />
                        <col class="col-w-speaker" />
                        <col class="col-w-source" />
                        <col class="col-w-trans" />
                        <col class="col-w-status" />
                      </colgroup>
                      <thead>
                        <tr>
                          <th class="th-line">#</th>
                          <th class="th-speaker">Персонаж</th>
                          <th class="th-source">Оригинал (EN)</th>
                          <th class="th-trans">Перевод (RU)</th>
                          <th class="th-status">Статус</th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each sceneEntries as entry}
                          {@const spk = getSpeakerInfo(entry.speaker, entry.speaker_alias)}
                          <tr 
                            class:selected={selectedEntry?.id === entry.id}
                            onclick={() => selectedEntry = entry}
                          >
                            <td class="col-line">
                              <button
                                type="button"
                                class="btn-col-line"
                                onclick={(e) => { e.stopPropagation(); openSourceViewer(currentFile?.file_path, scene.name, entry.line_number); }}
                                title="Посмотреть строку #{entry.line_number} в исходном коде .rpy"
                              >
                                #{entry.line_number}
                              </button>
                            </td>
                            <td class="col-speaker">
                              {#if spk.isNarrator}
                                <span class="narrator-badge">Автор</span>
                              {:else if spk.isUi}
                                <span class="char-chip char-chip-ui" title="Интерфейсные строки">
                                  <span class="char-alias">[UI]</span>
                                  <span class="char-name">Интерфейс</span>
                                </span>
                              {:else}
                                <button 
                                  type="button"
                                  class="char-chip char-chip-interactive" 
                                  onclick={(e) => { e.stopPropagation(); openGlossary(spk.alias || spk.name); }}
                                  title="Открыть персонажа в глоссарии: {spk.name} (код: {spk.alias || '—'})"
                                >
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
                                </button>
                              {/if}
                            </td>
                            <td class="col-text en-text">{entry.original_text}</td>
                            <td class="col-text ru-text">
                              <div class="inline-trans-field">
                                <textarea
                                  class="table-trans-textarea"
                                  bind:value={entry.translated_text}
                                  oninput={(event) => recordTranslationEdit(entry, event.currentTarget.value)}
                                  disabled={loadingFile || loadingProject || savingTranslations}
                                  placeholder="Введите перевод..."
                                  rows="2"
                                  onfocus={() => selectedEntry = entry}
                                  onclick={(e) => { e.stopPropagation(); selectedEntry = entry; }}
                                ></textarea>
                              </div>
                            </td>
                            <td class="col-status">
                              {#if entry.translated_text && entry.translated_text.trim() !== ""}
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
                    {@const sceneTotal = scene.entries.length || scene.total_lines}
                    {@const sceneTrans = scene.entries.filter(e => e.translated_text && e.translated_text.trim() !== '').length}
                    {@const sceneUntrans = sceneTotal - sceneTrans}
                    <div class="empty-scene-card">
                      {#if sceneTotal === 0}
                        {#if (scene.choices?.length ?? 0) > 0}
                          <div class="empty-scene-icon-wrap icon-wrap-purple">
                            <Split size={28} class="text-purple-400" />
                          </div>
                        {:else}
                          <div class="empty-scene-icon-wrap icon-wrap-indigo">
                            <GitBranch size={28} class="text-indigo-400" />
                          </div>
                        {/if}
                      {:else if searchQuery}
                        <div class="empty-scene-icon-wrap icon-wrap-slate">
                          <Search size={28} class="text-slate-400" />
                        </div>
                      {:else if filterStatus === 'translated' && sceneTrans === 0}
                        <div class="empty-scene-icon-wrap icon-wrap-amber">
                          <Languages size={28} class="text-amber-400" />
                        </div>
                      {:else if filterStatus === 'untranslated' && sceneUntrans === 0}
                        <div class="empty-scene-icon-wrap icon-wrap-emerald">
                          <CheckCircle2 size={28} class="text-emerald-400" />
                        </div>
                      {:else}
                        <div class="empty-scene-icon-wrap icon-wrap-sky">
                          <Clapperboard size={28} class="text-sky-400" />
                        </div>
                      {/if}
                      <div class="empty-scene-info">
                        {#if sceneTotal === 0}
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
                          <div class="empty-scene-actions">
                            <button type="button" class="btn-return-graph" onclick={() => viewMode = 'scenes'}>
                              <GitBranch size={14} />
                              <span>Открыть переходы этой сцены в графе →</span>
                            </button>
                            <button type="button" class="btn-return-graph" onclick={() => openSourceViewer(currentFile?.file_path, scene.name)}>
                              <FileCode size={14} class="text-sky-400" />
                              <span>Посмотреть код развилки в .rpy</span>
                            </button>
                          </div>
                        {:else if searchQuery}
                          <h3>В сцене «{scene.name}» ничего не найдено по запросу «{searchQuery}»</h3>
                          <p>
                            {#if filterStatus === 'translated'}
                              Среди переведённых строк сцены (переведено: {sceneTrans} из {sceneTotal}) совпадений не найдено.
                            {:else if filterStatus === 'untranslated'}
                              Среди непереведённых строк сцены (без перевода: {sceneUntrans} из {sceneTotal}) совпадений не найдено.
                            {:else}
                              Всего строк диалогов в этой сцене: {sceneTotal}. Попробуйте изменить поисковый запрос или очистить поиск.
                            {/if}
                          </p>
                          <div class="empty-scene-actions">
                            <button type="button" class="btn-return-graph" onclick={() => searchQuery = ''}>
                              <span>Очистить поиск</span>
                            </button>
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                              <span>Показать все строки сцены</span>
                            </button>
                          </div>
                        {:else if filterStatus === 'translated' && sceneTrans === 0}
                          <h3>В сцене «{scene.name}» пока нет переведённых строк</h3>
                          <p>
                            Всего строк диалогов в этой сцене: {sceneTotal}.
                            Выберите фильтр «Все» или «Без перевода», чтобы приступить к переводу диалогов.
                          </p>
                          <div class="empty-scene-actions">
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                              <span>Показать все строки сцены</span>
                            </button>
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'untranslated'; searchQuery = ''; }}>
                              <span>Перейти к непереведённым ({sceneUntrans})</span>
                            </button>
                            <button type="button" class="btn-return-graph" onclick={() => openSourceViewer(currentFile?.file_path, scene.name)}>
                              <FileCode size={14} class="text-sky-400" />
                              <span>Посмотреть исходный код сцены (.rpy)</span>
                            </button>
                          </div>
                        {:else if filterStatus === 'untranslated' && sceneUntrans === 0}
                          <h3>Все строки в сцене «{scene.name}» уже переведены! 🎉</h3>
                          <p>
                            Все {sceneTotal} строк диалогов в этой сцене переведены (100%).
                            Переключитесь на «Все» или «Переведенные» для проверки.
                          </p>
                          <div class="empty-scene-actions">
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                              <span>Показать все строки сцены</span>
                            </button>
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'translated'; searchQuery = ''; }}>
                              <span>Показать переведённые ({sceneTrans})</span>
                            </button>
                          </div>
                        {:else}
                          <h3>В сцене «{scene.name}» нет строк, подходящих под выбранный фильтр</h3>
                          <div class="empty-scene-actions">
                            <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                              <span>Показать все строки сцены</span>
                            </button>
                          </div>
                        {/if}
                      </div>
                    </div>
                  {/if}
                {/each}

                {#if selectedSceneId === 'all' && visibleScenes.length > 0 && visibleScenes.every(s => filterSceneEntries(s.entries).length === 0)}
                  {@const fileTotal = currentFile ? currentFile.scenes.reduce((acc, s) => acc + (s.entries.length || s.total_lines), 0) : 0}
                  {@const fileTrans = currentFile ? currentFile.scenes.reduce((acc, s) => acc + s.entries.filter(e => e.translated_text && e.translated_text.trim() !== '').length, 0) : 0}
                  {@const fileUntrans = fileTotal - fileTrans}
                  <div class="empty-scene-card">
                    {#if fileTotal === 0}
                      <div class="empty-scene-icon-wrap icon-wrap-slate">
                        <FileText size={28} class="text-slate-400" />
                      </div>
                    {:else if searchQuery}
                      <div class="empty-scene-icon-wrap icon-wrap-slate">
                        <Search size={28} class="text-slate-400" />
                      </div>
                    {:else if filterStatus === 'translated' && fileTrans === 0}
                      <div class="empty-scene-icon-wrap icon-wrap-amber">
                        <Languages size={28} class="text-amber-400" />
                      </div>
                    {:else if filterStatus === 'untranslated' && fileUntrans === 0}
                      <div class="empty-scene-icon-wrap icon-wrap-emerald">
                        <CheckCircle2 size={28} class="text-emerald-400" />
                      </div>
                    {:else}
                      <div class="empty-scene-icon-wrap icon-wrap-sky">
                        <FileText size={28} class="text-sky-400" />
                      </div>
                    {/if}
                    <div class="empty-scene-info">
                      {#if fileTotal === 0}
                        <h3>В файле «{currentFile?.file_name}» нет диалоговых строк</h3>
                        <p>Этот файл содержит только метки навигации, меню выбора или код сценария без текста диалогов.</p>
                        <div class="empty-scene-actions">
                          <button type="button" class="btn-return-graph" onclick={() => openSourceViewer(currentFile?.file_path)}>
                            <FileCode size={14} class="text-sky-400" />
                            <span>Посмотреть исходный код файла (.rpy)</span>
                          </button>
                        </div>
                      {:else if searchQuery}
                        <h3>В файле «{currentFile?.file_name}» ничего не найдено по запросу «{searchQuery}»</h3>
                        <p>
                          {#if filterStatus === 'translated'}
                            Среди переведённых строк файла (переведено: {fileTrans} из {fileTotal}) совпадений не найдено.
                          {:else if filterStatus === 'untranslated'}
                            Среди непереведённых строк файла (без перевода: {fileUntrans} из {fileTotal}) совпадений не найдено.
                          {:else}
                            Попробуйте изменить поисковый запрос или очистить строку поиска.
                          {/if}
                        </p>
                        <div class="empty-scene-actions">
                          <button type="button" class="btn-return-graph" onclick={() => searchQuery = ''}>
                            <span>Очистить поиск</span>
                          </button>
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                            <span>Показать все строки файла</span>
                          </button>
                        </div>
                      {:else if filterStatus === 'translated' && fileTrans === 0}
                        <h3>В файле «{currentFile?.file_name}» пока нет переведённых строк</h3>
                        <p>
                          Всего строк диалогов в файле: {fileTotal}.
                          Выберите вкладку «Все» или «Без перевода», чтобы приступить к переводу диалогов.
                        </p>
                        <div class="empty-scene-actions">
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                            <span>Показать все строки файла</span>
                          </button>
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'untranslated'; searchQuery = ''; }}>
                            <span>Перейти к непереведённым ({fileUntrans})</span>
                          </button>
                          <button type="button" class="btn-return-graph" onclick={() => openSourceViewer(currentFile?.file_path)}>
                            <FileCode size={14} class="text-sky-400" />
                            <span>Посмотреть исходный код файла (.rpy)</span>
                          </button>
                        </div>
                      {:else if filterStatus === 'untranslated' && fileUntrans === 0}
                        <h3>Все строки в файле «{currentFile?.file_name}» уже переведены! 🎉</h3>
                        <p>Все {fileTotal} строк диалогов в файле переведены (100%). Переключитесь на вкладку «Все» или «Переведенные» для проверки.</p>
                        <div class="empty-scene-actions">
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                            <span>Показать все строки файла</span>
                          </button>
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'translated'; searchQuery = ''; }}>
                            <span>Показать переведённые ({fileTrans})</span>
                          </button>
                        </div>
                      {:else}
                        <h3>В файле нет диалоговых строк для отображения</h3>
                        <div class="empty-scene-actions">
                          <button type="button" class="btn-return-graph" onclick={() => { filterStatus = 'all'; searchQuery = ''; }}>
                            <span>Показать все строки файла</span>
                          </button>
                        </div>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        </div>
      </div>
      {/if}
      {:else}
        <div class="empty-workspace">
          <Languages size={48} class="empty-icon" />
          <h2>Проект не загружен</h2>
          <p>Выберите папку с установленной новеллой кнопкой «Обзор» сверху</p>
        </div>
      {/if}
    </main>
  </div>

  <!-- МОДАЛЬНОЕ ОКНО RPA & RPYC STUDIO -->
  {#if showToolsModal}
    <div 
      class="modal-backdrop" 
      onclick={(e) => { if (e.target === e.currentTarget) showToolsModal = false; }}
      onkeydown={(e) => { if (e.key === 'Escape') showToolsModal = false; }}
      role="dialog"
      aria-modal="true"
      aria-label="Студия архивов RPA и байткода RPYC"
      tabindex="-1"
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
              <!-- Карточка 1: Экспорт мода -->
              <div class="tool-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <Package size={16} class="text-amber-400" />
                    <h4>Экспорт мода перевода</h4>
                  </div>
                  <span class="tool-badge">Для игроков</span>
                </div>
                <p class="tool-desc">
                  Собирает <code>tl/russian</code> и сохранённые правки в архив <code>ru_lang.rpa</code>, который кладут в папку <code>game/</code>. Скрипты компилируются прямо в приложении под версию Ren'Py этой игры, сама игра при этом не запускается и не меняется.
                </p>

                <div class="tool-form-group">
                  <span class="tool-field-label">Папка источника (tl/russian)</span>
                  <div class="tool-readonly-path">{gameDir ? `${gameDir}\\tl\\russian` : 'Папка игры не выбрана'}</div>
                </div>

                <div class="tool-form-group">
                  <span class="tool-field-label">Путь к файлу .rpa (по умолчанию в данных приложения; вне папки игры)</span>
                  <input 
                    type="text" 
                    bind:value={packModCustomPath} 
                    placeholder="Оставьте пустым для сохранения в кэш приложения"
                    class="tool-input"
                  />
                </div>

                <button 
                  class="btn-tool-action btn-pack-action" 
                  onclick={() => exportMod(packModCustomPath.trim() || null)}
                  disabled={packingMod || !gameDir}
                >
                  {#if packingMod}
                    <Loader2 size={15} class="spin" />
                    <span>Компиляция и сборка мода...</span>
                  {:else}
                    <Download size={15} />
                    <span>Собрать мод</span>
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
                  <span class="tool-field-label">Куда распаковать (по умолчанию в кэш приложения; вне папки игры)</span>
                  <input 
                    type="text" 
                    bind:value={unpackTargetDir} 
                    placeholder="Оставьте пустым для кэша приложения"
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
                    <span class="tool-field-label">Сохранить как .rpy вне папки игры (опционально)</span>
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

              <!-- Карточка 2: Обновление кэша приложения -->
              <div class="tool-card full-width-card">
                <div class="tool-card-head">
                  <div class="tool-head-title">
                    <Trash2 size={16} class="text-rose-400" />
                    <h4>Обновление временного кэша скриптов</h4>
                  </div>
                  <span class="tool-badge tool-badge-rose">Cache Flush</span>
                </div>
                <p class="tool-desc">
                  Удаляет только распакованные скрипты в данных Translator и заново считывает архивы. Папка игры остаётся без изменений.
                </p>

                <button 
                  class="btn-tool-action btn-clean-action" 
                  onclick={refreshScriptCache}
                  disabled={cleaningCache || !gameDir}
                >
                  {#if cleaningCache}
                    <Loader2 size={15} class="spin" />
                    <span>Обновление кэша...</span>
                  {:else}
                    <Trash2 size={15} />
                    <span>Обновить временный кэш</span>
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
      onclick={(e) => { if (e.target === e.currentTarget) void closeGlossary(); }}
      onkeydown={(e) => { if (e.key === 'Escape') void closeGlossary(); }}
      role="dialog"
      aria-modal="true"
      aria-label="Глоссарий проекта"
      tabindex="-1"
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
              class="btn-modal-ai"
              onclick={() => showGlossaryAiModal = true}
              title="Автозаполнение через нейросеть (LM Studio / Ollama)"
            >
              <Sparkles size={14} class="text-violet-400" />
              <span>Заполнить через ИИ</span>
            </button>
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
            <button class="btn-close-modal" onclick={closeGlossary} aria-label="Закрыть">
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
                    <label class="edit-label" for="character-translation">
                      <span>Русский перевод имени:</span>
                    </label>
                    <input 
                      id="character-translation"
                      type="text" 
                      bind:value={selectedGlossaryChar.translated_name} 
                      oninput={() => glossaryDirty = true}
                      placeholder={selectedGlossaryChar.original_name}
                      class="sidebar-input sidebar-input-primary"
                    />
                    <span class="edit-hint">Отображается в диалогах, сюжетном графе и инспекторе</span>
                  </div>

                  <div class="edit-group">
                    <div class="edit-label" id="character-gender-label">
                      <span>Грамматический род (для согласования окончаний):</span>
                    </div>
                    <div class="gender-cards-grid" role="group" aria-labelledby="character-gender-label">
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
                    <label class="edit-label" for="character-notes">
                      <span>Контекст персонажа / Стиль речи:</span>
                    </label>
                    <textarea 
                      id="character-notes"
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
                    <label class="edit-label" for="term-source">
                      <span>Исходный термин (EN):</span>
                    </label>
                    <input 
                      id="term-source"
                      type="text" 
                      bind:value={selectedGlossaryTerm.source_term} 
                      oninput={() => glossaryDirty = true}
                      class="sidebar-input"
                    />
                  </div>

                  <div class="edit-group">
                    <label class="edit-label" for="term-translation">
                      <span>Русский перевод (RU):</span>
                    </label>
                    <input 
                      id="term-translation"
                      type="text" 
                      bind:value={selectedGlossaryTerm.translated_term} 
                      oninput={() => glossaryDirty = true}
                      placeholder="Перевод на русский..."
                      class="sidebar-input sidebar-input-primary"
                    />
                  </div>

                  <div class="edit-group">
                    <label class="edit-label" for="term-category">
                      <span>Категория термина:</span>
                    </label>
                    <select 
                      id="term-category"
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
                    <label class="edit-label" for="term-notes">
                      <span>Заметки / Контекст:</span>
                    </label>
                    <textarea 
                      id="term-notes"
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
              <span class="text-slate-400">Все изменения сохранены в данных приложения</span>
            {/if}
          </div>
          <div class="footer-actions">
            <button class="btn-secondary" onclick={closeGlossary}>Закрыть</button>
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
                  {#if activeProjectPath && projectKey(activeProjectPath) !== projectKey(path)}
                    <button
                      class="project-manager-rename-trigger"
                      title="Скопировать правки и глоссарий в открытый проект"
                      aria-label={`Скопировать данные ${projectDisplayName(path)} в открытый проект`}
                      disabled={loadingProject}
                      onclick={() => transferProjectData(path)}
                    >
                      <Import size={14} />
                    </button>
                  {/if}
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

  <!-- МОДАЛЬНОЕ ОКНО ПРОСМОТРА ИСХОДНОГО КОДА -->
  <SourceViewerModal
    show={showSourceViewer}
    filePath={sourceViewerPath}
    targetSceneName={sourceViewerTargetScene}
    targetLineNumber={sourceViewerTargetLine}
    scenes={currentFile ? currentFile.scenes.map(s => ({ id: s.id, name: s.name })) : []}
    onclose={() => showSourceViewer = false}
  />

  <!-- МОДАЛЬНОЕ ОКНО ИИ-АВТОЗАПОЛНЕНИЯ ГЛОССАРИЯ -->
  <GlossaryAiModal
    show={showGlossaryAiModal}
    gameDir={gameDir}
    characters={glossary.characters}
    terms={glossary.terms}
    initialTab={glossaryTab}
    onapply={handleGlossaryAiApply}
    onclose={() => showGlossaryAiModal = false}
  />

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

  .btn-browse {
    flex: 0 0 auto;
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
    .dir-bar .btn-browse span {
      display: none;
    }

    .btn-browse {
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
    padding: 0 16px;
    height: 44px;
    min-height: 44px;
    max-height: 44px;
    box-sizing: border-box;
    background: #12151b;
    border-bottom: 1px solid #1e232d;
    flex-wrap: nowrap;
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

  .file-code-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .btn-file-code {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 26px;
    min-height: 26px;
    max-height: 26px;
    padding: 0 9px;
    border-radius: 6px;
    background: #0c1a2e;
    border: 1px solid #0284c7;
    color: #f1f5f9;
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 600;
    line-height: 1;
    box-sizing: border-box;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-file-code:hover {
    background: #0369a1;
    border-color: #38bdf8;
    color: #ffffff;
  }

  .btn-file-action-quick {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 26px;
    min-height: 26px;
    max-height: 26px;
    padding: 0 9px;
    border-radius: 6px;
    background: #141a26;
    border: 1px solid #283446;
    color: #94a3b8;
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 500;
    line-height: 1;
    box-sizing: border-box;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-file-action-quick:hover {
    background: #1e293b;
    color: #f1f5f9;
    border-color: #475569;
  }

  .btn-inspect-code-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: transparent;
    border: none;
    padding: 2px 4px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 11.5px;
    color: #38bdf8;
    font-weight: 500;
    transition: background 0.15s;
  }

  .btn-inspect-code-link:hover {
    background: rgba(2, 132, 199, 0.2);
    text-decoration: underline;
  }

  .btn-tree-code {
    display: none;
    align-items: center;
    justify-content: center;
    padding: 2px;
    background: transparent;
    border: none;
    color: #64748b;
    border-radius: 3px;
    cursor: pointer;
    transition: all 0.15s;
  }

  .tree-file-row:hover .btn-tree-code {
    display: inline-flex;
  }

  .btn-tree-code:hover {
    color: #38bdf8;
    background: #1e293b;
  }

  .empty-scene-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin-top: 6px;
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
    padding: 0 9px;
    height: 26px;
    box-sizing: border-box;
    width: 220px;
    flex-shrink: 0;
  }

  .search-box input {
    background: transparent;
    border: none;
    color: #e2e8f0;
    font-size: 11.5px;
    flex: 1;
    min-width: 0;
    outline: none;
    padding: 0;
    font-family: inherit;
  }

  .filter-pills {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .filter-pills button {
    background: transparent;
    border: 1px solid #232935;
    color: #94a3b8;
    height: 26px;
    padding: 0 9px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 5px;
    font-size: 11.5px;
    line-height: 1;
    cursor: pointer;
  }

  .filter-pills button.active {
    background: #1e293b;
    color: #38bdf8;
    border-color: #0284c7;
  }

  /* TABLE WORKSPACE & INNER SIDEBAR */
  .table-workspace {
    flex: 1;
    display: flex;
    flex-direction: row;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }

  .table-main-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }

  .table-subtoolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 42px;
    min-height: 42px;
    max-height: 42px;
    box-sizing: border-box;
    padding: 0 14px;
    background: #0e1219;
    border-bottom: 1px solid #1a202c;
  }

  .table-subtoolbar-left {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
  }

  .table-subtoolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .table-scene-heading {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .subtoolbar-divider {
    width: 1px;
    height: 18px;
    background: #1e2636;
    margin: 0 4px;
    flex-shrink: 0;
  }

  .table-subtoolbar-progress {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: #94a3b8;
  }

  .scene-trans-stat {
    font-size: 11px;
    color: #94a3b8;
    white-space: nowrap;
  }

  .btn-sidebar-burger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    min-width: 24px;
    min-height: 24px;
    max-width: 24px;
    max-height: 24px;
    box-sizing: border-box;
    padding: 0;
    margin: 0;
    background: #141923;
    border: 1px solid #1e2636;
    border-radius: 4px;
    color: #94a3b8;
    cursor: pointer;
    transition: all 0.15s ease;
    flex-shrink: 0;
  }

  .btn-sidebar-burger :global(svg) {
    display: block;
    flex-shrink: 0;
  }

  .btn-sidebar-burger:hover {
    background: #1e293b;
    border-color: #38bdf8;
    color: #f1f5f9;
  }

  .btn-sidebar-burger.active {
    background: #152236;
    border-color: #0284c7;
    color: #38bdf8;
  }

  /* UNIFIED SCENE SIDEBAR */
  .scene-inner-sidebar {
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

  .scene-sidebar-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 14px;
    height: 42px;
    min-height: 42px;
    max-height: 42px;
    box-sizing: border-box;
    border-bottom: 1px solid #1a202c;
    background: #0e1219;
  }

  .scene-sidebar-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #94a3b8;
    text-transform: uppercase;
  }

  .scene-sidebar-search {
    position: relative;
    display: flex;
    align-items: center;
    padding: 8px 10px;
    border-bottom: 1px solid #171d28;
    background: #0b0e14;
    gap: 6px;
  }

  .scene-sidebar-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font: inherit;
    font-size: 12px;
  }

  .scene-sidebar-input::placeholder {
    color: #4b5563;
  }

  .scene-sidebar-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .scene-sidebar-empty {
    padding: 24px 12px;
    text-align: center;
    color: #4b5563;
    font-size: 12px;
  }

  .scene-sidebar-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    height: 32px;
    min-height: 32px;
    max-height: 32px;
    box-sizing: border-box;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: #94a3b8;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .scene-sidebar-item:hover {
    background: #141a26;
    border-color: #202b3d;
    color: #f1f5f9;
  }

  .scene-sidebar-item.active {
    background: #14243b;
    border-color: #0284c7;
    color: #38bdf8;
    font-weight: 600;
  }

  .scene-sidebar-item-left {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    flex: 1;
    height: 100%;
  }

  .scene-sidebar-seq {
    font-family: monospace;
    font-size: 10px;
    color: #4b5563;
    flex-shrink: 0;
    line-height: 1;
  }

  .scene-sidebar-name {
    font-size: 12px;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .scene-sidebar-item-right {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    height: 100%;
  }

  .mini-tag {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    box-sizing: border-box;
    border-radius: 3px;
    line-height: 1;
    flex-shrink: 0;
  }

  .mini-tag.tag-purple {
    background: rgba(168, 85, 247, 0.15);
    border: 1px solid rgba(168, 85, 247, 0.35);
    color: #c084fc;
  }

  .mini-tag :global(svg) {
    display: block;
    flex-shrink: 0;
  }

  .scene-sidebar-pct {
    font-size: 10px;
    font-weight: 600;
    font-family: monospace;
    color: #64748b;
    padding: 0 5px;
    height: 18px;
    min-height: 18px;
    max-height: 18px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
    background: #090c12;
    border-radius: 4px;
    border: 1px solid #1a2230;
  }

  .scene-sidebar-pct.pct-done {
    background: rgba(16, 185, 129, 0.12);
    border-color: rgba(16, 185, 129, 0.3);
    color: #34d399;
  }

  /* INLINE TABLE EDITING & ACTIONS */
  .btn-col-line {
    background: transparent;
    border: none;
    color: #64748b;
    font-family: monospace;
    font-size: 11px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    transition: all 0.12s;
  }

  .btn-col-line:hover {
    color: #38bdf8;
    background: #172033;
  }

  .inline-trans-field {
    width: 100%;
  }

  .table-trans-textarea {
    width: 100%;
    min-height: 44px;
    background: #101319;
    border: 1px solid #232b38;
    border-radius: 6px;
    color: #f1f5f9;
    font-family: inherit;
    font-size: 13px;
    line-height: 1.45;
    padding: 6px 9px;
    resize: vertical;
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;
    box-sizing: border-box;
    display: block;
  }

  .table-trans-textarea:hover {
    border-color: #334155;
    background: #131720;
  }

  .table-trans-textarea:focus {
    border-color: #38bdf8;
    background: #0f1624;
    box-shadow: 0 0 0 2px rgba(56, 189, 248, 0.2);
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

  .graph-canvas-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
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
    border: 1px solid rgba(56, 189, 248, 0.25);
    flex-shrink: 0;
  }

  .empty-scene-icon-wrap.icon-wrap-amber {
    background: rgba(245, 158, 11, 0.12);
    border-color: rgba(245, 158, 11, 0.35);
  }

  .empty-scene-icon-wrap.icon-wrap-emerald {
    background: rgba(16, 185, 129, 0.12);
    border-color: rgba(16, 185, 129, 0.35);
  }

  .empty-scene-icon-wrap.icon-wrap-purple {
    background: rgba(192, 132, 252, 0.12);
    border-color: rgba(192, 132, 252, 0.35);
  }

  .empty-scene-icon-wrap.icon-wrap-indigo {
    background: rgba(129, 140, 248, 0.12);
    border-color: rgba(129, 140, 248, 0.35);
  }

  .empty-scene-icon-wrap.icon-wrap-slate {
    background: rgba(100, 116, 139, 0.12);
    border-color: rgba(100, 116, 139, 0.35);
  }

  .empty-scene-icon-wrap.icon-wrap-sky {
    background: rgba(56, 189, 248, 0.12);
    border-color: rgba(56, 189, 248, 0.35);
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

  .graph-inline-pills {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
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

  .graph-mode-btn {
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
    color: #cbd5e1;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .graph-mode-btn:hover {
    background: #1a2232;
    border-color: #2b394e;
    color: #e2e8f0;
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
    line-clamp: 2;
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
    gap: 6px;
  }

  .focus-title-badge {
    color: #94a3b8;
    font-size: 11.5px;
    font-weight: 500;
  }

  .focus-scene-name {
    color: #f1f5f9;
    font-size: 13px;
    font-weight: 700;
  }

  .btn-toolbar-code {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    height: 22px;
    min-height: 22px;
    max-height: 22px;
    padding: 0 7px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    box-sizing: border-box;
    background: #111a2e;
    border: 1px solid #273549;
    border-radius: 4px;
    color: #94a3b8;
    cursor: pointer;
    margin: 0;
    transition: all 0.15s;
  }

  .btn-toolbar-code:hover {
    background: #1e293b;
    border-color: #0284c7;
    color: #38bdf8;
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
    table-layout: fixed;
    border-collapse: collapse;
    font-size: 13px;
  }

  .col-w-line {
    width: 55px;
  }

  .col-w-speaker {
    width: 155px;
  }

  .col-w-source {
    width: calc(50% - 145px);
  }

  .col-w-trans {
    width: calc(50% - 145px);
  }

  .col-w-status {
    width: 80px;
  }

  .grid-table th {
    background: #111419;
    padding: 7px 12px;
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
    border-bottom: 1px solid #1c222c;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    box-sizing: border-box;
  }

  .grid-table td {
    padding: 8px 12px;
    border-bottom: 1px solid #14171e;
    vertical-align: top;
    user-select: text;
    box-sizing: border-box;
    word-break: break-word;
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
    padding: 8px 4px 8px 10px !important;
  }

  .col-speaker {
    padding: 8px 8px !important;
  }

  button.char-chip {
    font-family: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
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
    animation: skeletonFadeIn 0.15s ease-out;
  }

  @keyframes skeletonFadeIn {
    from {
      opacity: 0;
      transform: translateY(2px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
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

  .btn-modal-ai {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: #1e1b4b;
    border: 1px solid #4338ca;
    color: #c7d2fe;
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-modal-ai:hover {
    background: #312e81;
    border-color: #6366f1;
    color: #ffffff;
    box-shadow: 0 0 12px rgba(99, 102, 241, 0.35);
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
