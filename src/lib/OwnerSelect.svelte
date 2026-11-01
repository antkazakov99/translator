<script lang="ts">
  import { NEW_CHARACTER, ownerKey, type FormOwner } from "./name-forms";

  let {
    value = "",
    label,
    blank,
    suggested = [],
    characters,
    terms = [],
    onchange
  }: {
    value?: string;
    label: string;
    /** Подпись пустого пункта: что будет, если ничего не выбрать. */
    blank: string;
    /** Записи, на форму которых это похоже. */
    suggested?: FormOwner[];
    characters: FormOwner[];
    terms?: FormOwner[];
    onchange: (value: string) => void;
  } = $props();

  const isSuggested = (owner: FormOwner) => suggested.some((own) => ownerKey(own) === ownerKey(owner));
  const suggestedName = (owner: FormOwner) => (owner.kind === "character" ? `Персонаж ${owner.names[0]}` : owner.names[0]);
</script>

<select
  class="owner-select"
  aria-label={label}
  {value}
  onchange={(event) => onchange(event.currentTarget.value)}
>
  <option value="">{blank}</option>
  <option value={NEW_CHARACTER}>Новый персонаж</option>
  {#if suggested.length > 0}
    <optgroup label="Похоже">
      {#each suggested as owner (ownerKey(owner))}
        <option value={ownerKey(owner)}>{suggestedName(owner)}</option>
      {/each}
    </optgroup>
  {/if}
  <optgroup label="Персонажи">
    {#each characters.filter((owner) => !isSuggested(owner)) as owner (ownerKey(owner))}
      <option value={ownerKey(owner)}>{owner.names[0]}</option>
    {/each}
  </optgroup>
  {#if terms.length > 0}
    <optgroup label="Термины">
      {#each terms.filter((owner) => !isSuggested(owner)) as owner (ownerKey(owner))}
        <option value={ownerKey(owner)}>{owner.names[0]}</option>
      {/each}
    </optgroup>
  {/if}
</select>

<style>
  .owner-select {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    padding: 5px 9px;
    border: 1px solid rgba(51, 65, 85, 0.8);
    border-radius: 6px;
    outline: none;
    background: #090d16;
    font-size: 12px;
    color: #f1f5f9;
    color-scheme: dark;
  }

  .owner-select:focus {
    border-color: #10b981;
  }
</style>
