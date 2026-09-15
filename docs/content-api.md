# Content API の実装方針

`docs/message-window-content.md` の Content API を、
コンパイラ・std・エンジンの 3 層にわたって実装するための方針。

trait (`docs/trait.md`) と enum (`docs/enum-and-match.md`) が入ったことで、
`Content` を enum に、`Into[Content]` を trait にする形が書けるようになった。

**まだ決まっていないことがある。** 「決めたいこと」の節を先に読んでほしい。

## いま何がどこまであるか

以下は**着手前**の状態である。段 1 〜 5 で変わったところは
「段 N を実装して分かったこと」にまとめてある。

| 層                   | 状態                                                                                                                           |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| ノベルの生テキスト行 | `NovelStmt::NovelWrite(String)` として 1 行 1 文になる。`$` は未対応                                                           |
| 待ち `>>`            | `NovelStmt::NovelWait` になる                                                                                                  |
| lang item            | `write(msg: String)` と `wait()` の 2 つ                                                                                       |
| MIR                  | `Stmt::NovelWrite` / `NovelWait` が lang item の呼び出しに落ちる                                                               |
| std                  | `game/content.biwa` に `Content` / `TextContent` / `Into[Content]` の骨格。`content_push` は書かれているがコメントアウト混じり |
| std                  | `game/window.biwa` の `MessageWindow::push_content` は中身が TODO                                                              |
| std                  | `game/config.biwa` に `Config` があるが、`Game` に繋がっていない                                                               |
| エンジン             | `sys_write(text)` が `TextBox.appendText` を呼ぶだけ。装飾も文字送りも無い                                                     |
| エンジン             | `waitForClick()` がクリックで `box.clear()` して解決する                                                                       |

つまり「生テキストを流し込む」ところまでは通っていて、
**装飾・メディア・文字送り・設定がまるごと無い**。

## 全体像

```
scene 本文
  "Hello! 私の名前は$blue(bold(italic("琵琶")))です。>>"
        │
        │ ① novel parser: 生テキストと埋め込み式に分ける
        ▼
  NovelStmt::ContentPush(Text("Hello! 私の名前は"))
  NovelStmt::ContentPush(Expr(blue(bold(italic("琵琶")))))
  NovelStmt::ContentPush(Text("です。\n"))
  NovelStmt::ContentFlushAndWait
        │
        │ ② mir_build: lang item の呼び出しに落とす
        ▼
  content_push(g, <..>)
  content_flush_and_wait(g)
        │
        │ ③ std: Content に正規化して Window API に渡す
        ▼
  game.window.message_window.push_content(c)
        │
        │ ④ syscall
        ▼
  sys_content_push_text(text, speed, size_unit, size_value, weight, color)
  sys_content_flush()
  sys_wait()
  sys_content_clear()
```

① と ② がコンパイラ、③ が std、④ がエンジンである。

## 1. 構文: `$` 埋め込み式

### 文法

`docs/message-window-content.md` の EBNF をそのまま採る。

```ebnf
<embeded-expression> ::= `$` `(` <expression> `)`
  | `$` <identifier> ( <argument-list> | <member-access-or-method-calling>* <method-calling> )
```

**どちらの形も必ず `)` で終わる。** これは重要な性質で、
自由テキストの中で式の範囲を決められるのはこの規則のおかげである。
`$player.hp` のような裸のメンバアクセスは書けず、`$(player.hp)` と括る。

### 読み方

いまのノベルパーサは、生テキスト行を `line_str()` で 1 本の文字列として取る。
`$` を入れるには、行の中を走査して**生テキストの断片と埋め込み式に切り分ける**。

`NovelLineHandler` は `begin_idx` / `end_idx` を持ち `proceed_to()` で進められるので、
式の部分だけを切り出して既存の式パーサに渡せる。

1. 行頭から `$` を探す。手前が空でなければ生テキストの断片として積む
2. `$` の直後から **式の範囲をバイト単位で先に測る**
   - `(` で始まるなら、対応する `)` まで。そのあとに `.` が続くなら継続する
   - 識別子で始まるなら、`.` と識別子の連なりを追い、`(` が来たら対応する `)` まで
   - **文字列リテラルの中の `(` `)` は数えない**。ここを間違えると
     `$foo(")")` で範囲が壊れる
3. `end_idx` をその範囲に狭めて `consume_expression()` を呼ぶ
4. 元の `end_idx` に戻し、`)` の次から生テキストの走査を続ける

先に範囲を測るのは、式パーサに「どこで止まるか」を教える手段が無いからである。
`$blue("琵琶")です。` を式パーサにそのまま食わせると、
`です。` をコードとして字句解析しようとして落ちる。

### `$` のエスケープは `\$`

生テキストに `$` をそのまま書きたい場合は `\$` と書く。

文字列リテラルのエスケープも将来 `\` で行うことにしてあるので、
記法を揃えておく。`\` は生テキストの中でもエスケープの導入記号になる
(いまは `\$` だけを解釈し、それ以外の `\x` はそのまま `\x` として出す)。

### `#` 行・`@` 行との関係

`$` が効くのは**生ノベルテキスト行の中だけ**である。
`#` 行はコードなので `$` は要らず、`@` (キャラ行) は今回触らない。

## 2. AST / HIR: novel statement の形

### いまの形

```rust
pub enum NovelStmt {
    ...
    NovelWrite(NovelMessage),   // msg: String
    NovelWait(NovelWait),
}
```

### 変える形

```rust
pub enum NovelStmt {
    ...
    /// Message Window に積む内容。生テキストか埋め込み式。
    ContentPush(NovelContent),
    /// `>>`。積んだ内容をまとめて出し、クリックを待つ。
    ContentFlushAndWait(NovelFlush),
}

pub enum NovelContent {
    /// 行に直接書かれたテキスト。
    Text { text: String, span: Span },
    /// `$...` の埋め込み式。
    Expr { expr: Exprs, span: Span },
}
```

HIR 側も同じ形にする (`Stmt::ContentPush` / `Stmt::ContentFlushAndWait`)。
`Stmt::NovelWrite` / `NovelWait` は置き換えで消える。

`ContentPush::Expr` の中身は**普通の式**なので、
名前解決・型推論・MIR 構築は既存の経路にそのまま乗る。
`#` 行の式と扱いが変わらない。

### `g` はどこから来るか

`content_push(g, ..)` の第 1 引数はゲームである。

scene の規約 (`biwac_scene`) が
「すべての scene は lang item `game` のみを引数に取り `game` を返す」
を既に強制しているので、**scene の唯一の引数**をそのまま渡せばよい。
`#endscene g` のようにユーザが名前で書く必要は無い。

MIR 構築の時点では引数は `VarId` で引けるので、
`Operand::Place(local_of(arg0))` を作って渡す。

### flush はいつ走るか

`>>` で走る。それ以外に、**scene が終わるときにも積み残しを出す**。
`#endscene g` と、本文が `>>` で終わらずに scene が終わる場合である
(決めたこと 2)。

## 3. std: `Content` と `Into[Content]`

### `Content`

既に書かれているものをほぼそのまま使う。

```biwa
enum Content {
  Text(TextContent),
  // 将来: Image(ImageContent),
}

struct TextContent {
  text: String,
  speed: ContentSpeedLevel,   // 設定値の何倍か
  size: ContentSizeLevel,     // 設定値の何倍か
  weight: TextWeight,
  color: ContentColor,        // Configured | Value(Color)
}
```

### `Into[Content]` の当て方

`impl String: Into[Content]` は書かれている。
`$(player.hp)` のような数値も通したいので `impl Int: Into[Content]` /
`impl Float: Into[Content]` が要る (プリミティブへの impl は std でだけ許される)。

ここで **2 つの制約にぶつかる**。

#### 制約 A: 1 つの型に `into` は 1 つしか置けない

biwa は「1 つの型にぶら下がる関連名は一意」という規則を持つ
(`[T as Trait]::foo()` の曖昧さ解消構文がまだ無いため)。

したがって `impl String: Into[Content]` と `impl String: Into[Foo]` は**共存できない**。
`Into[T]` は当面「その型にとって唯一の変換先」を表すものになる。

#### 制約 B: 制限つきジェネリクスは TypeScript で出力できない

```biwa
fn content_push[C: Into[Content], T, U](game: Game[T, U], content: C) {
  let c: Content = content.into();   // ← 制限越しのメソッド呼び出し
  ...
}
```

これは第 2 段で通るようになったが、**wasm だけ**である。
TypeScript はジェネリクスを保ったまま出力するので実装を選べず、
driver が codegen の手前で弾く。しかも弾くのはパッケージ単位なので、
**std がこれを持つと TypeScript の出力が丸ごと止まる**。

これを受け入れる (決めたこと 1)。TypeScript は
`docs/trait.md` の第 3 段が入るまで建たなくなる。

## 4. lang item

増やすもの。

| key                      | 種別 | 形                                                     |
| ------------------------ | ---- | ------------------------------------------------------ |
| `content`                | 型   | `enum Content`                                         |
| `content_push`           | 関数 | `fn content_push[T, U](game: Game[T, U], content: ..)` |
| `content_flush_and_wait` | 関数 | `fn content_flush_and_wait[T, U](game: Game[T, U])`    |

`write` / `wait` は `content_push` / `content_flush_and_wait` に置き換わって消える。

### `into` は lang item にしない

コンパイラは `content_push(g, <expr>)` を組み立てるだけで、
`.into()` は std の中で呼ばれる (決めたこと 1)。
したがって `Into` trait を lang item にする必要は無く、
`LangItemKind` に `Trait` を足す必要も無い。

`content_push` のシグニチャは
`fn content_push[C: Into[Content], T, U](game: Game[T, U], content: C)` なので、
lang item の検証はジェネリック引数を 3 つ要求する。

## 5. エンジンの syscall

設定は std が解決して**絶対値**にしてから渡す (決めたこと 3)。
エンジンは受け取った値をそのまま使う。

```
// text を 1 つ積む。まだ描画しない。
sys_content_push_text(
  text: String,
  speed: f32,       // 秒間何文字出るか
  size_unit: u32,   // 0 = vw, 1 = vh
  size_value: f32,  // 単位に対する値
  weight: u32,      // 100 ~ 900
  r: u32, g: u32, b: u32, a: u32,  // 各 0 ~ 255
);
// italic などの装飾はまだ運ばない (「装飾をどう運ぶか」を参照)。
//
// 色を 1 語 (0xRRGGBBAA) に詰めないのは、biwa にビット演算が無く、
// 算術で詰めると `a` が i32 の符号を跨ぐためである (段 3 の項を参照)。

// 将来
sys_content_push_image(path: String, size_unit: u32, size_value: f32);

// 積んだものをまとめて出し始める。
sys_content_flush();

// 枠を空にする。
//
// エンジンは自分の判断でクリアしない。いつ消えるかを決めるのは std である
// (決めたこと 2)。
sys_content_clear();

// クリックを待つ (既存)。
sys_wait();
```

`ContentColor::Configured` は std の側で設定値に解決されるので、
syscall には常に具体の色が渡る。フラグは要らない。

数値から `String` を作るのも syscall である
(`sys_int_to_string` / `sys_float_to_string`)。
wasm の `String` は externref で、実体はホスト側にしか無い。

`Size` を (単位, 値) の 2 つに割るのは、
wasm の引数に enum をそのまま渡せないためである。

wasm の import 宣言は `library/std/src/game/base_engine.biwa` の
`[[native(arch="wasm")]]` ブロックに足す。
TypeScript 側は `Sys` の番号を足して `BiwaSyscall` の記述子を返す
(ただし決めたこと 1 のとおり、TypeScript は当面建たない)。

## 6. エンジンの実装

### `TextBox` の作り直し

いまの `TextBox` は `<p>` 1 つに `textContent` を足すだけで、
装飾も文字送りも表現できない。作り直しが要る。

- 積まれた content を `{ text, color, size, weight, speed }` の**断片の列**として保持する
- `flush()` で断片の列を `<span>` に展開して DOM に入れ、
  **文字送りのアニメーションを開始**する
- 文字送りは断片ごとに速度が違いうるので、
  「いま何文字目まで見せているか」を断片の列に対する 1 本のカーソルで持つ
- クリックが来たとき、文字送り中なら**まず全部出す** (skip)。
  もう一度のクリックで `sys_wait` が解決する
  (canvas の `objects.skipSync()` と同じ形)

### 文字送りと `sys_wait` の関係

いまの `waitForClick` は「クリック → `box.clear()` → resolve」である。
**クリアをここから外す** (決めたこと 2)。

```
flush()  → 文字送り開始
click    → 文字送り中なら全部出して終わり (resolve しない)
click    → resolve する
         → std が sys_content_clear() を呼ぶ
```

エンジンは「いつ消えるか」を決めない。決めるのは std である。

### 設定

`Config` は `Game` が持つ (決めたこと 3) ので、エンジンは設定を知らない。
渡ってくる速度・サイズ・色はすべて解決済みの絶対値である。

`UserConfig` を実行時に変えるのは Biwa 側の値の書き換えになる。
以降の `content_push` から新しい値が渡るので、エンジン側の追従は要らない。

## 決めたこと

### 1. `Into[Content]` は制限つきで書く (TypeScript は止まる)

`docs/message-window-content.md` のとおりにする。

```biwa
[[lang="content_push"]]
fn content_push[C: Into[Content], T, U](game: Game[T, U], content: C) {
  let c: Content = content.into();
  game.window.message_window.push_content(c);
}
```

コンパイラは `content_push(g, <expr>)` を組み立てるだけでよく、
`.into()` を合成しない。したがって

- `Into` を lang item にする必要は**無い**
- `LangItemKind` に `Trait` を足す必要も**無い**
- 使う側のモジュールで `import std::convert::Into;` は**要らない**。
  制限の検査は「その型に impl があるか」だけを見ており、
  スコープにあるかは問わないためである
  (スコープを問うのは trait 越しに**名前を引く**ときだけ)

代わりに、**std がこれを持った時点で TypeScript ターゲットが止まる**。
制限つきジェネリクスの呼び出しは第 2 段では wasm でしか出せず、
driver がパッケージ単位で弾くためである。std が弾かれると、
std に依存するすべてのパッケージも建たない。

**TypeScript の復活は `docs/trait.md` の第 3 段 (witness 渡し) が入るまで待つ。**

### 2. `>>` は 1 ページ (クリックでクリアする)

```
こんにちは。>>          ← 積んだものを出す → クリック → 枠をクリア
私の名前は琵琶です。>>  ← 新しいページとして出る
```

**クリアはエンジンが勝手に行わず、std が明示的に呼ぶ。**

```biwa
[[lang="content_flush_and_wait"]]
fn content_flush_and_wait[T, U](game: Game[T, U]) {
  game.window.message_window.flush();   // sys_content_flush()
  game.window.wait();                   // sys_wait()
  game.window.message_window.clear();   // sys_content_clear()
}
```

いまの `waitForClick` はクリックが来たときに自分で枠をクリアしているが、
**それをやめて `sys_content_clear()` に切り出す**。

こうしておくと、将来「待つがクリアしない」API を足したくなったときに、
変更がコンパイラと std に閉じる。エンジンには何も足さなくてよい。

```biwa
// 将来。エンジンは変わらない。
fn content_flush_and_wait_keep[T, U](game: Game[T, U]) {
  game.window.message_window.flush();
  game.window.wait();
  // clear を呼ばない
}
```

文字送りが入るのでクリックは 2 段階になる。

```
flush()  → 文字送り開始
click    → 文字送り中なら全部出して終わり (まだ resolve しない)
click    → resolve する (クリアはしない。std が次に呼ぶ)
```

`>>` を書かずに scene が終わった場合も、積み残しを出してから終わる。

### 3. `Config` は `Game` が持つ

```biwa
[[lang="game"]]
struct Game[C, S] {
  name: String,
  characters: C,
  states: S,
  window: Window,
  config: Config,   // 追加
}
```

std が設定値を読んで**絶対値に直してから** syscall に渡す。
エンジンは渡された値をそのまま使い、設定を持たない。

```biwa
// TextContent -> syscall の引数
let speed = config.user.content_speed.text_per_sec * content.speed.mul;
let size  = config.user.content_size.mul(content.size.mul);
let color = match content.color {
  ContentColor::Configured => config.dev.default_text_color,
  ContentColor::Value(c) => c,
};
```

`Size` は `Vw` / `Vh` の enum なので、syscall には (単位, 値) の 2 つで渡す。

### 4. 今回の範囲

1 〜 4 をすべてやる。

- `$` 埋め込み式のパース
- `NovelStmt` の置き換えと lang item の差し替え
- std の `Content` 完成と syscall 発行
- エンジンの `TextBox` 作り直し + 文字送り

装飾 API (`red` / `bold` / ...) と画像 content は次の段に回す。

### 5. `Game` の初期化は `on_new_game()` が行う

playable package の `main.biwa` に、次の関数を**必ず定義する**。

```biwa
fn on_new_game() -> MyGame {
  Game {
    name = "デモ",
    characters = MyGameCharacters { .. },
    states = MyGameState { .. },
    window = Window::new(),
    config = Config::new(DeveloperConfig::new(..)),
  }
}
```

エンジンはゲーム開始時にこれを呼び、返ってきた値を `scene main` に渡す。

- 引数は取らない。戻り値は lang item `game` (`Game[_, _]`) でなければならない
- `main.biwa` (playable package のルートモジュール) に無ければエラー
- `scene main` と同じく「ランタイムが名前を知っているシンボル」なので、
  `biwac_scene` の規約に足す。いまの `WellKnownScene` は scene 専用なので、
  **関数も扱えるように一般化する**
- 生成物には `__biwa_on_new_game` として export する
  (`__biwa_entrypoint` と同じ流儀)

これで `config` / `characters` / `states` の初期化がすべてゲーム側に寄り、
エンジンが `Game` を組み立てる必要が無くなる。
wasm では `Game` が WasmGC の struct でホストから組めないので、
そもそもこの形しか成り立たない。

エンジン側の変更:

- TypeScript: `createInitialGame()` を捨て、`__biwa_on_new_game()` を呼ぶ
- wasm: `__biwa_entrypoint(null)` をやめ、
  `__biwa_on_new_game()` の戻り値をそのまま渡す

## 装飾をどう運ぶか (今回は見送り)

元の案では `sys_content_push_text` に `flags: u32` を持たせ、
italic や影付けをビットで表していた。**今回は `flags` も入れない。**

理由は、装飾が 2 種類に割れて、`flags` が効くのは片方だけだからである。

| 種類               | 例                                                                       | 形                  |
| ------------------ | ------------------------------------------------------------------------ | ------------------- |
| 真偽値だけのもの   | italic, 下線, 打ち消し線                                                 | ビット 1 つで足りる |
| **引数を持つもの** | 影 (色・ずれ・ぼかし)、縁取り (色・太さ)、ルビ (文字列)、リンク (行き先) | ビットでは表せない  |

ノベルゲームで実際に欲しくなるのは後者に寄っていて、
特に**縁取り**は画像の上に文字を置く都合でほぼ必須である。
つまり「`flags` では表せないもの」がすぐ来る。
そこを設計しないまま `flags` だけ先に固めると、
あとで 2 通りの運び方が並ぶことになる。

引数を持つ装飾まで含めた形は、必要になったときにまとめて決める。

### そのときの選択肢 (メモ)

- **ワイヤは `flags`、Biwa 側は `Vec[TextDecoration]`**。
  真偽値はビットに畳み、引数を持つものは syscall の引数を足して運ぶ。
  Biwa 側の enum は std に閉じているので増やすのが安い
- **属性を別の syscall で積む** (`sys_content_set_attr(kind, a, b, c)` を
  push の前に並べる)。引数を持つ装飾まで一様に扱えるが、
  「次の push に効く」という状態を持つ約束になり、syscall が増える
- **content をハンドルとして組み立てる**
  (`h = sys_content_new_text(..)`, `sys_content_set_shadow(h, ..)`, `sys_content_push(h)`)。
  もっとも拡張に強いが、ハンドルの寿命の管理が要る

## 段階分け (案)

| 段  | 内容                                                                                    |
| --- | --------------------------------------------------------------------------------------- |
| 1   | `$` 埋め込み式のパース (AST まで)。既存の `write` にそのまま繋いで動作確認              |
| 2   | `NovelStmt` を `ContentPush` / `ContentFlushAndWait` に置き換え、lang item を差し替える |
| 3   | std の `Content` を完成させ、`MessageWindow` から syscall を呼ぶ                        |
| 4   | エンジン: `TextBox` を断片の列 + 文字送りに作り直す                                     |
| 5   | 装飾 API (`red` / `bold` / `sized` / `paced`)。`italic` は syscall が運べないので除く   |
| 6   | 画像 content — 次の段                                                                   |

1 と 2 はコンパイラに閉じ、3 以降で std とエンジンが同時に動く。

**3 に入る前に「`Game` を誰が作るか」を決める必要がある。**
→ 決めたこと 5 のとおり `on_new_game()` にした。実装は段 2 に前倒しした
(理由は後述の「`on_new_game()` は段 2 に前倒しした」)。

## 段 1 を実装して分かったこと

### `>>` の前後は地続きのテキストとして扱う

`です。>>` と改行は `>>` を挟んで別の範囲になるが、
出力としては 1 つのテキストである。範囲ごとに切り出すと
`write("です。")` と `write("\n")` の 2 回になってしまうので、
範囲をまたいでテキストを溜め、`$` の位置でだけ区切る。

### 式の範囲は先に測ってから行を狭める

`NovelSourceStream` のトークン読み出しは `current_line` の
`[begin_idx, end_idx)` からしか読まない。
そこで、測った範囲でこのハンドラを一時的に差し替えてから
`consume_expression()` を呼ぶ。式パーサ自体には手を入れていない。

差し替えのあいだは `peeked` も退避する。戻し忘れると
式の直後のトークンが地の文の走査に漏れる。

### 段 1 の時点で使える形

`write(msg: String)` がそのまま埋め込み式の展開先になるので、
**`String` を返す式なら今の段階で動く**。

```biwa
こんにちは $(greet_name()) さん。>>
メソッドチェーンも $("a".concat("b")) 使える。
```

`Int` を返す式は `Expected String, but found Int` になる。
これは段 3 で `Into[Content]` が入ると通るようになる。

## 段 2 を実装して分かったこと

### novel statement は「普通の呼び出し」に落ちる

`NovelWrite` / `NovelWriteExpr` / `NovelWait` という
「中身が別々な 3 つの文」を `Stmt::NovelSyscall` 1 つに畳んだ。
中に入るのは `content_push(g, ..)` / `content_flush_and_wait(g)` という
**ごく普通の呼び出し式**である。

```rust
/// novel statement が展開された syscall の発行。
///
/// 中身は普通の呼び出し式である。それでも statement として残しているのは、
/// **どこで中断しうるか**をコード生成が知る必要があるからである。
pub struct NovelSyscallStmt {
    pub call: Expr,
    pub span: Span,
}
```

呼び出し式にしたことで、型推論・MIR 構築・単相化は
これを特別扱いしなくてよくなった。
制限つきジェネリクス (`content_push[C: Into[Content], T, U]`) の検査も
普通の呼び出しとして走る。

消えたのは以下である。

- `biwac_type_inferrer` の `check_novel_call` (lang item のシグネチャを手で検査していた)
- `biwac_mir_build` の `lower_syscall` と `lang_items` フィールド
- `biwac_generator` の TypeScript 側 `lang_item_fn_mangled` と `lang_items`

**残したのは statement という枠だけである。** 畳んで `Stmt::Expr` にしなかったのは、
TypeScript バックエンドが `yield` を置く位置を、
「ここは中断しうる」という情報から決めているからである。
式に落とすとその情報が消える。

### `g` は lowering の時点では「無いかもしれない」

`content_push(g, ..)` の `g` は scene の唯一の引数である。
`biwac_scene` がその規約を検査するが、**それが走るのは lowering の後**なので、
lowering の時点では引数が無い scene もありうる。

```rust
let game_var = args.args.first().and_then(|a| a.var_id.get().copied());
```

無ければ novel statement を落とす。規約違反そのものは scene の検査が報告するので、
ここで別のエラーを足すと同じ誤りが 2 回出る。

### `on_new_game()` は段 2 に前倒しした

`content_push` は `game.window.message_window` を辿る。
それまでの wasm ランタイムは `__biwa_entrypoint(null)` と呼んでいたので、
段 2 の時点で**必ず trap するようになった**。
`Game` を作る口が無いと動作確認ができないので、決めたこと 5 をここで実装した。

`biwac_scene` は「scene の表」から「ランタイムが直接呼ぶシンボルの表」に広げた。

```rust
pub enum WellKnownKind {
    /// `(Game[..]) -> Game[..]`。generator として出力される。
    Scene,
    /// `() -> Game[..]`。
    Fn,
}

well_known_symbol_table!(
    Main,      "main",        WellKnownKind::Scene, SceneRequirement::RequiredInPlayable;
    OnNewGame, "on_new_game", WellKnownKind::Fn,    SceneRequirement::RequiredInPlayable;
);
```

種別を持たせたのは、**シグネチャの検査と診断文を分けるため**である。
`on_new_game` に引数を書いたときに
「scene は `Game` を 1 つ取り `Game` を返す」と言われても意味が通らない。

```
Error: `on_new_game` must take no argument and return a `Game`,
       but it takes 1 argument(s) instead of 0
Error: the runtime calls `on_new_game` directly, so it must be a function
```

両バックエンドが `__biwa_on_new_game` という固定名で export する。
エンジン側は `createInitialGame()` を捨て、
`BiwaBackend` に `onNewGame` を持たせてこれを呼ぶ形にした。
wasm の Worker は `__biwa_on_new_game()` の戻り値を
そのまま `__biwa_entrypoint()` に渡す (JS からは中身を見ない)。

### 段 2 の時点で使える形

出力は段 1 と変わらない。経路だけが `write` から
`content_push` → `MessageWindow::push_content` → `write` に変わっている。
`MessageWindow::push_content` は `match content { Content::Text(t) => write(t.text) }` で、
**装飾を落として素のテキストだけを出している**。ここが段 3 で syscall に変わる。

`content_flush_and_wait` も今は `game.window.wait()` だけを呼ぶ。
flush と clear は段 3 で足す (決めたこと 2 の順序: `flush(); wait(); clear();`)。

### TypeScript の `yield` は作り直しが要る

TypeScript バックエンドは `Stmt::NovelSyscall` を `yield <call>` に落としているが、
`content_push` は syscall 記述子を返さないただの std 関数なので、
この形はもう正しくない。中断する syscall (`sys_wait`) は
`content_flush_and_wait` の**奥**にあり、statement の位置には無い。

TypeScript は tier 2 で、trait の段 2 以降そもそも建たない
(制限つきジェネリクスを含むため) ので今回は直していない。
`statement.rs` に NOTE を置いてある。

## 段 3 を実装して分かったこと

### 設定を潰す場所は `content`、絶対値を運ぶのは `window`

resolve の置き場所には 2 案あった。`MessageWindow::push_content(content, config)` と、
`content_push` の中で潰してから Window に渡す形である。後者にした。

```biwa
[[lang="content_push"]]
fn content_push[C: Into[Content], T, U](game: Game[T, U], content: C) {
  let c: Content = content.into();
  match c {
    Content::Text(text) => {
      game.window.message_window.push_text(
        text.text,
        resolved_speed(game.config, text.speed),
        resolved_size(game.config, text.size),
        text.weight,
        resolved_color(game.config, text.color),
      );
    }
  }
}
```

こうすると **Window API は設定という概念を持たない**。
`Window` はサードパーティも叩く低レベル層なので、
そこに `Config` を通すと「設定を見る層」が 2 つになる。
`Content` (相対値) を知るのは `std::game::content` だけ、
`Window` から下は絶対値だけ、という切り分けにした。

### 色は 4 引数で渡す

計画では `color: u32 // 0xRRGGBBAA` の 1 語だった。4 つに割った。

- **biwa にビット演算が無い** (`BinOperator` は算術と比較だけ)。
  `r * 16777216 + ...` と書くしか無い
- そう書くと `a` の最上位ビットが i32 の符号を跨ぐ。
  ホスト側には負の数が届く

`Size` を (単位, 値) に割ったのと同じ理由 — wasm の引数に載る形まで
std が落としてから渡す — なので、規則としては一貫している。

### `Int` / `Float` にも `Into[Content]` を当てた

`$(player.hp)` のような式を通すためである。孤児規則は
「impl 対象か trait 自身のいずれかが自分の package のもの」なので、
`Into` を持つ std でだけプリミティブへの impl が書ける。

`String` を数値から作るには syscall が要る
(wasm の `String` は externref で、実体はホスト側にしか無い)。
`sys_int_to_string` / `sys_float_to_string` を `local` として足した
(エンジンには用が無く Worker 内で完結する。`sys_string_concat` と同じ区分)。

f32 をそのまま `String()` に掛けると `0.30000001192092896` のような桁が出るので、
`toPrecision(9)` で f32 が表せる精度に丸めてある。

### `Game::new()` を通す形にした

`Game` に `config` が増えたので、構造体リテラルで組み立てると
ゲーム側が `Config` まで書くことになる。
`Game::new(name, characters, states)` が `Config::default()` を入れる形にし、
設定を変えたいゲームは `Game::with_config()` を呼ぶ。

```biwa
fn on_new_game() -> MyGame {
  std::game::Game::new("test1", MyGameCharacters {}, MyGameState {})
}
```

### `Self::assoc()` は名前解決が未対応だった

`Game::new()` の中身を `Self::with_config(..)` と書いたら落ちた。

```
compiler bug: path was not resolved before lowering:
  Path { abs_header: Some(SelfTyp(..)), segments: [PathSegment { .. }] }
```

`ImplResolveCtx::resolve_path` は `Self` ヘッダの `resolved_id` は埋めるが、
**その後ろのセグメントを解決していない**。
`Self` 単体 (`-> Self`、`Self { .. }`) は segments が空なので通っていて、
これまで誰も `Self::` を書いていなかったので露出していなかった。

いまは具体の型名 (`Game::with_config(..)`、`Config::new(..)`) で回避してある。
Content API とは別の話なので直していない。

### 枠をクリアするのは std である

```
flush()  → 出し始める
wait()   → クリックを待つ
clear()  → 枠を空にする
```

エンジンの `waitForClick()` から `box.clear()` を外した。
「待つが消さない」API を将来足すときに、
変更がコンパイラと std に閉じるようにするための切り分けである (決めたこと 2)。

wasm 経路では `sys_content_flush` を `FLUSH_AFTER_CAST` に入れる必要は無い。
直後の `sys_wait` が `call` で、`call` は溜めてある cast を必ず先に流すからである。
flush だけして待たない API が出てきたら足すこと。

### 段 3 の時点で出ているもの

`TextBox` は断片の列を持つようになり、
色・大きさ・太さが `<span>` の style として実際に効く。
**文字送りはまだ無い** (`flush()` で一度に全部出る)。段 4 で入る。

wasm の実行を覗くと、設定が解決された絶対値が届いているのが見える。

```
push "Hello, Biwa World!\n" speed=30 size=vh(3) weight=400 color=255,255,255,255
push "こんにちは、Biwaの世界。\n" speed=30 size=vh(3) weight=400 color=255,255,255,255
flush
wait
clear
```

`$native_add(40, 2)` は `"42"`、`$float_demo()` は `"3.25"` として押される。

## 段 4 を実装して分かったこと

### 出ていない文字は「消す」のではなく「隠す」

素直に書くと `textContent` を 1 文字ずつ伸ばすことになるが、これは**行がずれる**。
文字が増えるたびに折り返しの位置が変わるので、
最後の単語が次の行へ落ちるたびに既に出ている文字まで動く。

断片は最初から全文を DOM に入れ、まだ出ていない分を
`visibility: hidden` で隠す形にした。場所は取るので折り返しは最初から確定していて、
送りの途中で 1 文字も動かない。

```html
<span style="font-size:3vh; font-weight:400; color:rgba(255,255,255,1)"
  >見えている分<span style="visibility:hidden">まだ出ていない分</span></span
>
```

進めるのは `Text` ノードの `data` の差し替えだけなので、
要素を作り直さずに済む。

### カーソルは 1 本、端数は秒に戻して持ち越す

断片ごとに速度が違うので、「経過時間 → 何文字目」は一次式にならない。
列に対する 1 本のカーソルと、1 文字に満たない端数で持つ。

断片を出し切ってもフレームの時間が余ることがある。
このとき**端数を文字数のまま次の断片へ持ち越してはいけない**。
次の断片は速度が違うので、文字数の意味が変わる。
出し切った断片の速度で秒に戻してから渡す。

```ts
if (step < remaining) {
  seconds = 0;                        // まだ途中。時間を使い切った
} else {
  seconds = this.carry / fragment.speed;  // 余りを秒に戻す
  this.carry = 0;
  this.cursor += 1;
}
```

これを忘れると、速い断片が続くときに送りが目に見えて遅くなる。

時間を使わない断片 (空、出し切った、速度 0) は**時間の有無を見る前**に片付ける。
そうしないと、速度 0 の断片が「次にフレームが来るまで出ない」ことになる。

### クリック 1 回で進行中のものを全部畳む

飛ばせるものが 2 つになった。文字送りと sync 印の演出である。
順に消費させると、両方走っているときに 3 回クリックが要る。
1 回のクリックで両方を畳むことにした。

```ts
const skippedText = box.skip();
const skippedSync = objects.skipSync();
if (skippedText || skippedSync) return;   // 短絡させない
```

利用者から見た規則は「進行中のものがあれば 1 回目で畳み、次で進む」で、
段 3 までと変わらない。

### Ticker のコールバックは増やさない

文字送りも `main.ts` の唯一の Ticker コールバックから駆動する。
倍率は `objects.timeScale` を借りる。

```ts
renderer.app.ticker.add((ticker) => {
  objects.update(ticker.deltaMS);
  messageBox.update(ticker.deltaMS * objects.timeScale);
});
```

ポーズ・オート・スキップはいずれ時間の倍率で効かせるので、
文字送りだけ別の時計で動いていると 1 箇所では止められなくなる
(`engine/nodejs/CLAUDE.md` の規約)。

### 段 4 の時点でできていること

`docs/content-api.md` の 1 〜 4 が繋がった。scene の生テキストと `$` の埋め込み式が
`Content` になり、設定が解決され、syscall を渡って、速度どおりに 1 文字ずつ出る。

残っているのは装飾 API (`red` / `bold` / `italic`) と画像 content である。
`TextContent` は既に色・大きさ・太さ・速度を持ち、
エンジンまで通っているので、次の段で足すのは
**`Content` を受け取って `Content` を返す関数**だけになる。

## 段 5 を実装して分かったこと

### 装飾はコンパイラにも std の中核にも触らない

段 3 で `TextContent` が色・大きさ・太さ・速度を持ち、
段 4 でそれがエンジンまで通った。装飾 API はその上に乗るだけで、
**足したのは `std::game::content` の関数だけ**である。
lang item も syscall も増えていない。

```biwa
私は $blue(bold("言葉 琵琶")) (ことのは びわ)。
$big(slow("ゆっくり大きく")) 話すこともできます。 >>
```

```
push "私は "        speed=30 size=vh(2) weight=400 color=255,255,255,255
push "言葉 琵琶"     speed=30 size=vh(2) weight=700 color=0,0,255,255
push " (ことのは びわ)。\n" speed=30 size=vh(2) weight=400 color=255,255,255,255
push "ゆっくり大きく"  speed=15 size=vh(4) weight=400 color=255,255,255,255
```

### 形は「`Into[Content]` を取って `Content` を返す」

この形なので入れ子にできる。`bold(..)` が返す `Content` を
`blue(..)` が受けられるのは、`impl Content: Into[Content]` があるからである。

```biwa
fn blue[C: Into[Content]](content: C) -> Content {
  colored(content, Color::blue())
}

fn colored[C: Into[Content]](content: C, color: Color) -> Content {
  set_color(content.into(), ContentColor::Value(color))
}
```

**制限つきの型引数をそのまま別の制限つき関数へ渡せる**
(`blue` の `C: Into[Content]` が `colored` の制限を満たす) のは、
段 2 の obligation がそのまま働くからである。手を入れる必要は無かった。

`Int` / `Float` にも `Into[Content]` があるので、
`$bold(big(red(native_add(1, 2))))` のように数値にも掛かる。

### ジェネリクスは入口で剥がす

書き換えの実体は `Content` を取る非ジェネリック関数 4 つ
(`set_color` / `set_weight` / `scale_size` / `scale_speed`) に寄せた。

```biwa
fn scale_size(content: Content, mul: Float) -> Content {
  match content {
    Content::Text(text) => {
      text.size = ContentSizeLevel { mul = text.size.mul * mul };
      Content::Text(text)
    }
  }
}
```

`Content` のバリアントが増えたとき、`match` を足すのはこの 4 つだけで済む。
`red` / `blue` / `bold` / ... を全部直すことにはならない。

### 大きさと速度は掛け合わせる、色と太さは上書きする

`ContentSizeLevel` / `ContentSpeedLevel` は「設定の何倍か」なので、
入れ子にすると掛かる (`big(big(x))` は 4 倍)。絶対値にしないのは、
プレイヤーが文字サイズを変えたときに追従させるためである。

色と太さは倍率ではないので上書きになる。外側が勝つ。

### `italic` はまだ出せない

`sys_content_push_text` は italic を運んでいない。
真偽値の装飾だけを `flags` で先に通すこともできるが、
縁取りや影のような**引数を持つ装飾**がすぐ来るので、
そこをまとめて決めるまで見送っている
(「装飾をどう運ぶか」を参照)。運び方が決まれば、
`italic` は他の装飾と同じ 3 行で足せる。

### いま使える装飾

| 分類   | 一般形                  | 糖衣                                   |
| ------ | ----------------------- | -------------------------------------- |
| 色     | `colored(c, Color)`     | `red` / `green` / `blue` / `white` / `black` |
| 太さ   | `weighted(c, TextWeight)` | `bold` / `light`                      |
| 大きさ | `sized(c, Float)`       | `big` (2 倍) / `small` (0.5 倍)        |
| 速度   | `paced(c, Float)`       | `fast` (2 倍) / `slow` (0.5 倍)        |

使う側が `import std::game::content::red;` のように取り込む。
埋め込み式は普通の式なので、名前解決も普通に働く。

## 落とし穴

- **式の範囲の測り方**。文字列リテラルの中の括弧を数えないこと。
  `$foo(")")` と `$foo("(")` の両方で壊れないこと
- **`$` の直後が識別子でも `(` でもない場合**。エラーにするか、
  `$` をそのままテキストとして出すか。エスケープの決定と対になる
- **1 つの型に `into` は 1 つ**。`Into[T]` を複数当てられないので、
  std の中で `Into[String]` などを別に作りたくなったときに詰まる。
  段 3 で `Int` / `Float` にも当てたので、この枠はもう埋まっている
- **`Game` を毎回渡す**。`content_push(g, ..)` は scene の引数を毎回読むので、
  MIR では `_1` を何度も読むことになる。実害は無いはずだが、
  `Game` が値型なのでコピーの扱いを確認しておく
- **TypeScript と wasm でシグニチャが違ってよい**。
  `write` が既にそうしているように、syscall の native 実装は arch ごとに別に書ける
- **TypeScript が建たなくなる**。決めたこと 1 の結果である。
  `assets/tests` の TypeScript 側の検証も止まるので、
  wasm だけで確認する形に切り替える必要がある。
  加えて `yield` の置き場所そのものが正しくなくなっている (段 2 の項を参照)
- **`on_new_game()` の戻り値の型**。`Game[_, _]` であることしか要求しない。
  `scene main` が受け取る型と食い違っていても、いまの規約では検出できない
  (scene どうしの型の一致も検査していない)。型推論が拾うはずだが、
  診断が分かりにくくなりうる
- **`\` の扱い**。生テキストで `\$` だけをエスケープとして解釈する。
  将来 文字列リテラルのエスケープを入れるときに規則を揃えること
