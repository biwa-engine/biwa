# シンボルの可視性 (issue #8) 実装方針・状況

issue #8 「[feature] Visibility of symbols」の実装方針と進み具合のメモ。決まったこと・未決のこと・調べたことをここに集める。

状況: **段階 5 (private-in-public) まで、§7 の全段階を実装済み**。

## 0. スコープ

- **含む**:
  - 可視性 `pub` / `pub(package)` / `pub(super)` / 無印 (module private)。おおむね Rust と同じにする。
  - Rust 風の子モジュール宣言 `mod foo;` / `pub mod foo;` (issue のコメントで導入を決定)。
  - パスの先頭の `super::` (`super::super::..` も)。
  - 名前解決・型推論で「シンボルは見つかったが見えない」ときの専用のエラー。
  - std・フィクスチャ・`~/test1` への可視性の付与。
- **今回は範囲外**: `pub import` (再 export)、`self::` パス。
- **導入しない可能性がある**: `pub(in path)`、インラインの `mod foo { .. }`。

## 1. 決まったこと

1. **enum の variant には可視性を書けない。** 常に enum と同じ可視性になる (Rust と同じ)。variant のフィールドも同じ。
   書かれていたら「ここには書けない」とエラーにする。
2. **private は「定義したモジュールとその子孫」から見える** (Rust と同じ)。
   - std の ui のように、struct を `ui.biwa` で定義し impl を子モジュール (`ui/box.biwa` など) に置く構成がそのまま成り立つ。
3. **`pub import` と `self::` は今回は扱わない。** `pub(in path)` とインライン `mod` は導入しない可能性がある。
   ただし `pub import` は将来の導入が確定しているので、今回の設計はそれを前提にする (§5)。
4. **ファイルがあるのに `mod` 宣言されていないものはエラーにする。** 警告の仕組みは今回は導入しない (§5.1)。
5. **private-in-public は、実効可視性を正しく判定して定義側でエラーにする** (§5.2)。
   - 実効可視性は module tree の祖先の可視性で頭打ちになるので、Voldemort 型 (§4.2) は作れない。一旦はそれでよい。
   - sealed trait (§4.3) も作れない。必要になったら別の手段で表す。
6. **`pub import` (将来) は Rust と同じ規則にする。** 再 export は元の項目に**書かれた**可視性を超えられない
   (`pub(super) fn foo` を `pub import` することはできない)。祖先のモジュールによる頭打ちは超えられる
   (private なモジュールの中の `pub struct Foo` を、上のモジュールで `pub import` して公開できる)。§5.2 / §5.3。

## 2. 未決のこと

- (今は無い)
- 警告の仕組みは今回は作らない。作るときの候補は §5.1 に残した。

## 3. 今のコードの状態 (調査)

- **構文**: `pub` / `super` / `mod` はどれもキーワードではない (`biwac_lexer/src/lib.rs`)。
  パスの先頭に書けるのは `package::` (自パッケージの根)、依存パッケージ名、今いるモジュールの子、import した名前だけ
  (`AbsolutePathHeader` は `Package` / `SelfTyp` のみ)。
- **モジュール**: `biwac_package_loader` がディレクトリを走査し、見つけた `.biwa` をすべてモジュールにしている
  (`foo.biwa` + `foo/` を子として)。`mod` 宣言の仕組みは無い。
- **名前解決**: `NameTree` を `resolve_path_in_module` / `resolve_path_in_ty` / `resolve_path_in_ext_pkg`
  (`biwac_name_resolver/src/resolving/context/module_level.rs`) が 1 セグメントずつ辿る。
  表の項目 (`ModuleNameTreeItem` / `AssocNameTreeItem`) に可視性の情報は無い。
- **メンバとメソッド**: 型推論 (`biwac_type_inferrer/src/inferrer.rs`) が解決する。
  フィールドアクセスは `infer_member_access`、struct リテラルは `infer_struct_literal`、
  メソッド呼び出しは `CallTarget::Method` を決めるところ。
- **`.biwameta`**: シンボルヘッダに `vis` 欄と
  `DiskVisibility { Private, SuperModulePublic, PackagePublic, Public }` が既にある
  (`biwac_dependency_metadata/src/metadata/format.rs`)。ただし書き出しは常に `Public` (`metadata.rs`)。
  struct のメンバと関連 item には可視性の欄が無い。
- **import は再 export されない**: import した名前は `imports` に入るだけで、モジュールの子にはならない。
- **std**: 可視性を前提にしたコメント (`// pub(super)` など) が 56 か所ある。
  `game/ui.biwa` には「`pub import` も visibility も無いので型定義を同じファイルに集めている」という注記がある。
- **エラーの仕組み**: `ErrorHolder<'c, E: BiwacError> { errs, ctx }` (`biwac_base/src/error.rs`)。警告の仕組みは無い。

## 4. Rust ではどうなっているか (調査)

### 4.1 可視性を書ける場所と意味

- enum の variant には書けない (E0449: visibility qualifiers are not permitted here)。常に enum と同じ。
  variant のフィールドも同じ。外からの構築や網羅的な match を制限したいときは `#[non_exhaustive]` (crate 単位) を使う。
- trait の項目と trait impl の項目にも書けない (E0449)。trait と同じ可視性になる。
  可視性を書けるのは inherent impl (`impl Foo { .. }`) の項目だけ。
- struct のフィールドはフィールドごとに書け、既定は private。
  見えないフィールドが 1 つでもあると、外からは struct リテラルで作れない。
  パターンでは見えるフィールドしか書けない (残りは `..`)。
- private は「定義したモジュールとその子孫」、`pub(super)` は「親モジュールとその子孫」、`pub(crate)` は crate 内。
- `pub` は「祖先が許す範囲」で見える。実際には、パスの各セグメントがその場所から見えるかを 1 つずつ確かめる。
  そのため private なモジュールの中の `pub` 項目は、そのモジュールが見える範囲からしか届かない。
- 関連 item の可視性の基準は impl ブロックのあるモジュール (型の定義場所ではない)。

### 4.2 private-in-public: なぜ「警告だがコンパイルは通る」のか

前提の訂正: `private_interfaces` は clippy ではなく rustc 自身の lint (既定で warn)。

**経緯** (RFC 136 → RFC 2145 "type privacy and private-in-public lints"。lint 化は Rust 1.74):

- **RFC 136 (旧規則) は定義側のエラーだった** (E0445 / E0446)。
  「`vis_interface > vis_type` なら、その型を item のインターフェースに使ってはならない」という規則。これには 2 つの問題があった。
  1. **厳しすぎた (誤検出)**。判定が実際の到達可能性ではなく、その場の `pub` という字面に基づいていた。
     private なモジュールの中の `pub fn` が private な型を返すと、どちらも外から届かないのにエラーになった。
     回避するために型を `pub` にする (private なモジュールに置いたまま) 書き方が広まり、規則の意味が薄れた。
  2. **足りなかった (漏れ)**。RFC の言葉では「型推論が賢いので、private-in-public 規則では type privacy を保証するのに不十分だった」。
     ジェネリクス・trait の関連型・impl などを経由すると、private な型の値が規則をすり抜けて外に出られた。
- **RFC 2145 で、本当の保証を使用側の検査 ("type privacy") に移した。**
  private な型は、それが見えない場所で名前を書くことも、その型の値を得ること (式の型に現れること) もエラーになる。
  使用側で塞いだので、定義側の規則は健全性のためには要らなくなった。
- **それでも定義側の指摘は人間にとって有用なので、lint として残した。**
  lint ならヒューリスティクスを使え、字面の `pub` ではなく到達可能性 (effective visibility) で判定できるので、直感に近くなる。
  - `private_interfaces`: インターフェースの private な型 (warn)
  - `private_bounds`: where 節などの境界の private な型・trait (warn)
  - `unnameable_types`: 外から到達できるが名前を書けない型 (allow)
- **わざと残した柔軟性** (RFC の動機の例):
  - sealed trait: `pub fn f<T: PrivateTrait>(..)` で、外から impl できない trait を作る。
  - Voldemort 型: private なモジュールの中の `pub struct` を返す関数。外からその型の名前は書けないが、値は使える (メソッドを呼べる)。

**手元の rustc 1.98.1 での確認**:

- `mod m { struct Priv; .. pub fn make() -> Priv }` は定義側が警告
  (`type Priv is more private than the item make`) で、コンパイルは通る。
- 外から `m::make()` を呼ぶと、`let _ = m::make();` だけでも使用側が**エラー** (`type Priv is private`)。
  つまり Rust でも「名前は知らないが値は使える」は**本当に private な型では許されない**。
- 許されるのは、名前を書けないが可視性は `pub` の型 (Voldemort 型) だけ。
  `mod inner { pub struct Unnameable; .. }` を `pub fn` で返すと、外から `.hello()` を呼べた (既定では警告も無し)。

**まとめ**:

- 今の Rust では、権限の境界は使用側の type privacy (エラー) が守り、定義側は lint (警告) で指摘するという二重の形になっている。
- この形は設計として選ばれたというより、当初の定義側の規則 (RFC 136) が誤検出と漏れの両方を抱えて失敗し、
  使用側の検査を後から足して、定義側を lint に格下げしたという歴史的経緯の産物である。
  定義側では通るのに使うと必ずエラーになる関数が書ける、という意味でちぐはぐさは残っている。
- 型の名前を知らずに値を使う柔軟性は、Rust では「`pub` だが名前を書けない型」(Voldemort 型) で実現している。
  private な型を通して実現しているのではない。
- Biwa は歴史を持たないので、最初から実効可視性で判定する定義側の規則に一本化する (§5.2)。

### 4.3 sealed trait

- 「外から**使える** (境界に書ける・メソッドを呼べる) が、外から**実装はできない**」trait を作る Rust の慣用句。
- 書き方: 外から名前を書けない supertrait を付ける。

  ```rust
  mod private { pub trait Sealed {} }      // 外から名前を書けない (Voldemort な trait)
  pub trait Shape: private::Sealed {       // 外から使えるが、Sealed を impl できないので Shape も impl できない
      fn area(&self) -> f64;
  }
  impl private::Sealed for Circle {}       // 実装はこの crate の中だけ
  impl Shape for Circle { .. }
  ```

- 目的:
  - **後方互換を保ったまま trait を育てられる**: 外に実装が無いので、既定実装の無いメソッドを後から足しても誰も壊れない。
  - **実装する型の集合を閉じられる**: ライブラリ側が「この型たちだけ」を前提にできる (enum に近い使い方)。
  - 例: std の `SliceIndex` (`slice[..]` に渡せる型は std が決めた範囲・整数などに限る)。
- Biwa では: 実効可視性で頭打ちにするので「`pub` だが名前を書けない trait」は作れず、
  private な supertrait を付けると private-in-public でエラーになる。つまり今の規則では sealed trait は書けない。
  必要になったら、慣用句ではなく言語機能として直接表す (例: `#[sealed]` 属性で「定義したパッケージの中でしか impl できない」とする) 方が意図が明確。

## 5. 方針

### 5.1 `mod` 宣言の無いファイル → エラー

- 宣言の無いファイルは、ほぼ確実に書き忘れか消し忘れ。直し方も明らか (宣言を足すか、ファイルを消す) なので、エラーにしても困らない。
  Rust の「黙って無視」は、よく知られた落とし穴である。
- 警告の仕組みは今回は作らない。ad hoc な警告も足さない。後から作るときの候補:
  - (a) `WarningHolder` + `trait BiwacWarning` を別に作る (`&mut WarningHolder` を渡して push する)。
  - (b) `BiwacError` を「診断」に一般化し、深刻度を持たせる。
    (b) の方が LSP (診断の深刻度を既に区別している) と揃えやすく、(a) は「エラーがあれば止める」という今の流れに手を入れずに済む。

### 5.2 private-in-public → 定義側で、実効可視性を基準にエラー

**実効可視性**

項目の「どこから見えるか」の全体。使う側の検査 (§5.3) では要らず、この節の定義側の検査でだけ使う
(2 つの項目の見える範囲を比べるため)。

- 1 本の経路 (`a::b::Foo` のように項目に届く名前の並び) について、経路上の各段 (モジュール・項目) の可視性の範囲の共通部分を取る。
  - どの段の範囲もその項目自身を含む部分木なので、共通部分は**その中で最も狭い 1 つの部分木**になる。
  - 例: private なモジュールの中の `pub fn` は、そのモジュールの private の範囲 (親の部分木) で頭打ちになる。
- 今は項目に届く経路は定義の場所の 1 本しか無いので、実効可視性はその経路のものである。
- **`pub import` (将来) を入れると、再 export の分だけ項目に届く経路が増え、見える範囲が広がりうる。**
  - 再 export は書かれた可視性を超えられないが、祖先による頭打ちは超えられる (§1 の 6)。
    ```biwa
    // lib.biwa
    mod imp;              // private
    pub import imp::Foo;  // `Foo` はパッケージの外から `mylib::Foo` で届く

    // imp.biwa
    pub struct Foo {}     // 定義の経路 `imp::Foo` だけを見ると、ルートの部分木で頭打ち
    ```
  - このとき `pub fn make() -> Foo` は正しいコードである。定義の経路だけで `Foo` の実効可視性を求めると、誤ってエラーにする。
  - したがって実効可視性は、**届く経路ごとの範囲の和**にする。範囲の和は部分木 1 つになるとは限らない
    (別々の枝で `pub(super) import` した場合など) ので、範囲の集合として持つ。
  - 再 export の import が再 export されることもあるので、モジュール木と再 export の辺を合わせたグラフの上で求める
    (rustc の `EffectiveVisibilities` と同じ考え方)。
  - 実装は最初から「経路ごとの範囲の和」を扱える形にしておく。定義の経路だけを辿る作りにすると、`pub import` を入れたときに作り直しになる。
- 関連 item (inherent impl の項目): 項目の可視性 ∩ 型の実効可視性。
- impl ブロック (trait impl): trait の実効可視性 ∩ 型の実効可視性 ∩ impl の型引数・境界に現れる型の実効可視性 (Rust と同じ)。

**規則**

- item の実効可視性を V とすると、その**インターフェース**に現れる型・trait はすべて、実効可視性が V 以上でなければならない。
  型引数の中まで辿る (`Vec[Priv]` も違反)。

**インターフェース** (今ある構文):

- fn・native fn・scene の引数・戻り値・ジェネリック境界
- struct のメンバの型 (メンバの可視性 ∩ struct の可視性で判定)
- enum の variant のフィールドの型 (enum の可視性で判定)
- type alias・native type alias の右辺
- trait の項目のシグネチャ・ジェネリック境界
- inherent impl の項目のシグネチャ
- trait impl の項目のシグネチャ (impl の実効可視性で判定)

**将来の言語機能** (「今は無いから漏れない」とは考えない。入れるときに必ずインターフェースを定めること):

- `pub import`: 実効可視性の計算に再 export の経路を足す (上記)。再 export そのものはインターフェースを持たない。
- 関連型: trait 内の宣言の境界と、**impl 側の `type Out = T` の右辺**の両方をインターフェースに含める
  (impl の実効可視性で判定)。Rust の旧規則の漏れの典型はここだった。
- supertrait: インターフェースに含める (private な supertrait はエラー。sealed trait は §4.3 の別手段で)。
- `impl Trait` (戻り値の存在型)・trait object: 境界 (trait) はインターフェースに含める。
  隠れた具体型を含めるかは、「名前を出さずに private な型の値を外に出す」ことを許すかどうかの判断になるので、入れるときに決める。
- そのほか型が現れる構文 (const・static・既定の型引数など) を足すときも同じ。

**規則を守らせる仕組み**

- インターフェースの列挙を個々の検査に散らさず、「item → そのインターフェースに現れる型の列」を返す 1 つの関数にまとめる。
  item の種類を `match` で網羅させ、新しい item の種類を足したらコンパイルが通らないようにする。
- 負例のフィクスチャを item の種類ごとに置く。

**この方針で失うもの**: Voldemort 型 (§4.2) と sealed trait (§4.3)。一旦はそれでよい。

### 5.3 使う側の検査 (名前解決・型推論)

使う側では、実効可視性 (経路ごとの範囲の和) は要らない。

- **見える範囲は部分木**。`pub` は全体、`pub(package)` はそのパッケージのルート、何も書かない・`pub(super)` はそのモジュール。
  使う側のモジュールがその部分木に入っていれば見える。
  部分木の根をパッケージから始まるセグメントの列で表せば、「根の列が使う側のモジュールの列の接頭辞か」で判定できる
  (`ModId` の親を辿って比べても同じ。列はエラー文にもそのまま出せる)。
- **名前解決**: 書かれたパスを先頭から辿り、セグメントごとに、そのセグメントの項目の (書かれた) 可視性がその場所から見えるかを確かめる。
  - 祖先による頭打ちは、途中のモジュールのセグメントを確かめることで自然に効く。
  - 再 export を通るパス (将来) なら、再 export が作った名前のセグメント自身の可視性を見る。
    その `pub import` が元の項目に書かれた可視性を超えていないかは、import 宣言の側で別に確かめる。
    経路が何本あるかは関係しない。
- **型推論**: `<expr>.<identifier>` (フィールド・メソッド) では、その識別子自身の書かれた可視性の範囲だけを見る。祖先による頭打ちは掛けない
  (Rust のフィールドの検査も同じ)。
  - `<expr>` の型の値がその場所にあること自体は、§5.2 の定義側の規則 (見えない型をシグネチャに出せない) が保証する。
    Voldemort 型を作れないので、名前を書けない型の値が出てくることは無い。
  - struct リテラルとパターンは、書いたフィールドだけでなく全フィールドを見る (§6.4)。

## 6. 実装方針 (案)

### 6.1 構文 (lexer / parser / AST)

- キーワード `pub` / `super` / `mod` を足す。
- `Visibility { Private, Super, Package, Public }` を AST に足す (`pub(super)` / `pub(package)` は `pub` の後の括弧で読む)。
- 書ける場所: モジュール宣言、fn / native fn / scene、struct / enum / type alias / native type alias、trait、
  struct のメンバ、inherent impl の項目。
- 書けない場所 (書いたらエラー): enum の variant とそのフィールド、trait の項目、trait impl の項目。
- パスの先頭に `super::` を足す (`AbsolutePathHeader::Super(n)`)。ルートモジュールより上に出たらエラー。
- `mod foo;` / `pub mod foo;` を `Globals::Mod` として足す。

### 6.2 モジュール宣言とローダー

- ローダーを「ディレクトリにあるものを全部読む」から「宣言されたものだけを読む」に変える。
  親を構文解析してから子を辿る順になり、`try_load` の作りが変わる。
- 宣言があるのにファイルが無い → エラー。ファイルがあるのに宣言が無い → エラー (§5.1)。
- `ModId` の採番は決定論的に保つ (今の「ファイル名順」の考え方を維持。SVH が毎回変わらないように)。
- LSP も同じローダーを使う (`SourceParser`) ので、宣言の無いファイルを開いたときの扱いを決める。

### 6.3 可視性を「見える範囲」に直して表に載せる

- 宣言の可視性を「見える範囲のモジュール」に直す。
  - private → 定義したモジュール
  - `pub(super)` → その親モジュール (ルートモジュールでの `pub(super)` はエラー)
  - `pub(package)` → このパッケージのルート
  - `pub` → どこからでも
  - 関連 item は impl ブロックのあるモジュールを基準にする。
- 判定は「利用する側のモジュールが、範囲のモジュールそのものかその子孫か」(§5.3)。
- `ModuleNameTreeItem` と `AssocNameTreeItem` に範囲を持たせる。
  HIR の定義 (ValDef / TyDef / struct のメンバ) にも持たせる (型推論で使うため)。

### 6.4 検査する場所とエラー

「見つからない」ではなく、それ専用のエラーにする。

- **名前解決**: `resolve_path_in_module` / `resolve_path_in_ty` / `resolve_path_in_ext_pkg` で、
  セグメントを 1 つ辿るたびに見えるかを確かめる。
  import、型の注釈、関連関数のパス、途中のモジュールの可視性 (「祖先が許す範囲」) がまとめて効く。
  - 例: `ResolveError::PrivateItem { segment, kind, visible_in }` →
    「function `sys_ui_create` is private to module `std::game::base_engine`」
- **型推論**:
  - `infer_member_access`: 見えないフィールド → 「field `x` of struct `Foo` is private」。関数型のメンバを呼ぶ `self.run(x)` も同じ経路。
  - `infer_struct_literal`: 見えないフィールドが 1 つでもあれば作れない → 「cannot construct `Foo` here: field `x` is private」。
  - パターン: 見えないフィールドを書いたらエラー。
  - メソッド呼び出し: 見つかったメソッドが見えなければエラー (Rust と同じく、ほかの候補へは逃げない)。
  - 利用する側のモジュールは、推論中の関数から取る。持ち上げた無名関数は元の関数のモジュールを使う。
- **private-in-public** (§5.2): インターフェースの検査。名前解決の後 (型が引ける時点) に、実効可視性を求めてから走らせる。
- **検査しない経路**: lang item (novel statement の展開先 `content_push` など)、host export、エントリポイント。
  コンパイラやホストが DefId や名前で直接呼ぶので、パスを辿らない。これで正しい。

### 6.5 `.biwameta`

- シンボルヘッダの `vis` に本当の値を書く。struct のメンバと関連 item にも可視性を足し、版を上げる。
- 依存パッケージの項目は、表から消さずに可視性を持ったまま読み込む。
  「見つからない」ではなく「private です」と言えるようにするため。
- 可視性は依存する側の解決結果を変えるので、SVH に入れる (書き出しに載せれば自然に入る)。
- 単相化と生成コードには影響しない。依存の private な関数も、ジェネリックな関数の中身から呼ばれれば実体化される。

### 6.6 std・フィクスチャ・`~/test1`

- 既定が private になるので、std には全面的に `pub` を付ける。
  兄弟モジュールから呼ぶもの (`materialize_into` など) は `pub(super)` で、既存の `// pub(super)` コメントと一致する。
- std の約 30 ファイルに `mod` 宣言を足す。

### 6.7 LSP

- `tools/lsp` は自前の文法 (`biwa_lsp_parser`) を持っているので、`pub` / `super` / `mod` を読めるようにし、新しいエラーの表示もつなぐ。

## 7. 段階

1. 構文と `mod` 宣言。全部受理するが、可視性はまだ検査しない。std・フィクスチャ・`~/test1` に `mod` 宣言を足す。LSP の文法も対応する。
2. 可視性を HIR・名前の表・`.biwameta` に載せる (版を上げる)。まだ検査しない。
3. 名前解決での検査。同時に std に `pub` を付け、負例のフィクスチャ
   (private な関数・型、private なモジュール越し、依存パッケージの private、`super::`) を足す。
4. 型推論での検査 (フィールド・struct リテラル・パターン・メソッド) と負例。
5. private-in-public の検査 (§5.2) と負例。

3 以降で既定が実際に private になるので、std の書き換えは 3 にまとめる。

## 8. 進み具合

### 8.1 段階 1: 構文と `mod` 宣言 (実装済み)

可視性は構文として受理し AST に載せるだけで、まだ何も検査しない。

- **字句**: `pub` / `super` / `mod` をキーワードにした (`biwac_lexer`)。ノベル DSL の字句 (`biwac_novel_parser`) には `super` だけを足した
  (`$` / `#` の中に書けるのはパスまでなので)。どれも既存のコードで識別子として使われていないことを確かめた。
- **AST** (`biwac_ast`):
  - `Visibility { Private, Super(Span), Package(Span), Public(Span) }`。
  - `vis` を持つもの: fn・native fn・メソッド・native メソッド・struct・struct のメンバ (`StructMemberDecl { vis, id, typ }`。
    以前の `(Ident, TypRepr)` を置き換えた)・enum・type alias・native type alias・trait・scene・`ModDecl`。
  - `Globals::Mod(ModDecl { vis, id, span })`。
  - `AbsolutePathHeader::Super { depth, span }` (`depth` は `super` の数)。
- **構文** (`biwac_parser`):
  - 書く順は属性 → 可視性 → 宣言 (`[[native]] pub fn ..`)。
  - `<visibility> ::= "pub" ( "(" ( "super" | "package" ) ")" )?`。
  - 書けない場所 (enum の variant とそのフィールド、trait の項目、trait impl の項目、impl ブロック、import、native code) は
    読んだうえで `ParseError::NotAllowedHere` にする (「a visibility is not allowed on an enum variant (it has the same visibility as the enum).」など)。
    `pub import` も「not supported yet」としてここで拒否する。
  - `mod` 宣言には属性を付けられない (同じく `NotAllowedHere`)。
  - `super::` (`super::super::..`) はパスの先頭ならどこでも書ける (import・型・式・パターン、ノベル DSL の式も)。後には必ず識別子が要る。
- **パッケージローダー** (`biwac_package_loader`):
  - 今までどおり最初に `src/` 以下の `.biwa` をすべて読み、`ModId` をファイル名順に振る
    (採番が宣言の書き方で変わらないように。`.biwameta` の SVH が安定する)。
  - そのあとルートから構文解析し、`mod` 宣言を辿って**宣言されたものだけ**をモジュール木に入れる。
    子のファイルはそのモジュールのディレクトリ (`src/` または `src/a/` など) の `<name>.biwa`。
  - エラー:
    - `ModuleFileNotFound`: 宣言されたのにファイルが無い (「File for module `nowhere` not found.」+ 期待したパス)。
    - `UndeclaredModuleFile`: ファイルがあるのにどこからも宣言されていない。宣言を書くべきファイルを添える。
      対応するモジュールファイルの無いディレクトリの中の `.biwa` (`src/lost/inner.biwa` で `src/lost.biwa` が無い) もこれになる。
    - `DuplicatedModDecl`: 同じ名前の宣言が 2 つ。
    - `RootModuleNameDeclared`: ルートモジュールで `mod main;` / `mod lib;` (ルートモジュールそのものなので子にできない)。
  - 構文エラーのあるモジュールは、どの子が宣言されているか分からないので子を見ない。
  - `Pkg::try_load_tolerant`: 上のモジュール木の形の誤りでは失敗せず、読めた分の木と誤りの一覧を返す (LSP 用。下記)。
    コンパイラは今までどおり `Pkg::try_load` (誤りがあれば失敗)。
- **名前解決**: `ModuleNameTree` に親モジュール (`parent`) を持たせ、`super::` は `depth` だけ親を辿ってからの相対パスとして解決する。
  ルートより上を指せば `ResolveError::SuperBeyondRoot`。
- **既存のパッケージに `mod` 宣言を足した**: std (`library/std`)、フィクスチャ (`compiler/assets/tests` の std・test1・too_many_errors、
  LSP の minipkg)。可視性はまだ書いていない (すべて `mod`)。std のどれを `pub mod` にするかは段階 3 で決める。
  `~/test1` は 1 ファイルなので変更なし。
- **LSP** (`tools/lsp`):
  - 字句・文法・lowering・ハイライトを `pub` / `super` / `mod` に対応させた。可視性は宣言ノードの先頭の `Visibility` ノード、
    `mod` 宣言は `ModDecl` ノード。書けない場所の可視性は biwac_parser と同じ文面で構文エラーにする。
  - ロードを `try_load_tolerant` にした。新しいファイルを作ってから `mod` を書くまでの間にパッケージ全体の解析が止まらないようにするため。
    - 宣言されていないファイルを開くと、そのことだけを診断に出す (「This file is not declared as a module. Declare it with `mod <name>;` in `src/lib.biwa`.」)。
    - ファイルの無い宣言・重複した宣言は、宣言を書いたファイルを開いているとき、その宣言の位置に診断を出す。
    - ディスク上のファイルを読むので、保存していない `mod` 宣言の追加はまだ反映されない (既知の制約と同じ)。
- **テスト**:
  - compiler: 125 件 (追加分: パーサの可視性・`mod`・`super::` 4 件、ローダーの正例と 4 種のエラー 5 件、
    driver の `mod_tree` (正例。可視性の構文と `super::` を import・型・式で使う) と `super_beyond_root` 2 件)。
  - LSP: 99 件 (追加分: lowering 3 件、宣言されていないファイルがあっても解析が続くこと 2 件)。
  - std と `~/test1` を強制再ビルドし、ブラウザで Link と scene の開始まで動くことを確かめた。

### 8.2 段階 2: 可視性を HIR・名前の表・`.biwameta` に載せる (実装済み)

まだ検査はしない。載せるだけである。

- **型** (`biwac_hir::Visibility`):
  - `Visibility { declared: DeclaredVisibility, scope: VisibilityScope }`。
    書かれた形 (`Private` / `Super` / `Package` / `Public`) と、それが指す見える範囲の組。
  - `VisibilityScope { Public, Package(PackageId), Module(ModId) }`。`Module` はそのモジュールとその子孫。
    何も書かなければ宣言したモジュール、`pub(super)` ならその親。`ModId` はパッケージを含むので、依存パッケージの宣言も同じ形で表せる。
  - 書かれた形も持つのは、`.biwameta` に書くため (見える範囲から逆算すると、どのモジュールを基準にしたかを取り違えうる)。
- **見える範囲の決め方** (`biwac_name_resolver::visibility`):
  - 宣言の span がモジュールの `ModId` を持っているので、宣言したモジュールは span から取る。親はモジュール → 親の表 (`ModuleParents`) から引く。
  - 関連 item は impl ブロックのあるモジュールが基準。variant (とそのフィールド) と trait の項目は持ち主 (enum・trait) と同じ。
  - **trait impl の項目は `pub`** にした。trait impl の項目はスコープにある trait を経由してしか引けないので、
    見えるかどうかは trait 自身の可視性で決まり、項目に別の制限を持たせる意味が無いため。
  - 持ち上げた無名関数は、書いたモジュールの中に限る (名前で引けないので意味は無い)。
  - **ルートモジュールの `pub(super)` はエラー** (`ResolveError::SuperVisibilityInRoot`。「use `pub(package)` ..」と添える)。
    名前解決の最初に AST を見て報告する (`check_super_in_root`)。見える範囲を決める側は、続きの解析のために何も書かなかったのと同じ扱いにする。
- **HIR**: `vis` を持つもの: `FnDef`・`NativeFnDef`・`NovelSceneDef`・`StructDef` (メンバは `member_vis`)・`EnumDef`・`TypeAliasDef`・
  `NativeTypeAliasDef`・`TraitDef`。`Hir::mod_vis` に自パッケージのモジュールの可視性 (`mod` 宣言のもの。ルートは載らない)。
- **名前の表**: `ModuleNameTree::vis` (子の名前 → 可視性。子モジュールは `mod` 宣言のもの。読むのは `child_visibility`)、
  `AssocNameTreeItem::vis`。依存パッケージは `ExternalChildRef::vis` (見えないものも表から消さずに返す)。
  - ついでに、def collector の関連 item の登録 (関連関数・メソッド・native の 4 通りで同じことを書いていた) を 1 つのループにまとめた。
    DefId の採番順は変えていない。
- **`.biwameta`** (版 13 → 14):
  - シンボルヘッダの `vis` に書かれた形を書く (今までは常に `Public`)。variant は enum、trait の項目は trait、trait impl の項目は `Public`、
    trait impl ブロックそのもの (名前で引かれない) は `Public`、ルートモジュールは `Public`。
  - `DiskStructMember` に `vis` を足した (variant のフィールドも同じ型を使うので、enum の可視性を書く)。
  - SVH にシンボルの可視性とメンバの可視性を入れた (依存する側の名前解決の結果を変えるため)。
  - 依存する側: `DepMetadata::ext_visibility` が見える範囲を組み立てる。
    シンボルの持ち主 (それを子に持つモジュール・型・trait) の表を初回に作り、属するモジュールを辿る。
    関連 item は impl ブロックのモジュールを記録していないので型の属するモジュールで代える
    (依存する側からは `pub` 以外は見えないので判定は変わらない。エラーの文面に出すモジュール名が変わりうるだけ)。
- **テスト**:
  - compiler 128 件 (追加分: `.biwameta` を通した可視性の往復 (名前の表と型の定義の両方)、名前の表の可視性、ルートモジュールの `pub(super)` の拒否)。
  - `mod_tree` フィクスチャのルートの `pub(super)` を `pub(package)` に直した (段階 2 からエラー)。
  - LSP 99 件 (新しいエラーの文面を足しただけ)。
  - std と `~/test1` を強制再ビルドし、ブラウザで Link と scene の開始まで動くことを確かめた。

### 8.3 段階 3: 名前解決での検査 (実装済み)

- **判定**: `biwac_hir::Visibility::is_visible_from(from, parent_of)`。見える範囲の部分木に、パスを書いたモジュール `from` が入っていれば見える (§5.3)。
  `pub(package)` は `from` のパッケージで比べる。型推論 (段階 4) でも同じものを使う。
- **場所**: パスを辿る 4 か所 (`biwac_name_resolver` の `module_level.rs`) で、セグメントごとに確かめる。
  - モジュールの子 (`resolve_path_in_module`。`ModuleNameTree::child_visibility`)
  - 型の関連 item・variant (`resolve_path_in_ty`。`AssocNameTreeItem::vis`。`find_matched` が項目ごと返すようにした)
  - 依存パッケージのモジュールの子・型の関連 item (`resolve_path_in_ext_pkg` / `resolve_path_in_ext_ty`。`ExternalChildRef::vis`)
  - 祖先による頭打ちは、途中のモジュールのセグメントを確かめることで効く。
  - import・型の注釈・関連関数のパス・`super::` はすべてこの経路を通るので、まとめて効く。
    import した名前から始まるパスは、import のパスを解決するときに確かめてある。
  - 名前解決の文脈 (fn・impl・型定義・trait 定義) はどれも最後は `ModuleResolveCtx::resolve_path` に来るので、ここだけでよい。
    パスを書いたモジュールは `LocalTreeCtx::from` で持ち回る。
- **検査しないもの**:
  - trait 越しの解決 (`solve_assoc_fallback`): trait impl の項目は `pub` 扱い (§8.2)。trait がスコープにあることは import の側で確かめてある。
  - `Self::foo`: 名前解決はヘッダ (`Self`) だけを解決し、`foo` は型推論が引くので段階 4。`T::foo` (ジェネリック引数の trait の項目) も trait の可視性で足りる。
  - lang item・host export・エントリポイント (コンパイラ・ホストが DefId や名前で直接呼ぶ)。
- **エラー**: `ResolveError::InvisibleItem { segment, kind, vis }`。
  「`hidden_fn` is not visible here.」+「a value visible only in module `a` and its submodules」+「it is declared without `pub` (private to its module)」。
  依存パッケージの項目は「visible only in a module of its own package」(依存パッケージのモジュール名はまだ引けない)。
  - 見えなくても項目は見つかっているので、セグメントには解決結果を入れたまま先も辿り、可視性のエラーだけを返す。
    同じ import を何度使っても、エラーは 1 度だけになる。
  - LSP にも同じ診断を足した。
- **今のままの点**: import はそれを使うパスが解決されるときに初めて解決される (以前からの作り)。
  使われない import は、private を指していても報告されない (存在しないものを指していても報告されないのと同じ)。
- **フィクスチャ**:
  - コンパイラのテスト用のライブラリ (`compiler/assets/tests` の std・color・greeter・fn_lib) と、test1 の子モジュールの公開面を `pub` にした
    (テスト用の入力なので、細かく絞らず、`mod` 宣言・トップレベルの項目・inherent impl のメソッド・struct のメンバをすべて `pub` にした)。
    これをしないと、負例のフィクスチャ (`scene_main` など) が本来の理由ではなく可視性で落ちてしまう。全フィクスチャで可視性のエラーが出ないことを確かめた。
  - 負例 `vis_errors` (パッケージの中: private な関数の import、private なモジュール越し、private な関連関数、private な enum の variant) と
    `vis_dep` / `vis_dep_user` (依存パッケージの private・`pub(package)`・private なモジュールの中の `pub`)。
    見えるもの (`pub(super)` を親から、`pub(package)` を兄弟から、`super::` 経由) も同じフィクスチャに並べ、報告される名前の集合を確かめている。
- **テスト**: compiler 130 件、LSP 99 件。
  std (`library/std`。可視性は利用者が設定したもの) と `~/test1` を強制再ビルドし、ブラウザで Link と scene の開始まで動くことを確かめた。
  ただし `~/test1` の依存 `greeter` (Hub から取ったもの) は `trait Greeter` が `pub` でないため、手元の `.biwa_build/deps/greeter` だけ `pub trait` に直して確かめた
  (Hub 側の `greeter` も直して公開し直す必要がある。`self.display_name` (std の `Character` の private なメンバ) を読んでいるので、段階 4 でも引っかかる)。

### 8.4 段階 4: 型推論での検査 (実装済み)

- **使う側のモジュール**: いま推論している関数のモジュール (`FnTyCtx::module`。もともと trait のスコープの判定に使っていたもの)。
  持ち上げる無名関数は、外側の関数の推論の中で推論されるので同じモジュールになる。
- **判定**: `TyCtx::is_visible_from` (中身は `Visibility::is_visible_from`)。モジュールの親は `Hir::mod_parents` (名前解決が渡す) から引く。
  関数・メソッドの可視性は `TyCtx::get_value_visibility` (自パッケージは HIR の定義、依存パッケージは `DepMetadata::ext_visibility`)。
- **検査する場所** (`biwac_type_inferrer/src/inferrer.rs`):
  - メンバを読む (`infer_member_access`)・関数型のメンバを呼ぶ (`infer_dot_call` の値の呼び出し): メンバの可視性 (`StructDef::member_vis`)。
  - メソッドを呼ぶ (`infer_dot_call`): 実装の決まったもの (`MethodTarget::Direct`) はその可視性を見る。
    trait 越しのもの (`MethodTarget::Trait`) は見ない (trait impl の項目は `pub` 扱い。trait がスコープにあることで足りる)。
  - struct リテラル (`infer_struct_literal`): **メンバがすべて見えなければ作れない** (書いたメンバだけでなく全メンバを見る。Rust と同じ)。
    どのメンバを報告するかがぶれないよう、名前順に見る。
  - パターンは enum の variant しか分解しないので検査しない (variant とそのフィールドは enum と同じ可視性で、variant のパスは名前解決が確かめる)。
- **エラー**:
  - `TyError::InvisibleMember { ty, member, is_method, vis }`: 「Field `y` of `Point` is not visible here.」+「visible only in module `a` and its submodules」+「it is declared without `pub` ..」。
    `ty` は受け手の型 (`Int` などのメソッドもあるので型そのものを持つ)。
  - `TyError::InvisibleFieldInLiteral { def_id, field, span, vis }`: 「`Point` cannot be constructed here because field `run` is not visible.」。
  - 文面の「どこから見えるか」「どう書かれたか」は `Visibility::describe_scope` / `describe_declared` (HIR) にまとめ、名前解決のエラーと共有した。
  - LSP にも同じ診断を足した。
- **見つけた既存の不具合 (今回は直していない)**: 式の `Self::foo()` は名前解決の lowering で panic する
  (`foo` のセグメントがどこでも解決されない)。直ったら、`foo` の可視性もこの段で見る必要がある。
- **フィクスチャ**: `vis_members` (正例: pub なメンバ、親からの `pub(super)` なメンバ、getter、子モジュールの中での private の利用) と、
  負例 `vis_field_read` / `vis_field_call` / `vis_method` / `vis_struct_literal`、依存パッケージの `vis_dep_field_user` / `vis_dep_method_user`
  (`vis_dep` に `Item` を足した)。型推論は最初のエラーで止まるので、負例は 1 つずつ置いた。テストはエラーの種類と名前まで確かめる。
- **テスト**: compiler 132 件、LSP 99 件。
- **std の違反** (std 側は直していない): 型推論の検査を入れて初めて見つかったもの (struct リテラルを他のモジュールで書いている):
  - `game/character.biwa:62` の `Position { x = x, y = y }` (`Position` は `game::canvas`。メンバが private)
  - `game/config.biwa:78` の `ContentSpeed { text_per_sec = 30.0 }` (`ContentSpeed` は `game::content`。メンバが private)
  - 依存する側: Hub の `greeter` が std の `Character` の private なメンバ `display_name` を読んでいる。
  - これらを scratch のコピーでだけ仮に開けると、std と `~/test1` (greeter を含む) は他に違反なくコンパイルできることを確かめた。

### 8.5 段階 5: private-in-public (実装済み)

- **場所**: 名前解決の最後 (`biwac_name_resolver::interface_check`)。パスが解決済みの AST の上で行う。
  HIR では型エイリアスが右辺に展開されていて、`pub fn f() -> PrivAlias` の `PrivAlias` が見えなくなるためである。
  違反はまとめて報告する (型推論と違い、最初の 1 つで止まらない)。
- **実効可視性** (`EffectiveVisibility`): 経路ごとの見える範囲の和として持つ (§5.2。`pub import` を入れたら範囲を足す)。
  今は経路が定義の場所の 1 本なので、範囲は 1 つ (か空)。
  - モジュール: ルートは `pub`。子は `mod` 宣言の可視性と親の共通部分。
  - 型・trait・関数: 宣言の可視性とモジュールの共通部分。
  - struct のメンバ: メンバの可視性と struct の共通部分。enum の variant のフィールド・trait の項目: 持ち主 (enum・trait) と同じ。
  - inherent impl の項目: 項目の可視性 (impl ブロックのモジュールが基準) と型の共通部分。
    trait impl の項目: trait と型の共通部分 (impl の実効可視性)。
  - 範囲どうしの共通部分は、部分木なので入れ子なら狭い方、交わらなければ空。含むかどうかは §5.3 と同じ部分木の判定。
  - 依存パッケージ・組み込みの型と trait は `pub` として扱う (シグネチャに書けたなら、名前解決がその場所から見えることを確かめてある)。
- **インターフェース** (§5.2 の一覧のうち今ある構文): fn・native fn・scene の引数・戻り値・ジェネリック引数の制限、
  struct のメンバの型、enum の variant のフィールドの型、型エイリアスの右辺、trait のジェネリック引数の制限と項目のシグネチャ、
  impl の項目のシグネチャ (impl のジェネリック引数の制限も含む)。型引数・関数型の引数と戻り値の中まで辿る。
  - 型定義 (struct・enum) のジェネリック引数の制限はまだ構文として扱っていない (`.biwameta` にも書いていない) ので見ない。
  - 新しい項目の種類を足したら、`Checker::check_module` の `match` (網羅) にインターフェースを足すこと。
- **エラー**: `ResolveError::PrivateInPublic`。「`Priv` is less visible than `ng_ret`.」で、型の位置に「`Priv` is visible only in module `a` ..」、
  項目の名前に「`ng_ret` is visible in the root module ..」を付ける。LSP にも足した。
- **フィクスチャ**: `vis_interface`。違反 8 つ (戻り値・引数・関数型の中・型エイリアス・trait の制限・pub なメンバ・variant のフィールド・pub なメソッド) と、
  通るもの (private 同士、private なメンバ・メソッド、祖先による頭打ちで通る `ok_capped`) を並べ、報告される (項目, 型) の組を確かめる。
  `mod_tree` の `pub fn make() -> Pair` (`Pair` は `pub(package)`) が違反になったので `pub(package) fn` に直した。
- **テスト**: compiler 133 件、LSP 99 件。
- **std の違反** (std 側は直していない): `game/config.biwa` の `pub fn DeveloperConfig::new(.., default_content_size: Size, ..)`
  (`Size` は `pub(super)` で、見えるのは `game` の中だけ)。
  これと段階 4 の 3 件を scratch のコピーでだけ仮に開けると、std と `~/test1` (greeter を含む) は他に違反なくコンパイルできることを確かめた。

