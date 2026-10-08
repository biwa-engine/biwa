# Message Window Content

ノベルテキストが出力される画面上の範囲のことを Message Window などと呼称している。
なお、 window ではないことから Message Area などのほうが適切である可能性もあり、
呼称は変更される可能性がある。

## Content API

Biwaでは `scene` 内に記述された生テキストはそのままノベルテキストとして Message Window に出力される。
しかし、テキストに対する装飾やテキスト以外のメディア(画像)などの Message Window への出力を達成するために、
コンパイラとユーザ(ゲーム開発者)それぞれのためのAPIとして Content API を提供する。

Message Window に出力されるメディアを Content と呼称する。
メディアはテキストだけでなく画像なども取りうるためあえて Message という語彙を避けた。
また、Typstでは`content`型が存在し、それにならった。

Content API により、ユーザは以下のような記述が可能になる。

`$` で始まる埋め込み式が返す結果が、Contentとして出力の一部になる。

以下のコードでは、1 〜 4 に分解され、それぞれコンパイラによる lang item std 関数の呼び出しである。

```biwa
scene foo(g: MyGame) -> MyGame {{
    Hello! 私の名前は$blue(bold(italic("琵琶")))です。>>
//  ^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^^^^ ^^
//  1                2                          3     4
}}
```

実質的に以下のように変換される。

```biwa
fn foo(g: MyGame) -> MyGame {
    content_push(g, "Hello! 私の名前は"); // 1
    content_push(g, blue(bold(italic("琵琶")))); // 2
    content_push(g, "です。\n"); // 3
    content_flush_and_wait(g); // 4
}
```

このAPIは `std::game::content` で実装が進められている。

1 〜 3 のそれぞれのように、pushされるべき値は`std::game::content::Content` 型に変換可能である必要がある。
特に、生のノベルテキスト文字列は`std::string::String`でよいが、
埋め込み式によって返る値は`$(player.hp)`のように数値の可能性もあれば、
`$blue(bold(italic("琵琶")))`のように Content API を経由してすでに`Content`型になっている場合もある。
`Content`型は、装飾情報を内部に保持するとともに、将来的にテキスト以外のメディアに対応する意味での抽象化である。
そこで、`content_push()`などの引数では`trait std::convert::Into[Content]`を満たす型を取るものとする。

```biwa
[[lang="content_push"]]
fn content_push[C: Into[Content], S](game: Game[S], content: C) {
  let c: Content = content.into();
  // Window に画面操作の API が集約されており、
  // サードパーティもstdの抽象レイヤーも Window API を経由して操作を行うことになっている
  // 内部的に Content を処理し、syscallを介してエンジンにpushしている
  game.window.message_window.push_content(c);
}

[[lang="content_flush_and_wait"]]
fn content_flush_and_wait[S](game: Game[S]) {
  // 個々で初めてpushされてきたContentがまとめて出力開始される
  game.window.message_window.flush();
  // 将来的に各種イベントを受け取る機構が検討されている
  // 共通のイベントハンドラをエンジン側が呼び、イベントの種類ごとに振り分けがされるなどが考えられる
  // game.window.wait_event(Event::MouseClickLeft);
  // 現在の一時的な実装ではsys_wait()の単なるラッパ
  game.window.wait();
}

// ---- user APIs ----

// 埋め込み式で装飾で使われる API は引数は Into[Content] を取り、戻り値は Content を返す
fn red[C: Into[Content]](content: C) -> Content {
  match content {
    Content::Text(text) => {
      text.color = ContentColor::Value(Color::red());
      Content::Text(text)
    }
  }
}
```

## Content API を支えるエンジンのAPI(syscall)

基本的にはコンパイラのための Content API に近いが、よりプリミティブである。
特に、pushはメディアの種類ごとに syscall が異なるのは避けられない。

```
fn sys_content_push_text(
  message: String,
  font: String,
  size: u32, // 文字の高さ?
  weight: u32, // 100 ~ 900 などの値
  color: u32, // r, g, b, a で各 1 byte ずつ
  speed: f32, // text per sec
  flags: u32, // その他フラグ? 斜体か、影を付けるかなど?
);

fn sys_content_push_image(path: String);

fn sys_content_flush();

fn sys_wait(); // 既存
```

## scene 埋め込み式

scene 埋め込み式は以下の 2 にあたる。

```biwa
scene foo(g: MyGame) -> MyGame {{
    Hello! 私の名前は$blue(bold(italic("琵琶")))です。>>
//  ^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^^^^ ^^
//  1                2                          3     4
}}
```

パースの規則としては以下の通りである。

- ノベルテキスト行において `$` で始まる。
  - `#` は Message Window の出力に(多くの場合)関係しない命令(Canvasの出力には関係しうる)であり、`$` は Message Window の出力にそのままなる、という意味で両者を記号レベルで分けている。
- `$` に続けて開きカッコ `(` がある場合、続く式をパースしたうえで閉じカッコ `)` が出現するまでを範囲とする。
  - ただし、閉じカッコに続いてドット `.` が現れる場合は継続する。
- `$` に続けて開きカッコが現れない場合、識別子 `<identifier>` が現れなければならない。
  - 識別子に続いては、開きカッコ `(` やドット `.` が期待される。
    - 要は、関数呼び出しか、メンバアクセスか、メソッドチェーンであることを期待する。
    - 関数呼び出し単体か、いくつかのメンバアクセスやメソッドチェーンに続くメソッド呼び出しの形になるため、最後は必ず閉じカッコ `)` で終わる。

```ebnf
<embeded-expression> ::= `$` `(` <expression> `)`
  | `$` <identifier> ( <argument-list> | <member-access-or-method-calling>* <method-calling> )

<argument-list> ::= `(` ( <expression> `,` )* <expression>? `)`
<member-access-or-method-calling> ::= `.` <identifier> <argument-list>?
<method-calling> ::= `.` <identifier> <argument-list>
```
