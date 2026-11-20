import { resolveAssetUrl } from "../api/assets";
import { type AudioCommand, type EffectSpec, EffectKind, PassFilterParam, dbToGain, panGains } from "./Mixer";

/**
 * ミキサーの論理状態 (`Mixer`) の写しを、Web Audio で鳴らす。
 *
 * ネイティブの `crates/audio-kira` と同じ形にする (`docs/sound.md`):
 *
 * - バス = 入口の GainNode → effect の列 → フェーダの GainNode → 親の入口 (根は `destination`)。
 * - SoundObject = `AudioBufferSourceNode` → 左右に分けて倍率を掛ける (パン則は `panGains`。`StereoPannerNode` は
 *   ステレオの音で意味が違うので使わない) → バスの入口。ループの区間と回数は、区間を `loop` にして終わる時刻で止める。
 * - pass filter = `BiquadFilterNode` の highpass → lowpass (Q 0.707)。24 dB/oct は 2 段。
 *   無効 (20 Hz 以下 / 20 kHz 以上) のときは、可聴域の外の端に置いて実質素通しにする。
 * - compressor = `DynamicsCompressorNode` + makeup の GainNode。duck = GainNode。
 *
 * ブラウザは利用者の操作があるまで音を出させない。`AudioContext` が running になるまでは鳴らすのを保留し、
 * 解除されたときに、まだ論理的に鳴っている音だけを鳴らす (無限の音は頭から、有限の音は今の位置から)。
 */

/** パラメータの変化をならす時定数 [s]。フレームごとの段差が聞こえないように。 */
const SMOOTHING = 0.01;

interface BusNodes {
  input: GainNode;
  fader: GainNode;
  parent: number | null;
}

type EffectNodes =
  | { kind: "filter"; highs: BiquadFilterNode[]; lows: BiquadFilterNode[] }
  | { kind: "compressor"; node: DynamicsCompressorNode; makeup: GainNode }
  | { kind: "gain"; node: GainNode };

interface Voice {
  bus: number;
  source: AudioBufferSourceNode | null;
  left: GainNode;
  right: GainNode;
  merger: ChannelMergerNode;
  /** 保留中 (出力が許される前) の再生の指示と、指示を受けた時刻 [ms]。 */
  play: Extract<AudioCommand, { kind: "play" }>;
  requestedAt: number;
}

export class WebAudioOutput {
  private context: AudioContext | null = null;
  private readonly buses = new Map<number, BusNodes>();
  private readonly effects = new Map<number, EffectNodes>();
  private readonly voices = new Map<number, Voice>();
  private readonly buffers = new Map<number, AudioBuffer>();

  /** 最初の利用者の操作で音を許す。何度呼んでもよい。 */
  resume(): void {
    const ctx = this.context;
    if (ctx !== null && ctx.state === "suspended") void ctx.resume();
  }

  /**
   * 音のファイルを読んでデコードする。長さ [ms] を返し、読めなければ `null`。
   *
   * デコードには AudioContext が要るので、ここで作る (作っただけでは音は出ない。止まったまま作られる)。
   */
  async load(sound: number, path: string): Promise<number | null> {
    let url: string;
    try {
      url = resolveAssetUrl(path);
    } catch (e: unknown) {
      console.error(`[biwa] bad asset path "${path}":`, e);
      return null;
    }
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const data = await response.arrayBuffer();
      const buffer = await this.ensureContext().decodeAudioData(data);
      if (buffer.numberOfChannels > 2) {
        throw new Error(`${buffer.numberOfChannels}-channel audio is not supported yet (use mono or stereo files)`);
      }
      this.buffers.set(sound, buffer);
      return buffer.duration * 1000;
    } catch (e: unknown) {
      console.error(
        `[biwa] failed to load "${path}" (looked for ${url}; paths are relative to the package's \`assets/\`). ` +
          "Supported formats are MP3, WAV (PCM) and Ogg Vorbis (Safari plays Ogg from 18.4):",
        e,
      );
      return null;
    }
  }

  apply(commands: AudioCommand[]): void {
    for (const command of commands) {
      try {
        this.applyOne(command);
      } catch (e: unknown) {
        console.error("[biwa] sound output:", e);
      }
    }
  }

  private ensureContext(): AudioContext {
    if (this.context === null) {
      this.context = new AudioContext();
      this.context.addEventListener("statechange", () => this.onStateChange());
    }
    return this.context;
  }

  private applyOne(command: AudioCommand): void {
    const ctx = this.ensureContext();
    switch (command.kind) {
      case "openOutput":
        this.buildBus(command.bus, null, command.effects, command.volumeDb).fader.connect(ctx.destination);
        return;
      case "createBus": {
        const parent = this.buses.get(command.parent);
        if (parent === undefined) return;
        this.buildBus(command.bus, command.parent, command.effects, command.volumeDb).fader.connect(parent.input);
        return;
      }
      case "deleteBus":
        this.deleteBus(command.bus);
        return;
      case "play":
        this.play(command);
        return;
      case "stop":
        this.stop(command.object);
        return;
      case "setObject": {
        const voice = this.voices.get(command.object);
        if (voice !== undefined) this.setVoiceGain(voice, command.volumeDb, command.x);
        return;
      }
      case "setBus": {
        const bus = this.buses.get(command.bus);
        if (bus !== undefined) smooth(bus.fader.gain, dbToGain(command.volumeDb), ctx);
        return;
      }
      case "setEffect":
        this.setEffect(command.effect, command.param, command.value);
        return;
    }
  }

  private buildBus(id: number, parent: number | null, effects: EffectSpec[], volumeDb: number): BusNodes {
    const ctx = this.ensureContext();
    const input = ctx.createGain();
    const fader = ctx.createGain();
    fader.gain.value = dbToGain(volumeDb);
    let tail: AudioNode = input;
    for (const spec of effects) {
      const nodes = this.buildEffect(spec);
      this.effects.set(spec.id, nodes);
      for (const node of chain(nodes)) {
        tail.connect(node);
        tail = node;
      }
    }
    tail.connect(fader);
    const bus = { input, fader, parent };
    this.buses.set(id, bus);
    return bus;
  }

  private buildEffect(spec: EffectSpec): EffectNodes {
    const ctx = this.ensureContext();
    if (spec.kind === EffectKind.PassFilter) {
      const stages = spec.steep ? 2 : 1;
      const make = (type: BiquadFilterType): BiquadFilterNode[] =>
        Array.from({ length: stages }, () => {
          const node = ctx.createBiquadFilter();
          node.type = type;
          node.Q.value = Math.SQRT1_2;
          return node;
        });
      const nodes: EffectNodes = { kind: "filter", highs: make("highpass"), lows: make("lowpass") };
      setFilter(nodes, PassFilterParam.LowCut, spec.params[0], ctx, true);
      setFilter(nodes, PassFilterParam.HighCut, spec.params[1], ctx, true);
      return nodes;
    }
    if (spec.kind === EffectKind.Compressor) {
      const node = ctx.createDynamicsCompressor();
      node.knee.value = 0;
      const makeup = ctx.createGain();
      const nodes: EffectNodes = { kind: "compressor", node, makeup };
      spec.params.forEach((v, i) => setCompressor(nodes, i, v, ctx, true));
      return nodes;
    }
    const node = ctx.createGain();
    node.gain.value = dbToGain(spec.params[0] ?? 0);
    return { kind: "gain", node };
  }

  private setEffect(id: number, param: number, value: number): void {
    const ctx = this.ensureContext();
    const nodes = this.effects.get(id);
    if (nodes === undefined) return;
    if (nodes.kind === "filter") setFilter(nodes, param, value, ctx, false);
    else if (nodes.kind === "compressor") setCompressor(nodes, param, value, ctx, false);
    else smooth(nodes.node.gain, dbToGain(value), ctx);
  }

  private deleteBus(id: number): void {
    const doomed = [id];
    for (let i = 0; i < doomed.length; i++) {
      for (const [child, bus] of this.buses) {
        if (bus.parent === doomed[i]) doomed.push(child);
      }
    }
    for (const [object, voice] of [...this.voices]) {
      if (doomed.includes(voice.bus)) this.stop(object);
    }
    for (const bus of doomed) {
      const nodes = this.buses.get(bus);
      nodes?.fader.disconnect();
      nodes?.input.disconnect();
      this.buses.delete(bus);
    }
  }

  private play(command: Extract<AudioCommand, { kind: "play" }>): void {
    const ctx = this.ensureContext();
    const bus = this.buses.get(command.bus);
    if (bus === undefined || !this.buffers.has(command.sound)) return;
    const left = ctx.createGain();
    const right = ctx.createGain();
    const merger = ctx.createChannelMerger(2);
    left.connect(merger, 0, 0);
    right.connect(merger, 0, 1);
    merger.connect(bus.input);
    const voice: Voice = { bus: command.bus, source: null, left, right, merger, play: command, requestedAt: performance.now() };
    this.voices.set(command.object, voice);
    this.setVoiceGain(voice, command.volumeDb, command.x, true);
    if (ctx.state === "running") this.startSource(voice, command.offsetMs);
  }

  /** 音源を作って鳴らす。`offsetMs` から、区間と回数のとおりに。 */
  private startSource(voice: Voice, offsetMs: number): void {
    const ctx = this.ensureContext();
    const p = voice.play;
    const buffer = this.buffers.get(p.sound);
    if (buffer === undefined) return;
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    const splitter = ctx.createChannelSplitter(2);
    source.connect(splitter);
    splitter.connect(voice.left, 0);
    // モノラルは左右に同じものを流す。
    splitter.connect(voice.right, buffer.numberOfChannels > 1 ? 1 : 0);

    const length = buffer.duration;
    const loopEnd = p.loopEndMs === null ? length : Math.min(p.loopEndMs / 1000, length);
    const loopStart = Math.min(p.loopStartMs / 1000, loopEnd);
    const offset = Math.min(offsetMs / 1000, loopEnd);
    if (p.repeat !== 1) {
      source.loop = true;
      source.loopStart = loopStart;
      source.loopEnd = loopEnd;
    }
    const now = ctx.currentTime;
    source.start(now, offset);
    if (p.repeat >= 1) {
      // 区間の終わりまでを 1 回目、その後は区間を (repeat - 1) 回。終わる時刻ちょうどで止める。
      const total = Math.max(loopEnd - offset, 0) + (p.repeat - 1) * (loopEnd - loopStart);
      source.stop(now + total);
    }
    voice.source = source;
  }

  private stop(object: number): void {
    const voice = this.voices.get(object);
    if (voice === undefined) return;
    this.voices.delete(object);
    const ctx = this.ensureContext();
    // ぷつっと切れないよう、短く絞ってから止める。
    const end = ctx.currentTime + SMOOTHING * 3;
    for (const g of [voice.left, voice.right]) {
      g.gain.setTargetAtTime(0, ctx.currentTime, SMOOTHING / 2);
    }
    try {
      voice.source?.stop(end);
    } catch {
      // 既に止まっている。
    }
    setTimeout(() => voice.merger.disconnect(), 100);
  }

  private setVoiceGain(voice: Voice, volumeDb: number, x: number, immediate = false): void {
    const ctx = this.ensureContext();
    const gain = dbToGain(volumeDb);
    const [l, r] = panGains(x);
    if (immediate) {
      voice.left.gain.value = gain * l;
      voice.right.gain.value = gain * r;
    } else {
      smooth(voice.left.gain, gain * l, ctx);
      smooth(voice.right.gain, gain * r, ctx);
    }
  }

  /** 音が許された (または止められた)。保留していた音のうち、まだ鳴っているはずのものを鳴らす。 */
  private onStateChange(): void {
    const ctx = this.context;
    if (ctx === null || ctx.state !== "running") return;
    for (const voice of this.voices.values()) {
      if (voice.source !== null) continue;
      // 無限の音は頭から、有限の音は指示を受けてから経った分だけ進めた位置から。
      const elapsed = voice.play.repeat === 0 ? 0 : performance.now() - voice.requestedAt;
      this.startSource(voice, voice.play.offsetMs + elapsed);
    }
  }
}

function chain(nodes: EffectNodes): AudioNode[] {
  if (nodes.kind === "filter") return [...nodes.highs, ...nodes.lows];
  if (nodes.kind === "compressor") return [nodes.node, nodes.makeup];
  return [nodes.node];
}

function smooth(param: AudioParam, value: number, ctx: AudioContext): void {
  param.setTargetAtTime(value, ctx.currentTime, SMOOTHING);
}

function setParam(param: AudioParam, value: number, ctx: AudioContext, immediate: boolean): void {
  if (immediate) param.value = value;
  else smooth(param, value, ctx);
}

function setFilter(
  nodes: Extract<EffectNodes, { kind: "filter" }>,
  param: number,
  hz: number,
  ctx: AudioContext,
  immediate: boolean,
): void {
  const nyquist = ctx.sampleRate / 2;
  if (param === PassFilterParam.LowCut) {
    // 無効なら可聴域より下に置く (実質素通し)。
    const f = hz <= PassFilterParam.MinHz ? 1 : hz;
    for (const n of nodes.highs) setParam(n.frequency, f, ctx, immediate);
  } else if (param === PassFilterParam.HighCut) {
    // 無効ならナイキスト周波数に置く (実質素通し)。
    const f = hz >= PassFilterParam.MaxHz ? nyquist : Math.min(hz, nyquist);
    for (const n of nodes.lows) setParam(n.frequency, f, ctx, immediate);
  }
}

function setCompressor(
  nodes: Extract<EffectNodes, { kind: "compressor" }>,
  param: number,
  value: number,
  ctx: AudioContext,
  immediate: boolean,
): void {
  const c = nodes.node;
  switch (param) {
    case 0:
      setParam(c.threshold, Math.min(Math.max(value, -100), 0), ctx, immediate);
      return;
    case 1:
      setParam(c.ratio, Math.min(Math.max(value, 1), 20), ctx, immediate);
      return;
    case 2:
      setParam(c.attack, Math.min(Math.max(value / 1000, 0), 1), ctx, immediate);
      return;
    case 3:
      setParam(c.release, Math.min(Math.max(value / 1000, 0), 1), ctx, immediate);
      return;
    case 4:
      setParam(nodes.makeup.gain, dbToGain(value), ctx, immediate);
      return;
  }
}
