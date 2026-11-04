<script lang="ts">
  let {
    items,
    unselected,
    disabled = false,
    onchange,
    onempty
  }: {
    items: { id: string; name: string }[];
    unselected: string[];
    disabled?: boolean;
    onchange: (unselected: string[]) => void;
    onempty: () => void;
  } = $props();

  // В снятых могут остаться записи, которых в глоссарии уже нет
  let off = $derived(new Set(unselected));
  let selected = $derived(items.filter((item) => !off.has(item.id)).length);
</script>

<details class="picker">
  <summary>Выбрано: {selected} из {items.length}</summary>
  <div class="picker-toggles">
    <button class="btn-link" {disabled} onclick={() => onchange([])}>Выбрать все</button>
    <button class="btn-link" {disabled} onclick={() => onchange(items.map((item) => item.id))}>Снять все</button>
    <button class="btn-link" {disabled} onclick={onempty}>Незаполненные</button>
  </div>
  <div class="picker-grid">
    {#each items as item (item.id)}
      <label class="picker-check" title={item.name}>
        <input
          type="checkbox"
          checked={!off.has(item.id)}
          {disabled}
          onchange={(e) => {
            onchange(e.currentTarget.checked
              ? unselected.filter((id) => id !== item.id)
              : [...unselected, item.id]);
          }}
        />
        <span>{item.name}</span>
      </label>
    {/each}
  </div>
</details>

<style>
  .picker {
    font-size: 12px;
    color: #cbd5e1;
  }

  .picker summary {
    color: #94a3b8;
    cursor: pointer;
  }

  .picker-toggles {
    display: flex;
    gap: 10px;
    margin: 4px 0;
  }

  /* Записи бывают от слова до целой фразы, поэтому метка занимает ширину своего текста */
  .picker-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-height: 200px;
    overflow-y: auto;
    padding: 1px;
  }

  .picker-check {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
    padding: 4px 10px 4px 8px;
    border: 1px solid #334155;
    border-radius: 6px;
    background: #111827;
    font-size: 12px;
    line-height: 16px;
    color: #94a3b8;
    cursor: pointer;
  }

  .picker-check:hover {
    border-color: #818cf8;
  }

  .picker-check:has(input:checked) {
    border-color: #6366f1;
    background: #2a2870;
    color: #fff;
  }

  .picker-check:has(input:disabled) {
    opacity: 0.5;
    cursor: default;
  }

  .picker-check input {
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    margin: 0;
    accent-color: #818cf8;
  }

  .picker-check span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .btn-link {
    padding: 0;
    border: none;
    background: transparent;
    font-size: 11px;
    color: #a5b4fc;
    cursor: pointer;
  }

  .btn-link:hover:not(:disabled) {
    color: #e0e7ff;
  }

  .btn-link:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
