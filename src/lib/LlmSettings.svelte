<script lang="ts">
  import { onMount, untrack } from "svelte";
  import {
    ArrowDown,
    ArrowUp,
    Check,
    ChevronDown,
    ChevronRight,
    KeyRound,
    LoaderCircle,
    Plus,
    RefreshCw,
    Trash2,
    TriangleAlert,
    X,
  } from "lucide-svelte";
  import ModelCombobox from "./ModelCombobox.svelte";
  import {
    commands,
    type ActionDto_Serialize as ActionDto,
    type LlmSettingsDto_Serialize as Settings,
    type ProfileDto_Serialize as ProfileDto,
    type ProviderDto,
    type ProviderKind,
  } from "$lib/bindings";
  import { PROVIDER_KINDS, kindOf, newProviderName, optionalInteger, optionalTemperature, textModels } from "$lib/llm-catalog";

  let { onClose }: { onClose: () => void } = $props();

  type Tab = "providers" | "profiles" | "actions";

  interface ProviderDraft {
    /** Постоянный ключ карточки: у нового провайдера id ещё нет. */
    uid: string;
    saved: ProviderDto | null;
    name: string;
    kind: ProviderKind;
    url: string;
    /** Новый ключ. Пусто: остаётся сохранённый. */
    key: string;
    busy: boolean;
    error: string;
    confirmDelete: boolean;
  }

  interface ProfileDraft {
    uid: string;
    saved: ProfileDto | null;
    providerId: string;
    name: string;
    model: string;
    context: string;
    reasoning: string;
    timeout: string;
    params: string;
    open: boolean;
    busy: boolean;
    error: string;
    confirmDelete: boolean;
  }

  interface ActionDraft {
    saved: ActionDto;
    profile: string;
    temperature: string;
    batch: string;
    busy: boolean;
    error: string;
  }

  interface ModelList {
    loading: boolean;
    ok: boolean | null;
    message: string;
    models: string[];
  }

  const TABS: { id: Tab; label: string }[] = [
    { id: "providers", label: "Провайдеры" },
    { id: "profiles", label: "Модели" },
    { id: "actions", label: "Действия" },
  ];

  let tab = $state<Tab>("providers");
  let loading = $state(true);
  let loadError = $state("");
  let keysPersistent = $state(true);
  let defaults = $state({ reasoning: 8192, timeout: 300 });
  let providers = $state<ProviderDraft[]>([]);
  let profiles = $state<ProfileDraft[]>([]);
  let actions = $state<ActionDraft[]>([]);
  /** Списки моделей по `uid` провайдера. */
  let modelLists = $state<Record<string, ModelList>>({});
  let confirmClose = $state(false);

  const savedProviders = $derived(providers.flatMap((draft) => (draft.saved ? [draft.saved] : [])));
  const savedProfiles = $derived(profiles.flatMap((draft) => (draft.saved ? [draft.saved] : [])));
  const dirty = $derived(providers.some(providerDirty) || profiles.some(profileDirty));

  const numberText = (value: number | null) => (value === null ? "" : String(value));
  const needsKey = (kind: ProviderKind) => kind === "openai" || kind === "claude" || kind === "gemini";

  function providerDraft(saved: ProviderDto): ProviderDraft {
    return { uid: saved.id, saved, name: saved.name, kind: saved.kind, url: saved.url, key: "", busy: false, error: "", confirmDelete: false };
  }

  function profileDraft(saved: ProfileDto): ProfileDraft {
    return {
      uid: saved.id,
      saved,
      providerId: saved.providerId,
      name: saved.name,
      model: saved.model,
      context: numberText(saved.contextTokens),
      reasoning: numberText(saved.reasoningTokens),
      timeout: numberText(saved.timeoutSecs),
      params: saved.params,
      open: false,
      busy: false,
      error: "",
      confirmDelete: false,
    };
  }

  function actionDraft(saved: ActionDto): ActionDraft {
    return {
      saved,
      profile: saved.profile ?? "",
      temperature: numberText(saved.temperature),
      batch: numberText(saved.batchSize),
      busy: false,
      error: "",
    };
  }

  /** Раскладывает ответ по черновикам. Несохранённые профили остаются, если их провайдер жив. */
  function apply(settings: Settings, withProviders: boolean) {
    keysPersistent = settings.keysPersistent;
    defaults = { reasoning: settings.defaultReasoningTokens, timeout: settings.defaultTimeoutSecs };
    if (withProviders) providers = settings.providers.map(providerDraft);
    const unsaved = profiles.filter((draft) => !draft.saved && settings.providers.some((provider) => provider.id === draft.providerId));
    profiles = [...settings.profiles.map(profileDraft), ...unsaved];
    actions = settings.actions.map(actionDraft);
  }

  async function load(withProviders = true) {
    try {
      apply(await commands.llmSettings(), withProviders);
      loadError = "";
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function requestClose() {
    if (dirty && !confirmClose) {
      confirmClose = true;
      return;
    }
    onClose();
  }

  // Провайдеры

  function providerDirty(draft: ProviderDraft) {
    const saved = draft.saved;
    return !saved || draft.name !== saved.name || draft.url !== saved.url || draft.kind !== saved.kind || draft.key.trim() !== "";
  }

  function providerInput(draft: ProviderDraft) {
    return { id: draft.saved?.id ?? null, name: draft.name, kind: draft.kind, url: draft.url, key: draft.key.trim() || null };
  }

  function addProvider(kind: ProviderKind) {
    providers.push({
      uid: crypto.randomUUID(),
      saved: null,
      name: newProviderName(kind, providers.map((draft) => draft.name)),
      kind,
      url: kindOf(kind).url,
      key: "",
      busy: false,
      error: "",
      confirmDelete: false,
    });
  }

  async function saveProvider(draft: ProviderDraft) {
    draft.busy = true;
    draft.error = "";
    try {
      const saved = await commands.saveLlmProvider(providerInput(draft));
      draft.saved = saved;
      draft.name = saved.name;
      draft.url = saved.url;
      draft.key = "";
      if (!modelLists[draft.uid]?.ok) void loadModels(draft);
    } catch (e) {
      draft.error = String(e);
    } finally {
      draft.busy = false;
    }
  }

  async function deleteProvider(draft: ProviderDraft) {
    if (!draft.saved) {
      providers = providers.filter((other) => other.uid !== draft.uid);
      return;
    }
    draft.busy = true;
    try {
      await commands.deleteLlmProvider(draft.saved.id);
      providers = providers.filter((other) => other.uid !== draft.uid);
      delete modelLists[draft.uid];
      // Профили провайдера ушли каскадом, у действий мог обнулиться профиль.
      await load(false);
    } catch (e) {
      draft.error = String(e);
      draft.busy = false;
    }
  }

  async function forgetKey(draft: ProviderDraft) {
    if (!draft.saved) return;
    draft.busy = true;
    draft.error = "";
    try {
      await commands.forgetLlmKey(draft.saved.id);
      draft.saved = { ...draft.saved, hasKey: false };
    } catch (e) {
      draft.error = String(e);
    } finally {
      draft.busy = false;
    }
  }

  /** Облачному сервису без ключа отвечать нечем. */
  const canAsk = (draft: ProviderDraft) =>
    draft.url.trim() !== "" && (!needsKey(draft.kind) || draft.key.trim() !== "" || !!draft.saved?.hasKey);

  async function loadModels(draft: ProviderDraft) {
    modelLists[draft.uid] ??= { loading: false, ok: null, message: "", models: [] };
    const list = modelLists[draft.uid];
    list.loading = true;
    try {
      const all = await commands.llmModels(providerInput(draft));
      list.models = textModels(all);
      const hidden = all.length - list.models.length;
      list.ok = true;
      list.message = all.length > 0
        ? `Моделей: ${list.models.length}${hidden > 0 ? `, скрыто нетекстовых: ${hidden}` : ""}`
        : "Сервер ответил, но список моделей пуст";
    } catch (e) {
      list.ok = false;
      list.models = [];
      list.message = String(e);
    } finally {
      list.loading = false;
    }
  }

  const providerOf = (id: string) => providers.find((draft) => draft.saved?.id === id);

  // Списки моделей нужны, когда открыта вкладка моделей: ключ уходит только на адрес своего провайдера.
  $effect(() => {
    if (tab !== "profiles") return;
    untrack(() => {
      for (const draft of providers) {
        if (draft.saved && !modelLists[draft.uid] && canAsk(draft)) void loadModels(draft);
      }
    });
  });

  // Профили

  function profileDirty(draft: ProfileDraft) {
    const saved = draft.saved;
    if (!saved) return true;
    return (
      draft.providerId !== saved.providerId ||
      draft.name !== saved.name ||
      draft.model !== saved.model ||
      draft.context !== numberText(saved.contextTokens) ||
      draft.reasoning !== numberText(saved.reasoningTokens) ||
      draft.timeout !== numberText(saved.timeoutSecs) ||
      draft.params !== saved.params
    );
  }

  function addProfile() {
    const provider = savedProviders[0];
    if (!provider) return;
    profiles.push({
      uid: crypto.randomUUID(),
      saved: null,
      providerId: provider.id,
      name: "",
      model: "",
      context: "",
      reasoning: "",
      timeout: "",
      params: "",
      open: false,
      busy: false,
      error: "",
      confirmDelete: false,
    });
  }

  function pickProvider(draft: ProfileDraft) {
    const provider = providerOf(draft.providerId);
    if (provider && !modelLists[provider.uid] && canAsk(provider)) void loadModels(provider);
  }

  async function saveProfile(draft: ProfileDraft) {
    const context = optionalInteger(draft.context, 1);
    const reasoning = optionalInteger(draft.reasoning, 0);
    const timeout = optionalInteger(draft.timeout, 1);
    const wrong =
      context === undefined ? "Окно контекста: целое число больше нуля"
      : reasoning === undefined ? "Запас на размышления: целое число"
      : timeout === undefined ? "Таймаут: целое число секунд больше нуля"
      : "";
    if (wrong) {
      draft.error = wrong;
      draft.open = true;
      return;
    }
    draft.busy = true;
    draft.error = "";
    try {
      const saved = await commands.saveLlmProfile({
        id: draft.saved?.id ?? null,
        providerId: draft.providerId,
        name: draft.name,
        model: draft.model,
        contextTokens: context ?? null,
        reasoningTokens: reasoning ?? null,
        timeoutSecs: timeout ?? null,
        params: draft.params,
      });
      Object.assign(draft, { ...profileDraft(saved), uid: draft.uid, open: draft.open });
    } catch (e) {
      draft.error = String(e);
    } finally {
      draft.busy = false;
    }
  }

  async function deleteProfile(draft: ProfileDraft) {
    if (!draft.saved) {
      profiles = profiles.filter((other) => other.uid !== draft.uid);
      return;
    }
    draft.busy = true;
    try {
      await commands.deleteLlmProfile(draft.saved.id);
      profiles = profiles.filter((other) => other.uid !== draft.uid);
      // У действий с этим профилем он обнулился.
      actions = (await commands.llmSettings()).actions.map(actionDraft);
    } catch (e) {
      draft.error = String(e);
      draft.busy = false;
    }
  }

  /** Сдвигает сохранённый профиль на соседнее место среди сохранённых. */
  async function move(index: number, delta: -1 | 1) {
    const other = index + delta;
    if (other < 0 || other >= profiles.length || !profiles[index].saved || !profiles[other].saved) return;
    [profiles[index], profiles[other]] = [profiles[other], profiles[index]];
    try {
      await commands.reorderLlmProfiles(savedProfiles.map((profile) => profile.id));
    } catch (e) {
      loadError = String(e);
      await load(false);
    }
  }

  // Действия

  /** Провайдер, которому уйдёт действие: свой профиль или первый в списке. */
  function actionProvider(draft: ActionDraft) {
    const id = draft.profile || savedProfiles[0]?.id;
    const profile = savedProfiles.find((item) => item.id === id);
    return savedProviders.find((provider) => provider.id === profile?.providerId) ?? null;
  }

  async function saveAction(draft: ActionDraft) {
    const temperature = optionalTemperature(draft.temperature);
    const batch = optionalInteger(draft.batch, 1);
    if (temperature === undefined) {
      draft.error = "Температура: число от 0 до 2";
      return;
    }
    if (batch === undefined) {
      draft.error = "Размер пакета: целое число больше нуля";
      return;
    }
    draft.busy = true;
    draft.error = "";
    try {
      const saved = await commands.saveLlmAction(draft.saved.id, { profile: draft.profile || null, temperature, batchSize: batch });
      Object.assign(draft, actionDraft(saved));
    } catch (e) {
      draft.error = String(e);
    } finally {
      draft.busy = false;
    }
  }
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape") requestClose(); }} />

{#snippet modelStatus(provider: ProviderDraft | undefined)}
  {@const list = provider ? modelLists[provider.uid] : undefined}
  {#if provider}
    <button class="small" onclick={() => loadModels(provider)} disabled={list?.loading || !canAsk(provider)} title={canAsk(provider) ? "" : "Нужны адрес и ключ"}>
      {#if list?.loading}<LoaderCircle size={13} class="spin" />{:else}<RefreshCw size={13} />{/if}
      <span>Проверить</span>
    </button>
    {#if list?.ok === true}
      <span class="status ok"><Check size={13} /> {list.message}</span>
    {:else if list?.ok === false}
      <span class="status failed" title={list.message}><TriangleAlert size={13} /> {list.message}</span>
    {/if}
  {/if}
{/snippet}

{#snippet deleteControl(target: { busy: boolean; confirmDelete: boolean }, remove: () => void, label: string)}
  {#if target.confirmDelete}
    <span class="confirm">
      <span>Удалить?</span>
      <button class="small danger" onclick={remove} disabled={target.busy}>Да</button>
      <button class="small" onclick={() => (target.confirmDelete = false)}>Нет</button>
    </span>
  {:else}
    <button class="icon-button danger" aria-label={label} title={label} onclick={() => (target.confirmDelete = true)} disabled={target.busy}>
      <Trash2 size={14} />
    </button>
  {/if}
{/snippet}

<div
  class="backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.target === event.currentTarget) requestClose();
  }}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Настройки моделей">
    <header class="dialog-head">
      <h2>Настройки моделей</h2>
      <nav class="tabs" aria-label="Разделы">
        {#each TABS as item (item.id)}
          <button class="tab" class:active={tab === item.id} onclick={() => (tab = item.id)}>{item.label}</button>
        {/each}
      </nav>
      <button class="icon-button" aria-label="Закрыть" onclick={requestClose}><X size={16} /></button>
    </header>

    <div class="dialog-body">
      {#if !keysPersistent}
        <div class="banner warn" role="status">
          <TriangleAlert size={15} />
          <span>Хранилище ключей системы недоступно: ключи API забудутся при закрытии приложения.</span>
        </div>
      {/if}
      {#if loadError}
        <div class="banner error" role="alert">
          <TriangleAlert size={15} />
          <span>{loadError}</span>
          <button class="icon-button" aria-label="Скрыть ошибку" onclick={() => (loadError = "")}><X size={14} /></button>
        </div>
      {/if}

      {#if loading}
        <p class="muted"><LoaderCircle size={14} class="spin" /> Загрузка настроек</p>
      {:else if tab === "providers"}
        <p class="hint">Провайдер это сервер или аккаунт, куда уходят запросы. Ключи лежат в хранилище учётных данных системы.</p>
        {#each providers as draft (draft.uid)}
          {@const kind = kindOf(draft.kind)}
          <section class="card">
            <div class="card-head">
              <span class="badge">{kind.label}</span>
              <input class="input grow" bind:value={draft.name} aria-label="Название провайдера" placeholder="Название" />
              {@render deleteControl(draft, () => deleteProvider(draft), `Удалить провайдера ${draft.name}`)}
            </div>
            <div class="fields" class:single={draft.kind === "local"}>
              <label class="field">
                <span>Адрес{draft.kind === "local" ? " (LM Studio, Ollama)" : draft.kind === "compatible" ? " (OpenAI-совместимый API)" : ""}</span>
                <input class="input mono" bind:value={draft.url} placeholder={kind.url || "https://openrouter.ai/api/v1"} />
              </label>
              {#if draft.kind !== "local"}
                <label class="field">
                  <span>
                    Ключ API
                    {#if draft.saved?.hasKey}<span class="key-set"><KeyRound size={11} /> сохранён</span>{/if}
                  </span>
                  <input
                    class="input"
                    type="password"
                    autocomplete="off"
                    bind:value={draft.key}
                    placeholder={draft.saved?.hasKey ? "Новый ключ заменит сохранённый" : needsKey(draft.kind) ? "Нужен для запросов" : "Если сервер просит"}
                  />
                </label>
              {/if}
            </div>
            {#if kind.keyHint}<p class="hint">{kind.keyHint}</p>{/if}
            {#if draft.url.trim().startsWith("http://") && draft.kind !== "local" && (draft.key.trim() || draft.saved?.hasKey)}
              <p class="hint warn-text">По адресу http:// ключ уходит открытым текстом.</p>
            {/if}
            <div class="card-foot">
              {@render modelStatus(draft)}
              <span class="spacer"></span>
              {#if draft.saved?.hasKey}
                <button class="small" onclick={() => forgetKey(draft)} disabled={draft.busy}>Забыть ключ</button>
              {/if}
              {#if providerDirty(draft)}
                <button class="small primary" onclick={() => saveProvider(draft)} disabled={draft.busy}>
                  {#if draft.busy}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
                  <span>Сохранить</span>
                </button>
              {/if}
            </div>
            {#if draft.error}<p class="error-text">{draft.error}</p>{/if}
          </section>
        {:else}
          <p class="muted">Провайдеров пока нет.</p>
        {/each}
        <div class="add-row">
          <span class="muted">Добавить:</span>
          {#each PROVIDER_KINDS as kind (kind.id)}
            <button class="small" onclick={() => addProvider(kind.id)}><Plus size={12} /><span>{kind.label}</span></button>
          {/each}
        </div>
      {:else if tab === "profiles"}
        <p class="hint">Модель это профиль: провайдер, имя модели и лимиты. Первую в списке берут действия, у которых своя не выбрана.</p>
        {#if savedProviders.length === 0}
          <p class="muted">Сначала сохраните провайдера на вкладке «Провайдеры».</p>
        {/if}
        {#each profiles as draft, index (draft.uid)}
          {@const provider = providerOf(draft.providerId)}
          <section class="card">
            <div class="card-head">
              {#if draft.saved && index === 0}
                <span class="badge accent" title="Её берут действия без своей модели">По умолчанию</span>
              {:else}
                <span class="badge">{draft.saved ? index + 1 : "Новая"}</span>
              {/if}
              <input class="input grow" bind:value={draft.name} aria-label="Название модели" placeholder={draft.model || "Название, по умолчанию имя модели"} />
              <button class="icon-button" aria-label="Выше" title="Выше" onclick={() => move(index, -1)} disabled={!draft.saved || index === 0 || draft.busy}>
                <ArrowUp size={14} />
              </button>
              <button
                class="icon-button"
                aria-label="Ниже"
                title="Ниже"
                onclick={() => move(index, 1)}
                disabled={!draft.saved || !profiles[index + 1]?.saved || draft.busy}
              >
                <ArrowDown size={14} />
              </button>
              {@render deleteControl(draft, () => deleteProfile(draft), `Удалить модель ${draft.name || draft.model}`)}
            </div>
            <div class="fields">
              <label class="field">
                <span>Провайдер</span>
                <select class="input" bind:value={draft.providerId} onchange={() => pickProvider(draft)}>
                  {#each savedProviders as option (option.id)}
                    <option value={option.id}>{option.name}</option>
                  {/each}
                </select>
              </label>
              <div class="field">
                <span>Модель</span>
                <ModelCombobox
                  bind:value={draft.model}
                  models={provider ? (modelLists[provider.uid]?.models ?? []) : []}
                  label={`Модель профиля ${draft.name}`}
                  placeholder="Имя модели"
                />
              </div>
            </div>
            <button class="disclosure" onclick={() => (draft.open = !draft.open)} aria-expanded={draft.open}>
              {#if draft.open}<ChevronDown size={13} />{:else}<ChevronRight size={13} />{/if}
              <span>Дополнительно</span>
            </button>
            {#if draft.open}
              <div class="fields three">
                <label class="field">
                  <span>Окно контекста, токенов</span>
                  <input class="input" inputmode="numeric" bind:value={draft.context} placeholder="Не задано" />
                </label>
                <label class="field">
                  <span>Запас на размышления</span>
                  <input class="input" inputmode="numeric" bind:value={draft.reasoning} placeholder={String(defaults.reasoning)} />
                </label>
                <label class="field">
                  <span>Таймаут, секунд</span>
                  <input class="input" inputmode="numeric" bind:value={draft.timeout} placeholder={String(defaults.timeout)} />
                </label>
              </div>
              <label class="field">
                <span>Дополнительные поля запроса, JSON</span>
                <textarea class="input mono" rows="3" bind:value={draft.params} placeholder={'{ "top_p": 0.9 }'}></textarea>
              </label>
              <p class="hint">Поля приложения (модель, сообщения, температура, лимит ответа) отсюда не переопределяются.</p>
            {/if}
            <div class="card-foot">
              {@render modelStatus(provider)}
              <span class="spacer"></span>
              {#if profileDirty(draft)}
                <button class="small primary" onclick={() => saveProfile(draft)} disabled={draft.busy || !draft.model.trim()}>
                  {#if draft.busy}<LoaderCircle size={13} class="spin" />{:else}<Check size={13} />{/if}
                  <span>Сохранить</span>
                </button>
              {/if}
            </div>
            {#if draft.error}<p class="error-text">{draft.error}</p>{/if}
          </section>
        {/each}
        {#if savedProviders.length > 0}
          <div class="add-row">
            <button class="small" onclick={addProfile}><Plus size={12} /><span>Добавить модель</span></button>
          </div>
        {/if}
      {:else}
        <p class="hint">Пустое поле значит значение по умолчанию. Здесь настройки всех проектов.</p>
        <div class="actions">
          <div class="actions-head">
            <span>Действие</span>
            <span>Модель</span>
            <span>Температура</span>
            <span>Пакет</span>
          </div>
          {#each actions as draft (draft.saved.id)}
            {@const provider = actionProvider(draft)}
            <div class="action-row" class:busy={draft.busy}>
              <span class="action-title">{draft.saved.title}</span>
              <select class="input" bind:value={draft.profile} onchange={() => saveAction(draft)} aria-label={`Модель: ${draft.saved.title}`}>
                <option value="">{savedProfiles[0] ? `По умолчанию (${savedProfiles[0].name})` : "По умолчанию"}</option>
                {#each savedProfiles as profile (profile.id)}
                  <option value={profile.id}>{profile.name}</option>
                {/each}
              </select>
              <input
                class="input"
                inputmode="decimal"
                bind:value={draft.temperature}
                onchange={() => saveAction(draft)}
                placeholder={String(draft.saved.defaultTemperature)}
                disabled={provider !== null && !provider.takesTemperature}
                title={provider !== null && !provider.takesTemperature ? "ChatGPT не принимает температуру" : ""}
                aria-label={`Температура: ${draft.saved.title}`}
              />
              <input
                class="input"
                inputmode="numeric"
                bind:value={draft.batch}
                onchange={() => saveAction(draft)}
                placeholder={String(draft.saved.defaultBatchSize)}
                aria-label={`Размер пакета: ${draft.saved.title}`}
              />
              {#if draft.error}<p class="error-text wide">{draft.error}</p>{/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <footer class="dialog-foot">
      {#if confirmClose}
        <span class="warn-text">Есть несохранённые изменения.</span>
        <button class="small" onclick={() => (confirmClose = false)}>Остаться</button>
        <button class="small danger" onclick={onClose}>Закрыть без сохранения</button>
      {:else}
        <button class="small primary" onclick={requestClose}>Готово</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(5, 7, 10, 0.72);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 820px;
    height: min(720px, 100%);
    overflow: hidden;
    border: 1px solid #2a3548;
    border-radius: 12px;
    background: #0d0f12;
    color: #d4deeb;
    color-scheme: dark;
    font-family: Inter, "Segoe UI", system-ui, sans-serif;
    box-shadow: 0 24px 60px -12px rgba(0, 0, 0, 0.7);
  }

  .dialog-head,
  .dialog-foot {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    background: #11151d;
  }

  .dialog-head {
    border-bottom: 1px solid #2a3548;
  }

  .dialog-foot {
    justify-content: flex-end;
    border-top: 1px solid #2a3548;
  }

  h2 {
    margin: 0;
    color: #f1f5f9;
    font-size: 15px;
    font-weight: 650;
  }

  .tabs {
    display: flex;
    gap: 4px;
    flex: 1;
    margin-left: 12px;
  }

  .tab {
    padding: 6px 12px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: #8492a8;
    font-size: 12px;
    font-weight: 600;
  }

  .tab:hover {
    color: #f1f5f9;
  }

  .tab.active {
    border-color: #2a3548;
    background: #1a2433;
    color: #f1f5f9;
  }

  .dialog-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    flex: 1;
    padding: 14px 16px 18px;
    overflow-y: auto;
  }

  button {
    font: inherit;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .small {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border: 1px solid #34445d;
    border-radius: 8px;
    background: #1a2433;
    color: #d4deeb;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }

  .small:hover:not(:disabled) {
    border-color: #3b82f6;
    background: #20334f;
  }

  .small.primary {
    border-color: #2563eb;
    background: #1d4ed8;
    color: #f8fafc;
  }

  .small.primary:hover:not(:disabled) {
    background: #2563eb;
  }

  .small.danger:hover:not(:disabled) {
    border-color: #f87171;
    background: #3b1820;
    color: #fca5a5;
  }

  .icon-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #8290a5;
  }

  .icon-button:hover:not(:disabled) {
    background: #252e3d;
    color: #f1f5f9;
  }

  .icon-button.danger:hover:not(:disabled) {
    background: #3b1820;
    color: #fca5a5;
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid #2a3548;
    border-radius: 10px;
    background: #11151d;
  }

  .card-head,
  .card-foot,
  .add-row,
  .confirm {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .add-row {
    flex-wrap: wrap;
  }

  .confirm {
    font-size: 12px;
    color: #fca5a5;
  }

  .spacer {
    flex: 1;
  }

  .badge {
    padding: 3px 9px;
    border-radius: 999px;
    background: #1a2433;
    color: #9fb0c8;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }

  .badge.accent {
    background: rgba(59, 130, 246, 0.18);
    color: #93c5fd;
  }

  .fields {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
    gap: 10px;
  }

  .fields.single {
    grid-template-columns: minmax(0, 1fr);
  }

  .fields.three {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    color: #8492a8;
    font-size: 11px;
  }

  .key-set {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-left: 6px;
    color: #6ee7b7;
  }

  .input {
    box-sizing: border-box;
    min-width: 0;
    padding: 7px 10px;
    border: 1px solid #2a3548;
    border-radius: 7px;
    outline: none;
    background: #0b0e13;
    color: #f1f5f9;
    font: inherit;
    font-size: 12px;
  }

  .input:focus {
    border-color: #3b82f6;
  }

  .input:disabled {
    opacity: 0.5;
  }

  .grow {
    flex: 1;
  }

  .mono {
    font-family: ui-monospace, monospace;
  }

  textarea.input {
    resize: vertical;
  }

  .disclosure {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: 4px;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: #8492a8;
    font-size: 11px;
    font-weight: 600;
  }

  .disclosure:hover {
    color: #f1f5f9;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    overflow: hidden;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status.ok {
    color: #6ee7b7;
  }

  .status.failed {
    color: #fca5a5;
  }

  .hint,
  .muted {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: #6b7a90;
    font-size: 11px;
  }

  .muted {
    font-size: 12px;
  }

  .warn-text {
    color: #fbbf24;
    font-size: 12px;
  }

  .hint.warn-text {
    font-size: 11px;
  }

  .error-text {
    margin: 0;
    color: #fca5a5;
    font-size: 12px;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid #2a3548;
    border-radius: 10px;
    background: #11151d;
    color: #cbd5e1;
    font-size: 12px;
  }

  .banner span {
    flex: 1;
  }

  .banner.warn {
    border-color: #5b4a1c;
    color: #fcd34d;
  }

  .banner.error {
    border-color: #5b2330;
    color: #fca5a5;
  }

  .actions {
    display: flex;
    flex-direction: column;
    border: 1px solid #2a3548;
    border-radius: 10px;
    background: #11151d;
  }

  .actions-head,
  .action-row {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1.6fr) 110px 90px;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
  }

  .actions-head {
    border-bottom: 1px solid #2a3548;
    color: #6b7a90;
    font-size: 11px;
    font-weight: 600;
  }

  .action-row + .action-row {
    border-top: 1px solid #1c2433;
  }

  .action-row.busy {
    opacity: 0.7;
  }

  .action-title {
    color: #d4deeb;
    font-size: 12px;
  }

  .wide {
    grid-column: 1 / -1;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
