import { Channel } from "@tauri-apps/api/core";
import {
  commands,
  type GameRunDto_Serialize as GameRunResult,
  type GameStep,
  type NamesMode,
  type RunProgress_Deserialize as RunProgress,
  type StepResultDto,
} from "./bindings";
import { chainText, mergeFinished } from "./game-run";
import type { NameSuggestion } from "./glossary";
import { trackRun } from "./run-status.svelte";
import type { Picked } from "./suggest-run.svelte";

/**
 * Цепочка автоперевода: запуск, ход в общей полосе, остановка и отбор имён (все написания в ручном режиме,
 * спорные в «Авто»). Живёт в окне проекта, а не в панели: закрытая панель не обрывает запуск и не теряет
 * отбор и итог.
 */
export class GameRun {
  progress = $state<RunProgress | null>(null);
  stopping = $state(false);
  /** Итог последней цепочки. После отбора имён шаги до него и после складываются. */
  result = $state<GameRunResult | null>(null);
  /** Отбор имён: цепочка встала и ждёт его. */
  names = $state<Picked<NameSuggestion>[] | null>(null);
  /** Языки имён без правил: имена этих языков написаны без них. */
  missingRules = $state<string[]>([]);
  writing = $state(false);
  /** Шаги, отмеченные при последнем запуске: по ним строится итог по шагам. */
  requested = $state<GameStep[]>([]);
  /** Шаги после отбора и режим имён: с ними цепочка продолжится. Первым стоит шаг имён: он спросит формы. */
  private rest: GameStep[] = [];
  private mode: NamesMode = "auto";

  constructor(
    /** Проект окна: берётся при каждом вызове, окно может сменить проект. */
    private readonly project: () => string,
    /** Цепочка что-то записала: таблице пора перечитать прогресс. */
    private readonly onChanged: () => void,
  ) {}

  get active() {
    return this.progress !== null;
  }

  get text() {
    return this.progress ? chainText(this.progress, this.stopping) : "";
  }

  /** Запускает шаги. `earlier`: шаги, пройденные до отбора имён, для общего итога. */
  async start(steps: GameStep[], mode: NamesMode, onError: (message: string) => void, earlier: StepResultDto[] = []) {
    if (this.progress || this.names || steps.length === 0) return;
    this.mode = mode;
    this.stopping = false;
    if (earlier.length === 0) {
      this.result = null;
      this.requested = steps;
    }
    this.progress = { done: 0, total: 0, written: 0, message: null };
    const channel = new Channel<RunProgress>();
    channel.onmessage = (progress) => {
      if (this.progress) this.progress = progress;
    };
    const untrack = trackRun(this);
    try {
      const result = await commands.runGame(this.project(), steps, mode, channel);
      this.result = { ...result, finished: mergeFinished(earlier, result.finished) };
      if (result.names && result.names.items.length > 0) {
        this.names = result.names.items.map((item) => ({ ...item, selected: true }));
        this.missingRules = result.names.missingRules;
        this.rest = result.rest;
      }
    } catch (e) {
      onError(String(e));
    } finally {
      this.progress = null;
      untrack();
      this.onChanged();
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

  get chosen() {
    return this.names?.filter((item) => item.selected && item.translation.trim()) ?? [];
  }

  /** Пишет отмеченные написания и продолжает цепочку следующими шагами. */
  async writeNames(onError: (message: string) => void) {
    if (!this.names) return;
    this.writing = true;
    try {
      const picks = this.chosen.map((item) => ({ character: item.character, field: item.field, source: item.source, translation: item.translation.trim() }));
      if (picks.length > 0) await commands.writeNames(this.project(), picks);
    } catch (e) {
      onError(String(e));
      return;
    } finally {
      this.writing = false;
    }
    await this.continue(onError);
  }

  /** Сброс всего прогресса проекта: итог и отбор прошлой цепочки тоже уходят. `true`: сброшено. */
  async reset(onError: (message: string) => void) {
    if (this.progress) return false;
    try {
      await commands.resetGame(this.project());
    } catch (e) {
      onError(String(e));
      return false;
    }
    this.result = null;
    this.names = null;
    this.missingRules = [];
    this.requested = [];
    this.rest = [];
    this.onChanged();
    return true;
  }

  /** Отбрасывает написания и продолжает цепочку. */
  async skipNames(onError: (message: string) => void) {
    if (!this.names) return;
    await this.continue(onError);
  }

  private async continue(onError: (message: string) => void) {
    this.names = null;
    this.missingRules = [];
    const rest = this.rest;
    this.rest = [];
    this.onChanged();
    if (rest.length > 0) await this.start(rest, this.mode, onError, this.result?.finished ?? []);
  }
}
