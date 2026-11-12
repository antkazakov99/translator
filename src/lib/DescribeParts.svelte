<script lang="ts">
  import { untrack } from "svelte";
  import { Eraser, LoaderCircle, Sparkles, Undo2 } from "lucide-svelte";
  import { commands, type EntryPartsDto_Serialize as EntryParts, type OwnerDto } from "./bindings";
  import { runsBusy, type GlossaryRuns } from "./glossary";
  import { sceneLabel } from "./scene-graph";
  import { depthChanges } from "./reading-depth.svelte";

  /**
   * Кнопки-иконки шапки записи: описать персонажа или термин моделью по частям его сцен, сбросить заметки модели,
   * вернуть исключённые раньше сцены. Итог запуска показывает раздел.
   */
  let {
    projectId,
    owner,
    runs,
    confirming = $bindable(false),
    onDescribed,
    onError,
  }: {
    projectId: string;
    owner: OwnerDto;
    runs: GlossaryRuns;
    /** Открыт вопрос «Сбросить заметки?»: шапка на это время прячет остальные действия. */
    confirming?: boolean;
    /** Описание записано: карточка перечитывает запись. */
    onDescribed: () => Promise<void>;
    onError: (message: string) => void;
  } = $props();

  const run = $derived(runs.entries);
  let parts = $state<EntryParts | null>(null);

  /** Перечитывается при смене записи, а не при каждом новом объекте `owner` от карточки. */
  const ownerKey = $derived(`${owner.kind}:${owner.id}`);

  $effect(() => {
    ownerKey;
    // Новая глубина чтения меняет части следующего запуска.
    depthChanges.count;
    untrack(() => {
      confirming = false;
      parts = null;
      load(owner);
    });
  });

  async function load(current: OwnerDto = owner) {
    try {
      const found = await commands.entryParts(projectId, current);
      if (current.kind === owner.kind && current.id === owner.id) parts = found;
    } catch (e) {
      onError(String(e));
    }
  }

  const read = $derived(parts?.parts.filter((part) => part.note !== null).length ?? 0);

  async function describe() {
    const current = owner;
    const written = await run.start(
      (channel) => commands.describeEntry(projectId, current, null, channel),
      onError,
      (count, stopped) => {
        const text = count > 0 ? "Описание записано" : "Модель не нашла сведений для описания";
        return stopped ? `Остановлено. ${text}` : text;
      },
    );
    if (written.length > 0) await onDescribed();
    await load(current);
  }

  async function resetNotes() {
    confirming = false;
    try {
      await commands.resetEntryNotes(projectId, owner);
      await load();
    } catch (e) {
      onError(String(e));
    }
  }

  /** Сцены, исключённые раньше, снова идут модели. */
  async function restoreScenes() {
    const current = owner;
    try {
      for (const key of parts?.excluded ?? []) await commands.excludeScene(projectId, current, key, false);
    } catch (e) {
      onError(String(e));
    }
    await load(current);
  }

  function describeTitle() {
    if (!parts) return "Описать моделью";
    if (parts.parts.length === 0) return "Описать моделью. Модели нечего читать: сцен нет или все исключены";
    if (runsBusy(runs)) return "Описать моделью. Модель уже работает или ждёт отбор";
    return `Описать моделью. Частей сцен: ${parts.parts.length}, прочитано: ${read}, запуск прочитает: ${parts.picked}. Описание заменится`;
  }
</script>

{#if confirming}
  <span class="confirm-text" title="Следующее описание прочитает сцены заново">Сбросить заметки?</span>
  <button class="btn small danger" onclick={resetNotes}>Да</button>
  <button class="btn small" onclick={() => (confirming = false)}>Нет</button>
{:else}
  <button
    class="icon-button"
    disabled={!parts || parts.parts.length === 0 || runsBusy(runs)}
    aria-label="Описать моделью"
    title={describeTitle()}
    onclick={describe}
  >
    {#if run.active}<LoaderCircle size={14} class="spin" />{:else}<Sparkles size={14} />{/if}
  </button>
  {#if parts && read > 0}
    <button
      class="icon-button"
      aria-label="Сбросить заметки модели"
      title="Сбросить заметки модели: следующее описание прочитает сцены заново"
      onclick={() => (confirming = true)}><Eraser size={14} /></button
    >
  {/if}
  {#if parts && parts.excluded.length > 0}
    <button
      class="icon-button"
      aria-label="Вернуть исключённые сцены"
      title="Вернуть исключённые сцены: {parts.excluded.map(sceneLabel).join(', ')}"
      onclick={restoreScenes}><Undo2 size={14} /></button
    >
  {/if}
{/if}
