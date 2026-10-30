<script lang="ts">
  import { onMount, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    FileCode,
    FolderOpen,
    ExternalLink,
    Copy,
    Check,
    Search,
    X,
    ChevronUp,
    ChevronDown,
    Loader2,
    AlertCircle,
    Layers,
    Code,
    Hash
  } from "lucide-svelte";
  import { highlightRenpyLine } from "./renpy-highlight";

  interface SourceFileDetails {
    file_path: string;
    file_name: string;
    original_path: string | null;
    translation_path: string | null;
    content: string;
    original_content: string | null;
    total_lines: number;
    original_total_lines: number | null;
  }

  interface SceneItem {
    id: string;
    name: string;
    line_number?: number;
  }

  let {
    show = false,
    filePath = null,
    targetSceneName = null,
    targetLineNumber = null,
    scenes = [],
    onclose = () => {}
  }: {
    show: boolean;
    filePath: string | null;
    targetSceneName?: string | null;
    targetLineNumber?: number | null;
    scenes?: SceneItem[];
    onclose: () => void;
  } = $props();

  let loading = $state(false);
  let errorMsg = $state<string | null>(null);
  let fileDetails = $state<SourceFileDetails | null>(null);
  let activeTab = $state<"translation" | "original">("translation");

  let searchQuery = $state("");
  let searchInputEl = $state<HTMLInputElement | null>(null);
  let codeViewportEl = $state<HTMLDivElement | null>(null);
  let copied = $state(false);
  let activeMatchIndex = $state(0);
  let selectedScene = $state("");

  // Текст активного файла
  let currentContent = $derived.by(() => {
    if (!fileDetails) return "";
    if (activeTab === "original" && fileDetails.original_content != null) {
      return fileDetails.original_content;
    }
    return fileDetails.content;
  });

  // Текущий путь активного файла
  let currentActivePath = $derived.by(() => {
    if (!fileDetails) return filePath ?? "";
    if (activeTab === "original" && fileDetails.original_path) {
      return fileDetails.original_path;
    }
    return fileDetails.file_path;
  });

  // Строки файла
  let rawLines = $derived.by(() => {
    if (!currentContent) return [];
    return currentContent.split(/\r?\n/);
  });

  // Найденные совпадения поиска (индексы строк, начиная с 0)
  let matchedLineIndices = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return [];
    const matches: number[] = [];
    rawLines.forEach((line, idx) => {
      if (line.toLowerCase().includes(q)) {
        matches.push(idx);
      }
    });
    return matches;
  });

  // Загрузка файла при изменении filePath или show
  $effect(() => {
    if (show && filePath) {
      loadFile(filePath);
    }
  });

  // Сброс поиска при смене вкладки
  $effect(() => {
    if (activeTab) {
      activeMatchIndex = 0;
    }
  });

  async function loadFile(path: string) {
    loading = true;
    errorMsg = null;
    try {
      const details = await invoke<SourceFileDetails>("read_source_file", {
        filePath: path
      });
      fileDetails = details;
      // Если файл перевода пустой или это оригинальный файл, переключаем на оригинал
      if (details.original_content && (!details.content || !path.toLowerCase().includes("tl"))) {
        activeTab = "original";
      } else {
        activeTab = "translation";
      }

      await tick();
      scrollToTarget();
    } catch (err) {
      errorMsg = String(err);
    } finally {
      loading = false;
    }
  }

  // Скролл к целевой строке или сцене
  async function scrollToTarget() {
    if (!codeViewportEl || rawLines.length === 0) return;

    let targetIdx = -1;

    // 1. Приоритет целевой строки: если указан номер строки, скроллим строго к ней
    if (targetLineNumber != null && targetLineNumber > 0) {
      targetIdx = Math.min(targetLineNumber - 1, rawLines.length - 1);
    } else if (targetSceneName) {
      // 2. Если номер строки не указан, ищем метку начала сцены
      const cleanName = targetSceneName.trim();
      const labelPatColon = `label ${cleanName}:`;
      const labelPatSpace = `label ${cleanName} `;
      const tlPat = `translate russian ${cleanName}:`;
      const tlPatPrefix = `translate russian ${cleanName}_`;

      for (let i = 0; i < rawLines.length; i++) {
        const trimmed = rawLines[i].trim();
        if (
          trimmed.startsWith(labelPatColon) ||
          trimmed.startsWith(labelPatSpace) ||
          trimmed.startsWith(tlPat) ||
          trimmed.startsWith(tlPatPrefix)
        ) {
          targetIdx = i;
          break;
        }
      }
    }

    if (targetIdx !== -1) {
      await tick();
      const lineEl = document.getElementById(`code-line-${targetIdx}`);
      if (lineEl) {
        lineEl.scrollIntoView({ block: "center", behavior: "smooth" });
      }
    }
  }

  function jumpToScene(sceneName: string) {
    if (!sceneName) return;
    targetSceneName = sceneName;
    targetLineNumber = null;
    scrollToTarget();
  }

  function jumpToMatch(index: number) {
    if (matchedLineIndices.length === 0) return;
    const bounded = (index + matchedLineIndices.length) % matchedLineIndices.length;
    activeMatchIndex = bounded;
    const lineIdx = matchedLineIndices[bounded];
    const lineEl = document.getElementById(`code-line-${lineIdx}`);
    if (lineEl) {
      lineEl.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }

  function nextMatch() {
    jumpToMatch(activeMatchIndex + 1);
  }

  function prevMatch() {
    jumpToMatch(activeMatchIndex - 1);
  }

  async function openInExternalEditor() {
    if (!currentActivePath) return;
    try {
      await openPath(currentActivePath);
    } catch {
      try {
        await invoke("open_in_system_editor", { filePath: currentActivePath });
      } catch (e) {
        alert("Не удалось открыть в редакторе: " + e);
      }
    }
  }

  async function revealInExplorer() {
    if (!currentActivePath) return;
    try {
      await revealItemInDir(currentActivePath);
    } catch {
      try {
        await invoke("reveal_in_system_explorer", { filePath: currentActivePath });
      } catch (e) {
        alert("Не удалось открыть папку: " + e);
      }
    }
  }

  async function copyFullCode() {
    if (!currentContent) return;
    try {
      await navigator.clipboard.writeText(currentContent);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch {
      // Fallback
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (!show) return;
    if (event.key === "Escape") {
      event.preventDefault();
      onclose();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      searchInputEl?.focus();
      searchInputEl?.select();
    } else if (event.key === "F3") {
      event.preventDefault();
      if (event.shiftKey) {
        prevMatch();
      } else {
        nextMatch();
      }
    }
  }

  function isLineTarget(idx: number, lineText: string): boolean {
    if (targetLineNumber != null && targetLineNumber === idx + 1) {
      return true;
    }
    if (targetSceneName) {
      const clean = targetSceneName.trim();
      const trimmed = lineText.trim();
      return (
        trimmed.startsWith(`label ${clean}:`) ||
        trimmed.startsWith(`label ${clean} `) ||
        trimmed.startsWith(`translate russian ${clean}:`) ||
        trimmed.startsWith(`translate russian ${clean}_`)
      );
    }
    return false;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if show}
  <div
    class="source-modal-backdrop"
    onclick={(e) => {
      if (e.target === e.currentTarget) onclose();
    }}
    onkeydown={(e) => { if (e.key === 'Escape') onclose(); }}
    role="dialog"
    aria-modal="true"
    aria-label="Просмотр исходного кода скрипта"
    tabindex="-1"
  >
    <div class="source-modal-container">
      <!-- ШАПКА МОДАЛЬНОГО ОКНА -->
      <div class="source-modal-header">
        <div class="header-left">
          <div class="file-icon-badge">
            <FileCode size={20} class="text-sky-400" />
          </div>
          <div class="file-titles">
            <div class="title-row">
              <h3>{fileDetails?.file_name || "Просмотр файла скрипта"}</h3>
              {#if fileDetails}
                <span class="file-line-count-badge">
                  {rawLines.length} строк
                </span>
              {/if}
            </div>
            <p class="file-subpath" title={currentActivePath}>
              {currentActivePath}
            </p>
          </div>
        </div>

        <!-- ВКЛАДКИ: Перевод / Оригинал -->
        {#if fileDetails?.original_path && fileDetails?.translation_path}
          <div class="tabs-group">
            <button
              class="tab-btn"
              class:active={activeTab === "translation"}
              onclick={() => (activeTab = "translation")}
              title="Файл перевода (tl/russian/...)"
            >
              <Layers size={13} />
              <span>Перевод (tl/)</span>
              <small class="tab-badge">{fileDetails.total_lines}</small>
            </button>
            <button
              class="tab-btn"
              class:active={activeTab === "original"}
              onclick={() => (activeTab = "original")}
              title="Оригинальный скрипт игры (game/...)"
            >
              <Code size={13} />
              <span>Оригинал (game/)</span>
              {#if fileDetails.original_total_lines}
                <small class="tab-badge">{fileDetails.original_total_lines}</small>
              {/if}
            </button>
          </div>
        {/if}

        <!-- ДЕЙСТВИЯ В ШАПКЕ -->
        <div class="header-actions">
          <button
            class="btn-action"
            onclick={openInExternalEditor}
            title="Открыть файл во внешнем редакторе (VS Code, Notepad++ и т.д.)"
          >
            <ExternalLink size={14} />
            <span>В редакторе</span>
          </button>

          <button
            class="btn-action"
            onclick={revealInExplorer}
            title="Показать файл в проводнике Windows"
          >
            <FolderOpen size={14} />
            <span>В папке</span>
          </button>

          <button
            class="btn-action"
            onclick={copyFullCode}
            title="Скопировать весь код файла"
          >
            {#if copied}
              <Check size={14} class="text-emerald-400" />
              <span class="text-emerald-400">Скопировано!</span>
            {:else}
              <Copy size={14} />
              <span>Копировать</span>
            {/if}
          </button>

          <button
            class="btn-close-modal"
            onclick={onclose}
            aria-label="Закрыть просмотрщик"
            title="Закрыть (Esc)"
          >
            <X size={18} />
          </button>
        </div>
      </div>

      <!-- ПАНЕЛЬ ПОИСКА И НАВИГАЦИИ ПО СЦЕНАМ -->
      <div class="source-toolbar">
        <!-- Поиск по тексту -->
        <div class="search-box">
          <Search size={14} class="text-slate-400" />
          <input
            bind:this={searchInputEl}
            type="text"
            class="search-input"
            placeholder="Поиск по коду (Ctrl+F)..."
            bind:value={searchQuery}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (e.shiftKey) prevMatch();
                else nextMatch();
              }
            }}
          />
          {#if searchQuery}
            <span class="match-info">
              {#if matchedLineIndices.length > 0}
                {activeMatchIndex + 1} из {matchedLineIndices.length}
              {:else}
                Не найдено
              {/if}
            </span>
            <button
              class="btn-match-nav"
              disabled={matchedLineIndices.length === 0}
              onclick={prevMatch}
              title="Предыдущее совпадение (Shift+Enter)"
            >
              <ChevronUp size={14} />
            </button>
            <button
              class="btn-match-nav"
              disabled={matchedLineIndices.length === 0}
              onclick={nextMatch}
              title="Следующее совпадение (Enter)"
            >
              <ChevronDown size={14} />
            </button>
            <button
              class="btn-clear-search"
              onclick={() => (searchQuery = "")}
              title="Очистить"
            >
              ✕
            </button>
          {/if}
        </div>

        <!-- Быстрый переход к сцене файла -->
        {#if scenes && scenes.length > 0}
          <div class="scene-jump-box">
            <span class="jump-label">Перейти к сцене:</span>
            <select
              class="scene-select"
              bind:value={selectedScene}
              onchange={() => jumpToScene(selectedScene)}
            >
              <option value="">Выберите сцену...</option>
              {#each scenes as sc}
                <option value={sc.name}>
                  {sc.name}
                </option>
              {/each}
            </select>
          </div>
        {/if}

        <div class="toolbar-hint">
          <span>Нажмите <kbd>Ctrl+F</kbd> для поиска, <kbd>F3</kbd> далее, <kbd>Esc</kbd> закрыть</span>
        </div>
      </div>

      <!-- ТЕЛО ПРОСМОТРА КОДА -->
      <div class="source-content-area" bind:this={codeViewportEl}>
        {#if loading}
          <div class="loading-state">
            <Loader2 size={36} class="spin text-sky-400" />
            <p>Загрузка и чтение исходного кода...</p>
          </div>
        {:else if errorMsg}
          <div class="error-state">
            <AlertCircle size={36} class="text-rose-400" />
            <p class="error-title">Не удалось открыть файл</p>
            <p class="error-desc">{errorMsg}</p>
          </div>
        {:else if rawLines.length === 0}
          <div class="empty-state">
            <FileCode size={36} class="text-slate-600" />
            <p>Файл пуст или не содержит доступного текста</p>
          </div>
        {:else}
          <div class="code-table-container">
            <table class="code-table">
              <tbody>
                {#each rawLines as line, idx}
                  {@const lineNum = idx + 1}
                  {@const isTarget = isLineTarget(idx, line)}
                  {@const isMatch = matchedLineIndices.includes(idx)}
                  {@const isActiveMatch = matchedLineIndices[activeMatchIndex] === idx}
                  <tr
                    id={`code-line-${idx}`}
                    class="code-row"
                    class:target-line={isTarget}
                    class:match-line={isMatch}
                    class:active-match={isActiveMatch}
                  >
                    <td class="line-gutter">
                      <span class="line-num">{lineNum}</span>
                    </td>
                    <td class="line-content">
                      <pre class="line-pre">{@html highlightRenpyLine(line)}</pre>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>

      <!-- ПОДВАЛ МОДАЛКИ -->
      <div class="source-modal-footer">
        <div class="footer-info">
          {#if targetSceneName}
            <span class="target-tag">
              Сцена в фокусе: <strong>{targetSceneName}</strong>
            </span>
          {/if}
          {#if targetLineNumber}
            <span class="target-tag">
              Целевая строка: <strong>#{targetLineNumber}</strong>
            </span>
          {/if}
          <span class="encoding-tag">UTF-8 / Ren'Py Script</span>
        </div>

        <div class="footer-actions">
          <button class="btn-footer-close" onclick={onclose}>
            Закрыть
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .source-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(3, 7, 18, 0.75);
    backdrop-filter: blur(6px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    animation: fadeIn 0.15s ease-out;
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .source-modal-container {
    width: 100%;
    max-width: 1280px;
    height: 88vh;
    background: #090e17;
    border: 1px solid #1e293b;
    border-radius: 14px;
    box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.7), 0 0 0 1px rgba(255, 255, 255, 0.05);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* ШАПКА */
  .source-modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    background: #0d1424;
    border-bottom: 1px solid #1e293b;
    gap: 16px;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .file-icon-badge {
    width: 38px;
    height: 38px;
    border-radius: 9px;
    background: #0c1a2e;
    border: 1px solid #0284c7;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .file-titles {
    min-width: 0;
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .title-row h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 700;
    color: #f1f5f9;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-line-count-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    background: #1e293b;
    color: #94a3b8;
    border-radius: 12px;
    border: 1px solid #334155;
  }

  .file-subpath {
    margin: 2px 0 0 0;
    font-size: 11.5px;
    color: #64748b;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 480px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  }

  /* ВКЛАДКИ */
  .tabs-group {
    display: flex;
    align-items: center;
    background: #070c14;
    padding: 3px;
    border-radius: 8px;
    border: 1px solid #1e293b;
    gap: 3px;
  }

  .tab-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #94a3b8;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .tab-btn:hover {
    color: #f1f5f9;
    background: rgba(255, 255, 255, 0.04);
  }

  .tab-btn.active {
    background: #0284c7;
    color: #ffffff;
    font-weight: 600;
    box-shadow: 0 2px 6px rgba(2, 132, 199, 0.35);
  }

  .tab-badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.3);
  }

  /* ДЕЙСТВИЯ */
  .header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn-action {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    background: #131c2e;
    border: 1px solid #273549;
    border-radius: 7px;
    color: #cbd5e1;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-action:hover {
    background: #1e293b;
    color: #f8fafc;
    border-color: #38bdf8;
  }

  .btn-close-modal {
    width: 32px;
    height: 32px;
    border-radius: 7px;
    background: transparent;
    border: 1px solid transparent;
    color: #94a3b8;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-close-modal:hover {
    background: rgba(239, 68, 68, 0.15);
    color: #f87171;
    border-color: rgba(239, 68, 68, 0.3);
  }

  /* ТУЛБАР */
  .source-toolbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 20px;
    background: #0b111e;
    border-bottom: 1px solid #192336;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: #060a12;
    border: 1px solid #222f44;
    border-radius: 7px;
    padding: 4px 10px;
    width: 320px;
  }

  .search-box:focus-within {
    border-color: #0284c7;
    box-shadow: 0 0 0 1px #0284c7;
  }

  .search-input {
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font-size: 12.5px;
    width: 100%;
  }

  .search-input::placeholder {
    color: #64748b;
  }

  .match-info {
    font-size: 11px;
    color: #94a3b8;
    white-space: nowrap;
    padding: 0 4px;
  }

  .btn-match-nav {
    background: transparent;
    border: none;
    color: #94a3b8;
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
    border-radius: 4px;
  }

  .btn-match-nav:hover:not(:disabled) {
    background: #1e293b;
    color: #f8fafc;
  }

  .btn-match-nav:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .btn-clear-search {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    font-size: 12px;
    padding: 0 2px;
  }

  .btn-clear-search:hover {
    color: #f8fafc;
  }

  .scene-jump-box {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .jump-label {
    font-size: 12px;
    color: #94a3b8;
  }

  .scene-select {
    background: #060a12;
    border: 1px solid #222f44;
    border-radius: 7px;
    color: #e2e8f0;
    font-size: 12px;
    padding: 5px 10px;
    outline: none;
    max-width: 240px;
    cursor: pointer;
  }

  .scene-select:focus {
    border-color: #0284c7;
  }

  .toolbar-hint {
    margin-left: auto;
    font-size: 11px;
    color: #475569;
  }

  .toolbar-hint kbd {
    background: #131c2e;
    border: 1px solid #273549;
    border-radius: 4px;
    padding: 1px 4px;
    color: #94a3b8;
    font-size: 10px;
    font-family: inherit;
  }

  /* ОБЛАСТЬ КОДА */
  .source-content-area {
    flex: 1;
    overflow: auto;
    background: #070c14;
    position: relative;
  }

  .loading-state,
  .error-state,
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 12px;
    color: #94a3b8;
  }

  .error-title {
    font-size: 16px;
    font-weight: 700;
    color: #f87171;
    margin: 0;
  }

  .error-desc {
    font-size: 13px;
    color: #94a3b8;
    margin: 0;
    max-width: 500px;
    text-align: center;
  }

  .code-table-container {
    width: 100%;
    min-width: 100%;
  }

  .code-table {
    width: 100%;
    border-collapse: collapse;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    font-size: 12.5px;
    line-height: 1.6;
  }

  .code-row {
    transition: background 0.1s;
  }

  .code-row:hover {
    background: rgba(255, 255, 255, 0.025);
  }

  .line-gutter {
    width: 54px;
    padding: 0 12px 0 0;
    text-align: right;
    user-select: none;
    vertical-align: top;
    background: #060a12;
    border-right: 1px solid #162033;
    color: #475569;
    font-size: 11px;
  }

  .line-num {
    display: inline-block;
  }

  .line-content {
    padding: 0 16px;
    white-space: pre;
    vertical-align: top;
    color: #cbd5e1;
  }

  .line-pre {
    margin: 0;
    padding: 0;
    font-family: inherit;
    font-size: inherit;
    line-height: inherit;
    white-space: pre;
    overflow: visible;
  }

  /* Целевая строка (фокус сцены / строки) */
  .code-row.target-line {
    background: rgba(2, 132, 199, 0.18) !important;
    border-left: 3px solid #38bdf8;
  }

  .code-row.target-line .line-gutter {
    color: #38bdf8;
    font-weight: 700;
    background: #081728;
  }

  .code-row.target-line .line-content {
    color: #ffffff;
    font-weight: 500;
  }

  /* Совпадения поиска */
  .code-row.match-line {
    background: rgba(234, 179, 8, 0.12);
  }

  .code-row.active-match {
    background: rgba(234, 179, 8, 0.28) !important;
    border-left: 3px solid #eab308;
  }

  /* ПОДСВЕТКА СИНТАКСИСА REN'PY */
  :global(.tok-comment) {
    color: #64748b;
    font-style: italic;
  }

  :global(.tok-keyword) {
    color: #a855f7;
    font-weight: 600;
  }

  :global(.tok-string) {
    color: #34d399;
  }

  :global(.tok-label-line) {
    color: #38bdf8;
    font-weight: 700;
    text-decoration: underline decoration-sky-500/40;
  }

  :global(.tok-translate-line) {
    color: #f59e0b;
    font-weight: 700;
  }

  /* ПОДВАЛ */
  .source-modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 20px;
    background: #0d1424;
    border-top: 1px solid #1e293b;
  }

  .footer-info {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .target-tag {
    font-size: 11.5px;
    color: #94a3b8;
  }

  .target-tag strong {
    color: #38bdf8;
  }

  .encoding-tag {
    font-size: 11px;
    color: #475569;
    padding: 2px 6px;
    background: #070c14;
    border-radius: 4px;
    border: 1px solid #1e293b;
  }

  .btn-footer-close {
    padding: 6px 16px;
    background: #1e293b;
    border: 1px solid #334155;
    border-radius: 7px;
    color: #e2e8f0;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-footer-close:hover {
    background: #334155;
    color: #ffffff;
  }
</style>
