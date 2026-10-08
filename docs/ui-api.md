# UI API

最終的に目指すところ

- Phase1
  - Biwa Language 側からEngineが描画処理するUIを制御できるようにする
    - UI syscall の策定、実装 (engine, std)
    - UI を適切に抽象化した型、関数等のstdにおける実装 (std)
- Phase2
  - UI 記述のためのWebライクなXMLベースの記法 (compiler)
    - Biwa Language のサブsyntaxとして追加されるため、React的なUIとロジックを適切に紐づけた表現が可能になる
- Phase3
  - Event ハンドリングの統合
    - 関数を第一級の値として扱えるようにする (compiler)
    - `funcref` でengine側にWasm関数を渡せるようにする (compiler)

entrypoint に fn app() -> Window; が増える
起動時は一番最初にこれが呼び出され、windowが生成される

```biwa
fn app() -> Window {
  let save_data_list = std::game::load_save_data_list();

  <Window scene_page_id="scene" on_event=(on_event) >
    <Page page_id="main" background_image=(title_image) >
      <Vertical margin_left={vw(60)} >
        <Link text="NEW GAME" on_click_link="scene" />
        <Link text="LOAD" on_click_link="load" />
        <Link text="CONFIG" on_click_link="config" />
      </Vertical>
    </Page>

    <Page page_id="load" >
      <HorizontalGrid>
        save_data_list.
          iter().
          map(fn(d) {
            <Link
              text=(match d.name {
                Some(name) => name,
                None => d.timestamp.to_string(),
              })
              on_click_link=("scene?save_id=".concat(d.id))
              background_image=(d.last_snapshot)
            />
          })
      </HorizontalGrid>
    </Page>

    <Page page_id="config" >
    </Page>

    // Window で指定された scene_page_id="scene" の Page に遷移したとき、
    // fn on_new_game() で Game が生成され、
    // scene main() が開始する。
    //
    // page の link にパラメータという概念を導入(クエリパラメータみたいな)
    // ?save_id= から save_id が取得されるようにする
    // on_new_game() は引数に save_id: Option[SaveId] を受け取り、
    // Some のときは save_data_list からそのデータを使用して Game::load(),
    // None のときは Game::new(), した値を返すべき
    //
    // すべての Element には id: String を設定でき、
    // Page 側からcanvas, message_area を id で設定する。
    // on_new_game() は引数に GameWindow を受け取る。
    // GameWindow は GameWindow::new() で この2つの ui_id (u32) をそれぞれ Option で受け取っている。
    // (fn GameWindow::new() はホストにexport済み( [[host_export="<function name>"]] 的なホストに露出させるattributeを作ったほうが良い ))
    // 以降そのidを介して Canvas API や Content API の出力先が決定される。
    <Page page_id="scene" canvas=("canvas") message_area=("message_area") >
      <Canvas id=("canvas") />
      <MessageArea id=("message_area") />
    </Page>
  </Window>
}
```

## 改訂版: 最終的に目指すところ

- `Window` にはプロパティが追加される
  - `.main_scene: Scene[S]`: シーンをわたす
    `Scene[S]` は `type Scene[S] = fn(Game[S]) -> Game[S];`
    このゲームのカスタムのための型引数`S`を全体で共有するため`Window`も型引数を取る(`Window[S]`)
  - `.scene_page: ScenePage`: シーン実行時に遷移すべきページのUI
    エンジンのUIとstdに `ScenePage` を新設。以下2つをプロパティにもつ。もう文字列のidでElementが相互に参照し合う必要はない(syscall レベルでは ui_id で設定する)
    - `.canvas: Canvas`
    - `.message_area: MessageArea`
- `SceneStartButton` Element を新設。基本的なプロパティは`Button`と同じだが、
  - `on_click: fn(GameWindow) -> Game[S]`: 押されたときに Engine は Biwa Language 側のこのハンドラ関数をよびだす。
    `GameWindow` の生成はすでに `[[host_export]]` で達成済みなので、 Engine はそれを渡すだけ。
    Engine はシーンページに遷移し `Window` 全体に指定された `main_scene` に返ってきた `Game[S]` を渡してシーン本体の処理に入る。

```biwa
fn main() -> Window[S] {
  let save_data_list = std::game::load_save_data_list();

  let window = <Window
    on_event=(on_event)

    // fn(Game[S]) -> Game[S]
    main_scene=(scenario)
    scene_page=(
      <ScenePage
        canvas=(<Canvas />)
        message_area=(<MessageArea />)
      >
      </ScenePage>
    )
  >
    <Page page_id="main" background_image=(title_image) >
      <Vertical margin_left=(vw(60)) >
        <SceneStartButton
          text=("NEW GAME")
          // fn(GameWindow) -> Game[S]
          on_click=(fn(window) { Game::new(window, "test game", MyGameStates::new(), Condig::default()) })
        />
        <Link text="LOAD" on_click_link="load" />
        <Link text="CONFIG" on_click_link="config" />
      </Vertical>
    </Page>

    <Page page_id="load" >
      <HorizontalGrid>
        save_data_list.
          iter().
          map(fn(d) {
            <SceneStartButton
              text=(match d.name {
                Some(name) => name,
                None => d.timestamp.to_string(),
              })

              // fn(GameWindow) -> Game[S]
              on_click=(fn(window) { Game::load(window, d) })
              background_image=(d.last_snapshot)
            />
          })
      </HorizontalGrid>
    </Page>

    <Page page_id="config" >
    </Page>
  </Window>;

  window.show();
}
```

シーンの最初からの処理とセーブデータのロードによる再開のいずれもを統一的に扱えるための一歩になる。

エントリポイントは以下の変更により1つのみになる。

- `scene main` は廃止される。 `Window` から関数を渡せるため。
- `fn on_new_game` は廃止される。`Game[S]` オブジェクトの生成は `SceneStartButton` に渡したハンドラ関数で行えるため。
- `fn app` は `fn main` に改名。唯一のエントリポイントであるため `main` という名称でよい。

## Elements

- `Window`
  ウィンドウ。UIのrootとなるElement.
  - Properties:
    - `children: Iterable[Page]`
    - `scene_page_id: String` property に設定されたPageに遷移するとfn on_new_game()
    - `on_event: fn(Event)`
- `Page`
  ページ。Windowの子ElementはPageのリストである必要がある。
  page_id で区別され、Linkでページ遷移することができる
  - Properties:
    - `children: Iterable[Element]`
    - `page_id: String`
- `Layers`
  いくつかのレイヤー。
  子 Element は syscall で渡された順に上のレイヤーに積み上がっていく。
  - Properties:
    - `children: Iterable[Element]`
- `Box`
  領域を区切る最も基本的な Element.
  - Properties:
    - `child: Element`
- `Button`
  ボタン
  - Properties:
    - `on_click: fn(Event)` クリック時のハンドラ関数を設定可能
      ハンドラ関数の設定は現時点ではcompilerが関数を第一級の値として扱っていないためできず、しばらくはButtonは実装しない
    - child 持てない
- `Link`
  リンク。押されたときにページに飛ぶ
  - Properties:
    - `on_click_link: IntoLink` クリック時の遷移先ページを設定可能
    - child 持てない
- `Image`
  画像
  - Properties:
    - `image: Image`
    - child 持てない
- `Canvas`
  Canvas API により内部に自由に描画が可能な Element.
  通常、ゲーム進行中は1つのCanvasが存在し、描画はそこに行われる。
  Canvas Element として切り出しておくことで、
  将来的に複数キャンバスを使いたい場合やキャンバスのミラーリングをしたい場合に備える。
- `MessageArea`
  Content API によりノベル表現を出力可能な Element.
  通常、ゲーム進行中は1つのMessageAreaが存在し、scene内のノベル表現は Content API を介してそこに行われる。

### Layout Element

Biwa の UI API は、 Web のように document を記述するためでなく、
当初から UI を表現するために設計される。
したがって、UIの記述に特化した表現を設計に含めて良く、
軽量にUIのレイアウトを記述するための表現の Element として Layout 系 Element を導入する。
また、複数の子Elementを持ちたい場合、 Layout 系 Element を使う必要がある
(レイアウトを特に指定しない領域に対して複数の子Elementを配置可能にすると意図しない配置になりやすいため)。

- `HorizontalLayout`

```
<Horizontal>
  <elem/>
  <elem/>
  <elem/>
</Horizontal>
```

```
-----------------------
       |       |
 elem1 | elem2 | elem3
       |       |
-----------------------
```

- `VerticalLayout`

```
<Vertical>
  <elem/>
  <elem/>
  <elem/>
</Vertical>
```

```
-------|
       |
 elem1 |
       |
-------|
       |
 elem2 |
       |
-------|
       |
 elem3 |
       |
-------|
```

- `HorizontalGridLayout`
  - `column: Uint` の数だけ水平方向に並び、折り返してまた次の行になる

```
<HorizontalGrid column=3>
  <elem/>
  <elem/>
  <elem/>
  <elem/>
  <elem/>
</HorizontalGrid>
```

```
-----------------------
       |       |
 elem1 | elem2 | elem3
       |       |
-----------------------
       |       |
 elem4 | elem5 |
       |       |
----------------
```

// VerticalGridLayout もあるべきだが正直あまり使われないと思われる

## Properties

- `width`
- `height`
- `margin`
- `padding`

units: vw, vh, percent

- `text`
  - `text_font`
  - `text_size`
  - `text_weight`
  - `text_color`

- `background_color`
- `background_image`

この2つは排他

## Syscall

```
fn sys_ui_create(
  kind: u32, // Element kind
) -> u32; // ui_id

fn sys_ui_set_property(
  ui_id:  u32,    // 対象の UI id
  kind:   u32,    // property kind
  val_u:  u32,    // 値がu32の場合の値
  val_u2: u32,    // 値がu32の場合の値2つめ (widthなど値と単位があるものは value: u32, unit: u32 として val_u, val_u2 を使う)
  val_i:  i32,    // 値がi32の場合の値 (xなど値と単位があるものは value: i32, unit: u32 として val_i, val_u を使う)
  val_i2: i32,    // 値がi32の場合の値2つめ
  val_f:  f32,    // 値がf32の場合の値
  val_s:  string, // 値がstringの場合の値
);

fn sys_ui_push_child(
  parent: u32, // 親 Element
  child: u32,  // 子 Element
);
// 親 Element が子を
// - 複数持てる場合 => 一番後ろに追加
// - 1つのみ持てる場合 => ないならば追加、あるならば既存のものは削除されて場所を奪う
// - 持てない場合 => 見た目上は何も起こらない。childはEngine上のElementリストから削除される
//
// create と push を分けることで、
// プリミティブなsyscallしかないがElementのpropertyを十分に設定したうえで
// アトミックに UI Element を出現させることができる
```

## UI XML-based Syntax

UI Element 記法は以下のようなEBNFで表せる。
なお、UI Element は式(expression)の1つの形態である。
XMLのタグを閉じるための `>` は引数の値の`<expression>`に後続すると大なりと区別がつかず、
優先度的に大なりとしてパースされてしまうため、引数の値の式にはカッコを必要とした。

```ebnf
<ui-element>
::= `<` <path> (<identifier> `=` `(` <expression> `)` )* `>` (<expression> | <ui-element>* ) `</` <path> `>`
  | `<` <path> (<identifier> `=` `(` <expression> `)` )* `/>`
```

以下のXML-based UI expressionは、

```biwa
<foo::bar baz=(a) qux=(b(1, 2)) >
  <hoge/>
  <fuga/>
</foo::bar>
```

以下の関数呼び出しの糖衣構文にすぎない。

```biwa
foo::bar(
  vec(
    hoge(),
    fuga(),
  ),
  baz = a,
  qux = b(1, 2),
)
```

子要素の位置にある値は第一引数に渡される。

<identifier> `=` `(` <expression> `)` の代入表現は単にその名前の引数に値を渡しているに過ぎない。
導入が検討されているデフォルト引数に対する名前による引数渡しに近い
(ただしデフォルト引数に対する引数渡しが`Some()`でくくる必要がないのに対し、こちらではそのまま渡されるため`Some()`が必要になることが検討されている)。

XML-based UI expression が使えるのは以下のようなシグニチャの関数の呼び出しである
(`UiElement`, `UiPage`, `Window`, `Iterable`, `Vec`, `Vec::vec`(可変長引数により複数の値を渡して`Vec`を生成できる) はいずれも lang item)。

子要素は `Iterable[_]` ならコンパイラにより `vec()`で括られるし、
そうでないならそのまま渡される(コンパイラは当該関数の第一引数の型までは見に行く必要がある)。

関数呼び出しであるため、当該関数のimport状況によってはパスで参照したいことがあるため、
`<` のあとは <path> をパースする。

```biwa
fn(
  // 子Elementは取らないが引数は0個以上取れる
) -> UiElement;

fn(
  child: UiElement,
  // 子Element以外の引数も0個以上取れる
) -> UiElement;

fn[C: Iterable[UiElement]](
  children: C,
  // 子Element以外の引数も0個以上取れる
) -> UiElement;

// UiPage を返す場合も同様
fn[C: Iterable[UiElement]](
  children: C,
  // 子Element以外の引数も0個以上取れる
) -> UiPage;

fn[C: Iterable[UiPage]](
  children: C,
  // 子Element以外の引数も0個以上取れる
) -> Window;
```

`UiElement` は `enum` として実装する。
Element の種類ごとに分岐し適切な syscall を発行するため。
