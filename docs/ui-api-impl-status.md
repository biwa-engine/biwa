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

### 次にやること

- Step5: `test1` で実際に `Window`/`Page`/`Link` を組み立てて `biwa dev` し、
  ブラウザで表示・クリック遷移を確認する。
- Step2: `Image`, text 系 property, `background_image`。
- Step3: `Canvas`/`MessageArea` ポータル実装 (`Canvas`/`MessageWindow` の
  リネームも一緒に行う)。
