<script lang="ts">
  import Combobox from "./Combobox.svelte";
  import { ownerOptions, type FormOwner } from "./name-forms";

  let {
    label,
    suggested = [],
    characters,
    terms = [],
    onchange
  }: {
    label: string;
    /** Записи, на форму которых это похоже. */
    suggested?: FormOwner[];
    characters: FormOwner[];
    terms?: FormOwner[];
    onchange: (value: string) => void;
  } = $props();

  let query = $state("");
</script>

<Combobox
  text={query}
  options={(all) => ownerOptions(all ? "" : query, suggested, characters, terms)}
  {label}
  placeholder="Найти персонажа или термин…"
  empty="Таких записей нет"
  oninput={(next) => query = next}
  onpick={(id) => {
    query = "";
    onchange(id);
  }}
  onblur={() => query = ""}
/>
