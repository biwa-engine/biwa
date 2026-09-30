# UI API Phase1 実装方針

## 0. スコープ確定

- **含む**: UI syscall の策定・実装 (engine, std)、UI 要素の型・関数の std 実装
- **含まない**:
  - XML構文 (Phase2)
  - funcref / 関数を値として渡す仕組み (Phase3) → `Button`, `Window.on_event` は mdの記載通り未実装
  - `fn app() -> Window` エントリポイント、`scene_page_id` による scene 自動起動 (`on_new_game(save_id)` 配線)。これは `cli/src/runtime.rs` / `biwac_driver` の固定エントリ (`__biwa_entrypoint` / `__biwa_on_new_game`) を置き換える別の大きい変更であり、Phase1の「syscall+std抽象化」の範囲を超えるため見送る
- **Phase1でのUIの出し方**: 既存の `scene` / 通常の `fn` から命令的に呼び出す (`Window::new()...` のような chain API)。`Link.on_click_link` はページ遷移先が文字列なので funcref 不要 → 実装対象に含めるが、遷移は「同じ Window 内での Page 表示切り替え」に留め、scene 起動は行わない

## 1. 要素範囲 (段階実装)

| Step     | 要素                                                                        | Property                                                                                               |
| -------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| 1 (最小) | `Window`, `Page`, `Box`, `Link`, `Horizontal`, `Vertical`, `HorizontalGrid` | `width`/`height`/`margin`/`padding` (vw/vh/percent), `background_color`                                |
| 2        | `Image`                                                                     | `text`/`text_font`/`text_size`/`text_weight`/`text_color`, `background_image` (background_colorと排他) |
| 3        | `Canvas`, `MessageArea`                                                     | 既存 PixiJS canvas / `TextBox` シングルトンへの「ポータル」として実装                                  |

`Button` は funcref 前提のため mdの記載通り Phase1 では実装しない。

## 2. Syscall 設計

先の合意の通り、property 設定は文字列の有無で2系統に分割し、数値枠は既知の property (color の r/g/b/a 4値まで) を包含できる幅にする。

```
fn sys_ui_create(kind: u32) -> u32;                          // alloc

fn sys_ui_set_property(                                       // cast
  ui_id: u32, kind: u32,
  val_u1: u32, val_u2: u32, val_u3: u32, val_u4: u32,
  val_i1: i32, val_i2: i32,
  val_f: f32,
);

fn sys_ui_set_property_with_string(                            // cast
  ui_id: u32, kind: u32,
  val_u: u32, val_i: i32, val_f: f32,
  val_s: string,
);

fn sys_ui_push_child(parent: u32, child: u32);                 // cast
```

- `sys_ui_create` は `sys_create_object` と同型 (id 発行、`alloc`)
- `push_child` の挙動は md 記載通り (複数子OK→末尾追加 / 単一子→既存を置換 / 子を持てない→リストから削除) をエンジン側の `push_child` 実装に閉じ込める
- Element kind / property kind の番号表は `engine/nodejs/src/engine/api/transition.ts` の `Param`/`Curve` と同じ流儀で一元管理し、wasm 側 std は番号を `.wat` に直書き (ズレたらエンジンが名指しで弾く既存方針を踏襲)

## 3. エンジン側実装

- **番号表**: `engine/nodejs/src/engine/api/ui.ts` に `ElementKind` (Window/Page/Box/Link/Horizontal/Vertical/HorizontalGrid/…) と `PropertyKind` (Width/Height/Margin*/Padding*/BackgroundColor/…) を定義
- **UIツリー管理**: `engine/nodejs/src/engine/ui/UIObjects.ts` を新設 (`CanvasObjects.ts` 相当)。`ui_id → { kind, properties, children[], dom }` を保持し、DOM への反映を担当
- **DOM層**: `LayerManager` に UI 用 DOM 層を追加 (`defineDom("ui", zIndex)`)。`Horizontal`/`Vertical`/`HorizontalGrid` は flexbox (row / column / wrap) にマップ、`Box`/`Link` は `div`/`a` 相当
- **Window/Page**: Window生成時は子Pageを全て非表示にし、明示的に `show()` された Page のみ表示 (アニメーションなし、即切替)。複数 Window は非対応 (最後に作られたものが有効、程度の単純な制約でよいか要検討)
- **Canvas/MessageArea (Step3)**: 新規描画対象を作らず、既存の `Renderer` の `<canvas>` および `ComponentRegistry` の `TextBox` を UI ツリー中のプレースホルダ `div` にマウントするポータルとして実装
- **syscall配線** (`CLAUDE.md` の規約通り対で変更):
  - `wasm/contract.ts`: `sys_ui_create` = `"alloc"`, `sys_ui_set_property`/`_with_string`/`sys_ui_push_child` = `"cast"`
  - `wasm/host.ts`: dispatch 対応表に追加
  - TypeScript経路: `vm/syscall.ts`（中断しないので実は番号登録は不要。`base_engine.biwa` の native が直接 `api/ui.ts` を呼ぶだけでよい）
  - `api/ui.ts` に実処理 (`createUiElement`, `setUiProperty`, `setUiPropertyString`, `pushUiChild`) を実装し、TS/wasm 両経路から合流させる (`api/object.ts` と同じ構造)

## 4. std 側実装

- `library/std/src/game/base_engine.biwa` に `sys_ui_*` の native 宣言を追加 (`typescript`/`wasm` 両方、`sys_create_object` と同じ書式)
- 新規 `library/std/src/game/ui.biwa`:
  - `Window`, `Page`, `Box`, `Link`, `Horizontal`, `Vertical`, `HorizontalGrid` struct
  - 各 struct は内部に `ui_id: Uint` を持ち、`create → set_property* → push_child` の3段を隠蔽する chain API を提供 (mdの「アトミックに出現させる」意図をここで実現)
  - 例: `Vertical::new().width(vw(60.0)).child(link1).child(link2)` のような形（XML糖衣はPhase2）
  - `width`/`height`/`margin`/`padding` は既存の `Size` (vw/vh) を再利用しつつ `percent` を追加
  - `background_color` は既存 `Color` 型を再利用

## 5. 未確定・実装しながら詰める点

- `ElementKind`/`PropertyKind` の具体的な番号割当
- 単一 Window 前提でよいか (複数 Window は非対応と明言するか)
- `Link` の見た目 (CSSデフォルトのみか、std側で最低限のスタイルを持たせるか)
- Page切替に「まだ何もしない」でよいか (将来のtransition連携は据え置き)

## 6. 実装ステップ順

1. ✅ engine: `ElementKind`/`PropertyKind` 番号表 + `UIObjects.ts` (Window/Page/Box/Link/Layout) + UI用DOM層
2. ✅ engine: syscall の配線 (`contract.ts` / `wasm/host.ts` / `api/ui.ts`)
3. ✅ std: `base_engine.biwa` に `sys_ui_*` native 宣言追加
4. std: `game/ui.biwa` に Window/Page/Box/Link/Layout struct + chain API
5. `test1` パッケージで `biwa dev` して手動確認 (最小の Window+Page+Link)
6. Step2: `Image`, text系 property, `background_image`
7. Step3: `Canvas`/`MessageArea` ポータル実装

## 7. 実装状況 (Step 1〜3 完了、Step 4 でユーザー確認待ち)

### 変更したファイル

- 新規 `engine/nodejs/src/engine/api/ui.ts`: `ElementKind` / `PropertyKind` / `Unit` の番号表 + `isKnown*` 検査 + syscall 実装 (`allocUiId` / `createUiElement` / `setUiProperty` / `setUiPropertyString` / `pushUiChild`)
- 新規 `engine/nodejs/src/engine/ui/UIObjects.ts`: UI ツリーの管理と DOM への反映 (`CanvasObjects.ts` 相当)
- `engine/nodejs/src/engine/api/context.ts`: `EngineContext` に `ui: UIObjects` を追加
- `engine/nodejs/src/main.ts`: UI 用 DOM 層 (`"ui"`, z-index 25、Message Window の `20` より前面) を定義し `UIObjects` を生成・登録
- `engine/nodejs/src/engine/vm/wasm/contract.ts`: `sys_ui_create` (alloc) / `sys_ui_set_property` (cast) / `sys_ui_set_property_with_string` (cast) / `sys_ui_push_child` (cast) を登録
- `engine/nodejs/src/engine/vm/wasm/host.ts`: 上記 4 syscall のハンドラを追加
- `library/std/src/game/base_engine.biwa`: `sys_ui_*` の native 宣言 (`typescript`/`wasm` 両方) を追加

`vm/syscall.ts` / `vm/handlers.ts` (TypeScript 経路の中断する syscall 専用) は変更不要だった。UI syscall はすべて中断しないため、`sys_create_object` などと同様に std の native が `@biwa/engine/api/ui` を直接呼ぶだけで済む。

### 実装しながら確定した内容

- **ElementKind の番号**: `Window=0, Page=1, Box=2, Link=3, Horizontal=4, Vertical=5, HorizontalGrid=6`。7 以降は Step2/3 (`Image`/`Canvas`/`MessageArea`) 用に空けてある。`Button` は割り当てない。
- **PropertyKind の番号**: 数値系 (`sys_ui_set_property`) を `0`〜、文字列系 (`sys_ui_set_property_with_string`) を `100`〜、と帯を分けた (syscall がこれを見て振り分けるわけではなく、取り違えを早期発見するための整理)。
  - 数値系: `Width=0, Height=1, MarginLeft/Right/Top/Bottom=2..5, PaddingLeft/Right/Top/Bottom=6..9, BackgroundColor=10, Column=11`
  - 文字列系: `PageId=100, OnClickLink=101`
  - `Width`/`Height`/`Margin*`/`Padding*` は `val_u1`=単位(`Unit.Vw/Vh/Percent`)・`val_f`=値 の組で運ぶ。`val_i1`/`val_i2` は今のところ使う property が無いが (符号付きの位置指定などを見越して) syscall のシグネチャには残してある。
  - `BackgroundColor` は `val_u1..val_u4` に r/g/b/a (0〜255) をそのまま積む。1 語にパックしないのは `sys_content_push_text` の color 引数と同じ理由 (biwa にビット演算が無く、算術で詰めると `a` が符号を跨ぐ)。
- **`PageId` / `OnClickLink` の追加**: md のProperty表には無いが、Page の識別と Link の遷移先指定に必須なため、文字列系 property として実装した (structural な property として扱う)。
- **Window の直接マウント**: Window は誰の子にもならない (mdにもpush先が無い) ため、`sys_ui_create` の時点で UI 用 DOM 層のルートに直接マウントする。複数 Window を作った場合は両方ルートにマウントされ重なって表示される (非対応というより「未定義動作」に近い扱いで、Phase1 では単一 Window を前提とする)。
- **Page の初期表示**: 「最初に Window (実際には Window 直下) へ push された Page」が自動的に表示され、以降は Link の `on_click_link` が一致する `page_id` を持つ Page だけを表示する (他は `display: none`)。切替に演出は無い (即切り替え)。md には `scene_page_id` による scene 起動が書かれているが、[スコープ確定](#0-スコープ確定)の通り Phase1 では未実装。
- **push_child の「1つのみ持てる」場合の置換 (`Box`) / 「持てない」場合の削除 (`Link` など)**: どちらも対象ノードを `nodes` map から完全に削除する (`destroy()`)。削除は子孫にも再帰する。
- **Link のクリックと `waitForClick` の競合回避**: Link の DOM に張る click ハンドラで `event.stopPropagation()` を呼び、`host` まで伝播させない。伝播すると Message Window のクリック待ちも同時に反応してしまうため。
- **pointer-events の扱い**: UI 用 DOM 層および Window/Page/Box/Horizontal/Vertical/HorizontalGrid は `pointer-events: none` (クリックを奪わない)。`Link` だけ `pointer-events: auto` + `cursor: pointer`。Link に `width`/`height` 等の property が未設定だと 0 サイズになりクリックできない点は Step2 (text property) 以降で実用上解消される見込み。
- **`sys_ui_create` の id 空間**: wasm 経路では `wasm/worker.ts` の `alloc` 実装 (`nextObjectId` というグローバル単調増加カウンタ) をそのまま共有しており、canvas object の id と UI element の id は同じ数列から採番される。別々の Map (`CanvasObjects.objects` / `UIObjects.nodes`) で管理しているので衝突しても問題は無い。

### 動作確認

- `cargo run -p biwac -- -p /home/coder/test1 --target wasm` で `std` (今回の `sys_ui_*` 追加分を含む) のコンパイルが通ることを確認した。
- `--target typescript` は `game/content.biwa` の既存の「trait 境界付きジェネリック未対応」エラーで失敗するが、これは今回の変更と無関係の既知の tier2 制限であり、`base_engine.biwa` の差分は影響していない (差分は `base_engine.biwa` のみ)。
- `npx tsc --noEmit` (engine/nodejs) はエラー無し。
- 実際にブラウザ上で Window/Page/Link を表示して動作確認するのは Step4 (`game/ui.biwa`) 以降になる (現状は syscall を発行する Biwa コードが無い)。

### 次にやること (要ユーザー確認)

Step4 (`library/std/src/game/ui.biwa` の Window/Page/Box/Link/Layout struct + chain API) に進む前に、Step1〜3 の設計判断 (特に上記の「実装しながら確定した内容」) に異論が無いか確認してもらう。

## 8. Step4 実装 (完了)

### 設計: `UiElement` は enum、生成中は具体型

syscall はコンストラクタの都度発行しない。まず Biwa (Wasm runtime) 側で `UiElement`
の値としての木を組み立て、`Window::show()` / 各 Element の `materialize()` という
**唯一の消費点**で初めて木を再帰的に辿って `sys_ui_*` を発行し、エンジン側にデータ
構造をコピーする。Biwa 側の値が正で、エンジンへは一方的に副作用として反映される
だけ、という Biwa Engine 全体の構成をここでも踏襲した。

- `enum UiElement { Box(Box), Link(Link), Horizontal(Horizontal), Vertical(Vertical), HorizontalGrid(HorizontalGrid) }`
- `Window` / `Page` は `UiElement` の variant ではなく独立した型 (Window の子は
  Page のみ、という木構造の制約を型で表すため。`docs/ui-api.md` の XML 節が
  `UiElement` と `UiPage` を別の型として扱っているのに合わせた)
- 各具体型 (`Box`/`Link`/`Horizontal`/`Vertical`/`HorizontalGrid`) のビルダー
  メソッド (`.width(..)` 等) はその具体型に直接生やし、`UiElement` への変換
  (`Into[UiElement]`) は子として親に渡す最後の瞬間にだけ行う。こうすることで
  生成中・保持中は具体型のままなので、種類を取り違えたプロパティ呼び出しは
  コンパイルエラーになる (`Content`/`Into[Content]` と同じ枠組みを踏襲)
- variant で分岐する `match` は `impl UiElement { fn materialize(self) -> Uint }`
  の中の 1 箇所だけ

### ファイル構成

ユーザー指示により、struct 定義は `game/ui.biwa` に集約し、コンストラクタ・
ビルダーメソッド・`materialize()` は `game/ui/<element>.biwa` に分けた
(Biwa Language にはまだ pub use 的なエクスポートも visibility も無いため、
型定義自体は同じファイルに置かざるを得ない。将来的には定義自体も
`ui/<element>.biwa` に置き、`pub import <element>::<Element>;` の形になる見込み)。

- `game/ui.biwa`: `GameWindow`/`Canvas`/`MessageWindow`/`Position`/`Size` (既存、後述の通り一部リネーム) + `UiSize`/`vw`/`vh`/`percent` + element/property kind 番号 + `apply_layout_properties`/`set_size_property`/`push_children` (共通ヘルパ) + `enum UiElement` と `impl UiElement { materialize }` + Window/Page/Box/Link/Horizontal/Vertical/HorizontalGrid の struct 定義
- `game/ui/window.biwa`: `impl Window { new, push, show }`
- `game/ui/page.biwa`: `impl Page { new, push, materialize_into }` (Page は UiElement ではないので、木の消費点は `Window::show` 経由のここ)
- `game/ui/box.biwa`, `link.biwa`, `horizontal.biwa`, `vertical.biwa`, `horizontal_grid.biwa`: それぞれのコンストラクタ・レイアウト系ビルダーメソッド・`materialize()`・`impl <Element>: Into[UiElement]`

### 名前の衝突: `Window` を `GameWindow` にリネーム

既存の `struct Window { canvas: Canvas, message_window: MessageWindow }`
(`Game.window` が持つ、canvas と Message Window への参照の束。
`content_push` などが `game.window.message_window.push_text(..)` の形で使う、
UI Element とは無関係の既存の仕組み) が、新しい UI Element の root `Window`
と名前が衝突した。影響範囲が小さい (`game.biwa` の import・フィールド型・
コンストラクタ呼び出しのみ。`content.biwa` はフィールドアクセスなので無修正)
既存側を `GameWindow` にリネームして解消した。ユーザーへの確認はせず、
リバーシブルな内部リネームとして進めた。

**同じ衝突が Step3 (`Canvas`/`MessageArea` UI Element) で `Canvas`/`MessageWindow`
にも起こる。** そのときに同様のリネームが要る (`GameCanvas`/`GameMessageWindow` 等)。

### `UiSize` を新設 (既存の `Size` とは別型)

Message Window の `Size` (vw/vh のみ) を流用せず、`percent` を持つ別の enum
`UiSize` を新設した。理由: `Size` を拡張すると `MessageWindow.push_text` の
既存の match が非網羅になり (`Percent` に意味の無い text サイズへの対応を
迫られる)、影響範囲が UI Element の外まで広がるため。

### 実装中に踏んだ構文上の制限 (今後の参考に)

- **空ブロック `{}` は match アーム・if の中では書けない。** `Option::None => {}`
  はパースエラーになる (`{` の直後を構造体リテラルのフィールドとして読もうとして
  `}` で落ちる)。回避策: `match` ではなく `if x.is_some() { .. }` の形にして
  `None` 側の分岐そのものを書かない。
- **`if` に `else` が要るかどうかは、then 節の中身が「文」か「式」かで決まり、
  それは中身の構文形だけで決まる (前後の文脈やセミコロンの有無は無関係)。**
  たとえば `if cond { match x { A => expr, B => expr } }` は、`match` の各アームが
  「裸の式 (`pattern => expr,`)」なので `match` 全体が**式**として読まれ、
  結果 `if` も式になり `else` が必須になる。`if` の then 節を「文」として
  (= `else` 無しで) 書きたいなら、内側の `match` も文でなければならず、
  そのためには**各アームを `{ expr; }` の形 (ブロック + セミコロン) で書く**
  必要がある (1 つでもアームを裸の式にすると、その時点で `match` 全体が式になる)。
  `if` 全体の後ろに `;` を付けても、`match`/`if` は「式のあとの `;`」を
  それ自体では消費しないため効果が無い (`consume_expression_or_statement` が
  `KwMatch`/`KwIf` を特別扱いしており、後続の `;` を見ない)。
  `library/std/src/game/ui.biwa` の `set_size_property` / `apply_layout_properties`
  がこの形の実例になっている。

### 動作確認

- `cargo run -p biwac -- -p /home/coder/test1 --target wasm` で `std` を含めて
  コンパイルが通ることを確認 (test1 自体は新 API をまだ使っていない)。
- 別途 probe パッケージ (`std` に依存するだけの使い捨てパッケージ) で、
  実際に `Link::new("scene").width(vw(30.0)).into()` →
  `Horizontal::new(children).into()` → `Page::new("main", ..)` →
  `Window::new(..).show()` という一連の呼び出し連鎖が wasm ターゲットで
  型エラー無くコンパイルできることを確認した。
- ブラウザ上での実描画確認はまだ (Step5 で `biwa dev` を使って行う予定)。

### 次にやること (この節の時点)

- Step5: `test1` で実際に `Window`/`Page`/`Link` を組み立てて `biwa dev` し、
  ブラウザで表示・クリック遷移を確認する。
- Step2: `Image`, text 系 property, `background_image`。
- Step3: `Canvas`/`MessageArea` ポータル実装 (`Canvas`/`MessageWindow` の
  リネームも一緒に行う)。

## 9. Step5: 実機確認 (完了) — ついでにコンパイラの `while` バグを発見・修正

### やったこと

`test1/src/main.biwa` の `scene main` に、コメントで区切った差分として以下を追加した
(`// --- UI API 動作確認 (Phase1 Step5) ここから ---` 〜 `ここまで` で囲んである)。

- import 群 (`UiElement`/`Window`/`Page`/`Link`/`Vertical`/`vw`/`vh`/`Into`) を
  「UI API 動作確認用の追加」ブロックとして囲んで追加
- `fn build_demo_window() -> Window`: `"main"` / `"second"` の 2 Page を持つ
  Window を組み立てる。各 Page には、もう一方の page_id を `on_click_link` に
  持つ Link (`main`=青、`second`=赤、`width(vw(30))`/`height(vh(8))`/
  `margin_left(vw(5))`/`margin_top(vh(5))`) を 1 つ置いた
- `scene main` の冒頭に `#build_demo_window().show()` を追加

`biwa dev --target wasm` (既定) で起動し、Playwright (Chromium headless) で
実際にページを開いてスクリーンショットを撮り、Link のクリックも自動化して確認した。
このサンドボックスには Claude in Chrome 拡張が無かったため、代替手段として
Playwright を使った。

### 環境上の障害と対処 (今後のため記録)

- **Node.js のバージョン**: この環境の `node` は v18.19.1 だが、
  同梱エンジンの `vite@8`/`rolldown` は Node `^20.19.0 || >=22.12.0` を要求する。
  v18 では `node:util` の `styleText` が無く即座に落ち、
  v20.18.1 でも `rolldown` のネイティブバイナリ (optional dependency) が
  `npm install` で解決されず落ちた (`npm install` 自体が `^20.19.0` を
  満たさないと optional dependency を正しく解決しないように見える)。
  **v22.14.0 を使うことで解消した。** システムの Node には触れず、
  `/tmp/.../node-v22.14.0-linux-x64` に展開して `PATH` に前置するだけで
  (`.biwa_runtime/node_modules` を一度削除して作り直す必要がある)。
- **ブラウザでの確認手段**: Claude in Chrome が使えなかったため、
  Playwright (`npx playwright install --with-deps chromium`) を別途
  用意して headless Chromium でスクリーンショット・クリックを行った。

### 発見したバグ: `while` ループがコンパイラのバグで 2 周目に進まない

`Window::show()` (Page を 1 つずつ `materialize_into` する `while` ループ) を
実行すると、**2 つ目の Page が Window の子として登録されない**
(`no such page in this window: "second"`) という現象に遭遇した。

切り分けの結果 (要旨のみ残す):

- `Window::push`/`Vec.push` によるページの追加は正しく行われている
  (`w.pages.len()` を novel テキストに埋め込んで確認 → `2`)
- ページの順序を入れ替えると、**常に 2 番目に処理された要素だけ**が
  欠落する (page_id の値には無関係)
- 独立した最小の `while` ループ (`while i < 5 { total = total + 1; i = i + 1; }`)
  ですら `0` を返す (5 でも 1 でもなく `0`)

`.biwa_build/wasm/test1.wat` に出力された実際の wat を読んで特定した。
原因は `compiler/src/biwac_generator/src/arch/wasm/structure.rs` の
`Builder::do_tree`:

```rust
fn do_tree(&mut self, bb: BasicBlock) -> Vec<Structured> {
    if self.cfg.is_loop_header(bb) {
        self.context.push(Enclosing::Loop(bb));
        let inner = self.node_within(bb);
        self.context.pop();
        vec![Structured::Loop(Box::new(Structured::Block(inner)))]  // ← Block を合成
    } else {
        self.node_within(bb)
    }
}
```

ループの頭を `Structured::Loop(Box::new(Structured::Block(inner)))` という、
**`loop` の中にもう 1 段 `block` を合成する**形で包んでいるが、
`br` の相対深さを数える `context`(`depth_of`)にはこの合成 `block` の分が
積まれていなかった。そのため、ループ本体の中の `if`(= while の条件分岐)
から後方辺 (先頭へ戻る辺) へ戻る `br` の深さが実際より 1 つ浅く計算され、
**「ループの先頭に戻る」つもりが「合成した block の外(= ループそのものの外)」
に `br` してしまい、2 周目に入らずループを抜けていた**。

既存のテスト (`structure.rs` の `while_loop`) は「`Loop` ノードが存在すること」
と「`br` の深さが囲みの数を超えていないこと」しか見ておらず、
「有効だが的が違う深さ」を検出できていなかった
(`br` は 1 段浅くても "合成 block" という別の有効な囲みに収まってしまうため、
深さの上限チェックには引っかからない)。

**修正** (`structure.rs`):

- `Enclosing` に `LoopBody` variant を追加 (`br` の対象にはならないが、
  `If` と同様に深さの数え上げには 1 段加える)
- `do_tree` でループの頭を包む際、`Enclosing::Loop(bb)` に加えて
  `Enclosing::LoopBody` も `context` に積んでから `node_within` を呼ぶ
- `while_loop` テストを強化し、後方辺の `Br` が実際に `Loop` を指す深さに
  なっていることを、既知の CFG 形状から一意に定まる木を直接パターンマッチして
  検証するように変更 (単に「範囲内か」ではなく「意図した箇所を指しているか」
  を見る回帰テスト)

`cargo test`(workspace 全体)はすべて通過することを確認した。

### 影響範囲の補足

- MIR の `while` 文は、条件分岐 (`if`) を伴わない形ではおそらく存在しない
  (条件判定自体が `SwitchInt`/`If` に落ちるため)。つまり
  **要素 1 個より多く繰り返す `while` ループは、このバグの影響を
  実質的に必ず受けていたはず**である。`compiler/assets/tests/mir_fixture`
  に `while` の機能テストは存在したが、`--emit mir` で MIR 構築の形だけを
  確認するものであり (「TypeScript の codegen が while 文を todo!() のまま
  扱えていないため既定のビルド経路には乗せない」との注記どおり)、
  **wasm ターゲットで実行して結果を検証するテストではなかった**ため、
  これまで見つかっていなかったと考えられる。
- 今回の UI API の実装 (`Window::show`/`push_children` の `while` ループ) が、
  このリポジトリで `while` ループを実際に実行させた最初のケースだった
  可能性が高い。

### 動作確認 (最終)

- `main` ページ (青 Link, `on_click_link="second"`) が起動時に表示される
- クリックすると `second` ページ (赤 Link, `on_click_link="main"`) に切り替わる
- 再度クリックすると `main` に戻る
- コンソールエラー無し (`no such page` エラーは解消)
- スクリーンショット3枚 (main → second → main) で相互遷移を確認済み

### 次にやること (この節の時点)

- Step2: `Image`, text 系 property, `background_image`。
- Step3: `Canvas`/`MessageArea` ポータル実装 (`Canvas`/`MessageWindow` の
  リネームも一緒に行う)。
- `test1/src/main.biwa` の動作確認用の追加分は、コメントで囲んだままリポジトリに
  残っている (削除するかどうかはユーザー判断)。

## 10. Step2: `Image` / text 系 property / `background_image` (完了)

### 開発環境: Node.js を nvm でこの環境に正式導入

`biwa dev` 同梱エンジンの `vite@8`/`rolldown` が要求する Node `^20.19.0` を
満たすため、`nvm` (v0.40.1) を導入し `node v22.23.3` を `default` にした。
このシェル環境では `~/.bashrc` が非対話シェルで早期 `return` するため
(`nvm` の読み込み行が実行されない)、`/usr/local/bin/{node,npm,npx,corepack}`
に nvm でインストールした実体へのシンボリックリンクを張ることで、
対話・非対話どちらのシェルでも `node`/`npm` がこのバージョンに解決されるようにした
(`/usr/local/bin` は `/usr/bin` より `$PATH` で手前に来る)。
システムの apt 管理下の Node (`/usr/bin/node`, v18.19.1) は変更していない。
`claude` (このセッション自身) の動作に影響が無いことも確認済み。

### 設計

- **`Background` enum を新設**し、`background_color`/`background_image` の
  排他性を型で保証した (`enum Background { Color(Color), Image(String) }`)。
  Box/Link/Horizontal/Vertical/HorizontalGrid/Page の
  `background_color: Option[Color]` フィールドをすべて
  `background: Option[Background]` に置き換えた。
  ビルダーメソッド `.background_color(c)` / `.background_image(path)` は
  従来どおり両方生やしてあるが、内部で同じフィールドに書き込むため、
  後から呼んだ方が勝つ (両方は同時に設定できない)。
  `apply_layout_properties` の対応する引数も `background: Option[Background]`
  に変え、内部で数値 (`sys_ui_set_property`) か文字列
  (`sys_ui_set_property_with_string`) かを振り分ける `apply_background` を
  切り出して、`Page::materialize_into` からも共有した。
- **text 系 property は `Link` にだけ実装した。** `docs/ui-api.md` の
  Elements 節で text 属性が明示されているのは `Link` の例
  (`<Link text="NEW GAME" .../>`) のみで、Box や Layout 系には子として
  テキストを表示する Element が無いため。`apply_text_properties` という
  共有ヘルパーにしてあるので、将来他の Element にも text を持たせたくなったら
  フィールドを足すだけで済む。
- **`Image` Element は std 上 `UiImage` という型名にした。**
  `std::game::image::Image` (canvas に置く画像。`Image::new(path).show_in_canvas(..)`
  で使う、UI とは無関係の既存の型) と名前が衝突するため
  (Biwa に import のエイリアスが無く、同じスコープに両方 `Image` として
  import できない)。前回の `Window`→`GameWindow` のときは
  「新しい方の名前を死守し、古い方(あまり使われていない内部用)をリネーム」
  したが、今回は逆に**既存の `Image` が character の立ち絵・背景など
  広く使われすぎていてリネームが割に合わない**ため、新しい方を
  `UiImage` にリネームして解消した。エンジン側の `ElementKind.Image` の
  番号 (7) には影響しない (名前は std 側だけの問題)。
- `UiImage` の property は `image`(パス文字列) と width/height/margin/padding
  のみ。background は持たせていない (画像そのものが背景の役割を兼ねるため)。

### 追加した property kind (`api/ui.ts` / `game/ui.biwa` で対で管理)

- 数値 (`sys_ui_set_property`): `TextSize=12` (unit+value, Width と同じ形),
  `TextWeight=13` (100〜900, val_u1 のみ), `TextColor=14` (r,g,b,a)
- 文字列 (`sys_ui_set_property_with_string`): `Text=102`, `TextFont=103`,
  `BackgroundImage=104`, `Image=105`
- Element kind: `Image=7`

### エンジン側の変更 (`UIObjects.ts`)

- `ElementKind.Image` の DOM: `background-size: cover` 等を持つ `div`
  (画像用の `<img>` タグではなく、他の Element と同じ CSS 背景方式に揃えた)
- `Link` の既定スタイルに `display: inline-flex` + 中央寄せを追加
  (テキストを持つようになったため; Step1 時点では空の box だったので不要だった)
- `BackgroundColor` 設定時は `backgroundImage` を空にし、`BackgroundImage`
  設定時は `backgroundColor` を空にする (相互に上書きしたときの後始末。
  std の `Background` enum による排他性を、エンジン側の CSS 適用でも
  素直になぞっただけで、判定ロジックでは無い)
- `BackgroundImage`/`Image` property の値は
  `resolveAssetUrl()` (`api/assets.ts`、既存のアセット解決を再利用) を通す

### `test1` での確認

`build_demo_window()` (Step5 で追加した関数) に以下を追加した:

- "main" Page: `UiImage::new("sample.png")` (width 20vw / height 15vw) を
  Link の上に配置。Link には `text("つぎへ")` /
  `text_size(vh(3.0))` / `text_weight(700)` / `text_color(Color::white())` を追加
- "second" Page: `.background_image("josei_20_b.png")` を追加
  (Page 自体に背景画像を敷く確認)

`biwa dev` + Playwright (headless Chromium) で確認:

- "main" ページに `UiImage` の画像 (グラデーション画像) と、
  白太字中央寄せの「つぎへ」ラベル付き Link が表示される
- クリックで "second" ページに切り替わると、Page 全体に
  `background_image` (別のキャラクター画像) が敷かれ、
  「もどる」ラベル付き Link が重なって表示される
  (canvas 側の背景 (`background-countryside.jpg`) と紛れないよう、
  あえて別のアセットを使って CSS 側の background_image だと視覚的に
  確認できるようにした)
- 「もどる」をクリックすると "main" に戻り、表示が正しく戻る
- コンソールエラー無し

`cargo test` (compiler workspace 全体) と `tsc --noEmit` (engine/nodejs) は
どちらもエラー無し。

### 次にやること (この節の時点)

- Step3: `Canvas`/`MessageArea` ポータル実装。このとき `Canvas`/`MessageWindow`
  (既存の `GameWindow` 内の型) との名前衝突が起きる見込みなので、
  `Window`/`Image` でやった判断 (使用範囲が狭い方をリネーム) を踏襲する。

## 11. すべての Element に `id: String` を追加 (完了)

`docs/ui-api.md` 62 行目付近に追記された設計 (Page が `canvas`/`message_area`
property で子の Canvas/MessageArea を id 参照する、`on_new_game()` へ
`GameWindow` を渡す、等) の前段として、まず「すべての Element が
`id: String` を持て、エンジン側が `String → ui_id` の map を正しく持つ」
ところまでを実装した。`GameWindow::new()` のホスト export や
`on_new_game(window: GameWindow)` への signature 変更、Page の
`canvas`/`message_area` property 自体はまだ手を付けていない (Step3 の範囲)。

### 実装

- **property kind**: `Id = 106` (文字列)。数値/文字列どちらの帯にも寄せず、
  既存の文字列 property の続き番号にした (すべての Element に共通する
  property であり、特定の Element 専用ではないという位置づけ)。
- **エンジン側 (`UIObjects.ts`)**:
  - `idsByName: Map<string, number>` を追加。`resolveId(name): number | undefined`
    で引ける (Step3/host export から使う想定)。
  - `id` property 設定時、同じ名前が別の Element に既に使われていれば
    ログを出しつつ上書き (取り違えに気づけるようにする程度で、禁止はしない)。
  - id を付け替えたとき (2 回目の設定) は古いキーを消してから新しいキーを張る。
  - `destroy()` (push_child で子を持てない Element に潰されたときなど) で
    map からも消す。
  - devtools・自動テストから見えるよう `dom.dataset.biwaId` にも反映した
    (`docs/ui-api.md` の仕様には無い、実装上のおまけ)。
- **std 側**: 8 つの Element 構造体 (`Window`/`Page`/`Box`/`Link`/`Horizontal`/
  `Vertical`/`HorizontalGrid`/`UiImage`) すべてに `id: Option[String]` を追加し、
  `.id(value: String) -> Self` ビルダーを生やした。発行は共通の
  `apply_id(id: Uint, name: Option[String])` ヘルパーに集約し、各
  `materialize()`/`materialize_into()`/`Window::show()` から 1 行で呼ぶ形にした
  (`apply_background`/`apply_text_properties` と同じ扱い)。

### ハマった点: エンジンの生成物埋め込み

`id` property を実装して `biwa dev` で確認したところ
`[biwa] unknown or non-string ui property: 106` が出た。原因は
**`biwa` (CLI) バイナリが `engine/nodejs` を `rust_embed` でビルド時に
埋め込んでおり、`cargo build` し直さない限り `.biwa_runtime/` へ
展開される同梱エンジンが更新されない**ため
(`cli/CLAUDE.md`: 「エンジン変更時は `biwa` を再ビルドしてから `biwa dev`
し直す」という運用が必要)。`cargo build -p biwa` → `biwa dev` の再起動で解消した。
以降 engine/nodejs を触ったら CLI の再ビルドが要ることを覚えておく。

### `test1` での確認

`build_demo_window()` に `.id(...)` を追加 (Window に `"demo_window"`,
`main_page` に `"main_page"`, thumbnail の `UiImage` に `"thumbnail"`,
"つぎへ" の Link に `"to_second_link"`)。`biwa dev` + Playwright で
`document.querySelectorAll("[data-biwa-id]")` を直接読み、4 つすべてが
意図した種類・意図したテキストを持つ DOM 要素に正しく付いていることを
確認した (`window→つぎへもどる全体`, `page→つぎへ`, `image→空`,
`link→つぎへ`)。コンソールエラー無し。クリック遷移 (main⇄second) も
壊れていないことを確認済み。

`cargo test` (compiler workspace 全体) はエラー無し。

### 次にやること

- Step3 本題: Page の `canvas`/`message_area` property (`sys_ui_set_property_with_string`
  で id 文字列を運ぶだけで済むはず)。
- `on_new_game()` の signature を `GameWindow` を受け取る形に変える
  (現状は引数無し)。ホスト側 (`worker.ts`) が scene 開始前に
  `resolveId()` で canvas/message_area の ui_id を引き、
  `GameWindow::new(canvas_id, message_area_id)` (下記 `[[host_export]]` で
  export される) を呼んで `on_new_game()` に渡す、という配線が要る。
  `[[host_export]]` 自体は実装済みなので、残るのは
  `std::game::ui::GameWindow::new` へ実際に属性を付ける作業と、
  ホスト側 (TS) の呼び出しコードだけ。

## 12. コンパイラに `[[host_export="<name>"]]` を追加 (完了)

`GameWindow::new()` をホストから直接呼べるようにする前段として、
「属性で指定した名前で、トップレベルの `fn` を生成物から直接呼べる形で
export する」汎用の仕組みをコンパイラに追加した。wasm では
`(export "<name>" (func $...))`、TypeScript では別名 export
(`export { <mangled> as <name> }`) を、`__biwa_entrypoint` /
`__biwa_on_new_game` (`biwac_scene::WellKnownSymbol`) と同じ考え方・
同じコード経路で出す。

### 新設した crate: `biwac_host_export`

`biwac_lang_item` (`[[lang="..."]]` の登録簿) と対になる、
`[[host_export="..."]]` の登録簿。`HashMap<ValDefId, String>` 相当で、
**export 名の重複だけを検証する** (同一定義への属性の重複は
`biwac_attribute` の `AttrError::DuplicatedAttribute` が別に防ぐ)。
`compiler/Cargo.toml` のワークスペースメンバに追加した。

### 変更したパイプライン (`[[native(arch=...)]]` ではなく `[[lang="..."]]` を模倣)

`[[native(arch=...)]]` は AST フィルタリング (`retain_for_target`) だけで
完結し、値が codegen まで残らない。一方 `host_export` は
`(ValDefId, export 名)` という事実が **単相化の roots** と
**codegen の export 文** の両方まで生き残る必要があるため、
`[[lang="..."]]` (→ `LangItemTable`) と同じ経路を辿らせた:

1. **`biwac_attribute`**: `KnownAttr::HostExport` を追加
   (`AttrShape::Value(String)`, 対象は `Target::Fn` のみ)。
   `check.rs` に `host_export_name()` アクセサを追加 (`lang_key` と同形)。
2. **`biwac_name_resolver`**: 新設した `resolving/host_export_collector.rs`
   が `lang_item_collector.rs` と同じ構造で AST を再走査し (def collection
   直後、DefId は AST ノードの `OnceCell` に入っている)、
   `HostExportTable` を組み立てる。`ResolveOutput` に
   `host_exports: HostExportTable` フィールドを追加した。
3. **`biwac_driver`**:
   - `monomorphize_program` の roots に host export された `ValDefId` を
     追加 (`well_known_scenes` からの roots に `.chain(...)` するだけ)。
     **これが無いと host_export された関数が到達性で消される** — 単体テストで
     直接確認済み (後述)。
   - `biwac_generator::arch::wasm::emit` / `arch::typescript::generate`
     に `&host_exports` を引数で渡す。
4. **`biwac_generator`**:
   - `wasm/emit.rs`: `well_known` の `OnNewGame` export のすぐ後に、
     `host_exports` を回して `MonoInstance` を `def_id` で引き、
     `(export "<name>" (func $...))` を書く。見つからなければ
     (roots に入れ忘れ等のコンパイラ側のバグでしか起きないはずだが)
     `WasmError::MissingInstance` で落とす。
   - `arch/typescript.rs`: `Main`/`OnNewGame` の別名 export ループのすぐ後に、
     `host_exports` の分だけ同じ `export_alias(...)` を積む
     (TypeScript は単相化も到達性除去もしないので roots は不要)。

### 現状の制約: 自パッケージ限定 (→ §13 で案 (a) により解消)

`host_export_collector` は自パッケージの AST しか見ない。つまり
**依存パッケージ (`std` 等) が `[[host_export="..."]]` を付けても、
それを使う側 (playable package) のビルドでは export されない**。
`[[lang="..."]]` は `.biwameta` に永続化して依存側が読み直す
(`DiskLangItem` のバイナリエンコード) が、host_export はそこまでの
汎用の需要がまだ無いと判断し、いったん見送った。

**したがって `GameWindow::new` (std 側で定義) を host_export したい場合、
今のままでは効かない。** 対応案:
(a) `.biwameta` に host_export も持たせて lang item と同じ経路に乗せる
(実装コストが大きい)、
(b) playable package (test1) 側に `GameWindow::new` を呼ぶだけの薄い
ラッパー関数を書き、そちらに `[[host_export]]` を付ける (今すぐ使える)。
どちらを選ぶかはユーザー判断。

### テスト

- `biwac_host_export`: `HostExportTable::insert` の単体テスト
  (別名同士は OK / 同名の重複はエラーになる / 同じ def_id への
  再登録は自分自身との衝突と誤検出しない)。
- `biwac_driver::wasm_output`: `compiler/assets/tests/test1/src/main.biwa`
  に、**scene main からもどこからも呼ばれていない**
  `[[host_export="host_export_demo"]] fn host_export_demo() -> Int`
  を追加し、`.wat` に `(export "host_export_demo"` が現れることを確認。
  「呼ばれていないのに現れる」ことそのものが roots 追加の回帰テストになっている。
- ワークスペース全体 (`cargo test`) はすべて green。
- TypeScript ターゲットは `library/std` 自体が既存の tier2 制限
  (`content.biwa` の trait 境界付きジェネリック、本セッションの Step5 で
  遭遇したものと同じ) に阻まれてこの環境では実機確認できなかった
  (host_export とは無関係の既存の問題)。コードは wasm 側と同じパターンを
  踏襲しており、`biwac_generator` 単体のビルドは通っている。

### 余談: ディスク枯渇

作業中に `/` の空き容量が尽き、リンカがクラッシュする事象が発生した。
原因は Rust の incremental compilation キャッシュ
(`compiler/target/debug/incremental` 等、長時間のセッションで肥大化) と、
Step5 で導入した Playwright の Chromium キャッシュ (`~/.cache/ms-playwright`)
だった。両方削除して 2.2GB 復旧した。再度ブラウザでの見た目確認が要る場合は
`npx playwright install --with-deps chromium` からやり直しになる。

## 13. `[[host_export]]` をパッケージ越しに効かせる (完了)

§12 の制約 (自パッケージ限定) を案 (a) で解消した。std 等の依存パッケージが
`[[host_export="..."]]` を付けた関数は、それを使う側 (playable package) の
ビルドで生成物から export され、単相化の roots にも入るので到達性で刈り取られない。

### `.biwameta` (フォーマットバージョン 8 → 9)

- **`DiskSymbolHeader` に `flags: u32` を追加** (12B → 16B)。
  ビット 0 が `SYMBOL_FLAG_HOST_EXPORT`。依存側はボディをデコードせず
  ヘッダの走査だけで host export を拾える。
- **`DiskFnData` に `host_export: DiskVec<DiskStringOffset>` を追加**。
  export 名で、`trait_of` と同じく 0 個か 1 個で `Option` を表す。
- フラグは `push_body` がボディ (`host_export` が空でないか) から導く。
  別々に渡して食い違うことが無いようにするため。
- SVH に export 名を混ぜた。付け外し・改名は依存側の生成物を変えるので、
  インタフェースの変更として依存側を建て直させる。
- 読み出しは `DepMetadata::host_exports(pkg_id) -> Result<Vec<(ValDefId, &str)>, _>`。
  フラグが立っているのに export 名が無ければ `InconsistentHostExport` で落とす。

### 依存側での取り込み

- `collect_host_exports` が `external_packages` (推移閉包すべて) を受け取り、
  依存 → 自パッケージの順に `HostExportTable` に登録する (lang item と同じ流れ)。
  以降の単相化 roots・wasm の export は表をそのまま使うので変更不要だった。
- export 名の重複はパッケージをまたいでも禁止 (生成物の export は 1 つの名前空間)。
  相手が依存側にある場合は位置を持たないので、note でそれを伝える。
- `HostExportTable::iter()` を export 名順にした。`HashMap` の順序のままだと
  roots と `.wat` の export の並びがビルドごとに揺れうるため。
- TypeScript: 依存の関数はこのモジュールに import されているとは限らないので、
  `export { <mangled> as <name> } from "./<pkg>.ts"` で定義元から再 export する。

### テスト

- `greeter` フィクスチャ (ライブラリ) に、test1 から呼ばれない
  `[[host_export="greeter_host_export_demo"]] fn greeter_host_export_demo()` を追加。
  `wasm_output` で test1 の `.wat` に `(export "greeter_host_export_demo"` が出ることを確認
  (依存からの取り込みを止めるとこのテストが落ちることも確認済み)。
- `host_export_is_recorded_in_metadata`: `greeter.biwameta` から
  `host_exports()` で export 名と greeter の DefId が読め、SVH が再計算と一致すること。
- `biwac_host_export`: `iter()` が export 名順であること。
- `biwac_generator`: `export_alias` の自パッケージ版と再 export 版の出力。
- TypeScript の通しの確認は std の既存の tier2 制限 (`content.biwa`) により今回も不可。

### 残り

- `GameWindow::new` は **関連関数** なので、`host_export` の付与対象
  (`Target::Fn` = トップレベル `fn` のみ) に入らない。std にトップレベルの
  ラッパー関数を置いて属性を付けるか、付与対象を関連関数へ広げる必要がある。
