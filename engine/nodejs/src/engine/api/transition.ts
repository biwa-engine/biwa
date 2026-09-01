/**
 * 遷移 (transition) の番号と曲線の定義。
 *
 * ここは std との合意点である。TypeScript ターゲットの std は
 * `@biwa/engine/api/transition` としてこれを取り込むが、
 * **wasm ターゲットの std は `.wat` に整数を直書きする**ので、
 * 番号はこのファイルを写したものになる。
 * 写し間違いが黙って通らないよう、範囲外の値はエンジンが名指しで叱る
 * (`vm/wasm/worker.ts` が未知の import 名を叱るのと同じ方針)。
 *
 * 設計は `docs/media-object-model.md` にある。
 */

/** 遷移できるパラメータ。値は canvas オブジェクトの状態の添字でもある。 */
export const Param = {
  /** 画像の中心の x 座標 [px]。canvas の中央が原点で、右が正。 */
  X: 0,
  /** 画像の中心の y 座標 [px]。canvas の中央が原点で、**上が正**。 */
  Y: 1,
  /** 幅 [px]。 */
  W: 2,
  /** 高さ [px]。 */
  H: 3,
  /** 不透明度。0 (透明) 〜 255 (不透明)。 */
  Alpha: 4,
  /** 回転 [度]。z 軸まわりで、回転の中心は画像の中心。 */
  Theta: 5,
} as const;

export const PARAM_COUNT = 6;

/**
 * 遷移のかかり方。
 *
 * 0〜15 が一度限りのもの、16 以降が周期的なものである。
 * この区分は読む人のためのもので、API 上の区別ではない
 * (振る舞いは kind ごとに決まる)。
 *
 * NOTE: `EaseIn` は「ゆっくり始まって速く終わる」である。
 * `docs/media-syscall-wasm.md` の記述とは逆になっているが、
 * CSS をはじめ既存のツールがすべてこの意味で使っているため、そちらに合わせた。
 */
export const Curve = {
  /** 遷移しない。その時点の値で固定して終わる (周期系を止める手段でもある)。 */
  None: 0,
  /** 線形。 */
  Linear: 1,
  /** ゆっくり始まり、速く終わる。 */
  EaseIn: 2,
  /** 速く始まり、ゆっくり終わる。 */
  EaseOut: 3,
  /** ゆっくり始まり、中間で速く、ゆっくり終わる。 */
  EaseInOut: 4,
  /** 速く始まり、中間でゆっくり、速く終わる。 */
  EaseOutIn: 5,

  /** 正弦波。 */
  Sin: 16,
  /** 三角波。 */
  Triangle: 17,
  /** 矩形波。 */
  Square: 18,
  /** のこぎり波。0 から振幅まで線形に増えて戻る。 */
  Saw: 19,
} as const;

/** 周期系の kind の下限。これ以上は終端を持たない。 */
const PERIODIC_BASE = 16;

/** 終端を持たない (= 完了しない) 遷移か。 */
export function isPeriodic(kind: number): boolean {
  return kind >= PERIODIC_BASE;
}

export function isKnownParam(param: number): boolean {
  return Number.isInteger(param) && param >= 0 && param < PARAM_COUNT;
}

export function isKnownKind(kind: number): boolean {
  return KNOWN_KINDS.has(kind);
}

const KNOWN_KINDS = new Set<number>(Object.values(Curve));

/**
 * 一度限りの遷移の進み具合。
 *
 * `p` は経過時間の比 (0〜1) で、返すのは値の比 (0〜1) である。
 */
export function curveAt(kind: number, p: number): number {
  switch (kind) {
    case Curve.Linear:
      return p;
    case Curve.EaseIn:
      return p * p * p;
    case Curve.EaseOut:
      return 1 - (1 - p) ** 3;
    case Curve.EaseInOut:
      return p < 0.5 ? 4 * p * p * p : 1 - (-2 * p + 2) ** 3 / 2;
    case Curve.EaseOutIn:
      return p < 0.5 ? (1 - (1 - 2 * p) ** 3) / 2 : ((2 * p - 1) ** 3 + 1) / 2;
    default:
      // `None` はここに来ない (区間の評価側で弾く)。
      return p;
  }
}

/**
 * 周期系の波形。
 *
 * `phase` は周期内の位置 (0 以上 1 未満) で、返した値に振幅が掛かる。
 * どの波形も `phase = 0` で 0 を返すので、基準値から始まる。
 */
export function waveAt(kind: number, phase: number): number {
  switch (kind) {
    case Curve.Sin:
      return Math.sin(2 * Math.PI * phase);
    case Curve.Triangle:
      if (phase < 0.25) return 4 * phase;
      if (phase < 0.75) return 2 - 4 * phase;
      return 4 * phase - 4;
    case Curve.Square:
      // 0 から始めるため、前半を正、後半を負にする。
      return phase < 0.5 ? 1 : -1;
    case Curve.Saw:
      return phase;
    default:
      return 0;
  }
}
