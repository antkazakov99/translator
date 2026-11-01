<script lang="ts">
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { AlertCircle, Check, Loader2, RefreshCw, Settings2, X } from "lucide-svelte";
  import { LLM_ACTIONS, type LlmServer, type LlmSettings } from "./llm-settings";

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
    localUrl: "",
    externalUrl: "",
    externalKey: "",
    actions: {
      descriptions: { server: "local", model: "" },
      names: { server: "local", model: "" },
      terms: { server: "local", model: "" },
      termSearch: { server: "local", model: "" },
      tokens: { server: "local", model: "" }
    }
  });
  let checks = $state<Record<LlmServer, ServerCheck>>({ local: idleCheck(), external: idleCheck() });

  async function checkServer(server: LlmServer) {
    const check = checks[server];
    check.running = true;
    try {
      check.models = await invoke<string[]>("check_llm_models", {
        apiUrl: (server === "local" ? draft.localUrl : draft.externalUrl).trim(),
        apiKey: server === "external" ? draft.externalKey.trim() || null : null
      });
      check.ok = true;
      check.message = check.models.length > 0 ? `Найдено моделей: ${check.models.length}` : "Сервер доступен, список моделей пуст";
    } catch (e) {
      check.ok = false;
      check.models = [];
      check.message = String(e);
    } finally {
      check.running = false;
    }
  }

  // Ключ и адрес внешнего сервера проверяются только по кнопке: запрос уходит из приложения наружу
  $effect(() => {
    if (!show) return;
    // Проверка читает и пишет черновик: без untrack окно перезапускало бы её по кругу
    untrack(() => {
      draft = JSON.parse(JSON.stringify(settings));
      checks = { local: idleCheck(), external: idleCheck() };
      void checkServer("local");
    });
  });

  function save() {
    onsave($state.snapshot(draft));
    onclose();
  }
</script>

{#snippet checkRow(server: LlmServer)}
  {@const check = checks[server]}
  <div class="check-row">
    <button class="btn-check" onclick={() => checkServer(server)} disabled={check.running}>
      {#if check.running}
        <Loader2 size={13} class="spin" />
      {:else}
        <RefreshCw size={13} />
      {/if}
      <span>Проверить сервер</span>
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
          <h4>Локальный сервер</h4>
          <label class="field">
            <span>Адрес (LM Studio, Ollama)</span>
            <input type="text" class="input mono" bind:value={draft.localUrl} placeholder="http://127.0.0.1:1234/v1" />
          </label>
          {@render checkRow("local")}
        </section>

        <section>
          <h4>Внешний сервер</h4>
          <div class="field-pair">
            <label class="field">
              <span>Адрес (OpenAI-совместимый API или сервер по IP)</span>
              <input type="text" class="input mono" bind:value={draft.externalUrl} placeholder="https://openrouter.ai/api/v1" />
            </label>
            <label class="field">
              <span>Ключ API</span>
              <input type="password" class="input" autocomplete="off" bind:value={draft.externalKey} />
            </label>
          </div>
          {@render checkRow("external")}
          <p class="note">Ключ хранится на этом компьютере без шифрования. По адресу http:// он передаётся открытым текстом.</p>
        </section>

        <section>
          <h4>Модель для каждого действия</h4>
          <div class="actions-grid">
            {#each LLM_ACTIONS as action (action.id)}
              {@const chosen = draft.actions[action.id]}
              <div class="action-name">
                <span>{action.label}</span>
                <span class="note" class:warn={chosen.server === "external" && action.gameText}>
                  {chosen.server === "external" ? "Наружу уйдут" : "Модель получит"}: {action.sends}
                </span>
              </div>
              <select class="input" bind:value={chosen.server} aria-label={`Сервер: ${action.label}`}>
                <option value="local">Локальный</option>
                <option value="external">Внешний</option>
              </select>
              <input
                type="text"
                class="input"
                list={`llm-models-${chosen.server}`}
                bind:value={chosen.model}
                placeholder="Имя модели"
                aria-label={`Модель: ${action.label}`}
              />
            {/each}
          </div>
          {#each ["local", "external"] as const as server (server)}
            <datalist id={`llm-models-${server}`}>
              {#each checks[server].models as model (model)}
                <option value={model}></option>
              {/each}
            </datalist>
          {/each}
          <p class="note">Заметки по сценам привязаны к модели: после её смены части читаются заново.</p>
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

  .actions-grid {
    display: grid;
    grid-template-columns: minmax(0, 1.3fr) 7.5rem minmax(0, 1.2fr);
    gap: 8px 10px;
    align-items: center;
  }

  .action-name {
    display: flex;
    flex-direction: column;
    font-size: 13px;
    color: #e2e8f0;
  }

  .note {
    margin: 0;
    font-size: 11px;
    color: #64748b;
  }

  .note.warn {
    color: #fbbf24;
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
