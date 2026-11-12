<script lang="ts">
  import { Check, LoaderCircle, Sparkles } from "lucide-svelte";
  import EntryLines from "./EntryLines.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type EntryLineDto_Serialize as EntryLine } from "./bindings";
  import { matches, runsBusy, type Glossary, type GlossaryRuns, type Token } from "./glossary";

  let {
    projectId,
    data,
    runs,
    reload,
    patch,
    onOpenLine,
    onError,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    reload: () => Promise<void>;
    patch: (token: Token) => void;
    /** Реплика токена в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.tokens);
  const isEmpty = (token: Token) => !token.value?.trim() && !token.note?.trim();
  /** Токены из текста игры без значения и пояснения: модели есть что читать, а глоссарию нечего показать. */
  const empty = $derived(data.tokens.filter((token) => token.lines > 0 && isEmpty(token)));
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

  /** Отмеченное пишет `accept_tokens`, та же запись, что у автоперевода: значение и пояснение ложатся в пустые поля. */
  async function writeChosen() {
    const chosen = run.chosen
      .filter((item) => item.value?.trim() || item.note?.trim())
      .map((item) => ({ name: item.name, value: item.value?.trim() || null, note: item.note?.trim() || null }));
    writing = true;
    try {
      const written = await commands.acceptTokens(projectId, chosen);
      run.finish(`Записано токенов: ${written}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  type Filter = "all" | "empty";
  let filter = $state<Filter>("all");
  let query = $state("");
  /** Имя открытого токена. `null`: первый в списке. */
  let selected = $state<string | null>(null);
  let busy = $state(false);
  let draftValue = $state("");
  let draftNote = $state("");

  const shown = $derived(
    data.tokens.filter((token) => (filter === "all" || isEmpty(token)) && matches(query, token.name, token.value, token.note)),
  );
  const current = $derived(shown.find((token) => token.name === selected) ?? shown[0] ?? null);

  /** Что последним пришло в поля: глоссарий перечитывается по ходу запуска модели, набранное при этом не теряется. */
  let seen = { name: "", value: "", note: "" };
  $effect.pre(() => {
    const name = current?.name ?? "";
    const value = current?.value ?? "";
    const note = current?.note ?? "";
    if (name !== seen.name || value !== seen.value) draftValue = value;
    if (name !== seen.name || note !== seen.note) draftNote = note;
    seen = { name, value, note };
  });

  /** Вторая строка пункта списка: значение или пояснение и число реплик. */
  function subtitle(token: Token) {
    const what = token.value ? `= ${token.value}` : (token.note ?? "Без значения");
    return `${what} · реплик: ${token.lines}`;
  }

  async function save(token: Token, value: string, note: string) {
    if (value.trim() === (token.value ?? "") && note.trim() === (token.note ?? "")) return;
    busy = true;
    try {
      await commands.saveToken(projectId, token.name, value, note);
      patch({ ...token, value: value.trim() || null, note: note.trim() || null });
    } catch (e) {
      onError(String(e));
    } finally {
      busy = false;
    }
  }

  function emptyText(): [string, string] {
    if (query.trim()) return ["Ничего не нашлось", "Измените запрос поиска"];
    if (filter === "empty") return ["Пустых токенов нет", "У каждого токена есть значение или пояснение"];
    return ["Токенов нет", "В тексте игры нет переменных в квадратных скобках"];
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Токены</strong>
    <span>Переменные в тексте игры. Значение подставляется в реплику перед переводом, токен без значения остаётся в тексте, а модель видит пояснение</span>
  </div>
  <div class="segmented" role="tablist" aria-label="Какие токены">
    <button role="tab" aria-selected={filter === "all"} class:on={filter === "all"} onclick={() => (filter = "all")}>Все · {data.tokens.length}</button>
    <button role="tab" aria-selected={filter === "empty"} class:on={filter === "empty"} onclick={() => (filter = "empty")}>
      Пустые · {data.tokens.filter(isEmpty).length}
    </button>
  </div>
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
          <strong><code class="code">[{item.name}]</code></strong>
          <span>Реплик: {token?.lines ?? 0}</span>
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
  <!-- Та же раскладка, что у говорящих, персонажей и терминов: список, запись токена, его реплики. -->
  <div class="master with-lines">
    <div class="list-side">
      <input class="input" placeholder="Поиск по имени, значению и пояснению" bind:value={query} aria-label="Поиск токенов" />
      <div class="items">
        {#each shown as token (token.name)}
          <button class="item" class:active={token.name === current?.name} onclick={() => (selected = token.name)}>
            <strong><code class="code">[{token.name}]</code></strong>
            <span>{subtitle(token)}</span>
          </button>
        {:else}
          <p class="faint">{emptyText()[0]}</p>
        {/each}
      </div>
    </div>

    {#if current}
      {@const token = current}
      <div class="editor" class:saving={busy}>
        <div class="editor-head">
          <strong><code class="code">[{token.name}]</code></strong>
        </div>
        <label class="field"
          >Значение<input
            class="input"
            placeholder="Как в игре: mother"
            bind:value={draftValue}
            onchange={() => save(token, draftValue, draftNote)}
          /></label
        >
        <label class="field"
          >Пояснение для модели<input
            class="input"
            placeholder={draftValue.trim() ? "Не нужно: есть значение" : "Например: имя героя, вводит игрок"}
            bind:value={draftNote}
            disabled={Boolean(draftValue.trim())}
            onchange={() => save(token, draftValue, draftNote)}
          /></label
        >
        <span class="faint">
          {draftValue.trim() ? "Значение подставляется в реплику перед переводом" : "Без значения токен остаётся в тексте как есть"}
        </span>
      </div>
      <EntryLines
        subject={`token:${token.name}`}
        load={() => commands.tokenLines(projectId, current?.name ?? "")}
        empty="В этой версии игры токен в репликах не встречается"
        {onOpenLine}
        {onError}
      />
    {:else}
      <div class="empty">
        <strong>{emptyText()[0]}</strong>
        <span>{emptyText()[1]}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* Имя токена акцентным моноширинным, как код говорящего. */
  .code {
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 400;
  }
</style>
