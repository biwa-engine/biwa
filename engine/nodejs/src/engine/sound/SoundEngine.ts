import { resolveAssetUrl } from "../api/assets";
import { Mixer } from "./Mixer";
import { WebAudioOutput } from "./WebAudioOutput";

/**
 * 音の全体。ミキサーの論理状態 (`Mixer`) と、それを鳴らす Web Audio (`WebAudioOutput`) と、
 * 音のファイルの番号付けを束ねる (`docs/sound.md`)。
 *
 * Ticker に自分のコールバックは登録しない。`main.ts` の 1 本から、canvas と同じ時計で `update` される。
 */
export class SoundEngine {
  readonly mixer = new Mixer();
  private readonly output = new WebAudioOutput();
  /** URL → 音のファイルの番号。同じファイルは 1 度だけ読む。 */
  private readonly sounds = new Map<string, number>();
  private now = 0;

  /** パスに音のファイルの番号を振る。初めてのパスなら読み込みを始める。 */
  register(path: string): number {
    const url = resolveAssetUrl(path);
    const known = this.sounds.get(url);
    if (known !== undefined) return known;
    const sound = this.sounds.size + 1;
    this.sounds.set(url, sound);
    void this.output.load(sound, path).then((length) => {
      this.mixer.soundLoaded(sound, length, this.now);
      this.flush();
    });
    return sound;
  }

  /** エンジン時計を `now` まで進める。 */
  update(now: number): void {
    this.now = now;
    this.mixer.update(now);
    this.flush();
  }

  /** エンジン時計の今。 */
  get clock(): number {
    return this.now;
  }

  /** 論理状態の変化を Web Audio に当てる。 */
  flush(): void {
    const commands = this.mixer.takeCommands();
    if (commands.length > 0) this.output.apply(commands);
  }

  /** 利用者の操作があった。ブラウザが音を許す。 */
  resume(): void {
    this.output.resume();
  }
}
