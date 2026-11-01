<script lang="ts" module>
  let nextListId = 0;
</script>

<script lang="ts">
  import { ChevronDown } from "lucide-svelte";

  interface Option {
    id: string;
    label: string;
    /** Подпись над первым пунктом группы. Пункты одной группы идут подряд. */
    group?: string;
  }

  let {
    text,
    options,
    label,
    placeholder = "",
    empty,
    selected = "",
    disabled = false,
    oninput,
    onpick,
    onblur
  }: {
    text: string;
    /** Пункты списка. `all` значит, что список открыли целиком, без поиска по тексту. */
    options: (all: boolean) => Option[];
    label: string;
    placeholder?: string;
    /** Подпись пустого списка. */
    empty: string;
    selected?: string;
    disabled?: boolean;
    oninput: (text: string) => void;
    onpick: (id: string) => void;
    onblur?: () => void;
  } = $props();

  const LIST_HEIGHT = 240;
  const listId = `combobox-list-${nextListId++}`;

  let root = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let open = $state(false);
  let showAll = $state(false);
  let active = $state(-1);
  let listStyle = $state("");
  let shown = $derived(options(showAll));

  $effect(() => {
    if (active >= 0) list?.querySelector(".option.active")?.scrollIntoView({ block: "nearest" });
  });

  /** Список закреплён на экране: прокручиваемое окно обрезало бы его. */
  function openList(all: boolean) {
    if (!root) return;
    const box = root.getBoundingClientRect();
    const below = window.innerHeight - box.bottom;
    const edge = below < LIST_HEIGHT && box.top > below
      ? `bottom: ${window.innerHeight - box.top + 4}px`
      : `top: ${box.bottom + 4}px`;
    listStyle = `left: ${box.left}px; width: ${box.width}px; ${edge}`;
    showAll = all;
    active = -1;
    open = true;
  }

  function pick(id: string) {
    open = false;
    onpick(id);
  }

  function onkeydown(event: KeyboardEvent) {
    if (!open) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        openList(true);
      }
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (shown.length > 0) active = (active + 1) % shown.length;
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      if (shown.length > 0) active = (active <= 0 ? shown.length : active) - 1;
    } else if (event.key === "Enter" && active >= 0) {
      event.preventDefault();
      pick(shown[active].id);
    } else if (event.key === "Escape") {
      // Закрывается только список, а не всё окно
      event.stopPropagation();
      open = false;
    }
  }
</script>

<svelte:window
  onresize={() => open = false}
  onscrollcapture={(event) => { if (event.target !== list) open = false; }}
/>

<div class="combobox" bind:this={root}>
  <input
    type="text"
    class="input"
    role="combobox"
    aria-expanded={open}
    aria-controls={listId}
    aria-autocomplete="list"
    aria-label={label}
    autocomplete="off"
    spellcheck="false"
    {placeholder}
    {disabled}
    value={text}
    onfocus={() => openList(false)}
    onclick={() => { if (!open) openList(false); }}
    oninput={(event) => {
      oninput(event.currentTarget.value);
      openList(false);
    }}
    onblur={() => {
      open = false;
      onblur?.();
    }}
    {onkeydown}
  />
  <button
    type="button"
    class="toggle"
    class:flipped={open}
    tabindex="-1"
    {disabled}
    aria-label={`Показать список: ${label}`}
    onmousedown={(event) => event.preventDefault()}
    onclick={() => { if (open) open = false; else openList(true); }}
  ><ChevronDown size={14} /></button>

  {#if open}
    <div id={listId} class="list" role="listbox" aria-label={label} style={listStyle} bind:this={list}>
      {#each shown as option, index (option.id)}
        {#if option.group && option.group !== shown[index - 1]?.group}
          <p class="group" role="presentation">{option.group}</p>
        {/if}
        <button
          type="button"
          class="option"
          class:active={index === active}
          class:selected={option.id === selected}
          role="option"
          aria-selected={option.id === selected}
          tabindex="-1"
          onmousedown={(event) => event.preventDefault()}
          onclick={() => pick(option.id)}
          onmousemove={() => active = index}
        >{option.label}</button>
      {:else}
        <p class="empty">{empty}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .combobox {
    position: relative;
    min-width: 0;
  }

  .input {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    padding: 6px 28px 6px 10px;
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

  .toggle {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    padding: 0 7px;
    border: none;
    background: transparent;
    color: #64748b;
    cursor: pointer;
  }

  .toggle:hover:not(:disabled) {
    color: #f1f5f9;
  }

  .input:disabled,
  .toggle:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .toggle.flipped :global(svg) {
    transform: rotate(180deg);
  }

  .list {
    position: fixed;
    z-index: 10;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    max-height: 240px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid #334155;
    border-radius: 8px;
    background: #141824;
    box-shadow: 0 12px 28px -8px rgba(0, 0, 0, 0.8);
  }

  .option {
    flex-shrink: 0;
    padding: 5px 8px;
    border: none;
    border-radius: 5px;
    background: transparent;
    overflow: hidden;
    font-size: 12px;
    color: #cbd5e1;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .option.active {
    color: #f8fafc;
    background: rgba(99, 102, 241, 0.25);
  }

  .option.selected {
    color: #34d399;
  }

  .group {
    flex-shrink: 0;
    margin: 0;
    padding: 6px 8px 3px;
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
  }

  .empty {
    margin: 0;
    padding: 6px 8px;
    font-size: 11px;
    color: #94a3b8;
  }
</style>
