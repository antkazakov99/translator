<script lang="ts">
  import {
    Clapperboard,
    Split,
    GitBranch,
    Play,
    SlidersHorizontal,
    ExternalLink
  } from 'lucide-svelte';

  interface Props {
    scene?: { name: string; total_lines?: number; choices?: any[] } | null;
    isExternal?: boolean;
    size?: number;
    class?: string;
  }

  let { scene = null, isExternal = false, size = 14, class: className = '' }: Props = $props();

  let nodeType = $derived.by(() => {
    if (isExternal || !scene) return 'external';
    if (scene.name === 'Интерфейс и меню (strings)') return 'ui';
    if (scene.name.toLowerCase() === 'start') return 'start';
    if (scene.choices && scene.choices.length > 0) return 'choice';
    if ((scene.total_lines ?? 0) === 0) return 'router';
    return 'story';
  });
</script>

<span 
  class="scene-node-icon-wrap {className}" 
  style:width="{size}px" 
  style:height="{size}px"
  style:min-width="{size}px" 
  style:min-height="{size}px"
>
  {#if nodeType === 'external'}
    <ExternalLink {size} class="icon-node-external" />
  {:else if nodeType === 'ui'}
    <SlidersHorizontal {size} class="icon-node-ui" />
  {:else if nodeType === 'start'}
    <Play {size} class="icon-node-start" />
  {:else if nodeType === 'choice'}
    <Split {size} class="icon-node-choice" />
  {:else if nodeType === 'router'}
    <GitBranch {size} class="icon-node-router" />
  {:else}
    <Clapperboard {size} class="icon-node-story" />
  {/if}
</span>

<style>
  .scene-node-icon-wrap {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    line-height: 1;
    vertical-align: middle;
    box-sizing: border-box;
  }

  .scene-node-icon-wrap :global(svg) {
    display: block;
    flex-shrink: 0;
  }

  :global(.icon-node-external) { color: #f59e0b; flex-shrink: 0; }
  :global(.icon-node-ui) { color: #eab308; flex-shrink: 0; }
  :global(.icon-node-start) { color: #10b981; flex-shrink: 0; }
  :global(.icon-node-choice) { color: #c084fc; flex-shrink: 0; }
  :global(.icon-node-router) { color: #818cf8; flex-shrink: 0; }
  :global(.icon-node-story) { color: #38bdf8; flex-shrink: 0; }
</style>
