<script lang="ts">
  import { tick } from "svelte";
  import { Check, ChevronDown, ChevronUp, Copy, FileCode, LoaderCircle, Search, TriangleAlert, X } from "lucide-svelte";
  import { commands, type SourceFileDto } from "./bindings";
  import { highlightRenpyLine } from "./renpy-highlight";
  import { matchingLines } from "./source-search";

  /**
   * Текст скрипта или файла перевода из снимка проекта: папка игры не нужна.
   * Номер строки тот же, что в колонке реплик таблицы.
   */
  let {
    projectId,
    files,
    file,
    line = null,
    onClose,
  }: {
    projectId: string;
    /** Файлы таблицы: между ними можно переключаться. */
    files: { id: number; path: string }[];
    file: number;
    /** Строка, к которой прокрутить и которую подсветить. */
    line?: number | null;
    onClose: () => void;
  } = $props();

  let current = $state(untrackedFile());
  let target = $state<number | null>(untrackedLine());
  let source = $state<SourceFileDto | null>(null);
  let error = $state("");
  let loading = $state(false);
  let query = $state("");
  let active = $state(0);
  let copied = $state(false);
  let viewport = $state<HTMLDivElement | null>(null);
  let searchInput = $state<HTMLInputElement | null>(null);

  function untrackedFile() {
    return file;
  }
  function untrackedLine() {
    return line;
  }

  const lines = $derived(source ? source.text.split(/\r?\n/) : []);
  const matches = $derived(matchingLines(lines, query));
  const matchSet = $derived(new Set(matches));

  $effect(() => {
    void load(current);
  });

  $effect(() => {
    void query;
    active = 0;
  });

  async function load(id: number) {
    loading = true;
    error = "";
    source = null;
    try {
      const loaded = await commands.sourceFile(projectId, id);
      if (id !== current) return;
      source = loaded;
      await tick();
      if (target !== null) scrollTo(target - 1, "auto");
      else viewport?.scrollTo({ top: 0 });
    } catch (e) {
      if (id === current) error = String(e);
    } finally {
      if (id === current) loading = false;
    }
  }

  function scrollTo(index: number, behavior: ScrollBehavior = "smooth") {
    viewport?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "center", behavior });
  }

  function jump(step: number) {
    if (matches.length === 0) return;
    active = (active + step + matches.length) % matches.length;
    scrollTo(matches[active]);
  }

  function choose(id: number) {
    target = null;
    query = "";
    current = id;
  }

  async function copy() {
    if (!source) return;
    try {
      await navigator.clipboard.writeText(source.text);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch (e) {
      error = String(e);
    }
  }

  function onKey(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      searchInput?.focus();
      searchInput?.select();
    } else if (event.key === "F3") {
      event.preventDefault();
      jump(event.shiftKey ? -1 : 1);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.target === event.currentTarget) onClose();
  }}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Исходник">
    <header class="dialog-head">
      <FileCode size={16} />
      <select class="input file-select" value={current} onchange={(event) => choose(Number(event.currentTarget.value))} aria-label="Файл">
        {#each files as item (item.id)}
          <option value={item.id}>{item.path}</option>
        {/each}
      </select>
      {#if source}
        <span class="faint">Строк: {lines.length}</span>
        {#if source.decompiled}
          <span class="badge" title="Файл в игре скомпилирован: текст восстановлен приложением и может отличаться от авторского">Декомпилирован</span>
        {/if}
      {/if}
      <span class="spacer"></span>
      <button class="btn small" disabled={!source} title="Скопировать весь текст файла" onclick={copy}>
        {#if copied}<Check size={13} />{:else}<Copy size={13} />{/if}
        <span>{copied ? "Скопировано" : "Копировать"}</span>
      </button>
      <button class="icon-button" aria-label="Закрыть" title="Закрыть (Esc)" onclick={onClose}><X size={16} /></button>
    </header>

    <div class="search">
      <Search size={13} />
      <input
        bind:this={searchInput}
        class="input"
        placeholder="Поиск по файлу (Ctrl+F)"
        bind:value={query}
        onkeydown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            jump(event.shiftKey ? -1 : 1);
          }
        }}
      />
      {#if query.trim()}
        <span class="faint">{matches.length > 0 ? `${active + 1} из ${matches.length}` : "Не найдено"}</span>
        <button class="icon-button" disabled={matches.length === 0} aria-label="Предыдущее совпадение" title="Предыдущее (Shift+F3)" onclick={() => jump(-1)}>
          <ChevronUp size={14} />
        </button>
        <button class="icon-button" disabled={matches.length === 0} aria-label="Следующее совпадение" title="Следующее (F3)" onclick={() => jump(1)}>
          <ChevronDown size={14} />
        </button>
      {/if}
    </div>

    <div class="code" bind:this={viewport}>
      {#if loading}
        <p class="state"><LoaderCircle size={16} class="spin" /> Читаю файл</p>
      {:else if error}
        <div class="banner error" role="alert"><TriangleAlert size={14} /><span>{error}</span></div>
      {:else if lines.length === 0}
        <p class="state">Файл пуст</p>
      {:else}
        {#each lines as text, index (index)}
          <div
            class="code-line"
            data-index={index}
            class:target={target === index + 1}
            class:match={matchSet.has(index)}
            class:current={matches[active] === index}
          >
            <span class="number">{index + 1}</span>
            <!-- highlightRenpyLine экранирует весь текст файла -->
            <pre>{@html highlightRenpyLine(text)}</pre>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: var(--backdrop);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 1100px;
    height: 100%;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 24px 60px -12px rgba(0, 0, 0, 0.7);
  }

  .dialog-head,
  .search {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 10px 16px;
    background: var(--surface);
    border-bottom: 1px solid var(--divider);
    color: var(--text-muted);
  }

  .file-select {
    min-width: 0;
    max-width: 480px;
    color: var(--text-strong);
  }

  .spacer {
    flex: 1;
  }

  .faint {
    color: var(--text-faint);
    font-size: 11px;
    white-space: nowrap;
  }

  .search .input {
    flex: 1;
    max-width: 360px;
  }

  .code {
    flex: 1;
    overflow: auto;
    padding: 6px 0;
    background: var(--bg-sunken);
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .state {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 16px;
    color: var(--text-faint);
  }

  .code .banner {
    margin: 16px;
  }

  .code-line {
    display: flex;
    min-width: max-content;
    line-height: 1.6;
  }

  .code-line.match {
    background: var(--warning-bg);
  }

  .code-line.current {
    box-shadow: inset 2px 0 0 var(--warning-strong);
  }

  .code-line.target {
    background: var(--selected);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .number {
    flex: none;
    width: 56px;
    padding-right: 12px;
    color: var(--text-ghost);
    text-align: right;
    user-select: none;
  }

  pre {
    margin: 0;
    padding-right: 16px;
    color: var(--text-soft);
    font: inherit;
    white-space: pre;
  }

  pre :global(.tok-comment) {
    color: var(--text-ghost);
    font-style: italic;
  }

  pre :global(.tok-string) {
    color: var(--success);
  }

  pre :global(.tok-keyword) {
    color: var(--accent-text);
  }

  pre :global(.tok-label-line) {
    color: var(--warning);
    font-weight: 600;
  }

  pre :global(.tok-translate-line) {
    color: var(--info);
  }
</style>
