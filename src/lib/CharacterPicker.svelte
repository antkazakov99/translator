<script lang="ts">
  let {
    characters,
    unselected,
    disabled = false,
    onchange,
    onempty
  }: {
    characters: { alias: string; name: string }[];
    unselected: string[];
    disabled?: boolean;
    onchange: (unselected: string[]) => void;
    onempty: () => void;
  } = $props();
</script>

<details class="picker">
  <summary>Выбрано: {characters.length - unselected.length} из {characters.length}</summary>
  <div class="picker-toggles">
    <button class="btn-link" {disabled} onclick={() => onchange([])}>Выбрать всех</button>
    <button class="btn-link" {disabled} onclick={() => onchange(characters.map((c) => c.alias))}>Снять всех</button>
    <button class="btn-link" {disabled} onclick={onempty}>Незаполненные</button>
  </div>
  <div class="picker-grid">
    {#each characters as character (character.alias)}
      <label class="picker-check">
        <input
          type="checkbox"
          checked={!unselected.includes(character.alias)}
          {disabled}
          onchange={(e) => {
            onchange(e.currentTarget.checked
              ? unselected.filter((alias) => alias !== character.alias)
              : [...unselected, character.alias]);
          }}
        />
        <span>{character.name}</span>
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

  .picker-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 4px 12px;
  }

  .picker-check {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
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
