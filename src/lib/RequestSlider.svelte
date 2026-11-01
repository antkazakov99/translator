<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    id,
    label,
    status,
    max,
    value,
    floor = 1,
    ready = 0,
    disabled = false,
    onchange,
    ends
  }: {
    id: string;
    label: string;
    status: string;
    /** Шкала делится на `max` частей, и значение равно числу закрашенных. */
    max: number;
    value: number;
    /** Ниже этого значения бегунок не опускается. */
    floor?: number;
    /** Части, которые уже готовы: они выделены своим цветом. */
    ready?: number;
    disabled?: boolean;
    onchange: (value: number) => void;
    ends: Snippet;
  } = $props();

  // Если частей слишком много, зазоры съели бы сами сегменты
  const MAX_SEGMENTS_WITH_GAPS = 80;

  // Центр бегунка ходит между половинами его ширины по краям шкалы
  function stop(parts: number) {
    if (parts <= 0) return "0px";
    return parts >= max ? "100%" : `calc(7px + (100% - 14px) * ${parts / max})`;
  }

  function oninput(event: Event & { currentTarget: HTMLInputElement }) {
    const wanted = Math.max(event.currentTarget.valueAsNumber, floor);
    event.currentTarget.value = String(wanted);
    onchange(wanted);
  }
</script>

<div class="slider">
  <div class="slider-head">
    <label for={id}>{label}</label>
    <span class="slider-status">{status}</span>
  </div>
  <input
    {id}
    type="range"
    min="0"
    {max}
    {value}
    style={`--ready-stop: ${stop(ready)}; --parts-stop: ${stop(value)}; --segment: calc((100% + 2px) / ${max})`}
    class:segmented={max <= MAX_SEGMENTS_WITH_GAPS}
    {disabled}
    {oninput}
  />
  <div class="slider-ends">{@render ends()}</div>
</div>

<style>
  .slider {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
    font-size: 12px;
    color: #cbd5e1;
  }

  .slider-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .slider-status {
    margin-left: auto;
    font-size: 11px;
    color: #94a3b8;
  }

  .slider input {
    --track: linear-gradient(
      to right,
      #38bdf8 var(--ready-stop),
      #10b981 var(--ready-stop) var(--parts-stop),
      #1e293b var(--parts-stop)
    );
    appearance: none;
    width: 100%;
    height: 16px;
    margin: 0;
    background: transparent;
    cursor: pointer;
  }

  .slider input:disabled {
    cursor: default;
  }

  .slider input::-webkit-slider-runnable-track {
    height: 8px;
    border-radius: 4px;
    background-image: var(--track);
  }

  /* Зазоры уже шкалы на ширину бегунка: его центр ходит ровно по границам сегментов.
     Слой короче ещё на зазор, чтобы последний не отрезал от конца шкалы кусочек */
  .slider input.segmented::-webkit-slider-runnable-track {
    background-image:
      repeating-linear-gradient(
        to right,
        transparent 0 calc(var(--segment) - 2px),
        #0f172a calc(var(--segment) - 2px) var(--segment)
      ),
      var(--track);
    background-position: 7px center, 0 0;
    background-repeat: no-repeat;
    background-size: calc(100% - 16px) 100%, 100% 100%;
  }

  .slider input::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -3px;
    border: 2px solid #0f172a;
    border-radius: 50%;
    background: #f8fafc;
  }

  .slider input:disabled::-webkit-slider-thumb {
    background: #64748b;
  }

  .slider input:focus-visible {
    outline: 1px solid #10b981;
    outline-offset: 2px;
    border-radius: 4px;
  }

  .slider-ends {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: #64748b;
  }
</style>
