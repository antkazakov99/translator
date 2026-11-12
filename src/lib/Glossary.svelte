<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { activeRun } from "./run-status.svelte";
  import { Braces, BookText, LoaderCircle, MessagesSquare, SpellCheck, TriangleAlert, Users, X } from "lucide-svelte";
  import GlossaryCharacters from "./GlossaryCharacters.svelte";
  import GlossaryNameRules from "./GlossaryNameRules.svelte";
  import GlossarySpeakers from "./GlossarySpeakers.svelte";
  import GlossaryTerms from "./GlossaryTerms.svelte";
  import GlossaryTokens from "./GlossaryTokens.svelte";
  import { commands, type EntryLineDto_Serialize as EntryLine } from "./bindings";
  import type { Character, Glossary, Term, Token } from "./glossary";
  import { glossaryRuns } from "./suggest-run.svelte";

  let {
    projectId,
    onOpenLine,
  }: {
    projectId: string;
    /** Реплика персонажа или термина в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
  } = $props();

  /** Запуски модели и их отбор переживают переход между разделами. */
  const runs = glossaryRuns();

  type Section = "speakers" | "characters" | "rules" | "terms" | "tokens";
  let data = $state<Glossary | null>(null);
  let error = $state("");
  let character = $state<number | null>(null);
  let term = $state<number | null>(null);

  /** Раздел переживает перезагрузку страницы. */
  const sectionKey = $derived(`translator.glossary.${projectId}`);
  /** Выбранные персонаж и термин переживают переход к реплике в таблицу и обратно. */
  const pickedKey = $derived(`translator.glossary-picked.${projectId}`);
  let section = $state<Section>("speakers");

  onMount(() => {
    try {
      const saved = sessionStorage.getItem(sectionKey);
      if (saved === "speakers" || saved === "characters" || saved === "rules" || saved === "terms" || saved === "tokens") section = saved;
      const picked = JSON.parse(sessionStorage.getItem(pickedKey) ?? "{}");
      if (typeof picked.character === "number") character = picked.character;
      if (typeof picked.term === "number") term = picked.term;
    } catch {
      // Без хранилища раздел и выбор просто не запомнятся.
    }
    load();
  });

  $effect(() => {
    const picked = JSON.stringify({ character, term });
    try {
      sessionStorage.setItem(pickedKey, picked);
    } catch {
      // Без хранилища выбор просто не запомнится.
    }
  });

  /**
   * Запуск модели (разбор сцен, автоперевод, описания) пишет в глоссарий по ходу: пока он идёт, список
   * перечитывается не чаще раза в `REFRESH_MS` и ещё раз после конца. Открытая карточка держит свой черновик.
   */
  const REFRESH_MS = 1500;
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  let wasRunning = false;

  function refreshSoon() {
    if (refreshTimer) return;
    refreshTimer = setTimeout(() => {
      refreshTimer = null;
      load();
    }, REFRESH_MS);
  }

  $effect(() => {
    const run = activeRun.current;
    // Строка хода меняется с каждым запросом: по ней видно, что запуск продвинулся.
    if (run && run.text) refreshSoon();
    const running = run !== null;
    if (wasRunning && !running) load();
    wasRunning = running;
  });

  onDestroy(() => {
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  function open(next: Section) {
    section = next;
    try {
      sessionStorage.setItem(sectionKey, next);
    } catch {
      // Без хранилища раздел просто не запомнится.
    }
  }

  async function load() {
    try {
      data = await commands.glossary(projectId);
      error = "";
    } catch (e) {
      error = String(e);
    }
  }

  function patchCharacter(next: Character) {
    if (data) data.characters = data.characters.map((item) => (item.id === next.id ? next : item));
  }

  function patchTerm(next: Term) {
    if (data) data.terms = data.terms.map((item) => (item.id === next.id ? next : item));
  }

  function patchToken(next: Token) {
    if (data) data.tokens = data.tokens.map((item) => (item.name === next.name ? next : item));
  }

  const sections = $derived([
    { id: "speakers" as const, label: "Говорящие", icon: MessagesSquare, count: data?.speakers.filter((item) => item.state !== "discarded").length ?? 0, hint: "" },
    { id: "characters" as const, label: "Персонажи", icon: Users, count: data?.characters.length ?? 0, hint: "" },
    { id: "rules" as const, label: "Правила имён", icon: SpellCheck, count: null, hint: "" },
    { id: "terms" as const, label: "Термины", icon: BookText, count: data?.terms.length ?? 0, hint: "" },
    { id: "tokens" as const, label: "Токены", icon: Braces, count: data?.tokens.length ?? 0, hint: "" },
  ]);
</script>

<div class="glossary">
  <nav class="sections" aria-label="Разделы глоссария">
    {#each sections as item (item.id)}
      <button class="section" class:active={section === item.id} title={item.label} aria-label={item.label} onclick={() => open(item.id)}>
        <item.icon size={14} />
        <span class="section-name">{item.label}</span>
        {#if data && item.count !== null}<span class="section-count">{item.hint ? `${item.hint}: ${item.count}` : item.count}</span>{/if}
      </button>
    {/each}
  </nav>

  <section class="content">
    {#if error}
      <div class="banner error page-error" role="alert">
        <TriangleAlert size={14} />
        <span>{error}</span>
        <button class="icon-button" aria-label="Скрыть ошибку" title="Скрыть ошибку" onclick={() => (error = "")}><X size={14} /></button>
      </div>
    {/if}

    {#if !data}
      <div class="empty"><LoaderCircle size={20} class="spin" /><span>Загрузка глоссария</span></div>
    {:else if section === "speakers"}
      <GlossarySpeakers
        {projectId}
        {data}
        {runs}
        reload={load}
        onError={(message) => (error = message)}
        onOpenCharacter={(id) => {
          character = id;
          open("characters");
        }}
        {onOpenLine}
      />
    {:else if section === "characters"}
      <GlossaryCharacters
        {projectId}
        {data}
        {runs}
        bind:selected={character}
        reload={load}
        patch={patchCharacter}
        {onOpenLine}
        onError={(message) => (error = message)}
      />
    {:else if section === "rules"}
      <GlossaryNameRules {projectId} {runs} onError={(message) => (error = message)} />
    {:else if section === "terms"}
      <GlossaryTerms
        {projectId}
        {data}
        {runs}
        bind:selected={term}
        reload={load}
        patch={patchTerm}
        {onOpenLine}
        onError={(message) => (error = message)}
      />
    {:else}
      <GlossaryTokens {projectId} {data} {runs} reload={load} patch={patchToken} {onOpenLine} onError={(message) => (error = message)} />
    {/if}
  </section>
</div>

<style>
  .glossary {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 260px;
    padding: 10px;
    border-right: 1px solid var(--divider);
    background: var(--surface-side);
  }

  .section {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    text-align: left;
  }

  .section:hover,
  .section.active {
    border-color: var(--border-selected);
    background: var(--hover);
    color: var(--text);
  }

  .section-name {
    flex: 1;
    font-weight: 600;
  }

  .section-count {
    color: var(--text-faint);
    font-size: 10px;
  }

  .content {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  /* В узком окне от разделов остаются иконки: место нужно списку, записи и её репликам. */
  @media (max-width: 1100px) {
    .sections {
      width: auto;
    }

    .section-name,
    .section-count {
      display: none;
    }
  }

  /* Общее для разделов глоссария. */

  .glossary :global(.toolbar) {
    display: flex;
    gap: 14px;
    align-items: center;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
  }

  .glossary :global(.toolbar-title) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .glossary :global(.toolbar-title strong) {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
  }

  .glossary :global(.toolbar-title span),
  .glossary :global(.faint) {
    color: var(--text-faint);
    font-size: 11px;
  }

  .glossary :global(.page-error) {
    margin: 10px 16px 0;
  }

  /* Баннер над списком и записью: области начинаются с разделителя, а не вплотную к баннеру. */
  .glossary :global(.page-error + .master) {
    margin-top: 10px;
    border-top: 1px solid var(--divider);
  }

  .glossary :global(.empty) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    align-items: center;
    justify-content: center;
    padding: 24px;
    color: var(--text-faint);
    font-size: 12px;
    text-align: center;
  }

  .glossary :global(.empty strong) {
    color: var(--text-soft);
    font-size: 14px;
    font-weight: 600;
  }

  /* Таблица раздела: строки на всю ширину через разделители, как реплики в таблице перевода. */
  .glossary :global(.list) {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 16px;
  }

  .glossary :global(.list > *) {
    flex-shrink: 0;
  }

  .glossary :global(.list > .row) {
    display: flex;
    gap: 12px;
    align-items: center;
    justify-content: space-between;
    padding: 8px 16px;
    border-bottom: 1px solid var(--divider-row);
  }

  .glossary :global(.list > .row:hover) {
    background: var(--hover-row);
  }

  /*
   * Список и запись: области на всю высоту раздела через разделители, как сцены и описание в таблице.
   * Ряд по высоте раздела, каждая область листается сама.
   */
  .glossary :global(.master) {
    display: grid;
    flex: 1;
    grid-template-columns: clamp(200px, 22%, 260px) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    min-height: 0;
  }

  /* Персонажи и термины: список, поля записи и её реплики. */
  .glossary :global(.master.with-lines) {
    grid-template-columns: clamp(200px, 22%, 260px) clamp(300px, 36%, 420px) minmax(0, 1fr);
  }

  .glossary :global(.list-side) {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--divider);
    background: var(--surface-side);
  }

  .glossary :global(.list-side > .input) {
    margin: 10px 8px 8px;
  }

  .glossary :global(.items) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    padding: 0 8px 8px;
  }

  .glossary :global(.items > .faint) {
    margin: 0;
    padding: 4px 10px;
  }

  .glossary :global(.item) {
    display: flex;
    flex: 0 0 auto;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: transparent;
    text-align: left;
  }

  .glossary :global(.item:hover),
  .glossary :global(.item.active) {
    border-color: var(--border-selected);
    background: var(--hover);
  }

  .glossary :global(.item strong) {
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .glossary :global(.item > span) {
    overflow: hidden;
    color: var(--text-faint);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Новая запись: поле и кнопка одной строкой внизу списка. */
  .glossary :global(.list-side > .add) {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 8px;
    border-top: 1px solid var(--divider);
  }

  .glossary :global(.list-side > .add .input) {
    flex: 1;
    min-width: 0;
  }

  .glossary :global(.editor) {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 14px 16px;
  }

  .glossary :global(.master.with-lines > .editor) {
    border-right: 1px solid var(--divider);
  }

  /* Без панели реплик поля не растягиваются на всю ширину окна. */
  .glossary :global(.master:not(.with-lines) > .editor) {
    box-sizing: border-box;
    width: 100%;
    max-width: 760px;
  }

  .glossary :global(.editor.saving) {
    opacity: 0.75;
  }

  /* Переключатель по своей ширине, а не на всю строку поля. */
  .glossary :global(.editor .field > .segmented) {
    align-self: flex-start;
  }

  .glossary :global(.editor .page-error) {
    margin: 0;
  }

  .glossary :global(.master > .empty) {
    grid-column: 2 / -1;
  }

  /* Шапка записи: название и действия одной строкой. */
  .glossary :global(.editor-head) {
    display: flex;
    gap: 8px;
    align-items: center;
    min-height: 28px;
  }

  .glossary :global(.editor-head strong) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Действия записи справа в шапке: кнопки-иконки вплотную, подтверждение на их месте. */
  .glossary :global(.editor-tools) {
    display: flex;
    flex-shrink: 0;
    gap: 2px;
    align-items: center;
  }

  .glossary :global(.editor-tools > .btn),
  .glossary :global(.editor-tools > .confirm-text),
  .glossary :global(.editor-tools > .faint),
  .glossary :global(.row-tools > .btn),
  .glossary :global(.row-tools > .confirm-text) {
    margin-left: 4px;
  }

  /* Действия строки таблицы справа: те же кнопки-иконки, подтверждение на их месте. */
  .glossary :global(.row-tools) {
    display: flex;
    flex-shrink: 0;
    gap: 2px;
    align-items: center;
    min-height: 28px;
  }

  .glossary :global(.editor-tools > .icon-button.active),
  .glossary :global(.row-tools > .icon-button.active) {
    background: var(--hover-icon);
    color: var(--text-strong);
  }

  .glossary :global(.confirm-text) {
    color: var(--text-soft);
    font-size: 12px;
    white-space: nowrap;
  }

  .glossary :global(.grid) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  /* Строка pill одной высоты с pill и без них: при переходе между записями поля ниже не прыгают. */
  .glossary :global(.badges) {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    min-height: 22px;
  }


  .glossary :global(.field textarea) {
    resize: vertical;
  }

  /* Отбор предложений модели. */

  .glossary :global(.pick-head) {
    margin: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--divider);
    color: var(--text-faint);
    font-size: 11px;
  }

  .glossary :global(.pick-row) {
    display: grid;
    gap: 10px;
    align-items: center;
    padding: 8px 16px;
    border-bottom: 1px solid var(--divider-row);
  }

  .glossary :global(.pick-row.names) {
    grid-template-columns: auto minmax(160px, 1fr) minmax(140px, 1fr) 1.6fr;
  }

  .glossary :global(.pick-row.terms) {
    grid-template-columns: auto minmax(140px, 1fr) 1.6fr 160px;
  }

  .glossary :global(.pick-row.speakers) {
    grid-template-columns: auto minmax(140px, 1fr) 1.2fr auto;
  }

  .glossary :global(.pick-row.tokens) {
    grid-template-columns: auto minmax(120px, 0.8fr) 1fr 1.4fr;
  }

  .glossary :global(.pick-row.off) {
    opacity: 0.55;
  }

  .glossary :global(.pick-row .name),
  .glossary :global(.list > .row .name) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .glossary :global(.pick-row .name strong),
  .glossary :global(.list > .row .name strong) {
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .glossary :global(.pick-row .name span),
  .glossary :global(.list > .row .name span) {
    overflow: hidden;
    color: var(--text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .glossary :global(.pick-actions) {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
    padding: 10px 16px;
    border-top: 1px solid var(--divider);
  }
</style>
