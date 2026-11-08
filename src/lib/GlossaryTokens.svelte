<script lang="ts">
  import { Check, LoaderCircle, Sparkles } from "lucide-svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands } from "./bindings";
  import { matches, runsBusy, type Glossary, type GlossaryRuns, type Token } from "./glossary";

  let {
    projectId,
    data,
    runs,
    reload,
    patch,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    reload: () => Promise<void>;
    patch: (token: Token) => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.tokens);
  /** Токены из текста игры без значения и пояснения: модели есть что читать, а глоссарию нечего показать. */
  const empty = $derived(data.tokens.filter((token) => token.lines > 0 && !token.value?.trim() && !token.note?.trim()));
  let writing = $state(false);

  function runTitle() {
    if (empty.length === 0) return "У всех токенов из текста игры есть значение или пояснение";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель прочитает реплики с токеном, не видя его имени, и предложит значение и пояснение";
  }

  function startRun() {
    const names = empty.map((token) => token.name);
    run.start((channel) => commands.suggestTokens(projectId, names, null, channel), onError);
  }

  async function writeChosen() {
    const chosen = run.chosen.filter((item) => item.value?.trim() || item.note?.trim());
    writing = true;
    try {
      for (const item of chosen) await commands.saveToken(projectId, item.name, item.value?.trim() ?? "", item.note?.trim() ?? "");
      run.finish(`Записано токенов: ${chosen.length}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  let query = $state("");
  let busy = $state<string | null>(null);
  const shown = $derived(data.tokens.filter((token) => matches(query, token.name, token.value, token.note)));

  async function save(token: Token, value: string, note: string) {
    busy = token.name;
    try {
      await commands.saveToken(projectId, token.name, value, note);
      patch({ ...token, value: value.trim() || null, note: note.trim() || null });
    } catch (e) {
      onError(String(e));
    } finally {
      busy = null;
    }
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Токены</strong>
    <span>Переменные в тексте игры. Значение подставляется в реплику перед переводом, токен без значения остаётся в тексте, а модель видит пояснение</span>
  </div>
  <input class="input search" placeholder="Поиск" bind:value={query} aria-label="Поиск токенов" />
  <button class="btn small primary" disabled={empty.length === 0 || runsBusy(runs)} title={runTitle()} onclick={startRun}>
    {#if run.active}<LoaderCircle size={13} class="spin" />{:else}<Sparkles size={13} />{/if}
    <span>Предложить · {empty.length}</span>
  </button>
</div>

<SuggestStatus {run} {onError} />

{#if run.review}
  <div class="list">
    <p class="pick-head">Предложения модели. Отмеченные запишутся в токены, значение и пояснение можно поправить</p>
    {#each run.review as item (item.name)}
      {@const token = data.tokens.find((known) => known.name === item.name)}
      <div class="pick-row tokens" class:off={!item.selected}>
        <input type="checkbox" bind:checked={item.selected} aria-label="Записать токен {item.name}" />
        <div class="name">
          <strong><code>[{item.name}]</code></strong>
          <span>реплик: {token?.lines ?? 0}</span>
        </div>
        <input class="input" bind:value={item.value} placeholder="Значение" aria-label="Значение токена {item.name}" />
        <input class="input" bind:value={item.note} placeholder="Пояснение" aria-label="Пояснение токена {item.name}" />
      </div>
    {/each}
  </div>
  <div class="pick-actions">
    <button class="btn small" disabled={writing} onclick={() => run.discard()}>Отбросить</button>
    <button class="btn small primary" disabled={writing || run.chosen.length === 0} onclick={writeChosen}>
      {#if writing}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
      <span>Записать выбранные · {run.chosen.length}</span>
    </button>
  </div>
{:else}
<div class="list">
  {#if shown.length > 0}
    <div class="token-row head">
      <span>Токен</span>
      <span>Значение</span>
      <span>Пояснение для модели</span>
    </div>
  {/if}
  {#each shown as token (token.name)}
    <div class="token-row" class:busy={busy === token.name}>
      <div class="name">
        <code>[{token.name}]</code>
        <span>реплик: {token.lines}</span>
      </div>
      <input
        class="input"
        placeholder="Как в игре: mother"
        value={token.value ?? ""}
        onchange={(event) => save(token, event.currentTarget.value, token.note ?? "")}
        aria-label="Значение токена {token.name}"
      />
      <input
        class="input"
        placeholder={token.value ? "Не нужно: есть значение" : "Например: имя героя, вводит игрок"}
        value={token.note ?? ""}
        disabled={Boolean(token.value)}
        onchange={(event) => save(token, token.value ?? "", event.currentTarget.value)}
        aria-label="Пояснение токена {token.name}"
      />
    </div>
  {:else}
    <div class="empty">
      <strong>Токенов нет</strong>
      <span>В тексте игры нет переменных в квадратных скобках</span>
    </div>
  {/each}
</div>
{/if}

<style>
  .token-row {
    display: grid;
    grid-template-columns: minmax(140px, 220px) 1fr 1.4fr;
    gap: 10px;
    align-items: center;
    padding: 6px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .token-row.head {
    color: var(--text-muted);
    font-size: 11px;
  }

  .token-row.busy {
    opacity: 0.55;
  }

  .name {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name code {
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .name span {
    color: var(--text-faint);
    font-size: 11px;
  }

  .search {
    width: 220px;
  }
</style>
