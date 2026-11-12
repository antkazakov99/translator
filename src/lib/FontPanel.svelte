<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { ChevronDown, ChevronRight, LoaderCircle, Trash2, TriangleAlert, Upload } from "lucide-svelte";
  import { commands } from "./bindings";
  import {
    ORIGINAL_SAMPLE,
    TARGET_SAMPLE,
    fontOrder,
    fontStatus,
    needsReplacement,
    sampleSegments,
    type FontCoverage,
    type GameFont,
  } from "./font-preview";

  /** Раздел «Шрифты» окна «Игра»: шрифты игры и их замены для языка перевода. Замены лежат в проекте и уходят в мод. */
  let { projectId }: { projectId: string } = $props();

  type Preview = { family: string } | { failed: true };

  let fonts = $state<GameFont[] | null>(null);
  let error = $state("");
  let busy = $state("");
  let showUnused = $state(false);
  /** Предпросмотр по хэшу файла: шрифт игры и замена грузятся в браузер один раз. */
  let previews = $state<Record<string, Preview>>({});
  let faces: FontFace[] = [];
  /** Файлы, которые сейчас грузятся: второй запрос за тем же файлом не нужен. */
  const pending = new Set<string>();
  let nextFamily = 0;
  let alive = true;

  const used = $derived((fonts ?? []).filter((font) => font.references > 0).sort(fontOrder));
  const unused = $derived((fonts ?? []).filter((font) => font.references === 0));
  const waiting = $derived(used.filter(needsReplacement).length);

  onMount(load);

  onDestroy(() => {
    alive = false;
    for (const face of faces) document.fonts.delete(face);
  });

  async function load() {
    try {
      fonts = await commands.gameFonts(projectId);
      error = "";
      for (const font of fonts) {
        void preview(font.hash, font.coverage);
        if (font.replacement) void preview(font.replacement.hash, font.replacement.coverage);
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function preview(hash: string, coverage: FontCoverage) {
    if (previews[hash] || pending.has(hash)) return;
    if (coverage.unreadable) {
      previews[hash] = { failed: true };
      return;
    }
    pending.add(hash);
    try {
      const data = await commands.fontData(projectId, hash);
      const bytes = Uint8Array.from(atob(data), (char) => char.charCodeAt(0));
      const family = `translator-font-${nextFamily++}`;
      const face = new FontFace(family, bytes);
      await face.load();
      if (!alive) return;
      document.fonts.add(face);
      faces.push(face);
      previews[hash] = { family };
    } catch {
      // Браузер отклоняет часть файлов, которые читает движок, например коллекции TTC.
      previews[hash] = { failed: true };
    } finally {
      pending.delete(hash);
    }
  }

  async function change(font: GameFont, action: () => Promise<unknown>) {
    busy = font.path;
    try {
      await action();
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = "";
    }
  }

  async function choose(font: GameFont) {
    const picked = await openDialog({
      title: `Шрифт вместо ${font.name}`,
      multiple: false,
      filters: [{ name: "Шрифты TTF и OTF", extensions: ["ttf", "otf", "TTF", "OTF"] }],
    });
    if (typeof picked === "string") await change(font, () => commands.replaceFont(projectId, font.path, picked));
  }
</script>

{#snippet sample(label: string, text: string, missing: string, shown: Preview | undefined)}
  <div class="sample">
    <span class="sample-label">{label}</span>
    {#if shown && "family" in shown}
      <p class="sample-text" style:font-family={`"${shown.family}"`}>
        {#each sampleSegments(text, missing) as segment, index (index)}
          {#if segment.missing}<span class="sample-missing" title="Этого символа нет в шрифте">{"□".repeat([...segment.text].length)}</span>{:else}{segment.text}{/if}
        {/each}
      </p>
    {:else}
      <p class="faint">{shown ? "Предпросмотр недоступен" : "Загрузка"}</p>
    {/if}
  </div>
{/snippet}

{#snippet samples(hash: string, coverage: FontCoverage)}
  {@render sample(ORIGINAL_SAMPLE.label, ORIGINAL_SAMPLE.text, coverage.missingOriginal, previews[hash])}
  {@render sample(TARGET_SAMPLE.label, TARGET_SAMPLE.text, coverage.missingTarget, previews[hash])}
{/snippet}

{#snippet card(font: GameFont)}
  {@const status = fontStatus(font.coverage)}
  <article class="font" class:waiting={needsReplacement(font)}>
    <div class="font-head">
      <div class="font-title">
        <strong>{font.name}</strong>
        <span>{font.references > 0 ? `Упоминается в скриптах: ${font.references}` : "В скриптах не упоминается"}</span>
      </div>
      <span class="badge" class:warn={status.kind === "letters" || status.kind === "marks"}>{status.label}</span>
    </div>
    {@render samples(font.hash, font.coverage)}

    {#if font.replacement}
      {@const replaced = fontStatus(font.replacement.coverage)}
      <div class="replacement">
        <div class="font-head">
          <strong>Замена: {font.replacement.sourceName}</strong>
          <span class="badge" class:warn={replaced.kind !== "ok"}>{replaced.label}</span>
        </div>
        {@render samples(font.replacement.hash, font.replacement.coverage)}
      </div>
    {/if}

    {#if font.locked}
      <p class="note warn-note"><TriangleAlert size={13} /> {font.locked}. Замена в моде не подействует</p>
    {/if}

    <div class="font-actions">
      <button class="btn small" disabled={busy !== "" || Boolean(font.locked)} onclick={() => choose(font)}>
        {#if busy === font.path}<LoaderCircle size={13} class="spin" />{:else}<Upload size={13} />{/if}
        <span>{font.replacement ? "Другой файл" : "Выбрать замену"}</span>
      </button>
      {#if font.replacement}
        <button class="btn small" disabled={busy !== ""} onclick={() => change(font, () => commands.removeFontReplacement(projectId, font.path))}>
          <Trash2 size={13} />
          <span>Убрать замену</span>
        </button>
      {/if}
    </div>
  </article>
{/snippet}

<div class="fonts">
  <div class="fonts-head">
    <strong>Шрифты</strong>
    {#if waiting > 0}<span class="badge warn">Нужна замена: {waiting}</span>{/if}
  </div>
  <p class="faint">
    Замена ложится в архив мода в tl/russian под именем исходного шрифта и действует только на русском. Файлы игры не меняются. Новый шрифт
    проверьте в игре: его ширина может отличаться
  </p>

  {#if error}
    <div class="banner error" role="alert"><TriangleAlert size={14} /><span>{error}</span></div>
  {/if}

  {#if !fonts}
    <p class="faint"><LoaderCircle size={13} class="spin" /> Проверяю шрифты игры</p>
  {:else if fonts.length === 0}
    <p class="faint">В игре нет своих файлов шрифтов: она берёт шрифт движка, а в нём кириллица есть</p>
  {:else}
    {#each used as font (font.path)}
      {@render card(font)}
    {:else}
      <p class="faint">Скрипты не упоминают ни один шрифт из файлов игры</p>
    {/each}
    {#if unused.length > 0}
      <button class="disclosure" onclick={() => (showUnused = !showUnused)} aria-expanded={showUnused}>
        {#if showUnused}<ChevronDown size={13} />{:else}<ChevronRight size={13} />{/if}
        <span>Не упоминаются в скриптах: {unused.length}</span>
      </button>
      {#if showUnused}
        <p class="faint">Игра может обращаться к ним по имени, собранному в коде, или не использовать совсем</p>
        {#each unused as font (font.path)}
          {@render card(font)}
        {/each}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .fonts {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .fonts-head,
  .font-head {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
  }

  .fonts-head strong {
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .font-title strong,
  .font-head strong {
    color: var(--text-strong);
    font-size: 12px;
    font-weight: 600;
  }

  .font {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  .font.waiting {
    border-color: var(--warning-border);
  }

  .font-title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .font-title span,
  .faint,
  .sample-label {
    color: var(--text-faint);
    font-size: 11px;
  }

  .faint {
    margin: 0;
  }

  .badge.warn {
    border-color: var(--warning-border);
    background: var(--warning-bg);
    color: var(--warning);
  }

  .sample {
    display: grid;
    grid-template-columns: 70px 1fr;
    gap: 8px;
    align-items: baseline;
  }

  .sample-text {
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--text);
    font-size: 16px;
  }

  .sample-missing {
    color: var(--danger);
  }

  .replacement {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 6px;
    border-top: 1px dashed var(--divider);
  }

  .note {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 0;
    font-size: 11px;
  }

  .warn-note {
    color: var(--warning);
  }

  .font-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .disclosure {
    display: flex;
    gap: 4px;
    align-items: center;
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-soft);
    font-size: 12px;
    cursor: pointer;
  }
</style>
