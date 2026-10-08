# ホストとの関数の受け渡し

Biwa の関数の値 (`fn(A) -> R`。名前の付いた関数・scene・無名関数) を syscall でホスト (エンジン) に渡し、
ホストが後でそれを呼ぶときの規定。UI のハンドラ (`Window.main_scene`、`SceneStartButton.on_click`、
いずれ `Button.on_click` / `Window.on_event`) の土台である。

関数の値そのものの言語仕様は `function-as-the-first-class-type.md`、
UI の改訂版の流れは `ui-api.md` と `ui-api-impl-status.md` §19 にある。

## 要約

- Biwa → ホスト: syscall の引数で関数を渡す。wasm では import の引数の型は `funcref`。
- ホストは関数を**ハンドラの表** (番号 → 関数) に預け、番号 (handle) で指す。
  wasm では表は Worker にあり、メインスレッドには番号だけが届く。
- ホスト → Biwa: 預かった関数を、ハンドラの種類ごとに決まった形で呼ぶ。引数・戻り値の中身は見ない。
- 呼んでよいのは Biwa のコードが走っていないときだけ (再入しない)。
- 表の項目は持ち主 (UI Element) が消えるときに手放す。
- 型の安全は Biwa 側が保証する。ホストは型を知らない。

## Biwa → ホスト

### wasm

std の native が関数の値を `funcref` の引数として import に渡す。

```wat
(import "biwa:engine" "sys_ui_set_handler"
  (func $sys_ui_set_handler (param i32 i32 funcref)))
```

```biwa
[[native(arch="wasm")]]
fn sys_ui_set_handler[A, R](ui_id: Uint, kind: Uint, f: fn(A) -> R) {{
  local.get 0
  local.get 1
  local.get 2
  call $sys_ui_set_handler
}}
```

- 生成物の中で関数の値は `(ref null $F)` (`$F` は関数型ごとの型) である。
  どの `(ref null $F)` も `funcref` の部分型なので、そのまま `funcref` の引数に渡せる。
  std の native は単相化後の型名 (`$__fn.fN`) を知らなくてよく、ジェネリックな native 1 つで済む。
- 単相化に追加の仕組みは要らない。渡す関数は `Const::FnDef` として根から到達するので実体が作られ、
  `ref.func` のための `(elem declare func ..)` も既に出ている。
- JS 側には呼べる関数 (wasm の関数のラッパー、`length` は引数の数) として届く。

### TypeScript

関数の値は普通の JS の関数である。std の native が `retainHandler(f)` (`@biwa/engine/api/handler`) で
メインスレッドの表に預け、番号に替えてから syscall の実装を呼ぶ。
scene (generator function) を預けた場合、呼ぶ側が kernel で回す必要がある (未実装。TypeScript は tier 2)。

## ハンドラの表

関数 (wasm の関数のラッパー) は `postMessage` できず、UI の DOM はメインスレッドにある。
そこで関数は**関数が生きているスレッドの表**に預け、他所には番号だけを渡す。

| ターゲット | 表の場所                          | メインスレッドに届くもの |
| ---------- | --------------------------------- | ------------------------ |
| wasm       | Worker (`vm/wasm/worker.ts`)      | 番号                     |
| TypeScript | メインスレッド (`api/handler.ts`) | 番号                     |

- 番号は 1 から振る単調増加の整数で、0 は「無し」に取っておく。ui_id と同じく決定的である。
- wasm では syscall の区分 `retain` (`vm/wasm/contract.ts`) が、引数のうち関数であるものを表に預けて番号に替え、
  あとは `cast` と同じく積んで返る。
- メインスレッドは番号を UI Element ごとに「ハンドラの種類 → 番号」で持つ (`UIObjects`)。
  関数そのものは持たない。

## 寿命

- 表が関数を握っている間、その関数は GC されない。
- 番号は**持ち主の Element が消えるとき**に手放す。
  - 同じ Element・同じ種類に設定し直したときは、古い番号を手放す。
  - 設定できなかったとき (Element が無い、種類を知らない、その Element には付けられない) は、
    その番号をすぐに手放す。誰も呼ばないのに握り続けないため。
- wasm では、メインスレッドが手放す番号を Worker に `{ kind: "release", handles }` で送り、Worker が表から外す。
  Worker は Biwa のコードを実行している間イベントループに帰らないので、
  知らせが処理されるのは Worker が手すきになってからである。手放すのが遅れるだけで、害は無い。
- Window はいまのところ消える経路が無いので、Window のハンドラはページが閉じるまで生きる。

## ホスト → Biwa

ホストは預かった関数を、ハンドラの種類ごとに決まった形で呼ぶ。

- 引数・戻り値は wasm の型の規則で JS と行き来する。
  - `i32` / `f32` (`Int` / `Uint` / `Float` / `Bool`): JS の数。
  - struct・enum などの GC 参照 (`GameWindow`、`Game[S]` …): 中身の見えない JS の値。
    ホストは受け取ったものをそのまま次の呼び出しに渡すだけで、中を見ない。
    `S` のような型引数はホストに見えない。
  - `externref` (`String` など): JS の値そのもの。
- 形の合わない値を渡すと、wasm の境界で `TypeError` になる
  (例: `fn(Game[S]) -> Game[S]` に数を渡す)。`null` は境界を通るが、Biwa 側で使った時点で trap する。
  したがってホストは種類ごとの形を守らなければならない。
- 型の安全は Biwa 側が保証する。std の型付きの API (`Window[S].main_scene(Scene[S])` など) が
  種類ごとの形に合う関数しか `sys_ui_set_handler` に渡さず、UI の木の型引数 `S` が
  `main_scene` と `on_click` の `Game[S]` を揃える。ホストは kind の番号しか見ない。

### 呼んでよいとき

**Worker が Biwa のコードを実行していないときだけ**呼んでよい。

- 今の流れでは、UI を表示した後 (`show` が返った後) にイベントを待っている状態がこれに当たる。
  Worker はイベントループに戻っているので、メインスレッドからの知らせを受けて呼べる。
- scene の実行中は、Worker は止まる syscall (`Atomics.wait`) の中にいる。そこから預かった関数を呼び返すこと
  (再入) はしない。scene の外でしか押されない `SceneStartButton` にはこれで足りる。
  scene の実行中にも押せる `Button.on_click` / `Window.on_event` (`ui-api.md` の Phase3) で、
  再入を許すか、イベントを積んで scene の切れ目で処理するかを決める。
- 呼び出しは Worker の中で同期に行う。呼んだ先が止まる syscall を出せば、そこで Worker が止まるのは
  scene と同じである。

## ハンドラの種類

番号は engine の `src/engine/api/ui.ts` の `HandlerKind` が正。

| kind | 名前                      | 付けられる Element            | 形                                    | 使う段階 |
| ---- | ------------------------- | ----------------------------- | ------------------------------------- | -------- |
| 0    | `WindowMainScene`         | `Window`                      | `Scene[S]` = `fn(Game[S]) -> Game[S]` | R4       |
| 1    | `SceneStartButtonOnClick` | `SceneStartButton` (まだ無い) | `fn(GameWindow) -> Game[S]`           | R6       |

種類を足すときは、`HandlerKind` と付けられる Element (`HANDLER_TARGETS`)、
それを設定する std の型付きの API、ホストの呼ぶ側を対で変更する。
今の native は引数 1 つの関数だけを扱う。他の個数が要れば個数ごとに native を足す (import は `funcref` のままでよい)。

## syscall

| syscall              | 引数                                  | wasm の区分 | 説明                                     |
| -------------------- | ------------------------------------- | ----------- | ---------------------------------------- |
| `sys_ui_set_handler` | `ui_id: Uint, kind: Uint, f: funcref` | `retain`    | Element にハンドラを設定する。積んで返る |
