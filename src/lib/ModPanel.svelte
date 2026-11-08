<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { Check, FolderOpen, LoaderCircle, Package, PackageMinus, PackagePlus, TriangleAlert, X } from "lucide-svelte";
  import { commands, type ModResultDto, type ProjectDto_Serialize as Project } from "./bindings";

  /** Сборка `ru_lang.rpa`, установка в привязанную папку игры и снятие. */
  let { project, onClose }: { project: Project; onClose: () => void } = $props();

  type Action = "build" | "install" | "uninstall";
  let busy = $state<Action | null>(null);
  let result = $state<{ text: string; path: string | null; warning: string; leftOut: string[] } | null>(null);
  let error = $state("");

  const gameReady = $derived(Boolean(project.gameRoot) && !project.gameMissing);
  const gameHint = $derived(
    !project.gameRoot ? "Папка игры не привязана" : project.gameMissing ? "Папки игры нет на диске" : "",
  );

  function summary(built: ModResultDto) {
    const parts = [`файлов: ${built.files}`, `переводов: ${built.written}`];
    if (built.skipped > 0) parts.push(`не вошло: ${built.skipped}`);
    return parts.join(" · ");
  }

  async function run(action: Action) {
    busy = action;
    error = "";
    result = null;
    try {
      if (action === "build") {
        const built = await commands.buildMod(project.id);
        result = { text: `Мод собран: ${summary(built)}`, path: built.path, warning: "", leftOut: built.leftOut };
      } else if (action === "install") {
        const built = await commands.installMod(project.id);
        result = {
          text: `Мод установлен в игру: ${summary(built)}`,
          path: built.path,
          warning: built.tlOnDisk
            ? "В папке игры есть свой перевод game/tl/russian: игра берёт его раньше архива, и часть правок не покажет. Чтобы игра взяла мод, этот перевод удаляют"
            : "",
          leftOut: built.leftOut,
        };
      } else {
        await commands.uninstallMod(project.id);
        result = { text: "Мод снят с игры", path: null, warning: "", leftOut: [] };
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
</script>

<div class="card mod-panel">
  <div class="head">
    <div class="title">
      <strong>Мод перевода</strong>
      <span>Архив ru_lang.rpa: перевод, подписи говорящих и переключатель языка. Собирается без папки игры, оригиналы игры не меняются</span>
    </div>
    <button class="icon-button" aria-label="Закрыть мод" title="Закрыть" onclick={onClose}><X size={14} /></button>
  </div>

  <div class="actions">
    <button class="btn small primary" disabled={busy !== null} title="Собрать архив в папке проекта" onclick={() => run("build")}>
      {#if busy === "build"}<LoaderCircle size={13} class="spin" />{:else}<Package size={13} />{/if}
      <span>Собрать</span>
    </button>
    <button
      class="btn small"
      disabled={busy !== null || !gameReady}
      title={gameHint || "Собрать мод заново и положить в папку game игры"}
      onclick={() => run("install")}
    >
      {#if busy === "install"}<LoaderCircle size={13} class="spin" />{:else}<PackagePlus size={13} />{/if}
      <span>Установить в игру</span>
    </button>
    <button class="btn small" disabled={busy !== null || !gameReady} title={gameHint || "Убрать ru_lang.rpa из папки игры"} onclick={() => run("uninstall")}>
      {#if busy === "uninstall"}<LoaderCircle size={13} class="spin" />{:else}<PackageMinus size={13} />{/if}
      <span>Снять с игры</span>
    </button>
    {#if gameHint}<span class="faint">{gameHint}: установка недоступна</span>{/if}
  </div>

  {#if result}
    <div class="banner" role="status">
      <Check size={14} />
      <span>{result.text}</span>
      {#if result.path}
        <button class="btn small" onclick={() => reveal(result!.path!)}>
          <FolderOpen size={13} />
          <span>Показать файл</span>
        </button>
      {/if}
    </div>
    {#if result.leftOut.length > 0}
      <div class="banner warn" role="status">
        <TriangleAlert size={14} />
        <span>Не вошли файлы перевода игры: {result.leftOut.join(", ")}. В них есть операторы, которые приложение пока не компилирует: переводы этих файлов в мод не попали</span>
      </div>
    {/if}
    {#if result.warning}
      <div class="banner warn" role="status">
        <TriangleAlert size={14} />
        <span>{result.warning}</span>
      </div>
    {/if}
  {/if}
  {#if error}
    <div class="banner error" role="alert">
      <TriangleAlert size={14} />
      <span>{error}</span>
    </div>
  {/if}
</div>

<style>
  .mod-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 10px 16px 0;
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

  .title strong {
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
</style>
