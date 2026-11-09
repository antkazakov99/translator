import { Channel } from "@tauri-apps/api/core";
import { commands, type RunProgress_Deserialize as RunProgress } from "./bindings";
import type { GlossaryRuns } from "./glossary";

export function glossaryRuns(): GlossaryRuns {
  return {
    names: new SuggestRun("Имена"),
    terms: new SuggestRun("Термины"),
    speakers: new SuggestRun("Подписи"),
    tokens: new SuggestRun("Токены"),
    entries: new WriteRun("Описание"),
  };
}

/** Предложение в отборе: отмеченные записываются в глоссарий. */
export type Picked<T> = T & { selected: boolean };

/**
 * Запуск предложений модели в разделе глоссария: ход, остановка и отбор. Живёт в `Glossary.svelte`,
 * чтобы запуск и отбор пережили переход между разделами.
 */
export class SuggestRun<T> {
  progress = $state<RunProgress | null>(null);
  stopping = $state(false);
  /** Отбор: `null`, пока предложений нет. */
  review = $state<Picked<T>[] | null>(null);
  notice = $state("");
  /** Что стоит знать перед отбором, например языки имён без правил. Задаёт вызов запуска, живёт до конца отбора. */
  warning = $state("");

  /** Как запуск называется в баннере хода: «Термины». */
  constructor(readonly label: string) {}

  get active() {
    return this.progress !== null;
  }

  /** Отбор открыт: другие запуски ждут, пока его запишут или отбросят. */
  get waiting() {
    return this.review !== null;
  }

  /** Запускает и открывает отбор. Ошибка запуска не теряет готовых предложений: они тоже идут в отбор. */
  async start(call: (channel: Channel<RunProgress>) => Promise<{ items: T[]; error?: string | null }>, onError: (message: string) => void) {
    if (this.progress || this.review) return;
    this.notice = "";
    this.warning = "";
    this.stopping = false;
    this.progress = { done: 0, total: 0, written: 0, message: null };
    const channel = new Channel<RunProgress>();
    channel.onmessage = (progress) => {
      if (this.progress) this.progress = progress;
    };
    try {
      const result = await call(channel);
      if (result.error) onError(result.error);
      if (result.items.length > 0) {
        this.review = result.items.map((item) => ({ ...item, selected: true }));
      } else if (!result.error) {
        this.notice = this.stopping ? "Остановлено. Модель ничего не предложила" : "Модель ничего не предложила";
      }
    } catch (e) {
      onError(String(e));
    } finally {
      this.progress = null;
    }
  }

  async stop(onError: (message: string) => void) {
    if (!this.progress) return;
    this.stopping = true;
    try {
      await commands.cancelLlmRun();
    } catch (e) {
      onError(String(e));
    }
  }

  /** Закрывает отбор с итогом записи. */
  finish(notice: string) {
    this.review = null;
    this.notice = notice;
    this.warning = "";
  }

  discard() {
    this.review = null;
    this.warning = "";
  }

  get chosen() {
    return this.review?.filter((item) => item.selected) ?? [];
  }

  /** «Термины: пакет 1 из 3 · предложений: 12». */
  get text() {
    if (!this.progress) return "";
    if (this.stopping) return `${this.label}: останавливается после текущего запроса`;
    const { done, total, written } = this.progress;
    if (total === 0) return `${this.label}: подготовка`;
    return `${this.label}: пакет ${Math.min(done + 1, total)} из ${total} · предложений: ${written}`;
  }
}

/**
 * Запуск, который пишет сам, без отбора: описания сцен. Итог это число записанных и ошибка, которая
 * оборвала запуск.
 */
export class WriteRun {
  progress = $state<RunProgress | null>(null);
  stopping = $state(false);
  notice = $state("");
  readonly waiting = false;

  constructor(readonly label: string) {}

  get active() {
    return this.progress !== null;
  }

  /** `notice` получает число записанных и признак остановки. Возвращает записанное. */
  async start(
    call: (channel: Channel<RunProgress>) => Promise<{ written: string[]; error?: string | null }>,
    onError: (message: string) => void,
    notice: (written: number, stopped: boolean) => string,
  ): Promise<string[]> {
    if (this.progress) return [];
    this.notice = "";
    this.stopping = false;
    this.progress = { done: 0, total: 0, written: 0, message: null };
    const channel = new Channel<RunProgress>();
    channel.onmessage = (progress) => {
      if (this.progress) this.progress = progress;
    };
    try {
      const result = await call(channel);
      if (result.error) onError(result.error);
      this.notice = notice(result.written.length, this.stopping);
      return result.written;
    } catch (e) {
      onError(String(e));
      return [];
    } finally {
      this.progress = null;
    }
  }

  async stop(onError: (message: string) => void) {
    if (!this.progress) return;
    this.stopping = true;
    try {
      await commands.cancelLlmRun();
    } catch (e) {
      onError(String(e));
    }
  }

  /** «Описания: запрос 2 из 7 · записано: 1». */
  get text() {
    if (!this.progress) return "";
    if (this.stopping) return `${this.label}: останавливается после текущего запроса`;
    const { done, total, written } = this.progress;
    if (total === 0) return `${this.label}: подготовка`;
    return `${this.label}: запрос ${Math.min(done + 1, total)} из ${total} · записано: ${written}`;
  }
}
