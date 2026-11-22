import { isKnownKind } from "../api/transition";
import { Automation } from "../automation/Automation";

/**
 * 音のミキサーの論理状態 (`docs/sound.md`)。
 *
 * **論理状態が正で、音の実体は写しである。** 時間・遷移・`after`・ダッキングの判定・削除はすべてここで決め、
 * Web Audio にはその結果を `AudioCommand` として渡すだけにする (`WebAudioOutput.ts`)。
 * ネイティブの `engine/desktop-rust/crates/core/src/sound.rs` と同じ規則で、同じ指示を出す
 * (エンジンが 2 つある間は二重に書く。canvas と同じ)。
 *
 * - id 空間は SoundObject・Bus・Effect の 3 つ (Worker がそれぞれ 1 から採番する)。
 * - バスは木で、根は 1 本だけが出力に繋がる。エンジンは木を保持するだけで、どのバスが何のためかは知らない。
 * - 音量はすべて dB。-60 dB 以下は無音。
 * - パラメータの遷移は canvas と同じ `Automation` で動かす。Hz は log2 に直して持ち、対数の上で補間する。
 * - 開始・停止・stop_all は `start` (= `sys_timeline_fire`) で発火する。
 */

/** これ以下の dB は無音。 */
export const SILENCE_DB = -60;

export const EffectKind = { PassFilter: 1, Compressor: 20, Duck: 30 } as const;

export const PassFilterParam = { LowCut: 0, HighCut: 1, MinHz: 20, MaxHz: 20000 } as const;
const COMPRESSOR_DEFAULTS = [0, 1, 10, 100, 0];
const DUCK_DEFAULTS = [0, 80, 500];
const DuckParam = { Amount: 0, Attack: 1, Release: 2 } as const;
const ObjectParam = { Volume: 0, X: 1, Y: 2, Rate: 3 } as const;

export interface EffectSpec {
  id: number;
  kind: number;
  /** 24 dB/oct (pass filter)。 */
  steep: boolean;
  /** 鳴らす側から見たパラメータ (pass filter は Hz、compressor は 5 つ、duck は今の dB)。 */
  params: number[];
}

/** 鳴らす側への指示。ネイティブの `AudioCommand` と同じ。 */
export type AudioCommand =
  | { kind: "openOutput"; bus: number; effects: EffectSpec[]; volumeDb: number }
  | { kind: "createBus"; bus: number; parent: number; effects: EffectSpec[]; volumeDb: number }
  | { kind: "deleteBus"; bus: number }
  | {
      kind: "play";
      object: number;
      sound: number;
      bus: number;
      offsetMs: number;
      loopStartMs: number;
      loopEndMs: number | null;
      repeat: number;
      volumeDb: number;
      x: number;
    }
  | { kind: "stop"; object: number }
  | { kind: "setObject"; object: number; volumeDb: number; x: number }
  | { kind: "setBus"; bus: number; volumeDb: number }
  | { kind: "setEffect"; effect: number; param: number; value: number };

type Parent = { kind: "none" } | { kind: "bus"; id: number } | { kind: "output" };

interface Bus {
  parent: Parent;
  children: number[];
  effects: number[];
  materialized: boolean;
  deleteAt: number | null;
}

interface Effect {
  kind: number;
  /** 挿したバス。作っただけで挿していない effect は何もしない。 */
  bus: number | null;
  steep: boolean;
  key: number;
  duckGain: number;
}

type ObjectState =
  | { kind: "idle" }
  | { kind: "scheduled"; at: number; offset: number; repeat: number }
  | { kind: "waiting"; offset: number; repeat: number }
  | { kind: "playing"; end: number };

interface SoundObject {
  sound: number;
  bus: number;
  loopStart: number;
  loopEnd: number;
  state: ObjectState;
  fired: number;
  sounding: boolean;
  sent: [number, number];
}

type Pending =
  | { kind: "start"; object: number; after: number; offset: number; repeat: number }
  | { kind: "stop"; object: number; after: number }
  | { kind: "stopAll"; bus: number; after: number };

type Stop = { kind: "object"; id: number } | { kind: "all"; bus: number; before: number };

interface SyncBatch {
  objects: number[];
  stops: Array<[number, Stop]>;
}

const obj = (id: number): string => `o:${id}`;
const busKey = (id: number): string => `b:${id}`;
const fx = (id: number): string => `e:${id}`;

export class Mixer {
  private readonly buses = new Map<number, Bus>();
  private readonly effects = new Map<number, Effect>();
  private readonly objects = new Map<number, SoundObject>();
  private root: number | null = null;
  private readonly params = new Automation();
  private pending: Pending[] = [];
  private stops: Array<[number, Stop]> = [];
  private syncBatches: SyncBatch[] = [];
  private fireSeq = 0;
  private readonly lengths = new Map<number, number>();
  private readonly sentBus = new Map<number, number>();
  private readonly sentEffect = new Map<string, number>();
  private commands: AudioCommand[] = [];
  private lastNow = 0;

  // --- バス ---------------------------------------------------------------

  busCreate(id: number): void {
    if (this.buses.has(id)) throw new Error(`sound bus ${id} already exists`);
    this.buses.set(id, { parent: { kind: "none" }, children: [], effects: [], materialized: false, deleteAt: null });
    this.params.define(busKey(id), 0, 0);
  }

  /** effect の列の末尾に、作ってある effect を挿す。繋いだ後は挿せない。1 つの effect は 1 つのバスにしか挿せない。 */
  busAddEffect(bus: number, effect: number): void {
    const b = this.bus(bus);
    if (b.parent.kind !== "none") {
      throw new Error(`sound bus ${bus} is already attached; effects must be added before attaching`);
    }
    const e = this.effects.get(effect);
    if (e === undefined) throw new Error(`no such sound effect: ${effect}`);
    if (e.bus !== null) {
      throw new Error(`sound effect ${effect} is already in sound bus ${e.bus}; an effect can be in only one bus`);
    }
    e.bus = bus;
    b.effects.push(effect);
  }

  // --- effect -------------------------------------------------------------

  /** effect を作る (まだどのバスにも挿さない)。パラメータは既定値で始まる。 */
  effectCreate(id: number, kind: number, cfg1: number, cfg2: number): void {
    if (kind !== EffectKind.PassFilter && kind !== EffectKind.Compressor && kind !== EffectKind.Duck) {
      throw new Error(`unknown sound effect kind: ${kind}`);
    }
    if (kind === EffectKind.Duck && cfg2 !== 0) {
      throw new Error(`duck mode ${cfg2} is not supported (only 0 = activity)`);
    }
    if (this.effects.has(id)) throw new Error(`sound effect ${id} already exists`);
    this.effects.set(id, {
      kind,
      bus: null,
      steep: kind === EffectKind.PassFilter && cfg1 === 1,
      key: kind === EffectKind.Duck ? cfg1 : 0,
      duckGain: 0,
    });
    const defaults =
      kind === EffectKind.PassFilter
        ? [Math.log2(PassFilterParam.MinHz), Math.log2(PassFilterParam.MaxHz)]
        : kind === EffectKind.Compressor
          ? COMPRESSOR_DEFAULTS
          : DUCK_DEFAULTS;
    defaults.forEach((v, i) => this.params.define(fx(id), i, v));
  }

  busAttach(bus: number, parent: number): void {
    if (bus === parent) throw new Error(`sound bus ${bus} cannot be its own parent`);
    const p = this.bus(parent);
    const b = this.bus(bus);
    if (b.parent.kind !== "none") throw new Error(`sound bus ${bus} is already attached`);
    if (this.ancestors(parent).includes(bus)) {
      throw new Error(`attaching sound bus ${bus} under ${parent} would make a cycle`);
    }
    b.parent = { kind: "bus", id: parent };
    p.children.push(bus);
    if (p.materialized) this.materialize(bus);
  }

  outputAttach(bus: number): void {
    if (this.root !== null) {
      throw new Error(
        `sound bus ${this.root} is already connected to the output; only one root can be connected (ignored bus ${bus})`,
      );
    }
    const b = this.bus(bus);
    if (b.parent.kind !== "none") {
      throw new Error(`sound bus ${bus} is attached under another bus and cannot be the root`);
    }
    b.parent = { kind: "output" };
    this.root = bus;
    this.materialize(bus);
  }

  busDelete(bus: number, after: number, now: number): void {
    this.bus(bus).deleteAt = now + Math.max(after, 0);
  }

  busAddStopAll(bus: number, after: number): void {
    this.bus(bus);
    this.pending.push({ kind: "stopAll", bus, after: Math.max(after, 0) });
  }

  busAddTransition(bus: number, param: number, kind: number, value: number, after: number, duration: number): void {
    this.bus(bus);
    if (param !== 0) throw new Error(`unknown sound bus param: ${param}`);
    this.checkKind(kind);
    this.params.push(busKey(bus), 0, kind, value, after, duration);
  }

  busSetParam(bus: number, param: number, value: number): void {
    this.bus(bus);
    if (param !== 0) throw new Error(`unknown sound bus param: ${param}`);
    this.params.set(busKey(bus), 0, value);
  }

  // --- effect -------------------------------------------------------------

  private effectValue(effect: number, param: number, value: number): number {
    const e = this.effects.get(effect);
    if (e === undefined) throw new Error(`no such sound effect: ${effect}`);
    if (!this.params.has(fx(effect), param)) {
      throw new Error(`unknown param ${param} for sound effect ${effect}`);
    }
    if (e.kind === EffectKind.PassFilter) {
      return Math.log2(clamp(value, PassFilterParam.MinHz, PassFilterParam.MaxHz));
    }
    return value;
  }

  effectAddTransition(effect: number, param: number, kind: number, value: number, after: number, duration: number): void {
    const v = this.effectValue(effect, param, value);
    this.checkKind(kind);
    this.params.push(fx(effect), param, kind, v, after, duration);
  }

  effectSetParam(effect: number, param: number, value: number): void {
    this.params.set(fx(effect), param, this.effectValue(effect, param, value));
  }

  // --- 再生 ---------------------------------------------------------------

  create(
    id: number,
    sound: number,
    bus: number,
    volumeDb: number,
    x: number,
    y: number,
    loopStartMs: number,
    loopEndMs: number,
  ): void {
    if (this.objects.has(id)) throw new Error(`sound object ${id} already exists`);
    this.bus(bus);
    this.objects.set(id, {
      sound,
      bus,
      loopStart: Math.max(loopStartMs, 0),
      loopEnd: Math.max(loopEndMs, 0),
      state: { kind: "idle" },
      fired: 0,
      sounding: false,
      sent: [NaN, NaN],
    });
    this.params.define(obj(id), ObjectParam.Volume, volumeDb);
    this.params.define(obj(id), ObjectParam.X, clamp(x, -1, 1));
    this.params.define(obj(id), ObjectParam.Y, clamp(y, -1, 1));
  }

  addStart(id: number, after: number, offset: number, repeat: number): void {
    // 終わった (解放した) SoundObject への操作は黙って無視する (鳴り終わった SE を止めるのはよくあること)。
    if (!this.objects.has(id)) return;
    this.pending.push({
      kind: "start",
      object: id,
      after: Math.max(after, 0),
      offset: Math.max(offset, 0),
      repeat: Math.max(repeat, 0),
    });
  }

  addStop(id: number, after: number): void {
    // 終わった (解放した) SoundObject への操作は黙って無視する (鳴り終わった SE を止めるのはよくあること)。
    if (!this.objects.has(id)) return;
    this.pending.push({ kind: "stop", object: id, after: Math.max(after, 0) });
  }

  addTransition(id: number, param: number, kind: number, value: number, after: number, duration: number): void {
    // 終わった (解放した) SoundObject への操作は黙って無視する (鳴り終わった SE を止めるのはよくあること)。
    if (!this.objects.has(id)) return;
    if (param === ObjectParam.Rate) throw new Error("the sound object param `rate` (3) is reserved");
    if (param !== ObjectParam.Volume && param !== ObjectParam.X && param !== ObjectParam.Y) {
      throw new Error(`unknown sound object param: ${param}`);
    }
    this.checkKind(kind);
    const v = param === ObjectParam.Volume ? value : clamp(value, -1, 1);
    this.params.push(obj(id), param, kind, v, after, duration);
  }

  /** 音のファイルが読めた (`lengthMs`)、または読めなかった (`null`。長さ 0 として扱う)。 */
  soundLoaded(sound: number, lengthMs: number | null, now: number): void {
    this.lengths.set(sound, Math.max(lengthMs ?? 0, 0));
    for (const [id, o] of this.objects) {
      if (o.sound === sound && o.state.kind === "waiting") {
        this.begin(id, now, o.state.offset, o.state.repeat);
      }
    }
  }

  start(sync: boolean, now: number): void {
    this.params.start(sync, now);
    const pending = this.pending;
    this.pending = [];
    if (pending.length === 0) return;
    this.fireSeq += 1;
    const seq = this.fireSeq;
    const batch: SyncBatch = { objects: [], stops: [] };
    for (const op of pending) {
      if (op.kind === "start") {
        const o = this.objects.get(op.object);
        if (o === undefined) continue;
        o.state = { kind: "scheduled", at: now + op.after, offset: op.offset, repeat: op.repeat };
        o.fired = seq;
        if (sync) batch.objects.push(op.object);
      } else {
        const stop: [number, Stop] =
          op.kind === "stop"
            ? [now + op.after, { kind: "object", id: op.object }]
            : [now + op.after, { kind: "all", bus: op.bus, before: seq }];
        (sync ? batch.stops : this.stops).push(stop);
      }
    }
    if (sync && (batch.objects.length > 0 || batch.stops.length > 0)) {
      this.syncBatches.push(batch);
    }
  }

  // --- sync -----------------------------------------------------------------

  hasSync(): boolean {
    return this.params.hasSync() || this.syncBatches.some((b) => this.batchAlive(b));
  }

  private batchAlive(batch: SyncBatch): boolean {
    if (batch.stops.length > 0) return true;
    return batch.objects.some((id) => {
      const s = this.objects.get(id)?.state;
      if (s === undefined) return false;
      if (s.kind === "scheduled" || s.kind === "waiting") return s.repeat > 0;
      if (s.kind === "playing") return Number.isFinite(s.end);
      return false;
    });
  }

  /** クリックで sync 印のものを畳む。畳むものがあったかを返す。 */
  skipSync(now: number): boolean {
    let skipped = this.params.skipSync(now);
    const batches = this.syncBatches;
    this.syncBatches = [];
    for (const batch of batches) {
      if (!this.batchAlive(batch)) continue;
      skipped = true;
      for (const id of batch.objects) {
        const o = this.objects.get(id);
        if (o === undefined) continue;
        const s = o.state;
        if (s.kind === "scheduled" && s.repeat === 0) {
          o.state = { kind: "scheduled", at: now, offset: s.offset, repeat: 0 };
        } else if (s.kind === "scheduled" || (s.kind === "waiting" && s.repeat > 0)) {
          this.release(id);
        } else if (s.kind === "playing" && Number.isFinite(s.end)) {
          this.release(id);
        }
      }
      for (const [, stop] of batch.stops) this.runStop(stop);
    }
    if (skipped) this.update(now);
    return skipped;
  }

  // --- 時計 -----------------------------------------------------------------

  update(now: number): void {
    const dt = Math.max(now - this.lastNow, 0);
    this.lastNow = now;

    for (const [id, b] of [...this.buses]) {
      if (b.deleteAt !== null && now >= b.deleteAt && this.buses.has(id)) this.deleteSubtree(id);
    }

    const due = this.stops.filter(([at]) => now >= at);
    this.stops = this.stops.filter(([at]) => now < at);
    for (const [, stop] of due) this.runStop(stop);
    const batchDue: Stop[] = [];
    for (const batch of this.syncBatches) {
      batchDue.push(...batch.stops.filter(([at]) => now >= at).map(([, s]) => s));
      batch.stops = batch.stops.filter(([at]) => now < at);
    }
    for (const stop of batchDue) this.runStop(stop);

    for (const [id, o] of [...this.objects]) {
      const s = o.state;
      if (s.kind === "scheduled" && now >= s.at) {
        if (this.lengths.has(o.sound)) {
          this.begin(id, s.at, s.offset, s.repeat);
        } else {
          o.state = { kind: "waiting", offset: s.offset, repeat: s.repeat };
        }
      }
      const after = this.objects.get(id)?.state;
      if (after?.kind === "playing" && now >= after.end) this.release(id);
    }

    this.params.update(now);
    this.updateDucks(dt);
    this.syncOut();
    this.syncBatches = this.syncBatches.filter((b) => this.batchAlive(b));
  }

  isAnimating(now: number): boolean {
    if (this.params.isAnimating(now) || this.stops.length > 0 || this.syncBatches.length > 0) return true;
    for (const b of this.buses.values()) if (b.deleteAt !== null) return true;
    for (const o of this.objects.values()) {
      const s = o.state;
      if (s.kind === "scheduled" || s.kind === "waiting") return true;
      if (s.kind === "playing" && Number.isFinite(s.end)) return true;
    }
    for (const [id, e] of this.effects) {
      if (e.kind === EffectKind.Duck && Math.abs(e.duckGain - this.duckTarget(id)) > 1e-6) return true;
    }
    return false;
  }

  takeCommands(): AudioCommand[] {
    const out = this.commands;
    this.commands = [];
    return out;
  }

  // --- 内部 -----------------------------------------------------------------

  private bus(id: number): Bus {
    const b = this.buses.get(id);
    if (b === undefined) throw new Error(`no such sound bus: ${id}`);
    return b;
  }

  private checkKind(kind: number): void {
    if (!isKnownKind(kind)) throw new Error(`unknown transition kind: ${kind}`);
  }

  private ancestors(bus: number): number[] {
    const out = [bus];
    let cur = this.buses.get(bus);
    while (cur !== undefined && cur.parent.kind === "bus") {
      out.push(cur.parent.id);
      cur = this.buses.get(cur.parent.id);
    }
    return out;
  }

  private inSubtree(bus: number, root: number): boolean {
    return this.ancestors(bus).includes(root);
  }

  private value(owner: string, param: number): number {
    return this.params.get(owner, param) ?? 0;
  }

  private backendParams(id: number): number[] {
    const e = this.effects.get(id)!;
    if (e.kind === EffectKind.PassFilter) {
      return [0, 1].map((p) => 2 ** this.value(fx(id), p));
    }
    if (e.kind === EffectKind.Compressor) {
      return [0, 1, 2, 3, 4].map((p) => this.value(fx(id), p));
    }
    return [e.duckGain];
  }

  private effectSpecs(bus: number): EffectSpec[] {
    return this.bus(bus).effects.map((id) => {
      const e = this.effects.get(id)!;
      return { id, kind: e.kind, steep: e.steep, params: this.backendParams(id) };
    });
  }

  private materialize(bus: number): void {
    const stack = [bus];
    while (stack.length > 0) {
      const id = stack.pop()!;
      const b = this.bus(id);
      if (b.materialized || b.parent.kind === "none") continue;
      const effects = this.effectSpecs(id);
      const volumeDb = this.value(busKey(id), 0);
      if (b.parent.kind === "output") {
        this.commands.push({ kind: "openOutput", bus: id, effects, volumeDb });
      } else {
        this.commands.push({ kind: "createBus", bus: id, parent: b.parent.id, effects, volumeDb });
      }
      this.sentBus.set(id, volumeDb);
      for (const e of b.effects) {
        this.backendParams(e).forEach((v, i) => this.sentEffect.set(`${e}#${i}`, v));
      }
      b.materialized = true;
      stack.push(...[...b.children].reverse());
    }
  }

  private begin(id: number, at: number, offset: number, repeat: number): void {
    const o = this.objects.get(id)!;
    const length = this.lengths.get(o.sound) ?? 0;
    const loopEnd = o.loopEnd > 0 ? Math.min(o.loopEnd, length) : length;
    const loopStart = Math.min(o.loopStart, loopEnd);
    // 読めなかった音 (長さ 0) は無限でもすぐ終わる。
    const end =
      repeat === 0
        ? loopEnd - loopStart > 0
          ? Infinity
          : at
        : at + Math.max(loopEnd - offset, 0) + (repeat - 1) * (loopEnd - loopStart);
    const materialized = this.buses.get(o.bus)?.materialized ?? false;
    const volumeDb = this.value(obj(id), ObjectParam.Volume);
    const x = this.value(obj(id), ObjectParam.X);
    o.state = { kind: "playing", end };
    if (materialized && end > at && length > 0) {
      o.sounding = true;
      o.sent = [volumeDb, x];
      this.commands.push({
        kind: "play",
        object: id,
        sound: o.sound,
        bus: o.bus,
        offsetMs: offset,
        loopStartMs: loopStart,
        loopEndMs: o.loopEnd > 0 ? loopEnd : null,
        repeat,
        volumeDb,
        x,
      });
    }
  }

  private release(id: number): void {
    const o = this.objects.get(id);
    if (o !== undefined) {
      this.objects.delete(id);
      if (o.sounding) this.commands.push({ kind: "stop", object: id });
    }
    this.params.remove(obj(id));
  }

  private runStop(stop: Stop): void {
    if (stop.kind === "object") {
      const o = this.objects.get(stop.id);
      if (o !== undefined && o.state.kind !== "idle") this.release(stop.id);
      return;
    }
    for (const [id, o] of [...this.objects]) {
      if (o.state.kind !== "idle" && o.fired < stop.before && this.inSubtree(o.bus, stop.bus)) {
        this.release(id);
      }
    }
  }

  private deleteSubtree(bus: number): void {
    const doomed: number[] = [];
    const stack = [bus];
    while (stack.length > 0) {
      const id = stack.pop()!;
      const b = this.buses.get(id);
      if (b === undefined) continue;
      stack.push(...b.children);
      doomed.push(id);
    }
    for (const [id, o] of [...this.objects]) {
      if (doomed.includes(o.bus)) {
        // 鳴らす側ではバスごと消えるので、止める指示は要らない。
        this.objects.delete(id);
        this.params.remove(obj(id));
      }
    }
    const b = this.bus(bus);
    if (b.materialized) this.commands.push({ kind: "deleteBus", bus });
    if (b.parent.kind === "bus") {
      const p = this.buses.get(b.parent.id);
      if (p !== undefined) p.children = p.children.filter((c) => c !== bus);
    }
    if (this.root === bus) this.root = null;
    for (const id of doomed) {
      const d = this.buses.get(id);
      if (d === undefined) continue;
      for (const e of d.effects) {
        this.effects.delete(e);
        this.params.remove(fx(e));
        for (const key of [...this.sentEffect.keys()]) {
          if (key.startsWith(`${e}#`)) this.sentEffect.delete(key);
        }
      }
      this.buses.delete(id);
      this.params.remove(busKey(id));
      this.sentBus.delete(id);
    }
  }

  private duckTarget(effect: number): number {
    const e = this.effects.get(effect)!;
    if (!this.buses.has(e.key)) return 0;
    for (const o of this.objects.values()) {
      if (o.state.kind === "playing" && this.inSubtree(o.bus, e.key)) {
        return -Math.abs(this.value(fx(effect), DuckParam.Amount));
      }
    }
    return 0;
  }

  private updateDucks(dt: number): void {
    for (const [id, e] of this.effects) {
      if (e.kind !== EffectKind.Duck) continue;
      const target = this.duckTarget(id);
      const amount = Math.abs(this.value(fx(id), DuckParam.Amount));
      const time = target < e.duckGain ? this.value(fx(id), DuckParam.Attack) : this.value(fx(id), DuckParam.Release);
      // attack / release は「下げ幅の全部を動く時間」。
      const step = time <= 0 || amount <= 0 ? Infinity : (amount / time) * dt;
      e.duckGain =
        Math.abs(target - e.duckGain) <= step ? target : e.duckGain + step * Math.sign(target - e.duckGain);
    }
  }

  private syncOut(): void {
    const EPS = 1e-4;
    const changed = (old: number | undefined, v: number): boolean => old === undefined || Math.abs(old - v) > EPS;
    for (const [id, b] of this.buses) {
      if (!b.materialized) continue;
      const v = this.value(busKey(id), 0);
      if (changed(this.sentBus.get(id), v)) {
        this.sentBus.set(id, v);
        this.commands.push({ kind: "setBus", bus: id, volumeDb: v });
      }
      for (const e of b.effects) {
        this.backendParams(e).forEach((value, param) => {
          const key = `${e}#${param}`;
          if (changed(this.sentEffect.get(key), value)) {
            this.sentEffect.set(key, value);
            this.commands.push({ kind: "setEffect", effect: e, param, value });
          }
        });
      }
    }
    for (const [id, o] of this.objects) {
      if (!o.sounding) continue;
      const v = this.value(obj(id), ObjectParam.Volume);
      const x = this.value(obj(id), ObjectParam.X);
      if (Math.abs(o.sent[0] - v) > EPS || Math.abs(o.sent[1] - x) > EPS || Number.isNaN(o.sent[0])) {
        o.sent = [v, x];
        this.commands.push({ kind: "setObject", object: id, volumeDb: v, x });
      }
    }
  }
}

/** dB を振幅の倍率にする。-60 dB 以下は 0。 */
export function dbToGain(db: number): number {
  return db <= SILENCE_DB ? 0 : 10 ** (db / 20);
}

/**
 * 位置 x (-1..1) からステレオの左右の倍率を決める (パン則)。ネイティブの `pan_gains` と同じ式。
 *
 * ステレオの音のパンは**バランス**の意味で、反対側を絞る。中央では両方 1、片側へ寄せると反対側が cos の曲線で 0 まで下がる。
 */
export function panGains(x: number): [number, number] {
  const c = clamp(x, -1, 1);
  const left = c > 0 ? Math.cos((c * Math.PI) / 2) : 1;
  const right = c < 0 ? Math.cos((-c * Math.PI) / 2) : 1;
  return [left, right];
}

function clamp(v: number, min: number, max: number): number {
  return Math.min(Math.max(v, min), max);
}
