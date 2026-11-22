# Sound Subsystem

BGM・SE・環境音・ボイスを鳴らす仕組み (issue #26)。概念・単位・syscall・std の API・エンジンの振る舞いを決める。
議論の経緯は `~/memo/biwa-sound-subsystem.md` (リポジトリの外) にある。ここには決まったことだけを書く。

canvas オブジェクト・遷移・Content API の既存の約束 (`media-object-model.md`、`content-api.md`) に従う。

## 要約

- `Sound` は音のファイル 1 つ、`SoundObject` は再生 1 回。
- 音は**バス**を通る。バスは木で、それぞれ音量と **effect** の列を持つ。
- 音量はすべて **dB**。経路に沿って**足し算**になる。-60 dB 以下は無音。
- 出力は当面**ステレオ**。音の位置は **position** (x と y、どちらも -1.0〜1.0)。
- runtime は最初に**何も用意しない**。std が `master` を作り、リミッタを挿して**出力**に繋ぐ。
- 開始・停止・変化は canvas の遷移と同じく、`add_*` で積んで `sys_start_transitions(sync)` で発火する。
- 形式 (Tier 1): **MP3・WAV (PCM)・Ogg Vorbis**。

## 用語

| Biwa での名前 | ほかでの呼び名 | 意味 |
|---|---|---|
| `Sound` | asset, clip | 音のファイル 1 つ。再生位置を持たない |
| `SoundObject` | voice, instance | 再生 1 回。音量・位置・再生状態を持つ |
| `Bus` | bus (Wwise, FMOD)、mixer group (Unity)、track (kira, DAW) | 音を束ねて音量と effect を掛ける経路。入れ子にできる |
| `Effect` | insert effect, DSP | バスに直列に挿す処理 (フィルタ、圧縮、ダッキング …) |
| 出力 | output device | 音の出口。バスではない |
| チャンネル | channel | 信号の本数 (L/R、5.1 の 6 本)。経路の名前には**使わない** |
| `position` | pan | 音の位置。x と y の 2 軸 |

channel (経路ではなく L/R の意味になる)、track (音楽では「曲」の意味が強い)、filter (周波数の処理の専門用語) は経路や総称の名前に使わない。
「別のバスが鳴っている間このバスを下げる」は ducking と呼ぶ。

## 信号の流れ

```
 音のファイル (mono / stereo)
   │ デコードして PCM に、ステレオに揃える
   ▼
 SoundObject      [音量 dB] [position]
   ▼
 Bus voice/akane  [effect 1] → [effect 2] → … → [音量 dB]
   ▼
 Bus voice        [duck …] → [音量 dB]
   ▼
 Bus master       [リミッタ] → [音量 dB]
   ▼
 出力 (ステレオ)
```

バスの中の順序は固定である:
**(子のバスとこのバスの SoundObject の和) → effect 1 → … → effect n → バスの音量 (フェーダ) → 親**

- effect は挿した順に掛かる。非線形なもの (圧縮・リミッタ) は順序で結果が変わる。リミッタは master の最後に置く。
- フェーダを最後に置くので、プレイヤーが音量を変えても圧縮の効き方は変わらない。

## 音量

- `dB = 20 · log10(倍率)`。0.5 倍が約 -6 dB、1 倍が 0 dB。経路の上では足し算になる (-6 + -3 + -2 = -11 dB)。
- **-60 dB 以下は無音**。無音へのフェードは -60 dB で終わる。
- dB の上での線形のフェードは自然に聞こえる (聞こえ方がおおよそ対数的なため)。

| 誰が | 単位 | 例 |
|---|---|---|
| syscall (エンジン) | dB (f32) | `volume_db = -6.0` |
| ゲームの作者 | `Volume::db(-6.0)`、`Volume::percent(50)`、`Volume::silent()` | |
| プレイヤー (設定画面) | 0〜100。std が `40 · log10(p / 100)` dB に直す | 50 → -12 dB |

- 100 → 0 dB、50 → -12 dB、25 → -24 dB、10 → -40 dB、0 → 無音。
- バスの音量は 0 dB 以下を推す。std は master を -3 dB にして余裕を残し、master にリミッタを挿す。
- **ラウドネス正規化** (段 2): `biwa build` が LUFS を測って補正の dB を持ち、`normalize` が付いた音に足す。
  既定は BGM・ボイスがオン、SE がオフ (決めるのは std)。`sys_sound_create` の `normalize` は今から入っていて、エンジンは段 2 まで無視する。

## バス

### 既定の木

```
master          (リミッタ。出力に繋がる)
├─ bgm          (voice が鳴る間だけ下がる)
├─ ambient
├─ se
├─ voice
│   └─ voice/<キャラ> (Character::new が作る)
└─ system       (UI の音)
```

- **エンジンは syscall に従って木を保持するだけで、どのバスが何のためかは知らない**。決めるのは std。
- third party のライブラリも、master や std のバスの下に自由にバスを足してよい。
- キャラごとの音量はサブバス (`voice/<キャラ>`) で行う。設定の変更が鳴っている声にも効き、キャラ単位で止められ、キャラ単位の effect も挿せる。

### 出力

- 出力はバスではない (音量も effect も無い)。master は std が作るただのバスで、`sys_sound_output_attach(bus)` で繋ぐ。
- **繋げる根は 1 本だけ**。2 本目以降は警告して無視する (すべての音が master のリミッタを通る保証)。
- 出力に繋がっていない木に流した音は鳴らないが、時間は進む (`await` も終わる)。

### 組み立てと削除

1. `sys_sound_effect_create(kind, cfg1, cfg2) -> effect` で effect を作る (どのバスにも挿さない。パラメータは既定値)。
2. `sys_sound_bus_create() -> bus`
3. `sys_sound_bus_add_effect(bus, effect)` (0 回以上)
4. `sys_sound_bus_attach(bus, parent)`、根は `sys_sound_output_attach(bus)`

- **effect は繋ぐ前にしか挿せない** (UI の「組み立ててから push_child」と同じ。kira の制約とも合う)。
- **1 つの effect は 1 つのバスにしか挿せない** (2 つ目は警告して無視する)。どのバスにも挿していない effect は何もしない。
- effect を作るのとバスに挿すのを分けたのは、effect の id (パラメータを後から動かすハンドル) を
  バスを作る前に手に入れるためである。std はこの段を `Bus::new(parent, volume, effects)` 1 回の中に隠し、
  途中の状態 (親の無いバス、effect の無いバス) を外に見せない。
- `sys_sound_bus_delete(bus, after)` は `after` の後に、バスと部分木 (子のバス・effect・中で鳴っている音) を消す。
  消した id への操作は無視する。消したバスを key にしていた duck は効かなくなり、release の時間で戻る。

### 寿命

- **Window の寿命**: master と std の既定の木。消さない。std の `WindowHandle` が持ち、`Game` に運ぶ。
- **Game の寿命**: キャラのボイスのバスや、scene の中で作ったバス。`Game` の drop の仕組みが入ったら、そこで消す (今は消えない)。

## Effect

離散の**設定** (cfg) は作るときに渡して変えない。**パラメータ**は f32 で、遷移で動かせる。

| kind | 名前 | 設定 (cfg1, cfg2) | パラメータ | 使いどころ |
|---|---|---|---|---|
| 1 | pass filter | cfg1: 傾き (0 = 12 dB/oct、1 = 24 dB/oct) | 0: low_cut Hz (20 以下は無効)、1: high_cut Hz (20000 以上は無効) | こもった音、電話の声 (300〜3400 Hz)、薄い音 |
| 20 | compressor | なし | 0: threshold dB、1: ratio、2: attack ms、3: release ms、4: makeup dB | 声の粒を揃える、master のリミッタ |
| 30 | duck | cfg1: key のバス、cfg2: mode (0 = activity。1 = level は予約) | 0: 下げ幅 dB、1: attack ms、2: release ms | ボイスが鳴る間 BGM を下げる |

- **pass filter**: 「範囲を通す」を下限 (highpass) と上限 (lowpass) の 2 つで表す。Q は 0.707 (盛り上がり無し) で固定。
  「無効」は 0 ではなく 20 Hz / 20 kHz で表す (Hz は対数で補間するので 0 は -∞ になる)。教科書の bandpass、EQ、notch は別の kind (後で)。
- **duck mode 0**: key のバスの部分木で SoundObject が 1 つでも鳴っていれば、下げ幅だけ下げる。attack / release は
  「下げ幅の全部を動く時間」。短すぎると音量が呼吸する (目安は attack 50〜100 ms、release 300〜800 ms)。
- **補間の空間**: dB は dB のまま線形、**Hz は対数** (1000 → 2000 と 2000 → 4000 が同じ速さに聞こえる)、ms と ratio は線形。

## 位置とチャンネル

- ファイルのチャンネル数と出力のチャンネル数は別物。モノラルは左右に同じものを流す。3 チャンネル以上はまだ扱わない (警告して無音)。
- **バスとチャンネルは直交する**。バスはステレオの信号 1 本を運び、音量や effect は全チャンネルに同じように掛かる。
- 位置は「音がどこにあるか」なので **SoundObject** の属性。
- L/R の音量で表さない: パン則が要る、スクリプトの時点でモノラルかステレオか分からない、ステレオのパンの意味が 2 通りある、5.1 や空間音響へ移せない。
- **x (-1 左 〜 1 右) と y (-1 後ろ 〜 1 前)**。ステレオでは y を捨てる。高さ (z) は後でパラメータを 1 つ足すだけで入る。
- 単位は canvas の -50〜50 とは揃えない (画面の中の場所と、スピーカに対する方向は別の量)。立ち絵に追従させるなら std が換算する。
- **パン則** (Biwa が決め、ネイティブと Web で同じ式): ステレオの音のパンは**バランス** (反対側を絞る)。
  中央で左右とも 1、x > 0 なら左を `cos(x · π/2)`、x < 0 なら右を `cos(-x · π/2)` 倍する。

## ミックスは木 (任意のグラフにしない)

Web Audio のような任意のグラフは組ませない。持つのはバスの木と、バスの中の直列の effect だけである。

- 木で本当に困るのは**センド** (1 つの音を複数の行き先へ、違う量で送る。残響のため) だけで、残響と一緒に後で足す。
  センドを足しても「音量の設定が効くのは自分の祖先だけ」「送り先は祖先でも子孫でもない (閉路を作らない)」は守る。
- 並列処理・帰還・変調・L/R を別に処理するものは effect の種類として足す。ゆっくりした揺れは既存の周期系の遷移 (`Sin` …) で書ける。
- 口パク (音量を測る) と発振器は経路の問題ではない (別の syscall / 音源の種類)。

## syscall (ABI 3)

- **id 空間は SoundObject・Bus・Effect の 3 つ**で、それぞれ 1 から振る (Worker / vm が採番する `alloc`)。
- 構造の操作 (作る・繋ぐ・止める・消す) は対象ごとの syscall。パラメータの syscall も対象ごとに分け、共通化はエンジンの内部で行う
  (canvas と同じ「時間で動かせるパラメータ」の仕組み。ネイティブ `crates/core/src/automation.rs`、Web `engine/automation/Automation.ts`)。
- 離散の設定はパラメータにしない (作るときの引数)。パラメータは常に f32。
- 時刻は ms (i32)、音量は dB (f32)。

```wat
(import "biwa:engine" "sys_sound_bus_create" (func (result i32)))                        ;; → bus
(import "biwa:engine" "sys_sound_bus_add_effect" (func (param i32 i32)))                  ;; bus effect (繋ぐ前だけ)
(import "biwa:engine" "sys_sound_bus_attach" (func (param i32 i32)))                      ;; bus parent
(import "biwa:engine" "sys_sound_output_attach" (func (param i32)))                       ;; bus
(import "biwa:engine" "sys_sound_bus_add_stop_all" (func (param i32 i32)))                ;; bus after
(import "biwa:engine" "sys_sound_bus_delete" (func (param i32 i32)))                      ;; bus after
(import "biwa:engine" "sys_sound_bus_add_transition" (func (param i32 i32 i32 f32 i32 i32)))   ;; bus param kind value after duration
(import "biwa:engine" "sys_sound_bus_set_param" (func (param i32 i32 f32)))               ;; bus param value (発火を待たない)
(import "biwa:engine" "sys_sound_effect_create" (func (param i32 i32 i32) (result i32)))  ;; kind cfg1 cfg2 → effect
(import "biwa:engine" "sys_sound_effect_add_transition" (func (param i32 i32 i32 f32 i32 i32)))
(import "biwa:engine" "sys_sound_effect_set_param" (func (param i32 i32 f32)))
;; path bus volume_db x y loop_start_ms loop_end_ms (0 = ファイルの終わり) normalize → id。まだ鳴らない。
(import "biwa:engine" "sys_sound_create" (func (param externref i32 f32 f32 f32 i32 i32 i32) (result i32)))
(import "biwa:engine" "sys_sound_add_start" (func (param i32 i32 i32 i32)))               ;; id after offset_ms repeat (0 = 無限)
(import "biwa:engine" "sys_sound_add_stop" (func (param i32 i32)))                        ;; id after
(import "biwa:engine" "sys_sound_add_transition" (func (param i32 i32 i32 f32 i32 i32)))  ;; id param kind value after duration
(import "biwa:engine" "sys_sound_preload" (func (param externref)))                       ;; path
```

| 対象 | param |
|---|---|
| SoundObject | 0: 音量 dB、1: x、2: y、3: rate (予約。まだ受け付けない) |
| Bus | 0: 音量 dB |
| Effect | 上の effect の表 |

遷移の kind の番号は canvas と同じ (`engine/nodejs/src/engine/api/transition.ts`)。

既存の syscall の意味の拡張:

- `sys_start_transitions(sync)`: canvas と音の積まれたもの (開始・停止・stop_all・3 種の遷移) をまとめて発火する。
- `sys_await_transitions()`: sync で始めた有限の音 (repeat ≥ 1) が鳴り終わるのも待つ。

std の値の操作として `sys_string_eq` (文字列を中身で比べる。`local`) も ABI 3 で足した (`change_bgm` が同じ曲かを見る)。

## std の API (`std::game::sound`)

```biwa
// 鳴らす
g.play_se(Sound::file("sounds/se/chime.wav"));
g.play_se_and_wait(Sound::file("sounds/se/chime.wav"));     // sync で鳴らして待つ。クリックで飛ばせる
g.play_ambient(Sound::file("sounds/ambient/wind.wav"), ms(2000));
g = g.change_bgm(Sound::new("sounds/bgm/morning.mp3", Volume::db(-4.0), SoundPosition::center(), SoundLoopRange::whole()), ms(1000));
                                                             // クロスフェード。同じファイルなら何もしない
g = g.stop_bgm(ms(3000));
biwa.voice("sounds/voice/biwa_001.ogg");                     // 同じキャラの前のボイスは止まる

// 値 (syscall を出さない)。すべてを渡す new と、既定値で埋める短いコンストラクタがある
Sound::new(path, volume, position, loop_range) / Sound::file(path)          // 0 dB・中央・ファイル全体
SoundPlayback::new(repetition, offset, fade_in, loudness) / SoundPlayback::once() / SoundPlayback::forever()
SoundRepetition::Infinite / SoundRepetition::Count(n)
Loudness::AsIs / Loudness::Normalize
SoundLoopRange::new(start, Some(end)) / SoundLoopRange::whole()
Volume::db(-6.0) / Volume::percent(50) / Volume::silent()
SoundPosition::center() / left() / right() / at(x, y)
hz(500.0)                                                    // Hz::hz。cx() と同じく import して使う
CompressionRatio::new(4.0)                                   // 4:1

// 鳴らす (返るときには鳴り始めている)
let object = bus.play(sound, SoundPlayback::once());
object.stop() / stop_after(d) / fade_out(d) / fade_to(v, d) / move_to(p, d)

// effect: 作った時点で完成している。手元に残した値がパラメータを動かすハンドルになる
let filter = PassFilter::new(FilterSlope::Steep, None, None);   // None はその側を削らない。PassFilter::open(slope) も同じ
Compressor::new(threshold, ratio, attack, release, makeup) / Compressor::limiter()
Duck::new(key_bus, Ducking::new(amount, attack, release)) / duck.set(Ducking::none())

// バス: 親・音量・effect の列を一度に渡す。返るときには木に繋がっている
let bus = Bus::new(BusParent::Bus(g.sound().buses().master()), Volume::db(0.0), Vec::of(BusEffect::PassFilter(filter)));
Bus::under(parent)                                           // 0 dB・effect 無し
filter.high_cut_to(Some(hz(500.0)), ms(1000));               // 水に潜る
bus.fade_to(v, d) / bus.set_volume(v) / bus.stop_all(fade) / bus.delete(after)

// 既定の木と設定
fn main() {
  let sound = SoundBuses::standard();         // 省けば Window::show() が作る
  // third party はここで Window の寿命のバスを足せる
  Window::new(main_scene, scene_page, pages).sound(sound).show();
}
g.sound().buses().se() / g.sound().volumes() / g.sound().set_volumes(Volumes::new(..))
DeveloperConfig::default().with_volumes(..).with_bgm_ducking(Some(Ducking::new(Volume::db(-10.0), ms(80), ms(500))))
```

- 鳴らすにはバスが要るので、issue の `Sound::load(p).play()` (引数なし) は、既定のバスを知っている `Game` のメソッドにした。
- プレイヤーの音量 (`Volumes`) と「`>>` でボイスを止めない」(`voice_continues`) は `UserConfig` にあり、Window の寿命で続く。
  `Window::show()` が既定の木に反映し、`GameSound::set_volumes` で変えると鳴っている音にも効く。
- `>>` でボイスを止めるのは std (`content_flush_and_wait`)。エンジンはクリックで音を止めない。
- SE はラウドネス正規化をしない (大小を意図して付けていることが多い)。BGM とボイスはする (段 2 で効く)。

## エンジンの振る舞い

**論理状態が正で、音の実体は写しである。** 時間・遷移・`after`・ダッキングの判定・削除はすべてミキサーの論理状態
(ネイティブ `crates/core/src/sound.rs`、Web `engine/sound/Mixer.ts`。同じ規則で書いてある) で決め、鳴らす側
(`crates/audio-kira`、`engine/sound/WebAudioOutput.ts`) にはその結果を指示 (`AudioCommand`) として渡す。
headless でも同じ論理が走る。

### 時計

- `after`・遷移・`sleep` はエンジン時計 (ポーズで止まる)。サンプルの進みはオーディオの時計で、**ポーズ中も鳴っている音は止まらない**。
- 開始の精度は 1 フレーム (約 16 ms)。値の変化は毎フレームの論理の値を、鳴らす側で 10〜20 ms ならして段差の音 (zipper) を消す。
- 終わる時刻は論理の側で数える: 有限の音は `(ループの終わり - offset) + (repeat - 1) × (ループの長さ)`。鳴らす側もその時刻ちょうどで止める。

### ブラウザの自動再生の制限

- 利用者の操作があるまで実際の再生を保留し、時間は進める。最初の入力 (pointerdown / keydown) で再開を頼む。
- 許されたとき、無限の音は頭から、有限の音でまだ鳴っているはずのものは今の位置から鳴らす。終わっているはずのものは鳴らさない。

### sync とクリック

- 有限の音 (repeat ≥ 1) の開始は、`once` の区間として数える。読み込みが終わるまでは「終わっていない」。無限の音は数えない。
- クリックで sync の束を畳むとき: 未開始の有限の音は捨て、鳴っている有限の音は止め、未開始の無限の音はすぐ始め、
  遷移は終値で固定し、束の中の停止は実行する。
- ボイスは sync なしで発火する。止めるのは std だけ。
- `stop_all` は、同じ発火で始めた音を巻き込まない (`voice()` は「前のボイスを止めて新しいボイスを鳴らす」を 1 回で発火する)。

### 誤り

- 無いファイル・読めないファイル・3 チャンネル以上のファイルは、長さ 0 の音として扱う (すぐ終わる。`await` は固まらない)。エンジンがログを出す。
- 終わった SoundObject への操作は黙って無視する (鳴り終わった SE を止めるのはよくあるため)。
- 無いバス・effect への操作、繋いだ後の `add_effect`、別のバスに挿した effect の `add_effect`、2 本目の根は警告して無視する。
- ネイティブで出力デバイスを開けなければ、警告して音を出さずに動く (論理状態は動く)。
- headless は出力を開かない。アセットがあれば音をデコードして長さだけを使う。

## セーブ・ロード (記録再生)

まだ実装していない。入れるときは次のようにする:

1. 音の syscall も他の syscall と同じ経路を通るので、記録される。id は Worker / vm で採番する (決定的)。
2. 早送りの間は論理状態だけを進める。
3. 早送りが終わったら、生きている無限の音 (BGM・環境音) を頭から鳴らす。有限の音は鳴らさない。バスと effect のパラメータは論理状態の値を当てる。

## 形式

| Tier | 意味 |
|---|---|
| Tier 1 | どのターゲットでも鳴ることを保証する: Web (最新の Chrome / Firefox / Safari)、ネイティブ (Windows / macOS / Linux)、WebView 版。docs で推す |
| Tier 2 | 実験的。鳴らないターゲットがある。そこでは警告して長さ 0 になる |

| 形式 | Tier | 備考 |
|---|---|---|
| MP3 | 1 | どこでも鳴る (特許は 2017 年に切れた) |
| WAV (PCM、16/24 bit 整数・32 bit 浮動小数) | 1 | 非圧縮で大きい。短い SE 向け。ADPCM などの圧縮した WAV は Tier 2 |
| Ogg Vorbis | 1 | ゲームの素材に多い。Safari は 18.4 から鳴る (古い Safari では Tier 2 と同じ扱い) |
| m4a (AAC) | 2 | Windows / macOS は OS のデコーダで鳴らす予定 (段 2)。Linux のネイティブでは鳴らさない |
| Opus、FLAC | 2 | |

- ネイティブのデコードは symphonia (MPL-2.0。`crates/audio-decode`)。AAC は特許の論点を避けるため同梱のデコーダでは鳴らさない。
- 鳴らすのは kira (`crates/audio-kira`)。Linux では ALSA (`libasound.so.2`) に動的リンクする (`desktop-build.md`)。
- 第三者ライセンスの告知を配布物に同梱する仕組みは別 issue。

## 後でやること

- resource の id で先読みする `sys_sound_load`、止まる preload。
- ラウドネス正規化。ダッキングの level モード。
- 残響・ディレイ・EQ と、センド。空間音響 (距離の減衰、z)。master のモノラル化とバランス。
- 5.1 のファイル (ステレオへのダウンミックス)。ストリーミングの BGM。
- UI の SE (UI Element の property で、wasm と往復しない)。スキップとオート (ボイスの終わりを待つ)。
- 音も止めるポーズ、裏に回ったときの消音。再生速度、口パク用の音量の取得、再生位置の取得。
- 地の文に埋め込む `$v("akane/001.mp3")`。`Game` の drop からの Game の寿命のバスの削除。
- m4a の OS のデコーダ (Windows: Media Foundation、macOS: AudioToolbox)。
