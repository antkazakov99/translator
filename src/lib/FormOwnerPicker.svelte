<script lang="ts">
  import { linkOptions, type FormOwner } from "./name-forms";

  let {
    label,
    suggested,
    characters,
    linked,
    onchange
  }: {
    label: string;
    /** Персонажи, на форму которых это похоже. */
    suggested: FormOwner[];
    characters: FormOwner[];
    /** Ключи уже связанных персонажей. */
    linked: string[];
    onchange: (id: string, checked: boolean) => void;
  } = $props();

  let query = $state("");
  let options = $derived(linkOptions(query, suggested, characters, linked));
</script>

<div class="links">
  <input type="text" class="links-search" aria-label={label} placeholder="Найти персонажа…" spellcheck="false" bind:value={query} />
  <div class="links-list">
    {#each options as option, index (option.id)}
      {#if option.group !== options[index - 1]?.group}
        <p class="links-group">{option.group}</p>
      {/if}
      <label class="links-row" title={option.name}>
        <input
          type="checkbox"
          checked={linked.includes(option.id)}
          onchange={(event) => onchange(option.id, event.currentTarget.checked)}
        />
        <span>{option.name}</span>
      </label>
    {:else}
      <p class="links-empty">Таких персонажей нет</p>
    {/each}
  </div>
</div>

<style>
  .links {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color-scheme: dark;
  }

  .links-search {
    box-sizing: border-box;
    width: 100%;
    padding: 6px 10px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
  }

  .links-search:focus {
    border-color: #10b981;
  }

  .links-list {
    display: flex;
    flex-direction: column;
    max-height: 240px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid #1e2638;
    border-radius: 6px;
    background: #0d111b;
  }

  .links-group {
    flex-shrink: 0;
    margin: 0;
    padding: 6px 6px 3px;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
  }

  .links-row {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 4px 6px;
    border-radius: 5px;
    font-size: 12px;
    color: #cbd5e1;
    cursor: pointer;
  }

  .links-row:hover {
    background: rgba(99, 102, 241, 0.18);
    color: #f8fafc;
  }

  .links-row input {
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    margin: 0;
    accent-color: #818cf8;
  }

  .links-row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .links-empty {
    margin: 0;
    padding: 6px;
    font-size: 11px;
    color: #94a3b8;
  }
</style>
