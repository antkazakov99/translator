<script lang="ts">
  import { Plus, X } from "lucide-svelte";
  import type { NameForm } from "./glossary";

  /** Другие формы имени или названия: обращения, прозвища, сокращения. Изменение уходит в `onchange` по `change`. */
  let {
    forms,
    hint,
    disabled = false,
    onchange,
  }: { forms: NameForm[]; hint: string; disabled?: boolean; onchange: (forms: NameForm[]) => void } = $props();

  let rows = $state<NameForm[]>([]);
  $effect.pre(() => {
    rows = forms.map((form) => ({ ...form }));
  });

  function commit() {
    onchange(rows.filter((form) => form.text.trim()).map((form) => ({ text: form.text.trim(), translation: form.translation?.trim() || null })));
  }

  function remove(index: number) {
    rows.splice(index, 1);
    commit();
  }
</script>

<div class="forms">
  <div class="forms-head">
    <span class="label">Другие формы</span>
    <button class="icon-button" aria-label="Добавить форму" title="Добавить форму" onclick={() => rows.push({ text: "", translation: null })} {disabled}>
      <Plus size={14} />
    </button>
  </div>
  {#each rows as form, index (index)}
    <div class="form-row">
      <input class="input" placeholder="Как написано в игре" bind:value={form.text} onchange={commit} {disabled} aria-label="Форма" />
      <input class="input" placeholder="Перевод" bind:value={form.translation} onchange={commit} {disabled} aria-label="Перевод формы" />
      <button class="icon-button" aria-label="Убрать форму" title="Убрать форму" onclick={() => remove(index)} {disabled}><X size={13} /></button>
    </div>
  {:else}
    <span class="hint">{hint}</span>
  {/each}
</div>

<style>
  .forms {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Подпись блока и «добавить» одной строкой. */
  .forms-head {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
    min-height: 28px;
  }

  .label {
    color: var(--text-muted);
    font-size: 11px;
  }

  .form-row {
    display: grid;
    grid-template-columns: 1fr 1fr 28px;
    gap: 6px;
    align-items: center;
  }

  .hint {
    color: var(--text-faint);
    font-size: 12px;
  }
</style>
