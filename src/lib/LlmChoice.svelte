<script lang="ts" module>
  import { invoke } from "@tauri-apps/api/core";
  import { apiProvider, readyToCheck, textModels, type LlmConnection } from "./llm-settings";

  // Список моделей подключения запрашивается один раз на всё окно
  const modelsByConnection = $state<Record<string, string[]>>({});
  const requestedModels = new Set<string>();

  async function loadModels(connection: LlmConnection) {
    const key = JSON.stringify([connection.provider, connection.url, connection.key]);
    if (requestedModels.has(key) || !readyToCheck(connection)) return;
    requestedModels.add(key);
    try {
      const models = await invoke<string[]>("check_llm_models", {
        apiUrl: connection.url.trim(),
        apiKey: connection.key.trim() || null,
        provider: apiProvider(connection.provider)
      });
      modelsByConnection[connection.id] = textModels(models);
    } catch {
      // Модель можно вписать вручную, а следующий выбор подключения спросит список заново
      requestedModels.delete(key);
      modelsByConnection[connection.id] = [];
    }
  }
</script>

<script lang="ts">
  import ModelCombobox from "./ModelCombobox.svelte";
  import { llmRun } from "./llm-run.svelte";
  import { connectionFor, type LlmAction, type LlmActionSettings, type LlmSettings } from "./llm-settings";

  let {
    action,
    settings,
    onselect
  }: {
    action: LlmAction;
    settings: LlmSettings;
    onselect: (action: LlmAction, choice: LlmActionSettings) => void;
  } = $props();

  let choice = $derived(settings.actions[action]);
  let connection = $derived(connectionFor(settings, action));

  $effect(() => {
    void loadModels(connection);
  });

  // Подключение по умолчанию не закрепляется за действием: оно следует за настройками, как и невыбранное
  function pickConnection(id: string) {
    onselect(action, { connection: id === settings.defaultConnection ? "" : id, model: "" });
  }
</script>

<div class="llm-choice">
  <select
    class="connection"
    value={connection.id}
    disabled={llmRun.active}
    aria-label="Подключение"
    onchange={(event) => pickConnection(event.currentTarget.value)}
  >
    {#each settings.connections as option (option.id)}
      <option value={option.id}>{option.name}</option>
    {/each}
  </select>
  <ModelCombobox
    bind:value={() => choice.model, (model) => onselect(action, { connection: choice.connection, model })}
    models={modelsByConnection[connection.id] ?? []}
    label="Модель"
    placeholder="Имя модели"
    fallback={connection.model.trim()}
    disabled={llmRun.active}
  />
</div>

<style>
  .llm-choice {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    gap: 6px;
    color-scheme: dark;
  }

  .connection {
    min-width: 0;
    padding: 5px 9px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
  }

  .connection:focus {
    border-color: #10b981;
  }

  .connection:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
