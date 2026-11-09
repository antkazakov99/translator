<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { Check, FolderInput, Link, LoaderCircle, RefreshCw, TriangleAlert, X } from "lucide-svelte";
  import {
    commands,
    type LineDto_Serialize as Line,
    type MissingLineDto_Serialize as MissingLine,
    type ProjectDto_Serialize as Project,
    type ScanReportDto,
  } from "./bindings";

  /** Новая версия игры: повторное считывание, смена папки и «Пропавшие». */
  let {
    project,
    onClose,
    onProject,
    onChanged,
  }: {
    project: Project;
    onClose: () => void;
    /** Папка игры сменилась. */
    onProject: (next: Project) => void;
    /** Снимок или перевод изменились: таблице пора перечитать файлы. */
    onChanged: () => void;
  } = $props();

  let busy = $state<"rescan" | "rebind" | null>(null);
  let report = $state<ScanReportDto | null>(null);
  let error = $state("");
  /** Папка, которая похожа на другую игру: ждёт подтверждения. */
  let otherGame = $state<{ root: string; shared: number; total: number } | null>(null);
  let missing = $state<MissingLine[]>([]);
  /** Пропавшая реплика, для которой выбирается замена, и реплики её файла. */
  let attaching = $state<{ id: number; lines: Line[]; query: string } | null>(null);

  const gameReady = $derived(Boolean(project.gameRoot) && !project.gameMissing);
  const gameHint = $derived(!project.gameRoot ? "Папка игры не привязана" : project.gameMissing ? "Папки игры нет на диске" : "");
  const candidates = $derived.by(() => {
    if (!attaching) return [];
    const query = attaching.query.trim().toLowerCase();
    const lines = query ? attaching.lines.filter((line) => line.sourceText.toLowerCase().includes(query)) : attaching.lines;
    return lines.slice(0, 50);
  });

  onMount(loadMissing);

  async function loadMissing() {
    try {
      missing = await commands.missingLines(project.id);
    } catch (e) {
      error = String(e);
    }
  }

  function summary(done: ScanReportDto) {
    const parts = [`реплик: ${done.lines}`, `новых: ${done.newLines}`];
    if (done.transferred > 0) parts.push(`перенесено переводов: ${done.transferred}`);
    if (done.drafts > 0) parts.push(`на проверку: ${done.drafts}`);
    return parts.join(" · ");
  }

  async function finish(done: ScanReportDto) {
    report = done;
    await loadMissing();
    onChanged();
  }

  async function rescan() {
    busy = "rescan";
    error = "";
    report = null;
    otherGame = null;
    try {
      await finish(await commands.rescanGame(project.id));
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function rebind(root: string, confirmed: boolean) {
    busy = "rebind";
    error = "";
    report = null;
    otherGame = null;
    try {
      const result = await commands.rebindProject(project.id, root, confirmed);
      if (result.kind === "looksLikeOtherGame") {
        otherGame = { root, shared: result.shared, total: result.total };
        return;
      }
      onProject(result.project);
      await finish(result.report);
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function chooseFolder() {
    const picked = await openDialog({ directory: true, title: "Корневая папка игры" });
    if (typeof picked === "string") await rebind(picked, false);
  }

  /** Реплики игры из того же файла, что и пропавшая. */
  async function startAttach(line: MissingLine) {
    error = "";
    try {
      const files = await commands.translationFiles(project.id);
      const file = files.find((item) => item.path === line.file);
      const scenes = file ? await commands.fileLines(project.id, file.id) : [];
      attaching = { id: line.id, lines: scenes.flatMap((scene) => scene.lines), query: "" };
    } catch (e) {
      error = String(e);
    }
  }

  async function attach(target: Line) {
    if (!attaching) return;
    try {
      await commands.attachMissingLine(project.id, attaching.id, target.id);
      attaching = null;
      await loadMissing();
      onChanged();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<div class="card scan-panel">
  <div class="head">
    <div class="title">
      <strong>Версия игры</strong>
      <span>После обновления игры считайте её заново: перевод перейдёт на новые реплики, а перевод исчезнувших останется в «Пропавших»</span>
    </div>
    <button class="icon-button" aria-label="Закрыть версию игры" title="Закрыть" onclick={onClose}><X size={14} /></button>
  </div>

  <div class="actions">
    <button class="btn small primary" disabled={busy !== null || !gameReady} title={gameHint || "Считать привязанную папку игры"} onclick={rescan}>
      {#if busy === "rescan"}<LoaderCircle size={13} class="spin" />{:else}<RefreshCw size={13} />{/if}
      <span>Считать заново</span>
    </button>
    <button class="btn small" disabled={busy !== null} title="Привязать другую папку и считать игру из неё" onclick={chooseFolder}>
      {#if busy === "rebind"}<LoaderCircle size={13} class="spin" />{:else}<FolderInput size={13} />{/if}
      <span>Сменить папку</span>
    </button>
    {#if gameHint}<span class="faint">{gameHint}: укажите папку игры</span>{/if}
  </div>

  {#if otherGame}
    <div class="banner warn choice" role="alert">
      <TriangleAlert size={14} />
      <span>Похоже на другую игру: из {otherGame.total} реплик проекта в папке нашлось {otherGame.shared}. Перевод остальных уйдёт в «Пропавшие»</span>
      <button class="btn small" onclick={() => rebind(otherGame!.root, true)}>Всё равно привязать</button>
      <button class="btn small" onclick={() => (otherGame = null)}>Отмена</button>
    </div>
  {/if}
  {#if report}
    <div class="banner" role="status">
      <Check size={14} />
      <span>Игра считана: {summary(report)}{report.drafts > 0 ? ". Перенесённые на проверку отмечены статусом «Модель»" : ""}</span>
    </div>
  {/if}
  {#if error}
    <div class="banner error" role="alert">
      <TriangleAlert size={14} />
      <span>{error}</span>
    </div>
  {/if}

  {#if missing.length > 0}
    <div class="missing">
      <strong class="section">Пропавшие: {missing.length}</strong>
      <span class="faint">Реплики прошлых версий с переводом. Перевод можно привязать к реплике игры</span>
      <ul>
        {#each missing as line (line.id)}
          <li>
            <div class="texts">
              <span class="source">{line.sourceText}</span>
              <span class="target">{line.translation || "Перевод очищен"}</span>
              <span class="faint">{line.file}{line.location ? `:${line.location}` : ""}</span>
            </div>
            <button class="btn small" class:active={attaching?.id === line.id} onclick={() => (attaching?.id === line.id ? (attaching = null) : startAttach(line))}>
              <Link size={13} />
              <span>Привязать</span>
            </button>
            {#if attaching?.id === line.id}
              <div class="picker">
                <input class="input" placeholder="Найти реплику по тексту" bind:value={attaching.query} />
                {#each candidates as target (target.id)}
                  <button class="candidate" onclick={() => attach(target)}>
                    <span class="source">{target.sourceText}</span>
                    {#if target.text}<span class="faint">{target.text}</span>{/if}
                  </button>
                {:else}
                  <span class="faint">{attaching.lines.length === 0 ? "Файла этой реплики в игре больше нет" : "Ничего не нашлось"}</span>
                {/each}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .scan-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 10px 16px 0;
    max-height: 50vh;
    overflow-y: auto;
  }

  .head {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }

  .title {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
  }

  .title strong,
  .section {
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .title span,
  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .missing {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: flex-start;
    padding: 8px 0;
    border-top: 1px solid var(--divider-row);
  }

  .texts {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .source {
    color: var(--text);
    font-size: 12px;
  }

  .target {
    color: var(--text-soft);
    font-size: 12px;
  }

  .picker {
    display: flex;
    flex-basis: 100%;
    flex-direction: column;
    gap: 4px;
  }

  .candidate {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    border: 0;
    border-radius: 6px;
    background: none;
    text-align: left;
    cursor: pointer;
  }

  .candidate:hover {
    background: var(--hover-row);
  }
</style>
