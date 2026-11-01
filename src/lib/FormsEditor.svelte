<script lang="ts">
  import { Plus, X } from "lucide-svelte";
  import type { NameForm } from "./name-forms";

  let {
    forms,
    label,
    hint,
    placeholder,
    translatedPlaceholder,
    onchange,
    onsplit
  }: {
    forms: NameForm[];
    label: string;
    hint: string;
    placeholder: string;
    translatedPlaceholder: string;
    onchange: (forms: NameForm[]) => void;
    onsplit: (form: NameForm) => void;
  } = $props();

  function edit(index: number, change: Partial<NameForm>) {
    onchange(forms.map((form, own) => (own === index ? { ...form, ...change } : form)));
  }
</script>

<div class="forms">
  <div class="forms-head">
    <span class="forms-label">{label}</span>
    <button class="forms-add" onclick={() => onchange([...forms, { source: "", translated: "" }])}>
      <Plus size={12} />
      <span>Добавить</span>
    </button>
  </div>
  {#each forms as form, index (index)}
    <div class="forms-row">
      <input
        type="text"
        class="forms-input"
        value={form.source}
        {placeholder}
        aria-label={`${label}: оригинал`}
        oninput={(event) => edit(index, { source: event.currentTarget.value })}
      />
      <input
        type="text"
        class="forms-input"
        value={form.translated}
        placeholder={translatedPlaceholder}
        aria-label={`${label}: перевод`}
        oninput={(event) => edit(index, { translated: event.currentTarget.value })}
      />
      <button
        class="forms-split"
        disabled={!form.source.trim()}
        title="Сделать отдельным термином"
        onclick={() => onsplit(form)}
      >В термин</button>
      <button
        class="forms-remove"
        title="Удалить форму"
        aria-label={`Удалить форму ${form.source}`}
        onclick={() => onchange(forms.filter((_, own) => own !== index))}
      ><X size={13} /></button>
    </div>
  {/each}
  {#if forms.length === 0}
    <span class="forms-hint">{hint}</span>
  {/if}
</div>

<style>
  .forms {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 16px;
  }

  .forms-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .forms-label {
    font-size: 12px;
    font-weight: 600;
    color: #94a3b8;
  }

  .forms-hint {
    font-size: 11px;
    color: #64748b;
  }

  .forms-add {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: none;
    background: transparent;
    font-size: 11px;
    color: #a5b4fc;
    cursor: pointer;
  }

  .forms-add:hover {
    color: #e0e7ff;
  }

  .forms-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 6px;
  }

  .forms-input {
    min-width: 0;
    padding: 5px 9px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
  }

  .forms-input:focus {
    border-color: #10b981;
  }

  .forms-split,
  .forms-remove {
    display: flex;
    align-items: center;
    padding: 0;
    border: none;
    background: transparent;
    font-size: 11px;
    color: #64748b;
    white-space: nowrap;
    cursor: pointer;
  }

  .forms-split:hover:not(:disabled),
  .forms-remove:hover {
    color: #f1f5f9;
  }

  .forms-split:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
