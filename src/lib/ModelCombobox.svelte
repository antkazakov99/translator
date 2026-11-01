<script lang="ts">
  import Combobox from "./Combobox.svelte";

  let {
    value = $bindable(""),
    models,
    label,
    placeholder = "",
    fallback = "",
    disabled = false
  }: {
    value: string;
    models: string[];
    label: string;
    placeholder?: string;
    /** Модель, которая будет использована при пустом значении. Показывается как обычный текст. */
    fallback?: string;
    disabled?: boolean;
  } = $props();

  // Пока идёт ввод, поле можно очистить: запасная модель вернётся, когда фокус уйдёт
  let editing = $state(false);
  let text = $derived(value || editing ? value : fallback);

  // Выбранная модель не сужает список: иначе в нём осталась бы только она
  function options(all: boolean) {
    const query = value.trim().toLowerCase();
    const shown = all || models.includes(value.trim())
      ? models
      : models.filter((model) => model.toLowerCase().includes(query));
    return shown.map((model) => ({ id: model, label: model }));
  }
</script>

<Combobox
  {text}
  {options}
  {label}
  {placeholder}
  {disabled}
  selected={text.trim()}
  empty={models.length > 0 ? "Таких моделей в списке нет" : "Список появится, когда подключение ответит. Проверьте адрес и ключ"}
  oninput={(next) => {
    editing = true;
    value = next;
  }}
  onpick={(model) => value = model}
  onblur={() => editing = false}
/>
