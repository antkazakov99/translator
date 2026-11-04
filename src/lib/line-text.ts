const TEXT_TAG = /\{[^}]*\}/g;
const TEXT_VARIABLE = /\[([A-Za-z_][\w.]*)(?:![a-z]+)?\]/g;

/**
 * Есть ли в реплике что переводить, как `is_wordless` в Rust. Токен без значения вроде [mc] подставит игра,
 * а токен со значением модель видит обычным словом, поэтому он считается по значению.
 */
export function hasWords(text: string, tokens: { name: string; value?: string }[]): boolean {
  const values = new Map(tokens.map((token) => [token.name, token.value?.trim() ?? ""]));
  const plain = text.replace(TEXT_TAG, "").replace(TEXT_VARIABLE, (_, name: string) => values.get(name) ?? "");
  return /\p{L}/u.test(plain);
}
