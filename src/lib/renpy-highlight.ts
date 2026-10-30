const KEYWORDS = [
  "\\b(?:jump|call|scene|show|hide|with|play|stop|queue|voice|pause|if|elif|pass|return|init|define|default|old|new)\\b",
  "\\b(?:menu|else|python):"
].join("|");

// Строки стоят первыми: слова внутри них не должны считаться ключевыми
const TOKENS = new RegExp(`"(?:[^"\\\\]|\\\\.)*"|'(?:[^'\\\\]|\\\\.)*'|${KEYWORDS}`, "g");

function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// Базовое форматирование строки Ren'Py. Результат выводится через {@html},
// поэтому весь текст файла проходит через escapeHtml.
export function highlightRenpyLine(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) return "&nbsp;";

  // Комментарий
  if (trimmed.startsWith("#")) {
    return `<span class="tok-comment">${escapeHtml(text)}</span>`;
  }

  // Метки сцены
  if (trimmed.startsWith("label ") && trimmed.endsWith(":")) {
    return `<span class="tok-label-line">${escapeHtml(text)}</span>`;
  }

  // Блоки перевода
  if (trimmed.startsWith("translate ") && trimmed.endsWith(":")) {
    return `<span class="tok-translate-line">${escapeHtml(text)}</span>`;
  }

  let out = "";
  let end = 0;
  for (const match of text.matchAll(TOKENS)) {
    const token = match[0];
    const className = token[0] === '"' || token[0] === "'" ? "tok-string" : "tok-keyword";
    out += escapeHtml(text.slice(end, match.index));
    out += `<span class="${className}">${escapeHtml(token)}</span>`;
    end = match.index + token.length;
  }
  return out + escapeHtml(text.slice(end));
}
