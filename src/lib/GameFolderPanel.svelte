<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    Check,
    FolderInput,
    FolderOpen,
    Link,
    LoaderCircle,
    Package,
    PackageMinus,
    PackagePlus,
    RefreshCw,
    TriangleAlert,
  } from "lucide-svelte";
  import Dialog from "./Dialog.svelte";
  import FontPanel from "./FontPanel.svelte";
  import {
    commands,
    type LineDto_Serialize as Line,
    type MissingLineDto_Serialize as MissingLine,
    type ModResultDto,
    type ProjectDto_Serialize as Project,
    type ScanReportDto,
  } from "./bindings";

  /** Окно «Игра»: папка игры (считывание, смена), мод перевода и «Пропавшие». */
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

  type Busy = "rescan" | "rebind" | "build" | "install" | "uninstall";
  let busy = $state<Busy | null>(null);
  let error = $state("");

  let report = $state<ScanReportDto | null>(null);
  /** Папка, которая похожа на другую игру: ждёт подтверждения. */
  let otherGame = $state<{ root: string; shared: number; total: number } | null>(null);

  let modResult = $state<{ text: string; path: string | null; warning: string; leftOut: string[] } | null>(null);

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

  /** Начало действия: прежние итоги и ошибка убираются. */
  function begin(action: Busy) {
    busy = action;
    error = "";
    report = null;
    otherGame = null;
    modResult = null;
  }

  function scanSummary(done: ScanReportDto) {
    const parts = [`реплик: ${done.lines}`, `новых: ${done.newLines}`];
    if (done.transferred > 0) parts.push(`перенесено переводов: ${done.transferred}`);
    if (done.drafts > 0) parts.push(`на проверку: ${done.drafts}`);
    return parts.join(" · ");
  }

  async function scanned(done: ScanReportDto) {
    report = done;
    await loadMissing();
    onChanged();
  }

  async function rescan() {
    begin("rescan");
    try {
      await scanned(await commands.rescanGame(project.id));
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function rebind(root: string, confirmed: boolean) {
    begin("rebind");
    try {
      const result = await commands.rebindProject(project.id, root, confirmed);
      if (result.kind === "looksLikeOtherGame") {
        otherGame = { root, shared: result.shared, total: result.total };
        return;
      }
      onProject(result.project);
      await scanned(result.report);
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

  function modSummary(built: ModResultDto) {
    const parts = [`файлов: ${built.files}`, `переводов: ${built.written}`];
    if (built.skipped > 0) parts.push(`не вошло: ${built.skipped}`);
    return parts.join(" · ");
  }

  async function mod(action: "build" | "install" | "uninstall") {
    begin(action);
    try {
      if (action === "build") {
        const built = await commands.buildMod(project.id);
        modResult = { text: `Мод собран: ${modSummary(built)}`, path: built.path, warning: "", leftOut: built.leftOut };
      } else if (action === "install") {
        const built = await commands.installMod(project.id);
        modResult = {
          text: `Мод установлен в игру: ${modSummary(built)}`,
          path: built.path,
          warning: built.tlOnDisk
            ? "В папке игры есть свой перевод game/tl/russian: игра берёт его раньше архива, и часть правок не покажет. Чтобы игра взяла мод, этот перевод удаляют"
            : "",
          leftOut: built.leftOut,
        };
      } else {
        await commands.uninstallMod(project.id);
        modResult = { text: "Мод снят с игры", path: null, warning: "", leftOut: [] };
      }
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function reveal(path: string) {
    try {
      await revealItemInDir(path);
    } catch (e) {
      error = String(e);
    }
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

<Dialog title="Игра" note="Папка игры, мод перевода и реплики, пропавшие после обновления игры" {onClose}>
  {#if error}
    <div class="banner error" role="alert">
      <TriangleAlert size={14} />
      <span>{error}</span>
    </div>
  {/if}

  <section>
    <strong class="section">Папка игры</strong>
    {#if gameHint}
      <span class="place warn" title={project.gameRoot ?? ""}><TriangleAlert size={11} />{gameHint}{project.gameRoot ? `: ${project.gameRoot}` : ""}</span>
    {:else}
      <span class="place" title={project.gameRoot ?? ""}>{project.gameRoot}</span>
    {/if}
    <span class="faint">После обновления игры считайте её заново: перевод перейдёт на новые реплики, а перевод исчезнувших останется в «Пропавших»</span>
    <div class="actions">
      <button class="btn small" disabled={busy !== null || !gameReady} title={gameHint || "Считать привязанную папку игры"} onclick={rescan}>
        {#if busy === "rescan"}<LoaderCircle size={13} class="spin" />{:else}<RefreshCw size={13} />{/if}
        <span>Считать заново</span>
      </button>
      <button class="btn small" disabled={busy !== null} title="Привязать другую папку и считать игру из неё" onclick={chooseFolder}>
        {#if busy === "rebind"}<LoaderCircle size={13} class="spin" />{:else}<FolderInput size={13} />{/if}
        <span>{project.gameRoot ? "Сменить папку" : "Указать папку"}</span>
      </button>
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
        <span>Игра считана: {scanSummary(report)}{report.drafts > 0 ? ". Перенесённые на проверку отмечены статусом «Модель»" : ""}</span>
      </div>
    {/if}
  </section>

  <section>
    <strong class="section">Мод</strong>
    <span class="faint">Архив ru_lang.rpa: перевод, подписи говорящих и переключатель языка. Собирается без папки игры, оригиналы игры не меняются</span>
    <div class="actions">
      <button class="btn small primary" disabled={busy !== null} title="Собрать архив в папке проекта" onclick={() => mod("build")}>
        {#if busy === "build"}<LoaderCircle size={13} class="spin" />{:else}<Package size={13} />{/if}
        <span>Собрать</span>
      </button>
      <button
        class="btn small"
        disabled={busy !== null || !gameReady}
        title={gameHint || "Собрать мод заново и положить в папку game игры"}
        onclick={() => mod("install")}
      >
        {#if busy === "install"}<LoaderCircle size={13} class="spin" />{:else}<PackagePlus size={13} />{/if}
        <span>Установить в игру</span>
      </button>
      <button class="btn small" disabled={busy !== null || !gameReady} title={gameHint || "Убрать ru_lang.rpa из папки игры"} onclick={() => mod("uninstall")}>
        {#if busy === "uninstall"}<LoaderCircle size={13} class="spin" />{:else}<PackageMinus size={13} />{/if}
        <span>Снять с игры</span>
      </button>
    </div>
    {#if modResult}
      <div class="banner" role="status">
        <Check size={14} />
        <span>{modResult.text}</span>
        {#if modResult.path}
          <button class="btn small" onclick={() => reveal(modResult!.path!)}>
            <FolderOpen size={13} />
            <span>Показать файл</span>
          </button>
        {/if}
      </div>
      {#if modResult.leftOut.length > 0}
        <div class="banner warn" role="status">
          <TriangleAlert size={14} />
          <span>Не вошли файлы перевода игры: {modResult.leftOut.join(", ")}. В них есть операторы, которые приложение пока не компилирует: переводы этих файлов в мод не попали</span>
        </div>
      {/if}
      {#if modResult.warning}
        <div class="banner warn" role="status">
          <TriangleAlert size={14} />
          <span>{modResult.warning}</span>
        </div>
      {/if}
    {/if}
  </section>

  <section>
    <FontPanel projectId={project.id} />
  </section>

  {#if missing.length > 0}
    <section>
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
    </section>
  {/if}
</Dialog>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  section + section {
    padding-top: 12px;
    border-top: 1px solid var(--divider);
  }

  .section {
    color: var(--text-strong);
    font-size: 13px;
    font-weight: 600;
  }

  .place {
    display: flex;
    gap: 4px;
    align-items: center;
    overflow: hidden;
    color: var(--text-soft);
    font-family: var(--font-mono);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .place.warn {
    color: var(--warning-strong);
    font-family: var(--font);
  }

  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    margin-top: 2px;
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
