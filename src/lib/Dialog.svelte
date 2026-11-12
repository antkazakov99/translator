<script lang="ts" module>
  /** Открытые окна по порядку: `Escape` закрывает только верхнее. */
  const opened: object[] = [];
</script>

<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { X } from "lucide-svelte";

  /**
   * Окно действия проекта («Автоперевод», «Версия игры», «Мод»): подложка, шапка с пояснением, тело и подвал.
   * Высота по содержимому, окно прижато к верху и не прыгает, когда содержимое растёт.
   */
  let {
    title,
    note = "",
    closeTitle = "Закрыть",
    onClose,
    actions,
    foot,
    children,
  }: {
    title: string;
    /** Пояснение под заголовком. */
    note?: string;
    closeTitle?: string;
    onClose: () => void;
    /** Кнопки-иконки окна слева от крестика. */
    actions?: Snippet;
    foot?: Snippet;
    children: Snippet;
  } = $props();

  const token = {};
  let dialog: HTMLDivElement;

  onMount(() => {
    opened.push(token);
    dialog.focus();
    return () => {
      const index = opened.indexOf(token);
      if (index >= 0) opened.splice(index, 1);
    };
  });

  function onKey(event: KeyboardEvent) {
    if (event.key !== "Escape" || opened.at(-1) !== token) return;
    event.preventDefault();
    onClose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="backdrop"
  role="presentation"
  onclick={(event) => {
    if (event.target === event.currentTarget) onClose();
  }}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} tabindex="-1" bind:this={dialog}>
    <header class="dialog-head">
      <div class="heading">
        <h2>{title}</h2>
        {#if note}<span>{note}</span>{/if}
      </div>
      <div class="head-actions">
        {@render actions?.()}
        <button class="icon-button" aria-label="Закрыть" title={closeTitle} onclick={onClose}><X size={16} /></button>
      </div>
    </header>
    <div class="dialog-body">
      {@render children()}
    </div>
    {#if foot}
      <footer class="dialog-foot">
        {@render foot()}
      </footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 10vh 24px 24px;
    background: var(--backdrop);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 820px;
    max-height: 100%;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 12px;
    outline: none;
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 24px 60px -12px rgba(0, 0, 0, 0.7);
  }

  .dialog-head,
  .dialog-foot {
    display: flex;
    flex: 0 0 auto;
    gap: 12px;
    padding: 12px 16px;
    background: var(--surface);
  }

  .dialog-head {
    align-items: flex-start;
    border-bottom: 1px solid var(--divider);
  }

  .dialog-foot {
    align-items: center;
    justify-content: flex-end;
    border-top: 1px solid var(--divider);
  }

  .heading {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  h2 {
    margin: 0;
    color: var(--text-strong);
    font-size: 15px;
    font-weight: 650;
  }

  .heading span {
    color: var(--text-faint);
    font-size: 11px;
  }

  .head-actions {
    display: flex;
    flex: 0 0 auto;
    gap: 2px;
    align-items: center;
  }

  .dialog-body {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
    padding: 14px 16px 18px;
    overflow-y: auto;
  }
</style>
