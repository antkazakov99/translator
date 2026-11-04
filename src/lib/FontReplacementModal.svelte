<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { AlertCircle, ChevronDown, ChevronRight, Loader2, Trash2, Type, Upload, X } from "lucide-svelte";
  import {
    ORIGINAL_SAMPLE,
    TARGET_SAMPLE,
    fontOrder,
    fontStatus,
    needsReplacement,
    originLabel,
    sampleSegments,
    type FontCoverage,
    type GameFont
  } from "./font-preview";

  let {
    show = false,
    gameDir,
    onclose = () => {},
    ontoast = () => {}
  }: {
    show: boolean;
    gameDir: string;
    onclose: () => void;
    ontoast: (text: string, type: "success" | "error") => void;
  } = $props();

  type Preview = { family: string } | { failed: true };

  let fonts = $state<GameFont[]>([]);
  let loading = $state(false);
  let loadError = $state("");
  let busyFont = $state("");
  let showUnused = $state(false);
  // Предпросмотр по шрифту и варианту: загруженный файл живёт, пока открыт раздел
  let previews = $state<Record<string, Preview>>({});
  let loadedFaces: FontFace[] = [];
  let nextFamily = 0;
  let session = 0;

  let usedFonts = $derived(fonts.filter((font) => font.references > 0).sort(fontOrder));
  let unusedFonts = $derived(fonts.filter((font) => font.references === 0));
  let waitingCount = $derived(usedFonts.filter(needsReplacement).length);

  const previewKey = (name: string, replacement: boolean) => `${replacement ? "new" : "game"}:${name}`;

  function forgetPreview(key: string) {
    delete previews[key];
  }

  function clearPreviews() {
    for (const face of loadedFaces) document.fonts.delete(face);
    loadedFaces = [];
    previews = {};
  }

  async function loadPreview(font: GameFont, replacement: boolean, current: number) {
    const key = previewKey(font.name, replacement);
    const coverage = replacement ? font.replacement?.coverage : font.coverage;
    if (!coverage || coverage.unreadable) {
      previews[key] = { failed: true };
      return;
    }
    try {
      const data = await invoke<ArrayBuffer>("read_font_file", {
        gameDir,
        name: font.name,
        replacement
      });
      const family = `translator-preview-${nextFamily++}`;
      const face = new FontFace(family, data);
      await face.load();
      if (current !== session) return;
      document.fonts.add(face);
      loadedFaces.push(face);
      previews[key] = { family };
    } catch {
      // Движок читает некоторые файлы, которые браузер отклоняет, например коллекции TTC
      if (current === session) previews[key] = { failed: true };
    }
  }

  function loadPreviews(list: GameFont[], current: number) {
    for (const font of list) {
      if (!previews[previewKey(font.name, false)]) void loadPreview(font, false, current);
      if (font.replacement && !previews[previewKey(font.name, true)]) {
        void loadPreview(font, true, current);
      }
    }
  }

  async function loadFonts() {
    const current = ++session;
    loading = true;
    loadError = "";
    try {
      const list = await invoke<GameFont[]>("list_game_fonts", { gameDir });
      if (current !== session) return;
      fonts = list;
      loadPreviews(list, current);
    } catch (e) {
      if (current === session) loadError = String(e);
    } finally {
      if (current === session) loading = false;
    }
  }

  // Шрифты читаются заново при каждом открытии: игра могла обновиться
  $effect(() => {
    if (show && gameDir) {
      void loadFonts();
    }
    return () => {
      session++;
      clearPreviews();
      fonts = [];
    };
  });

  onDestroy(clearPreviews);

  async function changeReplacement(font: GameFont, action: () => Promise<void>) {
    busyFont = font.name;
    try {
      await action();
      forgetPreview(previewKey(font.name, true));
      const current = session;
      const list = await invoke<GameFont[]>("list_game_fonts", { gameDir });
      if (current !== session) return;
      fonts = list;
      loadPreviews(list, current);
    } catch (e) {
      ontoast(String(e), "error");
    } finally {
      busyFont = "";
    }
  }

  async function chooseReplacement(font: GameFont) {
    const selected = await openDialog({
      title: `Шрифт вместо ${font.name}`,
      multiple: false,
      filters: [{ name: "Шрифты TTF и OTF", extensions: ["ttf", "otf", "TTF", "OTF"] }]
    });
    if (typeof selected !== "string") return;
    await changeReplacement(font, () =>
      invoke<void>("set_font_replacement", { gameDir, name: font.name, sourcePath: selected })
    );
  }

  function removeReplacement(font: GameFont) {
    return changeReplacement(font, () =>
      invoke<void>("remove_font_replacement", { gameDir, name: font.name })
    );
  }
</script>

{#snippet sample(label: string, text: string, missing: string, preview: Preview | undefined)}
  <div class="sample-row">
    <span class="sample-label">{label}</span>
    {#if preview && "family" in preview}
      <p class="sample-text" style:font-family={`"${preview.family}"`}>
        {#each sampleSegments(text, missing) as segment, index (index)}
          {#if segment.missing}<span class="sample-missing" title="Этого символа нет в шрифте">{"□".repeat([...segment.text].length)}</span>{:else}{segment.text}{/if}
        {/each}
      </p>
    {:else if preview}
      <p class="sample-unavailable">Предпросмотр недоступен</p>
    {:else}
      <p class="sample-unavailable">Загрузка…</p>
    {/if}
  </div>
{/snippet}

{#snippet samples(coverage: FontCoverage, preview: Preview | undefined)}
  {@render sample(ORIGINAL_SAMPLE.label, ORIGINAL_SAMPLE.text, coverage.missing_original, preview)}
  {@render sample(TARGET_SAMPLE.label, TARGET_SAMPLE.text, coverage.missing_target, preview)}
{/snippet}

{#snippet fontCard(font: GameFont)}
  {@const status = fontStatus(font.coverage)}
  {@const replacementStatus = font.replacement ? fontStatus(font.replacement.coverage) : null}
  <article class="font-card" class:font-card-waiting={needsReplacement(font)}>
    <header class="font-card-head">
      <div class="font-title">
        <h4>{font.name}</h4>
        <p>
          {originLabel(font.origin)} ·
          {font.references > 0 ? `упоминается в скриптах: ${font.references}` : "в скриптах не упоминается"}
        </p>
      </div>
      <span class="status status-{status.kind}">{status.label}</span>
    </header>

    {@render samples(font.coverage, previews[previewKey(font.name, false)])}

    {#if font.replacement && replacementStatus}
      <div class="replacement">
        <div class="replacement-head">
          <h5>Замена: {font.replacement.source_name}</h5>
          <span class="status status-{replacementStatus.kind}">{replacementStatus.label}</span>
        </div>
        {@render samples(font.replacement.coverage, previews[previewKey(font.name, true)])}
      </div>
    {/if}

    {#if font.locked}
      <p class="font-note font-note-warning"><AlertCircle size={14} /> {font.locked}. Замена в моде не подействует.</p>
    {:else if font.origin === "engine"}
      <p class="font-note">Этот шрифт входит в движок и может отвечать за особые символы интерфейса. Меняйте его, только если уверены.</p>
    {/if}

    <div class="font-actions">
      <button class="btn-secondary" disabled={!!busyFont || !!font.locked} onclick={() => chooseReplacement(font)}>
        {#if busyFont === font.name}<Loader2 size={14} class="spin" />{:else}<Upload size={14} />{/if}
        <span>{font.replacement ? "Другой файл" : "Выбрать замену"}</span>
      </button>
      {#if font.replacement}
        <button class="btn-ghost" disabled={!!busyFont} onclick={() => removeReplacement(font)}>
          <Trash2 size={14} />
          <span>Убрать замену</span>
        </button>
      {/if}
    </div>
  </article>
{/snippet}

{#if show}
  <div
    class="modal-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label="Подмена шрифтов"
    tabindex="-1"
    onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
    onkeydown={(e) => { if (e.key === "Escape") onclose(); }}
  >
    <div class="fonts-box">
      <header class="fonts-header">
        <div class="fonts-title">
          <Type size={18} class="text-sky-400" />
          <h3>Подмена шрифтов</h3>
          {#if waitingCount > 0}<span class="status status-letters">Нужна замена: {waitingCount}</span>{/if}
        </div>
        <button class="btn-icon" onclick={onclose} aria-label="Закрыть"><X size={16} /></button>
      </header>

      <div class="fonts-body">
        <p class="fonts-intro">
          Замена попадает в архив мода под именем исходного шрифта и действует только на языке перевода. Файлы игры не меняются. Формат может быть любым из TTF и OTF, совпадать с исходным он не обязан. Новый шрифт проверьте в игре: его ширина может отличаться.
        </p>

        {#if loading && fonts.length === 0}
          <div class="fonts-empty"><Loader2 size={18} class="spin" /> Поиск шрифтов в файлах и архивах игры…</div>
        {:else if loadError}
          <div class="fonts-empty fonts-error"><AlertCircle size={16} /> {loadError}</div>
        {:else if fonts.length === 0}
          <div class="fonts-empty">В игре не найдено файлов шрифтов.</div>
        {:else}
          <div class="font-list">
            {#each usedFonts as font (font.name)}
              {@render fontCard(font)}
            {/each}
          </div>
          {#if usedFonts.length === 0}
            <div class="fonts-empty">Скрипты не упоминают ни один шрифт из файлов игры.</div>
          {/if}
          {#if unusedFonts.length > 0}
            <button class="unused-toggle" onclick={() => showUnused = !showUnused} aria-expanded={showUnused}>
              {#if showUnused}<ChevronDown size={15} />{:else}<ChevronRight size={15} />{/if}
              <span>Не упоминаются в скриптах: {unusedFonts.length}</span>
            </button>
            {#if showUnused}
              <p class="font-note">Игра может обращаться к ним по имени, собранному в коде, или не использовать совсем.</p>
              <div class="font-list">
                {#each unusedFonts as font (font.name)}
                  {@render fontCard(font)}
                {/each}
              </div>
            {/if}
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 1.5rem;
    background: rgba(10, 14, 23, 0.78);
    backdrop-filter: blur(6px);
  }

  .fonts-box {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 880px;
    max-height: 90vh;
    overflow: hidden;
    color-scheme: dark;
    background: #0f172a;
    border: 1px solid #1e2638;
    border-radius: 14px;
    box-shadow: 0 25px 60px -15px rgba(0, 0, 0, 0.75);
  }

  .fonts-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    background: #141824;
    border-bottom: 1px solid #1e2638;
  }

  .fonts-title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .fonts-title h3 {
    margin: 0;
    color: #f1f5f9;
    font-size: 15px;
    font-weight: 600;
  }

  .fonts-body {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 20px 20px;
    overflow-y: auto;
  }

  .fonts-intro {
    margin: 0;
    color: #94a3b8;
    font-size: 13px;
    line-height: 1.5;
  }

  .fonts-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 28px 12px;
    color: #94a3b8;
    font-size: 13px;
  }

  .fonts-error {
    color: #fca5a5;
  }

  .font-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .font-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    background: #141824;
    border: 1px solid #1e2638;
    border-radius: 10px;
  }

  .font-card-waiting {
    border-color: rgba(251, 113, 133, 0.45);
  }

  .font-card-head,
  .replacement-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .font-title {
    min-width: 0;
  }

  .font-title h4,
  .replacement-head h5 {
    margin: 0;
    overflow-wrap: anywhere;
    color: #e2e8f0;
    font-size: 13px;
    font-weight: 600;
  }

  .font-title p {
    margin: 2px 0 0;
    color: #64748b;
    font-size: 12px;
  }

  .status {
    flex-shrink: 0;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
    white-space: nowrap;
  }

  .status-ok {
    color: #6ee7b7;
    background: rgba(16, 185, 129, 0.12);
  }

  .status-letters {
    color: #fda4af;
    background: rgba(244, 63, 94, 0.14);
  }

  .status-marks {
    color: #fcd34d;
    background: rgba(245, 158, 11, 0.14);
  }

  .status-symbols,
  .status-unknown {
    color: #cbd5e1;
    background: rgba(148, 163, 184, 0.14);
  }

  .sample-row {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr);
    gap: 10px;
    align-items: baseline;
  }

  .sample-label {
    color: #64748b;
    font-size: 11px;
  }

  .sample-text {
    margin: 0;
    overflow-wrap: anywhere;
    color: #f1f5f9;
    font-size: 20px;
    line-height: 1.35;
  }

  .sample-missing {
    color: #fb7185;
    font-family: system-ui, sans-serif;
  }

  .sample-unavailable {
    margin: 0;
    color: #64748b;
    font-size: 12px;
  }

  .replacement {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 10px;
    border-top: 1px solid #1e2638;
  }

  .font-note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: #94a3b8;
    font-size: 12px;
    line-height: 1.45;
  }

  .font-note-warning {
    color: #fcd34d;
  }

  .font-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .btn-secondary,
  .btn-ghost,
  .btn-icon,
  .unused-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: #cbd5e1;
    font-size: 12px;
    cursor: pointer;
    border-radius: 7px;
    transition: background 0.15s;
  }

  .btn-secondary {
    padding: 6px 10px;
    background: #1e2638;
    border: 1px solid #2b3549;
  }

  .btn-ghost {
    padding: 6px 10px;
    background: transparent;
    border: 1px solid transparent;
  }

  .btn-icon {
    padding: 6px;
    color: #64748b;
    background: transparent;
    border: none;
  }

  .unused-toggle {
    align-self: flex-start;
    padding: 4px 6px;
    color: #94a3b8;
    background: transparent;
    border: none;
  }

  .btn-secondary:hover:not(:disabled),
  .btn-ghost:hover:not(:disabled),
  .btn-icon:hover,
  .unused-toggle:hover {
    background: #263048;
  }

  .btn-secondary:disabled,
  .btn-ghost:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .btn-secondary:focus-visible,
  .btn-ghost:focus-visible,
  .btn-icon:focus-visible,
  .unused-toggle:focus-visible {
    outline: 2px solid #38bdf8;
    outline-offset: 1px;
  }

  @media (max-width: 560px) {
    .font-card-head,
    .replacement-head {
      flex-direction: column;
    }

    .sample-row {
      grid-template-columns: minmax(0, 1fr);
      gap: 2px;
    }
  }
</style>
