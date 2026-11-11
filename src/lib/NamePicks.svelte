<script lang="ts">
  import type { Picked } from "./suggest-run.svelte";
  import { languageList, nameFieldLabels, type NameSuggestion } from "./glossary";

  /**
   * Отбор написаний имён от модели: флажок, написание с вариантами и пояснением. Список правится на месте:
   * окно берёт отмеченные и их написания. Нужен разделу «Персонажи» и карточке «Вся игра».
   */
  let { items, owner }: { items: Picked<NameSuggestion>[]; owner: (id: number) => string } = $props();
</script>

<div class="name-picks">
  {#each items as item, index (index)}
    {@const variant = item.variants.find((variant) => variant.translation === item.translation.trim())}
    <div class="pick-row names" class:off={!item.selected}>
      <input type="checkbox" bind:checked={item.selected} aria-label="Записать написание «{item.source}»" />
      <div class="name">
        <strong>{item.source}</strong>
        <span>{nameFieldLabels[item.field]} · {owner(item.character)}{item.languages.length ? ` · ${languageList(item.languages)}` : ""}</span>
        {#each item.issues as issue, issueIndex (issueIndex)}<span class="issue">{issue}</span>{/each}
      </div>
      <input class="input" bind:value={item.translation} aria-label="Написание «{item.source}»" />
      <div class="variants">
        {#if item.variants.length > 1}
          <div class="segmented" role="radiogroup" aria-label="Варианты написания «{item.source}»">
            {#each item.variants as variant (variant.translation)}
              <button
                role="radio"
                aria-checked={item.translation.trim() === variant.translation}
                class:on={item.translation.trim() === variant.translation}
                title={variant.description ?? ""}
                onclick={() => (item.translation = variant.translation)}>{variant.translation}</button
              >
            {/each}
          </div>
        {/if}
        <span class="faint">{variant ? (variant.description ?? "") : item.translation.trim() ? "Своё написание" : ""}</span>
      </div>
    </div>
  {/each}
</div>

<style>
  .pick-row {
    display: grid;
    grid-template-columns: auto minmax(160px, 1fr) minmax(140px, 1fr) 1.6fr;
    gap: 10px;
    align-items: center;
    padding: 8px 4px;
    border-bottom: 1px solid var(--divider-row);
  }

  .pick-row.off {
    opacity: 0.55;
  }

  .name {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .name strong {
    overflow: hidden;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name span {
    overflow: hidden;
    color: var(--text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name .issue {
    color: var(--warning);
    white-space: normal;
  }

  .variants {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-start;
    min-width: 0;
  }

  .variants .segmented {
    flex-wrap: wrap;
  }

  .faint {
    color: var(--text-faint);
    font-size: 11px;
  }
</style>
