<script lang="ts">
  import { Link, LoaderCircle, MessageSquare, Undo2, UserPlus, Users, X, Trash2 } from "lucide-svelte";
  import Combobox from "./Combobox.svelte";
  import { commands, type Gender } from "./bindings";
  import { characterName, genders, matches, speakerCode, speakerName, type Glossary, type Speaker } from "./glossary";

  let {
    projectId,
    data,
    reload,
    onError,
    onOpenCharacter,
  }: {
    projectId: string;
    data: Glossary;
    reload: () => Promise<void>;
    onError: (message: string) => void;
    onOpenCharacter: (id: number) => void;
  } = $props();

  type Filter = Speaker["state"];
  const filters: { value: Filter; label: string }[] = [
    { value: "new", label: "Новые" },
    { value: "character", label: "Персонажи" },
    { value: "speaker", label: "Говорящие" },
    { value: "discarded", label: "Корзина" },
  ];

  let filter = $state<Filter>("new");
  let query = $state("");
  /** Говорящий, у которого открыт выбор персонажа. */
  let linking = $state<string | null>(null);
  let linkText = $state("");
  /** Говорящий, с которым идёт действие. */
  let busy = $state<string | null>(null);

  const count = (state: Filter) => data.speakers.filter((speaker) => speaker.state === state).length;
  const shown = $derived(data.speakers.filter((speaker) => speaker.state === filter && matches(query, speaker.label, speaker.key)));
  const characterById = $derived(new Map(data.characters.map((character) => [character.id, character])));

  async function act(key: string, work: () => Promise<unknown>) {
    busy = key;
    try {
      await work();
      await reload();
    } catch (e) {
      onError(String(e));
    } finally {
      busy = null;
    }
  }

  function linkOptions(speaker: Speaker) {
    return (all: boolean) =>
      data.characters
        .filter((character) => !speaker.characters.includes(character.id))
        .filter((character) => all || matches(linkText, characterName(character), character.translatedFirstName))
        .map((character) => ({ id: String(character.id), label: characterName(character) }));
  }

  function pick(speaker: Speaker, id: string) {
    linking = null;
    linkText = "";
    act(speaker.key, () => commands.linkSpeaker(projectId, speaker.key, [Number(id)]));
  }

  function saveEntry(speaker: Speaker, translation: string, gender: Gender | null) {
    act(speaker.key, () => commands.saveSpeakerEntry(projectId, speaker.key, translation, gender));
  }
</script>

<div class="toolbar">
  <div class="toolbar-title">
    <strong>Говорящие</strong>
    <span>Кто говорит в игре. Говорящего связывают с персонажем или оставляют ролью со своим переводом подписи</span>
  </div>
  {#if filter === "new" && count("new") > 0}
    <button class="btn small primary" disabled={busy !== null} onclick={() => act("all", () => commands.allSpeakersToCharacters(projectId))}>
      {#if busy === "all"}<LoaderCircle size={14} class="spin" />{:else}<Users size={14} />{/if}
      <span>Все в персонажи · {count("new")}</span>
    </button>
  {/if}
</div>

<div class="filters">
  <div class="segmented" role="tablist" aria-label="Какие говорящие">
    {#each filters as item (item.value)}
      <button role="tab" aria-selected={filter === item.value} class:on={filter === item.value} onclick={() => (filter = item.value)}>
        {item.label} · {count(item.value)}
      </button>
    {/each}
  </div>
  <input class="input search" placeholder="Поиск по подписи" bind:value={query} aria-label="Поиск говорящих" />
</div>

<div class="list">
  {#each shown as speaker (speaker.key)}
    <div class="speaker-row" class:busy={busy === speaker.key}>
      <div class="name">
        <strong title={speaker.key}>{speakerName(speaker)}</strong>
        <span class="meta" title={speaker.key}>
          {#if speakerCode(speaker)}<code>{speakerCode(speaker)}</code>{" · "}{/if}реплик: {speaker.lines}
        </span>
      </div>

      <div class="detail">
        {#if speaker.state === "character"}
          {#each speaker.characters as id, index (id)}
            {@const character = characterById.get(id)}
            <span class="badge" class:accent={index === 0} title={index === 0 ? "Основной код персонажа" : undefined}>
              <button class="link" onclick={() => onOpenCharacter(id)}>{character ? characterName(character) : id}</button>
              <button class="unlink" aria-label="Отвязать от персонажа" title="Отвязать от персонажа" onclick={() => act(speaker.key, () => commands.unlinkSpeaker(projectId, speaker.key, id))}>
                <X size={11} />
              </button>
            </span>
          {/each}
        {/if}
        {#if speaker.hasEntry}
          <input
            class="input entry"
            placeholder="Перевод подписи"
            value={speaker.translation ?? ""}
            onchange={(event) => saveEntry(speaker, event.currentTarget.value, speaker.gender)}
            aria-label="Перевод подписи"
          />
          <div class="segmented gender" role="radiogroup" aria-label="Род">
            {#each genders as item (item.label)}
              <button
                role="radio"
                aria-checked={speaker.gender === item.value}
                class:on={speaker.gender === item.value}
                onclick={() => saveEntry(speaker, speaker.translation ?? "", item.value)}>{item.label}</button
              >
            {/each}
          </div>
        {/if}
      </div>

      <div class="actions">
        {#if speaker.state === "discarded"}
          <button class="btn small" onclick={() => act(speaker.key, () => commands.restoreSpeaker(projectId, speaker.key))}>
            <Undo2 size={13} />
            <span>Вернуть</span>
          </button>
        {:else}
          {#if speaker.state === "new"}
            <button class="btn small" title="Создать персонажа с этим именем" onclick={() => act(speaker.key, () => commands.speakerToCharacter(projectId, speaker.key))}>
              <UserPlus size={13} />
              <span>Персонаж</span>
            </button>
          {/if}
          <button
            class="btn small"
            title="Связать с персонажем из глоссария"
            disabled={data.characters.length === 0}
            onclick={() => (linking = linking === speaker.key ? null : speaker.key)}
          >
            <Link size={13} />
            <span>Привязать</span>
          </button>
          {#if speaker.hasEntry}
            <button class="icon-button" aria-label="Убрать свой перевод подписи" title="Убрать свой перевод подписи" onclick={() => act(speaker.key, () => commands.deleteSpeakerEntry(projectId, speaker.key))}>
              <X size={13} />
            </button>
          {:else}
            <button class="btn small" title="Роль без персонажа: свой перевод подписи и род" onclick={() => act(speaker.key, () => commands.keepAsSpeaker(projectId, speaker.key))}>
              <MessageSquare size={13} />
              <span>Подпись</span>
            </button>
          {/if}
          {#if speaker.state === "new"}
            <button class="icon-button danger" aria-label="В корзину" title="В корзину: служебный код или лишняя подпись" onclick={() => act(speaker.key, () => commands.discardSpeaker(projectId, speaker.key))}>
              <Trash2 size={13} />
            </button>
          {/if}
        {/if}
      </div>

      {#if linking === speaker.key}
        <div class="picker">
          <span>С кем связать подпись «{speakerName(speaker)}»</span>
          <div class="picker-box">
            <Combobox
              text={linkText}
              options={linkOptions(speaker)}
              label="Персонаж"
              placeholder="Имя персонажа"
              empty="Персонажей нет"
              oninput={(text) => (linkText = text)}
              onpick={(id) => pick(speaker, id)}
            />
          </div>
          <button class="icon-button" aria-label="Закрыть выбор" title="Закрыть выбор" onclick={() => (linking = null)}><X size={13} /></button>
        </div>
      {/if}
    </div>
  {:else}
    <div class="empty">
      <strong>{filter === "new" ? "Все говорящие разобраны" : "Здесь пусто"}</strong>
      <span>{filter === "new" ? "Новые появятся после считывания новой версии игры" : "Говорящие этого вида появятся после разбора «Новых»"}</span>
    </div>
  {/each}
</div>

<style>
  .filters {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 10px 16px 0;
  }

  .search {
    flex: 1;
    max-width: 280px;
  }

  .speaker-row {
    display: grid;
    grid-template-columns: minmax(160px, 240px) 1fr auto;
    gap: 12px;
    align-items: center;
    padding: 8px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .speaker-row:hover {
    background: var(--hover-row);
  }

  .speaker-row.busy {
    opacity: 0.55;
  }

  .name {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .name strong {
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    overflow: hidden;
    color: var(--text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta code {
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--raised);
    font-size: 11px;
  }

  .detail {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    min-width: 0;
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

  .entry {
    width: 200px;
  }

  .picker {
    display: flex;
    grid-column: 1 / -1;
    gap: 8px;
    align-items: center;
    color: var(--text-muted);
    font-size: 11px;
  }

  .picker-box {
    width: 280px;
  }

  .actions {
    display: flex;
    gap: 6px;
    align-items: center;
    justify-content: flex-end;
  }
</style>
