/** Подпись сцены: название вместе с меткой из скрипта. Пока названия нет, остаётся метка. */
export function sceneLabel(scene: string, title = "") {
  const name = title.trim();
  return name ? `${name} (${scene})` : scene;
}
