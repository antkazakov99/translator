<script lang="ts">
  import { Check, Languages, Link, LoaderCircle, Trash2, Undo2, UserPlus, Users, X } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import EntryLines from "./EntryLines.svelte";
  import SuggestStatus from "./SuggestStatus.svelte";
  import { commands, type EntryLineDto_Serialize as EntryLine, type Gender } from "./bindings";
  import {
    characterName,
    genders,
    matches,
    runsBusy,
    speakerCode,
    speakerName,
    type Glossary,
    type GlossaryRuns,
    type Speaker,
  } from "./glossary";

  let {
    projectId,
    data,
    runs,
    reload,
    onError,
    onOpenCharacter,
    onOpenLine,
  }: {
    projectId: string;
    data: Glossary;
    runs: GlossaryRuns;
    reload: () => Promise<void>;
    onError: (message: string) => void;
    onOpenCharacter: (id: number) => void;
    /** Реплика говорящего в таблице перевода или в исходнике. */
    onOpenLine: (line: EntryLine, where: "table" | "source") => void;
  } = $props();

  const run = $derived(runs.speakers);
  /** Говорящие без персонажа с репликами и без перевода подписи: их подписи предложит перевести модель. */
  const untranslated = $derived(data.speakers.filter((speaker) => speaker.state === "speaker" && speaker.lines > 0 && !speaker.translation?.trim()));
  /** Говорящие без своего перевода подписи и рода: «Все в персонажи» берёт их. */
  const plain = $derived(data.speakers.filter((speaker) => speaker.state === "speaker" && !speaker.hasEntry));
  let writing = $state(false);

  function runTitle() {
    if (untranslated.length === 0) return "У всех говорящих без персонажа подпись переведена";
    if (runsBusy(runs)) return "Модель уже работает или ждёт отбор";
    return "Модель предложит перевод подписи и род по нескольким репликам говорящего без персонажа. Записывается только отмеченное в отборе";
  }

  function startRun() {
    const keys = untranslated.map((speaker) => speaker.key);
    run.start((channel) => commands.suggestSpeakers(projectId, keys, null, channel), onError);
  }

  /** Отмеченное пишет `accept_speakers`, та же запись, что у автоперевода: перевод и род ложатся в пустые поля. */
  async function writeChosen() {
    const chosen = run.chosen
      .filter((item) => item.translation.trim())
      .map((item) => ({ key: item.key, translation: item.translation.trim(), gender: item.gender }));
    writing = true;
    try {
      const written = await commands.acceptSpeakers(projectId, chosen);
      run.finish(`Записано подписей: ${written}`);
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      writing = false;
    }
  }

  type Filter = Speaker["state"];
  const filters: { value: Filter; label: string }[] = [
    { value: "speaker", label: "Говорящие" },
    { value: "character", label: "Персонажи" },
    { value: "discarded", label: "Корзина" },
  ];

  let filter = $state<Filter>("speaker");
  let query = $state("");
  /** Ключ открытого говорящего. `null`: первый в списке. */
  let selected = $state<string | null>(null);
  /** Открыт выбор персонажа для связи. */
  let linking = $state(false);
  let linkText = $state("");
  /** Идёт действие с открытым говорящим или со всеми. */
  let busy = $state(false);
  /** Перевод подписи в правке. */
  let draftText = $state("");

  const characterById = $derived(new Map(data.characters.map((character) => [character.id, character])));
  const count = (state: Filter) => data.speakers.filter((speaker) => speaker.state === state).length;
  const shown = $derived(
    data.speakers.filter((speaker) => speaker.state === filter && matches(query, speaker.label, speaker.key, speaker.translation, ...owners(speaker))),
  );
  const current = $derived(shown.find((speaker) => speaker.key === selected) ?? shown[0] ?? null);

  function owners(speaker: Speaker) {
    return speaker.characters.map((id) => {
      const character = characterById.get(id);
      return character ? characterName(character) : "";
    });
  }

  /** Что последним пришло в поле: глоссарий перечитывается по ходу запуска модели, набранное при этом не теряется. */
  let seen = { key: "", translation: "" };
  $effect.pre(() => {
    const key = current?.key ?? "";
    const translation = current?.translation ?? "";
    if (key !== seen.key) {
      linking = false;
      linkText = "";
    }
    if (key !== seen.key || translation !== seen.translation) draftText = translation;
    seen = { key, translation };
  });

  /** Вторая строка пункта списка: перевод подписи или персонажи и число реплик. Код стоит рядом с подписью. */
  function subtitle(speaker: Speaker) {
    const parts: string[] = [];
    if (speaker.state === "character") parts.push(owners(speaker).filter(Boolean).join(", "));
    else if (speaker.translation) parts.push(speaker.translation);
    parts.push(`реплик: ${speaker.lines}`);
    return parts.filter(Boolean).join(" · ");
  }

  /** Действие с говорящим. `leaves`: он уйдёт из списка раздела, открытым останется сосед на том же месте. */
  async function act(work: () => Promise<unknown>, leaves = false) {
    const index = current ? shown.findIndex((speaker) => speaker.key === current.key) : -1;
    busy = true;
    try {
      await work();
      await reload();
      if (leaves) selected = shown[Math.min(Math.max(index, 0), shown.length - 1)]?.key ?? null;
    } catch (e) {
      onError(String(e));
    } finally {
      busy = false;
    }
  }

  /** Свой перевод подписи и род. Пустые оба: своей записи у говорящего нет. */
  function saveEntry(speaker: Speaker, translation: string, gender: Gender | null) {
    const empty = !translation.trim() && gender === null;
    if (empty && !speaker.hasEntry) return;
    act(() => (empty ? commands.deleteSpeakerEntry(projectId, speaker.key) : commands.saveSpeakerEntry(projectId, speaker.key, translation.trim(), gender)));
  }

  function linkOptions(speaker: Speaker) {
    return (all: boolean) =>
      data.characters
        .filter((character) => !speaker.characters.includes(character.id))
        .filter((character) => all || matches(linkText, characterName(character), character.translatedFirstName))
        .map((character) => ({ id: String(character.id), label: characterName(character) }));
  }

  function pick(speaker: Speaker, id: string) {
    linking = false;
    linkText = "";
    act(() => commands.linkSpeaker(projectId, speaker.key, [Number(id)]), speaker.state === "speaker");
  }

  function emptyText(): [string, string] {
    if (query.trim()) return ["Никого не нашлось", "Измените запрос поиска"];
    if (filter === "speaker") return ["Говорящих без персонажа нет", "Все говорящие игры связаны с персонажами или лежат в корзине"];
    if (filter === "character") return ["Говорящих с персонажем нет", "Сделайте говорящего персонажем или привяжите его к персонажу из глоссария"];
    return ["Корзина пуста", "Сюда уходят служебные коды и лишние подписи"];
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Говорящие</strong>
    <span>Кто говорит в игре: персонаж, роль со своим переводом подписи или служебный код</span>
  </div>
  <div class="segmented" role="tablist" aria-label="Какие говорящие">
    {#each filters as item (item.value)}
      <button role="tab" aria-selected={filter === item.value} class:on={filter === item.value} onclick={() => (filter = item.value)}>
        {item.label} · {count(item.value)}
      </button>
    {/each}
  </div>
  {#if filter === "speaker" && plain.length > 0}
    <button
      class="btn small"
      disabled={busy}
      title="Каждый говорящий без своего перевода подписи и рода станет персонажем с именем из подписи"
      onclick={() => act(() => commands.allSpeakersToCharacters(projectId), true)}
    >
      {#if busy}<LoaderCircle size={14} class="spin" />{:else}<Users size={14} />{/if}
      <span>Все в персонажи · {plain.length}</span>
    </button>
  {/if}
  <button class="btn small primary" disabled={untranslated.length === 0 || runsBusy(runs)} title={runTitle()} onclick={startRun}>
    {#if run.active}<LoaderCircle size={14} class="spin" />{:else}<Languages size={14} />{/if}
    <span>Перевести подписи · {untranslated.length}</span>
  </button>
</div>

<SuggestStatus {run} {onError} />

{#if run.review}
  <div class="list">
    <p class="pick-head">Предложения модели. Отмеченные запишутся в подписи говорящих, перевод и род можно поправить</p>
    {#each run.review as item (item.key)}
      {@const speaker = data.speakers.find((known) => known.key === item.key)}
      {@const name = speaker ? speakerName(speaker) : item.key}
      <div class="pick-row speakers" class:off={!item.selected}>
        <input type="checkbox" bind:checked={item.selected} aria-label="Записать перевод «{name}»" />
        <div class="name">
          <strong title={item.key}>{name}</strong>
          <span>Сейчас: {speaker?.translation ?? "нет перевода"}</span>
        </div>
        <input class="input" bind:value={item.translation} placeholder="Перевод подписи" aria-label="Перевод подписи «{name}»" />
        <div class="segmented" role="radiogroup" aria-label="Род «{name}»">
          {#each genders as gender (gender.label)}
            <button role="radio" aria-checked={item.gender === gender.value} class:on={item.gender === gender.value} onclick={() => (item.gender = gender.value)}>
              {gender.label}
            </button>
          {/each}
        </div>
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
  <!-- Та же раскладка, что у персонажей и терминов: список, запись говорящего, его реплики. -->
  <div class="master with-lines">
    <div class="list-side">
      <input class="input" placeholder="Поиск по подписи и переводу" bind:value={query} aria-label="Поиск говорящих" />
      <div class="items">
        {#each shown as speaker (speaker.key)}
          <button class="item" class:active={speaker.key === current?.key} title={speaker.key} onclick={() => (selected = speaker.key)}>
            <strong>{speakerName(speaker)}{#if speakerCode(speaker)}<code class="code">{speakerCode(speaker)}</code>{/if}</strong>
            <span>{subtitle(speaker)}</span>
          </button>
        {:else}
          <p class="faint">{emptyText()[0]}</p>
        {/each}
      </div>
    </div>

    {#if current}
      {@const speaker = current}
      <div class="editor" class:saving={busy}>
        <div class="editor-head">
          <strong title={speaker.key}>{speakerName(speaker)}{#if speakerCode(speaker)}<code class="code">{speakerCode(speaker)}</code>{/if}</strong>
          <div class="editor-tools">
            {#if speaker.state === "discarded"}
              <button
                class="icon-button"
                disabled={busy}
                aria-label="Вернуть из корзины"
                title="Вернуть из корзины в «Говорящие»"
                onclick={() => act(() => commands.restoreSpeaker(projectId, speaker.key), true)}><Undo2 size={14} /></button
              >
            {:else}
              {#if speaker.state === "speaker"}
                <button
                  class="icon-button"
                  disabled={busy}
                  aria-label="Сделать персонажем"
                  title="Персонаж: создать персонажа с именем из подписи"
                  onclick={() => act(() => commands.speakerToCharacter(projectId, speaker.key), true)}><UserPlus size={14} /></button
                >
              {/if}
              <button
                class="icon-button"
                class:active={linking}
                disabled={busy || data.characters.length === 0}
                aria-label="Привязать к персонажу"
                aria-pressed={linking}
                title="Привязать: связать с персонажем из глоссария"
                onclick={() => (linking = !linking)}><Link size={14} /></button
              >
              {#if speaker.state === "speaker"}
                <button
                  class="icon-button danger"
                  disabled={busy}
                  aria-label="В корзину"
                  title="В корзину: служебный код или лишняя подпись. Свой перевод подписи остаётся с говорящим"
                  onclick={() => act(() => commands.discardSpeaker(projectId, speaker.key), true)}><Trash2 size={14} /></button
                >
              {/if}
            {/if}
          </div>
        </div>

        {#if linking}
          <Combobox
            text={linkText}
            options={linkOptions(speaker)}
            label="Персонаж"
            placeholder="С кем связать подпись"
            empty="Персонажей нет"
            oninput={(text) => (linkText = text)}
            onpick={(id) => pick(speaker, id)}
          />
        {/if}

        {#if speaker.state === "character"}
          <div class="field">
            <span>Персонажи</span>
            <div class="badges">
              {#each speaker.characters as id, index (id)}
                {@const character = characterById.get(id)}
                <span class="badge" class:accent={index === 0} title={index === 0 ? "Основной код персонажа" : undefined}>
                  <button class="link" onclick={() => onOpenCharacter(id)}>{character ? characterName(character) : id}</button>
                  <button
                    class="unlink"
                    aria-label="Отвязать от персонажа"
                    title="Отвязать от персонажа"
                    onclick={() => act(() => commands.unlinkSpeaker(projectId, speaker.key, id), speaker.characters.length === 1)}
                  >
                    <X size={11} />
                  </button>
                </span>
              {/each}
            </div>
          </div>
        {/if}

        {#if speaker.state !== "discarded"}
          <label class="field"
            >Перевод подписи<input
              class="input"
              placeholder={speaker.state === "character" ? "По имени персонажа" : "Как подпись будет в переводе"}
              bind:value={draftText}
              onchange={() => saveEntry(speaker, draftText, speaker.gender)}
            /></label
          >
          <div class="field">
            <span>Род</span>
            <div class="segmented" role="radiogroup" aria-label="Род">
              {#each genders as item (item.label)}
                <button
                  role="radio"
                  aria-checked={speaker.gender === item.value}
                  class:on={speaker.gender === item.value}
                  onclick={() => saveEntry(speaker, draftText, item.value)}>{speaker.state === "character" && item.value === null ? "Как у персонажа" : item.label}</button
                >
              {/each}
            </div>
          </div>
        {:else}
          <span class="faint">В корзине: в «Говорящих» его нет, модель его подпись не переводит</span>
        {/if}
      </div>
      <EntryLines
        subject={`speaker:${speaker.key}`}
        load={() => commands.speakerLines(projectId, current?.key ?? "")}
        empty="В этой версии игры у говорящего нет реплик"
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
  /* Код говорящего в скрипте рядом с подписью: акцентным моноширинным, как раньше, и с отступом от подписи. */
  .code {
    margin-left: 6px;
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 400;
  }

  .badge {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }

  .badge .link {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
  }

  .badge .link:hover {
    text-decoration: underline;
  }

  .unlink {
    display: inline-flex;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
  }

  .unlink:hover {
    color: var(--danger);
  }
</style>
