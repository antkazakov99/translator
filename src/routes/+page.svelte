<script lang="ts">
  import { onMount } from "svelte";
  import StartScreen from "$lib/StartScreen.svelte";
  import Workspace from "$lib/Workspace.svelte";
  import LlmSettings from "$lib/LlmSettings.svelte";
  import { commands, type ProjectDto_Serialize as Project } from "$lib/bindings";

  /** Проект, который нужно открыть заново после перезагрузки страницы. Не настройка, поэтому не в базе. */
  const OPENING_PROJECT_KEY = "translator.opening-project";

  let project = $state<Project | null>(null);
  let restoring = $state(true);
  let showLlmSettings = $state(false);

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
  {#key project.id}
    <Workspace {project} onBack={backToProjects} onSettings={() => (showLlmSettings = true)} onProject={(next) => (project = next)} />
  {/key}
{:else}
  <StartScreen onOpen={opened} onSettings={() => (showLlmSettings = true)} />
{/if}

{#if showLlmSettings}
  <LlmSettings {project} onClose={() => (showLlmSettings = false)} />
{/if}

<style>
  .blank {
    min-height: 100vh;
  }
</style>
