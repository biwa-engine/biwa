# scene の実行モデル: syscall による VM exit と再開

## 何を解きたいのか

Biwa の `scene` は、エンジンの API を呼びながら進む。

```biwa
scene main(g: MyGame) -> MyGame {{
  こんにちは。
  >>                      // クリック待ち
  #let drink = ask_drink(g)
  #if drink == "coffee" {
    珈琲ですね。
  }
}}
```

エンジンの API 呼び出しはシステムコールに近い。呼ぶとエンジン側 (kernel land) の
処理に入り、API によっては完了までブロックし、API によっては処理を積んで直ちに返る。

現在の biwac は scene を**同期関数**として TypeScript に吐く。
JavaScript の同期関数の中で「クリックが来るまで待つ」ことはできないため、
`wait` は即座に返るしかなく、シーンは最後まで一気に流れる。

やりたいのは:

- ブロッキング syscall で**実行を中断**し、制御をエンジンに返す
- エンジンがイベント (クリック、選択) を捌いたら、**同じ地点から再開**する
- scene のローカル変数は TS (JS) の普通のメモリ領域に置いたままにする
- 中断・再開の単位が明示的で、後からセーブ・ロードやスキップを載せられる

「レジスタに引数を積んで syscall 命令を発行し、ホストが結果を書き戻して再開する」
という形が作れれば達成できる。

## 結論

**generator function を VM、`yield` を syscall 命令として使う。** 実装済み。

- biwac は **scene を** `function*` として吐く
- syscall は `yield <記述子>` (= レジスタに積んで VM exit)。
  記述子を組み立てるのは std、`yield` を置くのはコンパイラ
- scene から scene への呼び出しは `yield*` (= 呼び出し先の VM exit を素通しする)
- エンジン側の kernel が `scene.next(result)` で結果を書き戻して再開する

プロトタイプで、ブロッキング待ち・プレイヤー入力による分岐・セーブ/ロードが
すべて動くことを確認した (後述)。syscall 1 回のコストは実測 **約 75ns**。

## 検討した方式

| 方式 | VM exit の実現 | コンパイラ変更 | 変数の置き場 | 直列化 | 判定 |
| --- | --- | --- | --- | --- | --- |
| A. `node:vm` | できない | なし | - | - | ✗ |
| B. generator + trampoline | `yield` | 中 (codegen) | 普通の JS スコープ | 不可 | **採用** |
| C. Worker + `Atomics.wait` | スレッドブロック | **なし** | 普通の JS スコープ | 不可 | 次点 |
| D. QuickJS(wasm) + ASYNCIFY | wasm スタック巻き戻し | なし | QuickJS ヒープ | 不可 | 過剰 |
| E. 自前 wasm + JSPI | スタックスイッチ | 特大 (新バックエンド) | wasm 線形メモリ | 不可 | 将来 |
| F. 自前バイトコード VM | 明示的な命令ポインタ | 大 (新バックエンド) | VM の配列 | **可能** | 将来 |

### A. `node:vm` — 使えない

`node:vm` は「別のグローバルコンテキストでスクリプトを実行する」ためのもので、
コルーチンではない。実行中のスクリプトを途中で止めて再開する機能は無い。
[pause/resume の要望](https://github.com/nodejs/node/issues/33359)は
[10 年以上前から](https://github.com/nodejs/node-v0.x-archive/issues/7801)出ているが実装されていない。
`timeout` オプションは中断ではなく強制終了である。

そもそもエンジンはブラウザ (Vite + PixiJS) で動くので、Node.js 専用 API は使えない。
「Node.js の VM で」という発想の受け皿になるのは、JS 実行系そのものを持ち込む D か、
JS の言語機能である B になる。

### B. generator + trampoline — 採用

JavaScript が言語として持っているコルーチンが generator である。
これは「レジスタに積んで VM exit」そのものが書ける。

```ts
// 生成コード (ゲーム側の scene)
export function* main(g) {
  yield write("こんにちは。");   // ← 積んで exit。戻り値が書き戻される
  yield wait();
  return g;
}
```

```ts
// エンジン側 (kernel)
async function run(gen) {
  let send;
  for (;;) {
    const { value, done } = gen.next(send);        // ← VM 再開
    if (done) return value;
    send = await handlers[value.sys](...value.args); // ← 処理して結果を書き戻す
  }
}
```

性質:

- **ローカル変数は普通の JS スコープに残る。** V8 は generator のローカルをヒープに
  置くが、アクセスはレジスタ番号によるインデックス参照でスタックと同じ速さである。
  中断時の追加コストはほぼ無く (命令ポインタをスロットに書くだけ)、
  生成コードは `let x = ...` のまま読める。
- **ブロッキングと非ブロッキングを handler 側だけで決められる。**
  Promise を返せばブロッキング、即座に返せば非ブロッキング。
  scene 側のコードは何も変わらない。
- **中断が明示的。** kernel が `next` を呼ばなければ VM は止まったまま。
  スキップ・オート・速度調整・ポーズがそのまま実装できる。
- **中断できる。** `gen.return()` で巻き戻せる (シーン強制終了、タイトルへ戻る)。
- **デバッグできる。** 生成物は普通の TypeScript なので、
  devtools でステップ実行でき、スタックにも scene が出る。

コスト:

- `yield*` による委譲は素の関数呼び出しより遅い
  ([一般に 2 倍前後](https://hiddentao.com/archives/2014/02/14/javascript-generator-delegation-and-coroutine-performance/))。
  実測は後述の通り syscall 1 回あたり約 75ns で、
  ノベルゲームの実行頻度 (1 シーンで数千回) では完全に無視できる。
- 関数の「色」が増える。generator である関数は generator からしか呼べない。
  → generator にするのを scene に限ることで、色の伝播そのものを避けた (後述)。
- **generator の実行状態は直列化できない。** セーブは別立てで設計する (後述)。

### C. Worker + SharedArrayBuffer + `Atomics.wait` — 次点

scene を Worker で走らせ、syscall では共有メモリに引数を書いて `Atomics.notify`、
そのまま `Atomics.wait` でスレッドごとブロックする。メインスレッド (kernel) が
処理して結果を共有メモリに書き、`Atomics.notify` で起こす。

「レジスタに積んで syscall 命令を発行する」に最も近いのはこれで、
しかも **biwac を一切変更しなくてよい** (今の同期コードのまま動く) のが強い。

ただし:

- `Atomics.wait` は[メインスレッドでは使えない](https://v8.dev/features/atomics)。
  scene 側が Worker になるのは必然 (DOM も PixiJS も触れない = kernel land との分離は明確になる)。
- `SharedArrayBuffer` は cross-origin isolation (COOP/COEP ヘッダ) が要る。
  開発サーバでは設定できるが、配布形態に制約が出る。
- ブロック中の Worker は
  [postMessage を処理できない](https://github.com/nodejs/node/issues/21417)。
  つまり**戻り値も共有メモリ経由で符号化して渡す**必要がある。
  文字列や構造体を返す syscall のたびにエンコード/デコードを書くことになる。
- デバッグが難しい。スタックが 2 本に割れ、ブレークポイントも跨げない。

B が採れない場合の代案として持っておく価値はあるが、
コンパイラを触れる以上、先に B を採る。

### D. QuickJS (wasm) + ASYNCIFY

生成した TypeScript を、wasm にコンパイルされた QuickJS の中で走らせる。
[quickjs-emscripten](https://github.com/justjake/quickjs-emscripten) の ASYNCIFY ビルドは
「VM 内の同期コードからホストの async 関数を呼ぶ」ことができる。
wasm モジュール全体を中断してホストの Promise を待ち、完了したら再開する。

文字通りの VM ではあるが、

- サイズ 2 倍・速度 40% という[明示された代償](https://github.com/justjake/quickjs-emscripten)がある
- 一度に 1 つの中断しか持てない
- ホストと VM の間で値のマーシャリングが要る (PixiJS を触るには結局全部橋渡し)
- 変数は QuickJS のヒープに載る (「普通の JS メモリ」ではなくなる)

サンドボックス (サードパーティ製 MOD の実行など) が要件になったら再検討する。

### E. 自前 wasm + JSPI

[JSPI](https://v8.dev/blog/jspi) は 2025 年 4 月に W3C Wasm CG で標準化され、
Chrome 137 / Firefox 139 で出荷済み。wasm から Promise を返す JS API を呼ぶと
wasm スタックが中断され、解決後に再開される。まさに VM exit だが、
biwac に wasm バックエンドを作る話になるので今回のスコープ外。
将来ネイティブ配布 (Tauri) と合わせて検討する余地はある。

### F. 自前バイトコード VM

biwac がバイトコードを吐き、TS でインタプリタを書く。
ノベル/ナラティブエンジンの主流はこれで、
[ink](https://github.com/inkle/ink) は `StoryState` (変数・コールスタック・
現在位置) を JSON に直列化してセーブに使う。
[Yarn Spinner](https://docs.yarnspinner.dev/components/asynchronous-programming) も
スタックマシンの VM を持ち、コマンドハンドラが「中断するか継続するか」を返す。

**唯一、実行状態をそのまま保存できる**方式である。
一方で「変数を普通の JS メモリで扱いたい」という要求とは正面から衝突し、
TypeScript 生成という現在の設計も捨てることになる。

いま採るには早いが、**syscall の ABI を同じに保っておけば、
後から VM だけ差し替えられる**。B の設計はそれを意識して決める。

## 実装 (採用方式 B)

生成コードは以下の形になる。**generator になるのは scene だけ**である。

```ts
// scene: generator function として出力される
export function* _ZN5test14mainE(__lv1: Game<...>): Generator<unknown, Game<...>, any> {
	yield _ZN3std4game11base_engine5writeE("  Hello, World! \n");
	yield _ZN3std4game11base_engine4waitE();
	let __lv2: number = _ZN5test13addE(1, 2);   // 普通の関数はそのまま
	if (__lv2 > 2) {
		yield _ZN3std4game11base_engine5writeE("    Nice! \n");
		yield _ZN3std4game11base_engine4waitE();
	}
	return __lv1;
}
```

```ts
// std: syscall の記述子を組み立てるだけの普通の関数 (libc のスタブにあたる)
export function _ZN3std4game11base_engine4waitE(): Syscall {
	return { sys: Sys.Wait, args: [] };
}
```

### 役割分担

| 層 | 役割 | 対応するもの |
| --- | --- | --- |
| std (`base_engine.biwa`) | syscall の記述子を組み立てる | レジスタに引数を積む |
| コンパイラ (codegen) | scene の中に `yield` を置く | syscall 命令 |
| エンジン (`src/engine/vm/kernel.ts`) | 記述子を見て処理し、結果を書き戻して再開する | kernel land |

syscall 番号の定義はエンジン (`src/engine/vm/syscall.ts`) にあり、
std が `@biwa/engine/vm/syscall` として取り込む。コンパイラは番号を知らない。

### 中断できるのは scene の中だけ

generator にするのを scene に限ったので、次の性質になる。

- **中断する syscall** (クリック待ち) は scene の中にしか現れない。
  novel statement の展開先なので、これは構文上も保証される。
- **中断しない syscall** (画像を出す、名前を変える) は普通の関数呼び出しでよい。
  std の native 実装がエンジンの API を直接呼び、積んで即座に返る。
- 普通の `fn` の中からブロッキング API を呼ぶことは**できない**。
  必要になったら、syscall に到達しうる関数を generator にする色付け解析を入れる
  (Biwa には第一級関数が無いので、呼び出しグラフ上の伝播で厳密に決められる)。
- scene から scene を呼ぶ場合は `yield*` で委譲する。
  呼び出し先の syscall がそのまま外側の kernel まで抜ける。
  ただし判定できるのは自パッケージの scene だけで、
  パッケージを跨いだ scene 呼び出しには `.biwameta` に種別を載せる必要がある。

### 型

generator function の戻り値型は `Generator` でなければ TypeScript が受け付けないので、
scene の戻り値型は `Generator<unknown, <scene の戻り値>, any>` として出力する。
yield する値の型 (syscall 記述子) はエンジンが知っていればよいので `unknown`、
書き戻される値は syscall ごとに違うので `any` とした。

std 側の syscall 記述子の型は lang item `syscall` として宣言する。
型推論はこれを使って「novel statement の展開先が syscall を返すこと」を検査する。

### 待ちコマンド `>>`

ノベルテキスト中の `>>` が待ちになる。

```biwa
  こんにちは。 >>      // この行を書いてから待つ
  >>                   // 待つだけ
```

### エンジン側 (kernel)

```
src/engine/vm/
  syscall.ts   # syscall 番号と記述子の型 (std との唯一の合意点)
  kernel.ts    # VM を回すループ
  handlers.ts  # syscall 番号 → 実装の対応表
```

kernel は `scene.next(send)` で再開し、`yield` された記述子を見て実装を呼ぶ。
実装が Promise を返せばブロッキング syscall で、解決するまで scene を再開しない。
値をそのまま返せば非ブロッキング syscall で、scene はそのまま走り続ける。
非ブロッキング syscall が続いてフレームを落とさないよう、
8ms を超えたら一度 `requestAnimationFrame` に制御を返す。

### 変更した箇所

| 対象 | 内容 |
| --- | --- |
| `biwac_generator` | scene を `generator: true` に、戻り値型を `Generator<...>` に、novel statement を `yield <lang item 呼び出し>` に、scene 呼び出しを `yield*` に |
| `biwac_lang_item` | lang item `syscall` (syscall 記述子の型) を追加 |
| `biwac_type_inferrer` | novel statement の展開先の戻り値を `Syscall` として検査 |
| `biwac_novel_parser` | 待ちコマンド `>>` のパース |
| `library/std` | `write` / `wait` が syscall 記述子を返すように。`Syscall` 型を追加 |
| `engine/nodejs` | `src/engine/vm/` (kernel・syscall 定義・対応表)、`main.ts` を VM 駆動に |
| `cli` | 変更なし (`entry.ts` のスタブはそのまま使える) |

## セーブ・ロード

generator の実行状態は直列化できない。JS にコルーチンのスナップショットを
取る手段は無い (F を採らない限りこれは変わらない)。
しかし Biwa の設計はこれを迂回できる。

- ゲームの状態は `Game` (`characters` / `states`) に集約されており、これは素のデータで直列化できる
- scene は `Game` を受け取って `Game` を返す関数である
- syscall の結果 (クリック、選択肢の選択) 以外に外部入力が無い

したがって **「シーンの開始地点 + syscall の戻り値ログ」** をセーブに持てば、
ロード時に同じ scene を最初から再実行し、ログを食わせて副作用を止めたまま
早送りすることで、**同じ実行地点に到達できる**。いわゆる記録再生である。

これは Ren'Py がやっていることに近い。Ren'Py は
[文の開始時点でセーブし、現在の文と戻り先だけを保存する](https://www.renpy.org/doc/html/save_load_rollback.html)。
ink のように実行状態そのものを JSON にする方式 (F) との中間にあたる。

要件:

- scene が決定的であること (乱数は `Game` の状態からシードする)
- 早送り中は syscall を実行せず、ログの値を返すこと (kernel の 1 分岐で済む)
- スクリプトを書き換えたセーブデータは再生が破綻しうる。
  シーンのフィンガープリントを保存しておき、不一致ならシーン先頭に戻す

## プロトタイプの結果

`scratchpad/vm-poc.mjs` に、生成コードを手書きで模したものと kernel を書いて確認した。

- ブロッキング待ちが効く (クリックが来るまで次の行を描画しない)
- **プレイヤーの入力による分岐が書ける** (`choice` の戻り値で `if` が分かれる)。
  現在の command queue 方式では原理的にできない部分である
- 途中でセーブ → 別の実行で syscall ログを再生 → 同じ地点から再開、が成立する。
  再生中は副作用が出ず、再開後の選択だけが実際に描画された
- 速度: syscall (`yield*` + `yield` + `next`) 100 万回で **74.6ms** (≒ 75ns/回)。
  素の関数呼び出し 100 万回は 2.3ms。比では 32 倍だが、
  1 シーンあたり数千回のオーダーなら 1ms に満たない

## 残っていること

- **セーブ・ロード**: kernel に syscall のログと早送りを実装する。
  中断しない syscall (画像を出すなど) は現在エンジンの API を直接呼んでいるため、
  再生中に副作用を止められない。記録再生を入れるときは、
  これらも kernel を経由させる必要がある (`yield` は不要で、登録するだけでよい)。
- **色付け**: 普通の `fn` からブロッキング API を呼べるようにする場合に必要。
- **パッケージを跨いだ scene 呼び出し**: `.biwameta` に scene かどうかを持たせる。
- **スキップ・オート・ポーズ**: kernel が `next()` を呼ぶ間隔と条件を変えるだけで載る。
  中断は `scene.return()` で巻き戻せる。

ABI を syscall 記述子に閉じ込めてあるので、
将来 F (自前バイトコード VM) に移すときもエンジン側の syscall 実装は再利用できる。
