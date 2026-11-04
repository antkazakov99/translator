<script lang="ts">
  import { normalizeReadingDepth, type ReadingDepth } from "./character-parts";
  import RequestSlider from "./RequestSlider.svelte";

  let { value, partsAvailable = 50, disabled = false, onchange }: {
    value: ReadingDepth;
    partsAvailable?: number;
    disabled?: boolean;
    onchange: (value: ReadingDepth) => void;
  } = $props();
  const id = $props.id();
  let partsLimit = $derived(Math.max(50, partsAvailable, value.max));

  function change(field: keyof ReadingDepth, amount: number) {
    const next = { ...value, [field]: amount };
    // При уменьшении максимума двигаем минимум вместе с ним.
    if (field === "max" && Number.isFinite(next.max)) next.min = Math.min(next.min, Math.max(1, Math.floor(next.max)));
    onchange(normalizeReadingDepth(next));
  }
</script>

<div class="depth-settings">
  <div class="depth-heading">
    <span>Объём чтения</span>
    <strong>{value.percent}% · от {value.min} до {value.max} частей</strong>
  </div>
  <div class="depth-percent">
    <RequestSlider
      id={`${id}-percent`} label="Желаемый процент" status={`${value.percent}%`}
      max={100} value={value.percent} {disabled} onchange={(amount) => change("percent", amount)}
    >
      {#snippet ends()}<span>1%</span><span>весь текст · 100%</span>{/snippet}
    </RequestSlider>
  </div>
  <div class="depth-bounds">
    <div>
      <RequestSlider
        id={`${id}-min`} label="Минимум частей" status={String(value.min)}
        max={partsLimit} value={value.min} {disabled} onchange={(amount) => change("min", amount)}
      >
        {#snippet ends()}<span>1</span><span>{partsLimit}</span>{/snippet}
      </RequestSlider>
    </div>
    <div>
      <RequestSlider
        id={`${id}-max`} label="Максимум частей" status={String(value.max)}
        max={partsLimit} value={value.max} {disabled} onchange={(amount) => change("max", amount)}
      >
        {#snippet ends()}<span>1</span><span>{partsLimit}</span>{/snippet}
      </RequestSlider>
    </div>
  </div>
  <p class="depth-note">Число частей по проценту округляется вверх и ограничивается минимумом и максимумом. Если частей меньше минимума, читаются все. Готовые части сохраняются.</p>
</div>

<style>
  .depth-settings { margin-top: 12px; padding: 12px 14px; border: 1px solid #263449; border-radius: 10px; background: #0f172a; }
  .depth-heading { display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 4px 12px; color: #94a3b8; font-size: 11px; }
  .depth-heading strong { color: #cbd5e1; font-weight: 500; font-variant-numeric: tabular-nums; }
  .depth-percent { padding-bottom: 10px; border-bottom: 1px solid #263449; }
  .depth-bounds { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 4px 20px; }
  .depth-bounds > div { min-width: 0; }
  .depth-note { margin: 10px 0 0; color: #94a3b8; font-size: 11px; line-height: 1.5; }
</style>
