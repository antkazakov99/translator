<script lang="ts">
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, Loader2, Plus, RefreshCw, Settings2, Trash2, X } from "lucide-svelte";
  import ModelCombobox from "./ModelCombobox.svelte";
  import {
    LLM_PROVIDERS,
    apiProvider,
    newConnection,
    readyToCheck,
    textModels,
    withoutConnection,
    type LlmConnection,
    type LlmProvider,
    type LlmSettings
  } from "./llm-settings";

  let {
    show = false,
    settings,
    onsave = () => {},
    onclose = () => {}
  }: {
    show: boolean;
    settings: LlmSettings;
    onsave: (settings: LlmSettings) => void;
    onclose: () => void;
  } = $props();

  type ServerCheck = { running: boolean; ok: boolean | null; message: string; models: string[] };
  const idleCheck = (): ServerCheck => ({ running: false, ok: null, message: "", models: [] });

  let draft = $state<LlmSettings>({
    connections: [],
    defaultConnection: "",
    actions: {
      descriptions: { connection: "", model: "" },
      names: { connection: "", model: "" },
      terms: { connection: "", model: "" },
      tokens: { connection: "", model: "" },
      speakers: { connection: "", model: "" },
      lines: { connection: "", model: "" },
      review: { connection: "", model: "" }
    }
  });
  let checks = $state<Record<string, ServerCheck>>({});

  const providerOf = (id: LlmProvider) => LLM_PROVIDERS.find((provider) => provider.id === id)!;

  async function checkServer(connection: LlmConnection) {
    checks[connection.id] ??= idleCheck();
    const check = checks[connection.id];
    check.running = true;
    try {
      const models = await invoke<string[]>("check_llm_models", {
        apiUrl: connection.url.trim(),
        apiKey: connection.key.trim() || null,
        provider: apiProvider(connection.provider)
      });
      check.models = textModels(models);
      const hidden = models.length - check.models.length;
      check.ok = true;
      check.message = models.length > 0
        ? `Найдено моделей: ${check.models.length}${hidden > 0 ? `, скрыто нетекстовых: ${hidden}` : ""}`
        : "Сервер доступен, список моделей пуст";
    } catch (e) {
      check.ok = false;
      check.models = [];
      check.message = String(e);
    } finally {
      check.running = false;
    }
  }

  function checkIfReady(connection: LlmConnection) {
    if (readyToCheck(connection)) void checkServer(connection);
  }

  // Список моделей нужен сразу: ключ при этом уходит только на адрес своего подключения
  $effect(() => {
    if (!show) return;
    // Проверка читает и пишет черновик: без untrack окно перезапускало бы её по кругу
    untrack(() => {
      draft = JSON.parse(JSON.stringify(settings));
      checks = {};
      draft.connections.forEach(checkIfReady);
    });
  });

  function addConnection(provider: LlmProvider) {
    draft.connections.push(newConnection(provider, draft.connections));
  }

  function removeConnection(id: string) {
    draft = withoutConnection($state.snapshot(draft), id);
    delete checks[id];
  }

  function save() {
    onsave($state.snapshot(draft));
    onclose();
  }
</script>

{#snippet checkRow(connection: LlmConnection)}
  {@const check = checks[connection.id] ?? idleCheck()}
  <div class="check-row">
    <button class="btn-check" onclick={() => checkServer(connection)} disabled={check.running}>
      {#if check.running}
        <Loader2 size={13} class="spin" />
      {:else}
        <RefreshCw size={13} />
      {/if}
      <span>Проверить</span>
    </button>
    {#if check.ok === true}
      <span class="check-status ok"><Check size={13} /> {check.message}</span>
    {:else if check.ok === false}
      <span class="check-status failed" title={check.message}><AlertCircle size={13} /> {check.message}</span>
    {/if}
  </div>
{/snippet}

{#if show}
  <div
    class="modal-backdrop"
    onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
    onkeydown={(e) => { if (e.key === "Escape") onclose(); }}
    role="dialog"
    aria-modal="true"
    aria-label="Настройки LLM"
    tabindex="-1"
  >
    <div class="settings-box">
      <div class="settings-header">
        <div class="settings-title">
          <Settings2 size={18} />
          <h3>Настройки LLM</h3>
        </div>
        <button class="btn-close" onclick={onclose} aria-label="Закрыть"><X size={16} /></button>
      </div>

      <div class="settings-body">
        <section>
          <h4>Подключения</h4>
          {#each draft.connections as connection (connection.id)}
            {@const provider = providerOf(connection.provider)}
            <div class="connection">
              <div class="connection-head">
                <span class="connection-kind">{provider.label}</span>
                <input type="text" class="input connection-name" bind:value={connection.name} aria-label="Название подключения" />
                <label class="connection-default" title="Это подключение получают действия, у которых не выбрано своё">
                  <input type="radio" name="llm-default-connection" value={connection.id} bind:group={draft.defaultConnection} />
                  <span>По умолчанию</span>
                </label>
                <button
                  class="btn-remove"
                  onclick={() => removeConnection(connection.id)}
                  disabled={draft.connections.length < 2}
                  aria-label={`Удалить подключение ${connection.name}`}
                  title="Удалить подключение"
                ><Trash2 size={14} /></button>
              </div>
              <div class="field-pair" class:single={connection.provider === "local"}>
                <label class="field">
                  <span>{provider.urlHint}</span>
                  <input
                    type="text"
                    class="input mono"
                    bind:value={connection.url}
                    onchange={() => checkIfReady(connection)}
                    placeholder="https://openrouter.ai/api/v1"
                  />
                </label>
                {#if connection.provider !== "local"}
                  <label class="field">
                    <span>Ключ API</span>
                    <input
                      type="password"
                      class="input"
                      autocomplete="off"
                      bind:value={connection.key}
                      onchange={() => checkIfReady(connection)}
                    />
                  </label>
                {/if}
              </div>
              {#if provider.keyHint}
                <p class="note">{provider.keyHint}</p>
              {/if}
              <div class="field">
                <span>Модель по умолчанию</span>
                <ModelCombobox
                  bind:value={connection.model}
                  models={checks[connection.id]?.models ?? []}
                  label={`Модель по умолчанию: ${connection.name}`}
                  placeholder="Имя модели"
                />
              </div>
              {@render checkRow(connection)}
            </div>
          {/each}
          <div class="add-row">
            <span class="note">Добавить:</span>
            {#each LLM_PROVIDERS as provider (provider.id)}
              <button class="btn-check" onclick={() => addConnection(provider.id)}><Plus size={12} /><span>{provider.label}</span></button>
            {/each}
          </div>
          <p class="note">Ключи хранятся на этом компьютере без шифрования. По адресу http:// они передаются открытым текстом.</p>
          <p class="note">Для отдельного действия подключение и модель меняются в панели «Обработка через LLM». Заметки по сценам привязаны к модели: после её смены части читаются заново.</p>
        </section>

      </div>

      <div class="settings-footer">
        <button class="btn-secondary" onclick={onclose}>Отмена</button>
        <button class="btn-primary" onclick={save}><Check size={14} /><span>Сохранить</span></button>
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

  .settings-box {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 760px;
    max-height: 90vh;
    overflow: hidden;
    color-scheme: dark;
    background: #0f172a;
    border: 1px solid #1e2638;
    border-radius: 14px;
    box-shadow: 0 25px 60px -15px rgba(0, 0, 0, 0.75);
  }

  .settings-header,
  .settings-footer {
    display: flex;
    align-items: center;
    padding: 14px 20px;
    background: #141824;
  }

  .settings-header {
    justify-content: space-between;
    border-bottom: 1px solid #1e2638;
  }

  .settings-footer {
    justify-content: flex-end;
    gap: 10px;
    border-top: 1px solid #1e2638;
  }

  .settings-title {
    display: flex;
    align-items: center;
    gap: 10px;
    color: #94a3b8;
  }

  .settings-title h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: #f8fafc;
  }

  .btn-close {
    display: flex;
    padding: 6px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: #94a3b8;
    cursor: pointer;
  }

  .btn-close:hover {
    color: #f8fafc;
    background: rgba(255, 255, 255, 0.08);
  }

  .settings-body {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 16px 20px;
    overflow-y: auto;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  h4 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #94a3b8;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    font-size: 11px;
    color: #94a3b8;
  }

  .field-pair {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
    gap: 10px;
  }

  .field-pair.single {
    grid-template-columns: minmax(0, 1fr);
  }

  .connection {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid #1e2638;
    border-radius: 8px;
    background: #141824;
  }

  .connection-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .connection-kind {
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(99, 102, 241, 0.18);
    font-size: 11px;
    color: #c7d2fe;
    white-space: nowrap;
  }

  .connection-name {
    flex: 1;
  }

  .connection-default {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: #94a3b8;
    white-space: nowrap;
    cursor: pointer;
  }

  .btn-remove {
    display: flex;
    padding: 5px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: #94a3b8;
    cursor: pointer;
  }

  .btn-remove:hover:not(:disabled) {
    color: #fb7185;
    background: rgba(244, 63, 94, 0.1);
  }

  .btn-remove:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .add-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .input {
    min-width: 0;
    padding: 6px 10px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
  }

  .input:focus {
    border-color: #10b981;
  }

  .mono {
    font-family: ui-monospace, monospace;
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .btn-check,
  .btn-secondary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    border: 1px solid rgba(71, 85, 105, 0.6);
    border-radius: 6px;
    background: rgba(51, 65, 85, 0.6);
    font-size: 12px;
    color: #cbd5e1;
    white-space: nowrap;
    cursor: pointer;
  }

  .btn-check:hover:not(:disabled),
  .btn-secondary:hover {
    color: #f8fafc;
    background: rgba(71, 85, 105, 0.8);
  }

  .check-status {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .check-status.ok {
    color: #34d399;
  }

  .check-status.failed {
    color: #fb7185;
  }

  .note {
    margin: 0;
    font-size: 11px;
    color: #64748b;
  }

  .btn-primary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border: 1px solid rgba(52, 211, 153, 0.5);
    border-radius: 6px;
    background: #059669;
    font-size: 12px;
    font-weight: 600;
    color: #fff;
    cursor: pointer;
  }

  .btn-primary:hover {
    background: #047857;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>
