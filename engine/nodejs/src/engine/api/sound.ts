import { engine } from "./context";

/**
 * 音の syscall の実装 (`docs/sound.md`)。
 *
 * `vm/wasm/host.ts` がこれを呼ぶ。id は Worker が SoundObject・Bus・Effect の空間ごとに採番して渡す
 * (`contract.ts` の `alloc`)。どれも積んで返る syscall なので、誤りはゲームを止めずにログに出す。
 */

function guarded(name: string, f: () => void): void {
  try {
    f();
  } catch (e: unknown) {
    console.error(`[biwa] ${name}: ${e instanceof Error ? e.message : String(e)}`);
  }
  engine().sound.flush();
}

const mixer = () => engine().sound.mixer;

export function soundBusCreate(bus: number): void {
  guarded("sys_sound_bus_create", () => mixer().busCreate(bus));
}

export function soundEffectCreate(effect: number, kind: number, cfg1: number, cfg2: number): void {
  guarded("sys_sound_effect_create", () => mixer().effectCreate(effect, kind, cfg1, cfg2));
}

export function soundBusAddEffect(bus: number, effect: number): void {
  guarded("sys_sound_bus_add_effect", () => mixer().busAddEffect(bus, effect));
}

export function soundBusAttach(bus: number, parent: number): void {
  guarded("sys_sound_bus_attach", () => mixer().busAttach(bus, parent));
}

export function soundOutputAttach(bus: number): void {
  guarded("sys_sound_output_attach", () => mixer().outputAttach(bus));
}

export function soundBusAddStopAll(bus: number, after: number): void {
  guarded("sys_sound_bus_add_stop_all", () => mixer().busAddStopAll(bus, after));
}

export function soundBusDelete(bus: number, after: number): void {
  guarded("sys_sound_bus_delete", () => mixer().busDelete(bus, after, engine().sound.clock));
}

export function soundBusAddTransition(
  bus: number,
  param: number,
  kind: number,
  value: number,
  after: number,
  duration: number,
): void {
  guarded("sys_sound_bus_add_transition", () => mixer().busAddTransition(bus, param, kind, value, after, duration));
}

export function soundBusSetParam(bus: number, param: number, value: number): void {
  guarded("sys_sound_bus_set_param", () => mixer().busSetParam(bus, param, value));
}

export function soundEffectAddTransition(
  effect: number,
  param: number,
  kind: number,
  value: number,
  after: number,
  duration: number,
): void {
  guarded("sys_sound_effect_add_transition", () =>
    mixer().effectAddTransition(effect, param, kind, value, after, duration),
  );
}

export function soundEffectSetParam(effect: number, param: number, value: number): void {
  guarded("sys_sound_effect_set_param", () => mixer().effectSetParam(effect, param, value));
}

/** SoundObject を作る。`normalize` (ラウドネス正規化) は段 2 で入れる。それまでは受け取って捨てる。 */
export function soundCreate(
  id: number,
  path: string,
  bus: number,
  volumeDb: number,
  x: number,
  y: number,
  loopStartMs: number,
  loopEndMs: number,
  _normalize: number,
): void {
  guarded("sys_sound_object_create", () => {
    const sound = engine().sound.register(path);
    mixer().create(id, sound, bus, volumeDb, x, y, loopStartMs, loopEndMs);
  });
}

export function soundAddStart(id: number, after: number, offsetMs: number, repeat: number): void {
  guarded("sys_sound_object_add_start", () => mixer().addStart(id, after, offsetMs, repeat));
}

export function soundAddStop(id: number, after: number): void {
  guarded("sys_sound_object_add_stop", () => mixer().addStop(id, after));
}

export function soundAddTransition(
  id: number,
  param: number,
  kind: number,
  value: number,
  after: number,
  duration: number,
): void {
  guarded("sys_sound_object_add_transition", () => mixer().addTransition(id, param, kind, value, after, duration));
}

export function soundPreload(path: string): void {
  guarded("sys_sound_preload", () => {
    engine().sound.register(path);
  });
}
