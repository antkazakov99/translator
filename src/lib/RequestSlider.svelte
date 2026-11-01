<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    id,
    label,
    status,
    min,
    max,
    value,
    trackStyle,
    segmented = false,
    disabled = false,
    oninput,
    ends
  }: {
    id: string;
    label: string;
    status: string;
    min: number;
    max: number;
    value: number;
    trackStyle?: string;
    segmented?: boolean;
    disabled?: boolean;
    oninput: (event: Event & { currentTarget: HTMLInputElement }) => void;
    ends: Snippet;
  } = $props();

  // Центр бегунка ходит между половинами его ширины по краям шкалы
  let fillStyle = $derived.by(() => {
    const share = max > min ? (value - min) / (max - min) : 0;
    return `--ready-stop: 0px; --parts-stop: ${share >= 1 ? "100%" : `calc(7px + (100% - 14px) * ${share})`}`;
  });
</script>

<div class="slider">
  <div class="slider-head">
    <label for={id}>{label}</label>
    <span class="slider-status">{status}</span>
  </div>
  <input {id} type="range" {min} {max} {value} style={trackStyle ?? fillStyle} class:segmented {disabled} {oninput} />
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
