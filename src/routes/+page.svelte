<script lang="ts">
  import { onMount } from "svelte";
  import { ArrowLeft, TriangleAlert } from "lucide-svelte";
  import StartScreen from "$lib/StartScreen.svelte";
  import { commands, type ProjectDto_Serialize as Project } from "$lib/bindings";

  /** Проект, который нужно открыть заново после перезагрузки страницы. Не настройка, поэтому не в базе. */
  const OPENING_PROJECT_KEY = "translator.opening-project";

  let project = $state<Project | null>(null);
  let restoring = $state(true);

  function remember(id: string | null) {
    try {
      if (id) sessionStorage.setItem(OPENING_PROJECT_KEY, id);
      else sessionStorage.removeItem(OPENING_PROJECT_KEY);
    } catch {
      // Без хранилища страница просто откроется со списка проектов.
    }
  }

  onMount(async () => {
    let id: string | null = null;
    try {
      id = sessionStorage.getItem(OPENING_PROJECT_KEY);
    } catch {
      id = null;
    }
    if (id) {
      try {
        project = await commands.openProject(id);
      } catch {
        remember(null);
      }
    }
    restoring = false;
  });

  function opened(next: Project) {
    project = next;
    remember(next.id);
  }

  async function backToProjects() {
    const id = project?.id;
    project = null;
    remember(null);
    if (id) await commands.closeProject(id).catch(() => {});
  }
</script>

{#if restoring}
  <div class="blank"></div>
{:else if project}
  <main class="workspace">
    <header class="bar">
      <button class="back" onclick={backToProjects}>
        <ArrowLeft size={15} />
        <span>Проекты</span>
      </button>
      <div class="title">
        <strong>{project.title}</strong>
        <span>
          {#if !project.gameRoot}
            Папка игры не привязана
          {:else if project.gameMissing}
            <TriangleAlert size={11} /> Папки нет на диске: {project.gameRoot}
          {:else}
            {project.gameRoot}
          {/if}
        </span>
      </div>
    </header>
    <section class="placeholder">
      <p>Проект открыт. Таблица перевода, граф сцен и глоссарий переезжают на 2.0 и появятся здесь следующими шагами.</p>
    </section>
  </main>
{:else}
  <StartScreen onOpen={opened} />
{/if}

<style>
  .blank {
    min-height: 100vh;
  }

  .workspace {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 16px;
    border-bottom: 1px solid #202937;
    background: #11151d;
  }

  .back {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid #34445d;
    border-radius: 8px;
    background: #1a2433;
    color: #d4deeb;
    padding: 6px 10px;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .back:hover {
    border-color: #3b82f6;
    background: #20334f;
  }

  .title {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title strong {
    color: #f1f5f9;
    font-size: 14px;
  }

  .title span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    color: #74839b;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .placeholder {
    display: flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    padding: 24px;
    color: #74839b;
    font-size: 13px;
    text-align: center;
  }

  .placeholder p {
    max-width: 460px;
  }
</style>
