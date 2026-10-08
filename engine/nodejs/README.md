# Biwa Engine (Node.js based edition)

Biwa エンジンの基盤エンジン実装 (Node.js 版)。
描画は PixiJS (WebGL) と DOM、ビルドと開発サーバは Vite。

このディレクトリは `biwa` コマンドに同梱され、`biwa dev` 実行時に
プロジェクトの `.biwa_runtime/` へ展開される。Vite のルートはそこになる。
エンジン単体でも `npm run dev` で起動でき、その場合はゲームが無い状態
(`src/game/entry.ts` のプレースホルダ) で立ち上がる。

## ゲームコードとの境界

コンパイラは Biwa のコードを **wasm または TypeScript** に変換する。
ゲーム側の入口は `biwa dev` が生成するスタブ `src/game/entry.ts` を経由して呼ばれる。
起動の流れはどちらのターゲットでも同じである:

1. `__biwa_app` (= `fn app()`) を呼ぶ。ゲーム側がその中で `Window[S]` を組み立てて
   `show()` し、UI の syscall が出る。**エンジンは UI を何も置かない**。
   Window は scene を映す Page (`scene_page_id`) と scene 本体 (`main_scene` のハンドラ) を
   Page より先に持っていなければならず、欠けていればゲームを止める (`UiContractError`)
2. Window の `scene_page_id` の Page に遷移するのを待つ (`UIObjects.onScenePageEntered`)
3. その Page の `canvas` / `message_area` の ui_id で `__biwa_std_game_window_new` を呼び、
   `__biwa_on_new_game(window)` → `__biwa_entrypoint` (= `scene main`) を始める
   (2 回目以降の遷移は未定義。いまは最初の 1 回だけ)

```
src/main.ts
  ├─ kind: "wasm"        → runWasm(url, scenePage)          ← Worker で wasm を走らせる
  └─ kind: "typescript"  → kernel.run(entrypoint(game))     ← scene main (generator)
```

どちらで来ても、エンジン API の実装 (`src/engine/api/*`) は同じものが呼ばれる。
違うのは「どこで動いていて、どうやって中断するか」だけである。

エンジンが知っているのは以下だけで、パッケージ名や生成されたシンボル名は知らない。

| 規約                              | 実体                                                                      |
| --------------------------------- | ------------------------------------------------------------------------- |
| `src/game/entry.ts`               | `biwa dev` が生成するスタブ。default export が `BiwaBackend`              |
| `@biwa/engine/<path>`             | std から参照されるエンジン実装。Vite alias で解決 (TypeScript ターゲット) |
| `biwa:engine` / `biwa:runtime`    | wasm の import 名前空間 (wasm ターゲット)                                 |
| `BiwaGame` (`src/engine/game.ts`) | `std::game::Game` に対応する構造                                          |

## エンジン API は syscall である

ゲームコードから見ると、エンジンの API はシステムコールにあたる。
呼ぶとエンジン側 (kernel land) の処理に入り、
API によっては完了までブロックし、API によっては処理を積んで直ちに返る。

中断をどう作るかはターゲットで違う。

### TypeScript ターゲット

**scene は generator function として出力される。** これが VM にあたる。
`yield` が syscall 命令で、kernel が `next()` で結果を書き戻して再開する。

```ts
// 生成された scene
export function* main(g) {
  yield write("こんにちは。"); // ← レジスタに積んで VM exit
  yield wait(); // ← クリックが来るまで再開しない
  return g;
}
```

| 層                        | 役割                                     | 対応するもの         |
| ------------------------- | ---------------------------------------- | -------------------- |
| std (`base_engine.biwa`)  | syscall の記述子を組み立てる             | レジスタに引数を積む |
| コンパイラ (codegen)      | scene の中に `yield` を置く              | syscall 命令         |
| `src/engine/vm/kernel.ts` | 記述子を処理し、結果を書き戻して再開する | kernel land          |

```
src/engine/vm/
  syscall.ts   # syscall 番号と記述子の型 (std との唯一の合意点)
  kernel.ts    # VM を回すループ
  handlers.ts  # syscall 番号 → 実装の対応表
src/engine/api/
  context.ts    # 現在のエンジン実体 (syscall の実装が参照する)
  message.ts    # Content API (push / flush / clear) と waitForClick
  object.ts     # canvas オブジェクトの生成・遷移・削除
  transition.ts # param / kind の番号と曲線 (std との合意点)
```

kernel を通るのは**中断する syscall だけ**である。
中断しない syscall (canvas オブジェクトの操作など) は
std の native が `@biwa/engine/api/*` を直接呼ぶので、記述子にならない。

| syscall    | 対応する std の関数   | ふるまい                 |
| ---------- | --------------------- | ------------------------ |
| `Sys.Wait` | `base_engine::wait`   | クリックまで**中断する** |

Content API (`sys_content_push_text` / `sys_content_flush` /
`sys_content_clear`) は中断しないので番号を持たない。
std の native が `@biwa/engine/api/message` を直接呼ぶ。

`await_transitions` / `sleep` は TypeScript ターゲットには無い。
コンパイラが `yield` を置くのは今のところ novel statement の展開先だけで、
任意の関数呼び出しを中断させる手段が無いためである (wasm ターゲット専用)。

**その `Sys.Wait` すら、いまは TypeScript 経路では届かない。**
`sys_wait` を呼ぶのは std の `content_flush_and_wait()` という普通の関数で、
statement の位置には無いので `yield` が置かれない
(`docs/content-api.md` の段 2)。TypeScript は tier 2 なので当面このままである。

実装が Promise を返せばブロッキング syscall で、解決するまで scene を再開しない。
値をそのまま返せば非ブロッキング syscall で、scene はそのまま走り続ける。
非ブロッキング syscall が続いてフレームを落とさないよう、
8ms を超えたら一度 `requestAnimationFrame` に制御を返す。

### 中断できるのは scene の中だけ

generator にしているのは scene だけなので、

- 中断する syscall は scene の中にしか現れない (novel statement の展開先なので構文上そうなる)
- 中断しない API (画像を出す、名前を変える) は普通の関数呼び出しでよい。
  std の native 実装が `@biwa/engine/api/*` を直接呼ぶ
- 普通の `fn` からブロッキング API を呼ぶことはできない

kernel が `next()` を呼ばない限り VM は止まったままなので、
ポーズ・スキップ・オート・速度調整はこのループの外側で決められる。
`scene.return()` で巻き戻せば、シーンの強制終了もできる。

### wasm ターゲット

生成物は WasmGC を使った 1 つのモジュールで、エンジン API はホスト関数の import になる。

```wat
(import "biwa:engine" "sys_content_flush" (func $sys_content_flush))
(import "biwa:engine" "sys_wait"          (func $sys_wait))
```

wasm 自身には中断の仕組みが無いので、**wasm を Worker で走らせ、
ブロッキング syscall ではそのスレッドを `Atomics.wait` で止める**。
メインスレッドが処理して結果を `SharedArrayBuffer` に書き、`Atomics.notify` で起こす。
止まるのは Worker だけなので、その間も描画とイベント処理は動き続ける。

```
src/engine/vm/wasm/
  contract.ts  # import 名 → 区分 (local / cast / alloc / retain / call)。std との合意点
  bridge.ts    # SAB のプロトコル
  worker.ts    # wasm の instantiate と実行。ここが VM の中
  host.ts      # メインスレッド側の kernel。syscall を api/* に流す
```

| import                                          | 区分         | 実行される場所                    |
| ----------------------------------------------- | ------------ | --------------------------------- |
| `biwa:runtime` `string_const`                   | -            | Worker (memory から UTF-8 を復号) |
| `biwa:engine` `sys_content_push_text` ほか      | 積んで返る   | Main                              |
| `biwa:engine` `sys_wait`                        | **中断する** | Main                              |
| `biwa:engine` `sys_create_object`               | 積んで返る   | Main (id の採番のみ Worker)       |
| `biwa:engine` `sys_add_transition` ほか         | 積んで返る   | Main                              |
| `biwa:engine` `sys_await_transitions`           | **中断する** | Main                              |
| `biwa:engine` `sys_sleep`                       | **中断する** | Main                              |
| `biwa:engine` `sys_string_concat` / `sys_map_*` | -            | Worker                            |
| `biwa:engine` `sys_int_to_string` ほか          | -            | Worker                            |
| `biwa:engine` `sys_ui_set_handler`              | 積んで返る   | Main (関数は Worker に預ける)     |

`sys_create_object` は戻り値 (オブジェクト id) を持つが**中断しない**。
採番だけを Worker 内で行い、本体はメインスレッドへ投げるからである
(`contract.ts` の `alloc`)。素直に中断させると、
オブジェクトを 1 つ作るたびにスレッドが往復してしまう。

`sys_ui_set_handler` は Biwa の関数の値 (`funcref`) を受け取る。関数は
`postMessage` できないので Worker のハンドラの表に預け、メインスレッドには
番号だけを送る (`contract.ts` の `retain`)。持ち主の Element が消えると
メインスレッドが番号を Worker に返し、Worker は表から外す。
規定は `docs/host-function-values.md` にある。

積んで返る syscall はまとめて 1 通の `postMessage` で流している。
Worker はゲームの実行中にイベントループへ帰らない (生成物を同期に呼び切る) ので、
流す契機は `bridge.ts` が自分で決めている。
遷移の発火が 1 通に収まるおかげで、**メインスレッドは発火の途中でフレームを描けない。**

Worker 側に置くものがあるのは、`externref` / `anyref` が JS のオブジェクト参照
そのもので**スレッド境界を越えられない**からである。
`String` は externref、`Map` / `Option` は externref / anyref なので、
それらを触るだけの操作は wasm と同じスレッドに置くしかない。

`SharedArrayBuffer` を使うため、ページは cross-origin isolated である必要がある
(`vite.config.ts` が COOP/COEP を送っている)。**配信するサーバでも同じヘッダが要る。**

制限:

- 初期 `Game` を渡せない。WasmGC の struct を JS から作れないので `null` を渡している。
  scene が `g` のフィールドを読むと trap する
- ブロッキング syscall が返せるのは JSON にできる値だけである

設計の検討過程 (`node:vm` が使えない理由、Worker + `Atomics.wait` や
QuickJS + ASYNCIFY との比較、セーブ・ロードの方針) は
[`docs/execution-model.md`](../../docs/execution-model.md) にある。

## アセット

アセットはゲームのパッケージ直下の `assets/` に置かれる (`src/` の兄弟)。
`.biwa` が書くパスは**その `assets/` を基準とした相対パス**である。

`biwa dev` はそのディレクトリを `.biwa_runtime/public/assets` へリンクする。
`public/` の中身はそのまま URL のルートに出るので、
エンジンから見たアセットの位置は常にこうなる。

```
.biwa が書いたパス  →  `${import.meta.env.BASE_URL}assets/<path>`

  "bg/room.png"     →  /assets/bg/room.png
```

この変換は `src/engine/api/assets.ts` の `resolveAssetUrl()` 1 箇所にある。
画像以外の syscall (音・動画) が増えても同じ規約に従わせるためで、
アセットを読む API は必ずここを通すこと。

`..` で `assets/` の外へ出るパスは弾く。
将来はコンパイラがパスの実在も含めて静的に検査する。

## ディレクトリ構成

```
src/
  main.ts               # 起動: レンダラ初期化 → コンテキスト登録 → ターゲットに応じて実行
  engine/
    game.ts             # ゲームコードとの境界の型 (BiwaGame / BiwaEntrypoint / BiwaBackend)
    vm/                 # scene を駆動する kernel と syscall の定義
    vm/wasm/            # wasm 生成物を Worker で走らせる側
    api/                # syscall の実装 (std からも `@biwa/engine/api/*` として呼ばれる)
    api/assets.ts       # `.biwa` が書くアセットのパスを URL に直す
    api/transition.ts   # param / kind の番号と曲線・波形 (std との合意点)
    canvas/
      CanvasObjects.ts  # canvas オブジェクトと遷移の本体。Ticker で回る
    ui/
      UIObjects.ts      # UI Element のツリーと DOM。MessageArea が TextBox を持つ
    Renderer.ts         # host の大きさとエンジンの時計 (唯一の Ticker)
    canvas/CanvasSurfaces.ts  # Canvas Element ごとの描画先 (Element の中の `<canvas>`)
    LayerManager.ts     # canvas / DOM レイヤーの生成・参照管理
  components/
    TextBox.ts          # メッセージウィンドウ。断片の列と文字送り。MessageArea ごとに 1 つ
  game/
    entry.ts            # `biwa dev` が生成して上書きする (リポジトリのものはプレースホルダ)
```

## Message Window と Content API

scene の生テキストと `$` の埋め込み式は、std の `Content` を経て
`sys_content_push_text` としてエンジンに届く。
設計と決めごとは [`docs/content-api.md`](../../docs/content-api.md) にある。

- 出力先は UI Element `MessageArea` で、Content API の syscall は第一引数の
  ui_id でそれを指定する。MessageArea が 1 つずつ `TextBox` を持つ
  (`engine/ui/UIObjects.ts`)。枠の位置・大きさ・背景・余白は MessageArea の property が決める
- 届く値は**すべて解決済みの絶対値**である。速度・大きさ・色の設定は
  `Game` が持ち、std が潰してから渡す。**エンジンは設定を知らない**
- `push` は積むだけで何も起きない。`flush` で初めて文字送りが始まる
- **枠をクリアするのはエンジンの判断ではない。** `sys_content_clear` が来たときだけ消す。
  いまは std の `content_flush_and_wait()` がクリック待ちから戻った直後に呼ぶ
- 断片は最初から全文を DOM に入れ、まだ出ていない分を `visibility: hidden` で隠す。
  `textContent` を伸ばす形にすると、折り返しが変わって既に出ている行までずれる
- クリックは**進行中のものをまず畳む**。文字送りの途中なら残りを全部出し、
  sync 印の演出が走っていれば終端へ飛ばす。両方を 1 回のクリックで畳むので、
  テキストが進むのは次のクリックである
- 文字送りは `main.ts` の唯一の Ticker コールバックから、すべての MessageArea を
  まとめて駆動する (`UIObjects.update`)。倍率は `CanvasObjects.timeScale` を借りている。
  クリックでの送りの完了もすべての MessageArea に効く
- 装飾 (`$blue(bold("琵琶"))`) は std に閉じている。
  エンジンに届くのは解決済みの色・大きさ・太さ・速度だけで、
  **装飾 API が増えてもエンジンは変わらない**。
  italic のようにいまの syscall が運べない装飾だけが例外である

## canvas オブジェクトと遷移

画像などを canvas に置き、パラメータの遷移でアニメーションさせる。
規約と設計の理由は [`docs/media-object-model.md`](../../docs/media-object-model.md) にある。

- 出力先は UI Element `Canvas` で、`sys_create_object` の第一引数の ui_id で指定する。
  描画範囲はその Element の矩形で、はみ出しは切り取る (`engine/canvas/CanvasSurfaces.ts`)
- 座標は **出力先の Canvas の中央が原点**で、x は右が正、**y は上が正**。
  位置も回転も画像の中心を基準にする (PixiJS とは向きも単位も違うので、
  biwa 側の値を正として毎フレーム射影している)
- 遷移は `add_transition` で積み、`start_transitions` で発火する。
  発火はオブジェクトを跨いで一斉に起こる
- **置き換わるのは積まれた遷移が触れたパラメータだけ**である。
  背景のパンの最中にキャラクターが跳ねても、パンは死なない
- パラメータごとに、いつでも活性な区間は 1 つ。
  ある区間は同じパラメータの次の区間が始まった時点で終わる
- 周期系 (`sin` など) は基準値からの**偏差**を乗せるだけなので、
  区間が終われば値は基準値に戻る。止めるには `none` を置く

Ticker に登録するコールバックは `main.ts` の 1 つだけである。
オブジェクトごとに生やさないのは、リークを避けるためでもあるし、
ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためでもある。
時間は `performance.now()` ではなく Ticker の差分を積んだエンジン時計で測る。

### テキストと同期する演出 / しない演出

`start_transitions(sync)` の `sync` が 0 でなければ、
その演出はテキストの進行と結びつく。

| したいこと                                       | 書き方                            |
| ------------------------------------------------ | --------------------------------- |
| 走らせっぱなし (背景のパン、常時のゆらぎ)        | `start(0)`                        |
| テキストは進むが、次のクリックで先に演出が終わる | `start(1)`                        |
| 演出が終わるまでシーンを進めない                 | `start(1)` → `await_transitions()` |

クリック待ちの最中に進行中の sync 演出があれば、
クリックはまずそれを完了させて消費される (`api/message.ts`)。
完了の判定は終端を持つ区間だけを見るので、
ゆらぎが混じっていても待ちが固まることはない。

## レイヤーシステム

- **DOM レイヤー**: UI 描画 (HTMLElement, `pointer-events: none`)。名前で識別する。
  UI Element はすべてここに入り、Element どうしの前後は `Layers` の push 順で決まる
- **canvas レイヤー**: UI Element `Canvas` ごとの描画先 (`canvas/CanvasSurfaces.ts`) の中の
  PixiJS の Container。**整数の index** で識別し、`create_object` に渡された index の
  ものが必要に応じて作られる。背景・立ち絵・前景といった意味づけは std の仕事なので
  ここには無い。レイヤーは安いので、前後を細かく分けたければ index を分ければよい。
  描画先の `<canvas>` は Canvas Element の中にあるので、他の Element との前後は DOM の重なりに従う

DOM レイヤーと UI コンポーネントの定義は将来 XML で行う。

```xml
<layers>
  <layer id="message" type="dom" />
</layers>
```

## 技術スタック

| 用途                 | ライブラリ                  |
| -------------------- | --------------------------- |
| ビルド               | Vite + TypeScript (vanilla) |
| 描画                 | PixiJS v8 (WebGL/Canvas)    |
| アニメーション       | PixiJS Ticker (自前の遷移)  |
| デスクトップ出力     | Tauri (予定)                |
| パッケージマネージャ | npm                         |

- GSAP は採用しない (PixiJS Ticker と混在させると事故が起きやすいため)。
- npm を使うのは、利用者の環境に Node.js しか仮定しないため。

## 今後の実装予定

- [ ] セーブ・ロード (syscall ログの記録と再生)
- [ ] XML によるレイヤー・コンポーネント定義のローダー
- [ ] `std::game::window` の native (`MessageWindow` / `Canvas`) の実装
- [ ] `characters` / `states` の初期化をゲーム側から渡す口
- [ ] 動画 / GIF のバックエンド (`create_object` は拡張子で分ける前提で書いてある)
- [ ] `preload` (テクスチャが読めるまでオブジェクトは見えないので、
      その間のフェードインは見えないまま終わる)
- [ ] ポーズ・オート・スキップ (エンジン時計の `timeScale` と kernel の駆動間隔)
- [ ] Tauri アダプター (ファイルアクセスの抽象化)
