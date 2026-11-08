<script lang="ts">
  import { commands } from "./bindings";
  import { matches, type Glossary, type Token } from "./glossary";

  let {
    projectId,
    data,
    patch,
    onError,
  }: { projectId: string; data: Glossary; patch: (token: Token) => void; onError: (message: string) => void } = $props();

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
</div>

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
