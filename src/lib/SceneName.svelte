<script lang="ts">
  import { sceneLabel } from "./scene-label";

  let { scene, title = "" }: { scene: string; title?: string } = $props();
</script>

<!-- Две строки: в одной длинная метка не оставила бы места названию на узких панелях -->
<span class="scene-name" title={sceneLabel(scene, title)}>
  <strong>{title.trim() || scene}</strong>
  {#if title.trim()}<span class="scene-id">{scene}</span>{/if}
</span>

<style>
  .scene-name {
    display: inline-flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.2;
  }

  strong,
  .scene-id {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    font-size: 13px;
    font-weight: 700;
    color: #f1f5f9;
  }

  .scene-id {
    font-family: monospace;
    font-size: 10.5px;
    color: #64748b;
  }
</style>
