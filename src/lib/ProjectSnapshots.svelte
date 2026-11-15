<script lang="ts">
  import { onMount } from "svelte";
  import { commands, type ProjectSnapshotDto as Snapshot } from "$lib/bindings";
  import Dialog from "./Dialog.svelte";

  let { projectId, onClose, onRestore }: {
    projectId: string;
    onClose: () => void;
    onRestore: () => void;
  } = $props();

  let snapshots = $state<Snapshot[]>([]);
  let name = $state("");
  type Action =
    | { kind: "rename"; snapshot: Snapshot; title: string }
    | { kind: "restore" | "delete"; snapshot: Snapshot };
  let action = $state<Action | null>(null);
  let pending = $state("");
  let error = $state("");
  let saved = $state("");

  onMount(() => { void run("Загружаю снимки…", load); });

  async function load() {
    snapshots = await commands.listProjectSnapshots(projectId);
  }

  async function run(status: string, action: () => Promise<void>) {
    pending = status;
    error = "";
    saved = "";
    try {
      await action();
    } catch (e) {
      error = String(e);
    } finally {
      pending = "";
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim() || pending || action) return;
    await run("Сохраняю снимок…", async () => {
      await commands.saveProjectSnapshot(projectId, name);
      name = "";
      await load();
      saved = "Снимок сохранён";
    });
  }

  function select(next: Action | null) {
    action = next;
    saved = "";
    error = "";
  }

  async function rename(event: SubmitEvent) {
    event.preventDefault();
    if (pending || action?.kind !== "rename" || !action.title.trim()) return;
    const { snapshot, title } = action;
    await run("Переименовываю снимок…", async () => {
      await commands.renameProjectSnapshot(projectId, snapshot.id, title);
      action = null;
      await load();
      saved = "Снимок переименован";
    });
  }

  async function confirm() {
    if (pending || !action || action.kind === "rename") return;
    const { kind, snapshot } = action;
    if (kind === "restore") {
      await run("Восстанавливаю проект…", async () => {
        await commands.restoreProjectSnapshot(projectId, snapshot.id);
        onRestore();
      });
    } else {
      await run("Удаляю снимок…", async () => {
        await commands.deleteProjectSnapshot(projectId, snapshot.id);
        action = null;
        snapshots = snapshots.filter(({ id }) => id !== snapshot.id);
        saved = "Снимок удалён";
      });
    }
  }
</script>

<Dialog title="Снимки проекта" onClose={() => { if (!pending) onClose(); }}>
  <div class="snapshots">
    {#if action?.kind === "rename"}
      <form class="snapshot-form" onsubmit={rename}>
        <label>
          Новое название снимка
          <input bind:value={action.title} disabled={!!pending} required />
        </label>
        <button type="submit" class="btn primary" disabled={!!pending || !action.title.trim()}>Сохранить название</button>
        <button type="button" class="btn" disabled={!!pending} onclick={() => select(null)}>Отмена</button>
      </form>
    {:else if action}
      <div class="banner choice danger-zone">
        <div class="confirmation">
          {#if action.kind === "delete"}
            <strong>Удалить «{action.snapshot.title}»?</strong>
            <span>Снимок будет удалён без возможности восстановления.</span>
          {:else}
            <strong>Восстановить «{action.snapshot.title}»?</strong>
            <span>Снимок заменит текущее состояние проекта. Чтобы сохранить текущий результат, сначала создайте снимок.</span>
          {/if}
        </div>
        <button class="btn small danger" disabled={!!pending} onclick={confirm}>
          {action.kind === "delete" ? "Удалить" : "Восстановить"}
        </button>
        <button class="btn small" disabled={!!pending} onclick={() => select(null)}>Отмена</button>
      </div>
    {:else}
      <form class="snapshot-form" onsubmit={submit}>
        <label>
          Название снимка
          <input bind:value={name} disabled={!!pending} required />
        </label>
        <button type="submit" class="btn primary" disabled={!!pending || !name.trim()}>Сохранить снимок</button>
      </form>
    {/if}

    {#if pending || saved}
      <p role="status">{pending || saved}</p>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    {#if snapshots.length > 0}
      <ul>
        {#each snapshots as snapshot (snapshot.id)}
          <li>
            <div>
              <strong>{snapshot.title}</strong>
              <span>{new Date(snapshot.createdAt * 1000).toLocaleString("ru-RU")}</span>
            </div>
            <button class="btn small" disabled={!!pending || action !== null} onclick={() => select({ kind: "restore", snapshot })}>Восстановить</button>
            <button class="btn small" disabled={!!pending || action !== null} onclick={() => select({ kind: "rename", snapshot, title: snapshot.title })}>Переименовать</button>
            <button class="btn small" disabled={!!pending || action !== null} onclick={() => select({ kind: "delete", snapshot })}>Удалить</button>
          </li>
        {/each}
      </ul>
    {:else if !pending && !error}
      <p>Сохранённых снимков пока нет</p>
    {/if}
  </div>
</Dialog>

<style>
  .snapshots, ul {
    display: grid;
    gap: 12px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .snapshot-form, li {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px;
  }

  label, li div, .confirmation {
    display: grid;
    flex: 1;
    gap: 4px;
    min-width: 160px;
  }

  input {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-sunken);
    color: var(--text);
    font: inherit;
  }

  p, span {
    margin: 0;
    color: var(--text-muted);
  }

  .error {
    color: var(--danger);
  }

  strong {
    overflow-wrap: anywhere;
  }
</style>
