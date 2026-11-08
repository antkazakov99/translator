<script lang="ts">
  import { onMount } from "svelte";
  import { Braces, BookText, LoaderCircle, MessagesSquare, TriangleAlert, Users, X } from "lucide-svelte";
  import GlossaryCharacters from "./GlossaryCharacters.svelte";
  import GlossarySpeakers from "./GlossarySpeakers.svelte";
  import GlossaryTerms from "./GlossaryTerms.svelte";
  import GlossaryTokens from "./GlossaryTokens.svelte";
  import { commands } from "./bindings";
  import type { Character, Glossary, Term, Token } from "./glossary";
  import { glossaryRuns } from "./suggest-run.svelte";

  let { projectId }: { projectId: string } = $props();

  /** Запуски модели и их отбор переживают переход между разделами. */
  const runs = glossaryRuns();

  type Section = "speakers" | "characters" | "terms" | "tokens";
  let data = $state<Glossary | null>(null);
  let error = $state("");
  let character = $state<number | null>(null);

  /** Раздел переживает перезагрузку страницы. */
  const sectionKey = $derived(`translator.glossary.${projectId}`);
  let section = $state<Section>("speakers");

  onMount(() => {
    try {
      const saved = sessionStorage.getItem(sectionKey);
      if (saved === "speakers" || saved === "characters" || saved === "terms" || saved === "tokens") section = saved;
    } catch {
      // Без хранилища раздел просто не запомнится.
    }
    load();
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
    { id: "speakers" as const, label: "Говорящие", icon: MessagesSquare, count: data?.speakers.filter((item) => item.state === "new").length ?? 0, hint: "новых" },
    { id: "characters" as const, label: "Персонажи", icon: Users, count: data?.characters.length ?? 0, hint: "" },
    { id: "terms" as const, label: "Термины", icon: BookText, count: data?.terms.length ?? 0, hint: "" },
    { id: "tokens" as const, label: "Токены", icon: Braces, count: data?.tokens.length ?? 0, hint: "" },
  ]);
</script>

<div class="glossary">
  <nav class="sections" aria-label="Разделы глоссария">
    {#each sections as item (item.id)}
      <button class="section" class:active={section === item.id} onclick={() => open(item.id)}>
        <item.icon size={14} />
        <span class="section-name">{item.label}</span>
        {#if data}<span class="section-count">{item.hint ? `${item.hint}: ${item.count}` : item.count}</span>{/if}
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
      />
    {:else if section === "characters"}
      <div class="toolbar">
        <div class="toolbar-title">
          <strong>Персонажи</strong>
          <span>Имена с переводом и родом: модель видит их в каждой реплике, где они встречаются</span>
        </div>
      </div>
      <GlossaryCharacters {projectId} {data} bind:selected={character} reload={load} patch={patchCharacter} onError={(message) => (error = message)} />
    {:else if section === "terms"}
      <GlossaryTerms {projectId} {data} {runs} reload={load} patch={patchTerm} onError={(message) => (error = message)} />
    {:else}
      <GlossaryTokens {projectId} {data} {runs} reload={load} patch={patchToken} onError={(message) => (error = message)} />
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

  .glossary :global(.list) {
    flex: 1;
    overflow: auto;
    padding: 8px 16px 16px;
  }

  .glossary :global(.list > .row) {
    display: flex;
    gap: 12px;
    align-items: center;
    justify-content: space-between;
    padding: 8px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .glossary :global(.master) {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(220px, 300px) 1fr;
    gap: 16px;
    min-height: 0;
    padding: 12px 16px 16px;
  }

  .glossary :global(.list-side) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }

  .glossary :global(.items) {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    overflow: auto;
  }

  .glossary :global(.item) {
    display: flex;
    flex-direction: column;
    gap: 2px;
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
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }

  .glossary :global(.item > span) {
    overflow: hidden;
    color: var(--text-faint);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .glossary :global(.guess) {
    color: var(--warning);
    font-size: 10px;
    font-weight: 600;
  }

  .glossary :global(.add) {
    display: flex;
    gap: 6px;
  }

  .glossary :global(.add .input) {
    flex: 1;
    min-width: 0;
  }

  .glossary :global(.add.stacked) {
    flex-wrap: wrap;
  }

  .glossary :global(.add.stacked .name) {
    flex-basis: 100%;
  }

  .glossary :global(.editor) {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-self: start;
    max-height: 100%;
    overflow: auto;
  }

  .glossary :global(.editor.saving) {
    opacity: 0.75;
  }

  .glossary :global(.card-head) {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .glossary :global(.card-head strong) {
    flex: 1;
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .glossary :global(.confirm-text) {
    color: var(--text-soft);
    font-size: 12px;
  }

  .glossary :global(.grid) {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .glossary :global(.badges) {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .glossary :global(.card-actions) {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    justify-content: flex-end;
  }

  .glossary :global(.card-actions .picker) {
    width: 240px;
  }

  .glossary :global(.field textarea) {
    resize: vertical;
  }

  /* Отбор предложений модели. */

  .glossary :global(.pick-head) {
    margin: 4px 4px 8px;
    color: var(--text-faint);
    font-size: 11px;
  }

  .glossary :global(.pick-row) {
    display: grid;
    gap: 10px;
    align-items: center;
    padding: 8px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .glossary :global(.pick-row.terms) {
    grid-template-columns: auto minmax(140px, 1fr) 1.2fr 140px 1.2fr;
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

  .glossary :global(.pick-row .name) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .glossary :global(.pick-row .name strong) {
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .glossary :global(.pick-row .name span) {
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
