# enum と match

**実装済み。** 以下は方針と、実装の過程で分かったことである。

`docs/biwa-lang-type-system.md` の列挙型を実装する。
`match` が無いと enum から値を取り出せないので、今回はそこまでを 1 区切りとする。

`trait` は別途。ここでは扱わない。

## 決めたこと

- **variant にも DefId を振る**。`import package::color::Color::Red;` と書けば
  `Red` 単体で使えるようにするため (rustc と同じ)
- `DefIdKind` に増えるのは **`Variant` だけ**。enum は struct や type alias と同じく
  `Ty(TyDefId)` のままにする。enum かどうかは `TyDefId` から `TyDefKind` を引けば分かる
- payload の取り出しは rustc と同じ **`Downcast` 射影**で表す
- タプル形式のフィールド名は `_0`, `_1` を**そのまま生成コードにも出す**
- `.biwameta` の形式バージョンを 5 → 6 に上げる。既存の `.biwa_build` は無効になる

---

## 内部表現

### wasm — WasmGC の部分型

タグだけを持つ親型と、バリアントごとの子型を作る。

```wat
(rec
  (type $Opt      (sub     (struct (field $tag (mut i32)))))
  (type $Opt.None (sub $Opt (struct (field $tag (mut i32)))))
  (type $Opt.Some (sub $Opt (struct (field $tag (mut i32)) (field $_0 (mut i32)))))
)
```

| 操作     | 命令                                                    |
| -------- | ------------------------------------------------------- |
| 構築     | `struct.new $Opt.Some` (タグの定数を先に積む)           |
| 判別     | `struct.get $Opt $tag` → `SwitchInt`                    |
| 取り出し | `ref.cast (ref $Opt.Some)` → `struct.get $Opt.Some $_0` |
| 判定のみ | `ref.test (ref $Opt.Some)`                              |

**この形が通ることは確認済み**である。上の wat を、コンパイラが使っているのと同じ
`wat 1.258` でアセンブルし、`wasmparser 0.258` の validate を通し、
Node v22.22.3 で実行して、タグ取得・`ref.test`・親型経由の `struct.get`・
`ref.cast` 越しの取り出しがすべて期待どおり動いた。

`anyref` にタグと値を入れる案を採らない理由:

- payload が生の型のまま置ける。`Int` / `Float` を箱に入れなくてよい
- 現在の `Option[T] = anyref` にある「`Option[Int]` や `Option[String]` は書けない」
  という制限がそのまま消える
- 単相化された各実体で payload の型がそのまま残るので、backend が型を復元しなくてよい

`biwa-lang-type-system.md` の「各種の値を格納するのに必要なメモリサイズの最大値を取る」は
線形メモリ前提の記述である。WasmGC ではフィールドは型付きスロットなので重ね合わせられない。
ただしバリアントごとに型を分ければ各実体は自分の分しか持たないので、
狙っている「必要な最小のメモリサイズ」は結果的に達成される。

<!-- 実装後にこの段落は書き換えること。 -->

docはターゲットによらない表現をしているため、書き換えなくて良い。

型はすべて 1 つの `(rec ...)` グループに入れる。既存の struct と同じ場所で、
親型と子型が同じグループにあれば部分型として参照できる。

### TypeScript — 判別可能なユニオン

`any` は要らない。

```ts
type _ZN3std5types6option6OptionE<T> = { tag: 0 } | { tag: 1; _0: T };
```

`switch (x.tag)` や `if (x.tag === 0)` で TS 自身が絞り込むので、
payload へのアクセスにキャストも型注釈も要らない。
struct を素のオブジェクトリテラルで出しているのと同じ流儀で書ける。

### タグの値

**宣言順の 0 始まり**。`.biwameta` にも宣言順で載せるので、パッケージを跨いでも安定する。

MIR の `SwitchInt` はコメントに「将来の `match` もこれになる」と書いてあり、
そのまま使える。

### フィールド名

| 宣言                    | フィールド名 |
| ----------------------- | ------------ |
| `Bar` (unit)            | 無し         |
| `Bar(X)` (tuple)        | `_0`         |
| `Bar(X, Y)`             | `_0`, `_1`   |
| `Bar { x: X }` (struct) | `x`          |

タプル形式も名前を持つ形に正規化するので、MIR の
`PlaceElem::Field(InternedIdent, Ty)` がそのまま使える。
backend も struct と同じ経路を通る。

HIR は `shape` (Unit / Tuple / Struct) を別に持つ。
これは「宣言と構築・パターンの書き方が一致しているか」の検査にだけ使う。

---

## 構文

### enum 宣言

```biwa
enum Color {
  Red,
  Rgb(Int, Int, Int),
  Named { name: String, alpha: Int },
}

enum Option[T] {
  None,
  Some(T),
}
```

### match

`if` と同じく**式にも文にもなる**。既存の `consume_if_expression_or_statement` と
`ExprOrStmt` の作りをそのまま踏襲する。

```biwa
// 文として
match color {
  Color::Red => { ... }
  Color::Rgb(r, g, b) => { ... }
  Color::Named { name = n, alpha } => { ... }
  _ => { ... }
}

// 式として (すべてのアームが値を返す)
let n = match opt {
  Option::None => 0,
  Option::Some(x) => x,
};
```

アームは `<pattern> => <block>` または `<pattern> => <expr> ,` とする。
`if` の block 形と式形の扱いと揃えれば、`ExprOrStmt` の判定を再利用できる。

構造体形式のパターンは、構造体リテラルと同じく `=` を使う
(`Foo { x = x }` と書く言語なので `:` にしない)。
`{ alpha }` の省略形は「同名の変数に束縛する」。

**scrutinee には構造体リテラルを置けない。** `if` / `while` と同じ理由で、
直後にブロックの `{` が来るためである。
すでにある `consume_condition_expression` の仕組み (`no_struct_literal` フラグ) を
そのまま使う。

### 今回入れるパターン

- バリアントパターン: `Color::Red` / `Color::Rgb(a, b, c)` / `Color::Named { name = n }`
- 変数束縛: パターン内の識別子
- ワイルドカード: `_`
- 省略形: `Color::Named { alpha }`

**入れないもの** (次以降):

- ネストしたパターン (`Some(Rgb(r, g, b))`)
- リテラルパターン、範囲パターン
- `|` による選択、ガード (`if`)
- `..` によるフィールド省略
- struct に対する分解パターン、`let` パターン、`if let` 相当

ネストを入れないと決めることで、パターン照合は
「タグの一致 + 1 段の束縛」に閉じる。決定木を組む必要がなく、
`SwitchInt` 1 つと各アームでの `Downcast` に落ちる。

### トークンの追加

| クレート             | 追加                                                                                      |
| -------------------- | ----------------------------------------------------------------------------------------- |
| `biwac_lexer`        | `KwEnum` (`enum`), `KwMatch` (`match`), `KwUnderscore` (`_`), `MarkFatArrow` (`=>`)       |
| `biwac_novel_parser` | 同じものを `NCodeTkKind` にも。`token.rs` の 4 箇所 (定義・`as_name`・`Display`・pre_lex) |

`=>` は `=` の後読みで作る。`->` (`MarkArrow`) と同じ要領。

`_` は現状ただの識別子として lex される。`_` 単体のみキーワードにする。
`_probe` のような識別子には影響しない。

---

## DefId の設計

### VariantDefId

`impl_typed_def_id!` マクロで `VariantDefId` を足す。
`TyDefId` / `ValDefId` と同じく `DefId(pkg, local_idx)` の薄い包みである。

**外部パッケージのために、variant は `.biwameta` のシンボルでなければならない。**
`ExternalChildRef::sym_idx` がそのまま `PackageLocalDefId` になる作りなので、
variant にシンボル番号が無いと `VariantDefId` を組み立てられない。
`DiskSymbolKind::Variant = 5` を足し、enum のボディから
`variant_symbols: DiskVec<DiskSymbolIndex>` で参照する
(struct の `assoc_symbols` と同じ形)。

`DiskVariantData` には親 enum のシンボル番号 (`owner`) と宣言順の添字を持たせる。
理由は後述の「バリアントから親 enum を引く」を参照。

### DefIdKind — enum は `Ty` のまま

```rust
pub enum DefIdKind {
    Package(PackageId),
    Mod(ModId),
    Ty(TyDefId),                   // struct / enum / type alias / native type alias
    Variant(VariantDefId),         // 追加はこれだけ
    Val(ValDefId),
    Gen(GenDefId),
    LocalGen(LocalGenDefId),
    Var(VarId),
}
```

**enum に専用の腕を作らない。** struct も type alias も native type alias も
すべて `Ty(TyDefId)` になっている。enum も同じ「型」であって、
名前解決の段でそれ以外の扱いをする必要がある場面が無い。

- 型の位置 (`fn f(x: Color)`) — `ty_kind_from_typ_repr` が
  `TyKind::Defined(def_id, genargs)` を作る。struct と全く同じ経路
- 関連関数の解決 (`Color::from_hex(..)`) — `resolve_path_in_ty` が
  `TyNameTree::children` を引く。struct と全く同じ経路
- バリアントの解決 (`Color::Red`) — 同じ children から
  `AssocNameTreeItemKind::Variant` が返る

種別が要る場面 (MIR の構築、codegen の型出力、網羅性検査) はすべて
HIR が出来た後なので、`TyDefId` から `TyDefKind` を引けば分かる。
外部パッケージも `TyCtx::get_ty_impl` が `.biwameta` から遅延ロードするので同じである。

`DefIdKind::Enum` を分けると**取りこぼしが静かに壊れる**のも理由である。
`DefIdKind::Ty` に触れている 12 箇所のうち大半は網羅的な `match` ではなく、

```rust
// lowering/expressions.rs
let Some(PathSegmentResolution::Ok(DefIdKind::Ty(def_id))) = owner.resolved_id.get() else {
    return None;
};

// def_collector.rs
Some(PathSegmentResolution::Ok(DefIdKind::Ty(id))) => Some(*id),
_ => None,
```

のように「合わなければ `None`」で流す形をしている。
`Enum` を足しても**コンパイルエラーにならず**、
「型が見つからない」「エイリアスの右辺が解決できない」という形で静かに落ちる。

対して `TyDefKind` に `Enum` を足すのは網羅的な `match` を壊すので、
**必要な箇所がすべてコンパイルエラーとして出る**。
現在 `TyDefKind::Struct` に触れている 18 箇所、
`SymbolBody::Struct` に触れている 8 箇所が漏れなく洗い出される。
種別の分岐はこちらに寄せるのが安全である。

`TyDefIdKind` (name_resolver の `ty_def_id_kind_from_path` の戻り) も変更不要。

### 名前ツリー

variant は enum の `TyNameTree::children` に載せる。
関連関数と同じマップである。名前空間が型と値で分かれていないという前提どおり、
これで `Color::Red` が一意に解ける。

```rust
pub enum AssocNameTreeItemKind {
    Val(ValDefId),
    Variant(VariantDefId),   // 追加
}
```

`TyNameTree` は変更不要である。`module_item_to_def_id_kind` は
struct でも enum でも `DefIdKind::Ty(ty.def_id)` を返せばよいので、
「これは enum である」という印を名前ツリーに持たせる必要が無い。

**バリアント名と関連関数名は衝突する。** `enum Foo { Bar }` と
`impl Foo { fn Bar() }` は同じ children に載るのでエラーにすべきである。
`AssocNameTree::assocs` は今ジェネリック引数違いを許す `Vec` なので、
「種別が違うものが並んだら重複」という検査を `find_matched` の手前に足す。

### バリアントから親 enum を引く

`DefIdKind::Variant(VariantDefId)` だけでは、どの enum の何番目かが分からない。
パスの形によって親を辿れるかどうかが変わる。

| 書き方                                              | 親を辿れるか                                                          |
| --------------------------------------------------- | --------------------------------------------------------------------- |
| `Color::Red`                                        | パスの最後から 2 番目のセグメントが `DefIdKind::Ty(Color)` に解決済み |
| `Red` (import 済み)                                 | **辿れない**。import の解決結果は最終セグメントの `DefIdKind` だけを  |
| `segments[0]` に写すので、途中の `Color` が残らない |

後者があるので、**逆引き表が要る**。

```rust
pub struct VariantOwner {
    pub enum_def_id: TyDefId,
    pub index: u32,
}
```

- 自パッケージ分は `DefCollector` が variant を採番するときに作る。
  `lower()` に `impl_collector` を渡しているのと同じ要領で lowering へ運び、
  最終的に `Hir` が `variant_owners: HashMap<VariantDefId, VariantOwner>` として持つ
- 外部パッケージ分は `DiskVariantData::owner` (親 enum のシンボル番号) と
  `index` から復元する。`TyCtx::get_ty_impl` と同じく遅延で引く

パスから辿れる場合でも、常にこの表を使うことにする。経路を 1 本にしておかないと
`Color::Red` と `Red` で挙動が分かれる。

### import

`import package::color::Color::Red;` は既存の import 解決経路で通る。
`ModuleResolveCtx::resolve_path` の import 先の種別 dispatch
(`DefIdKind::Ty` / `DefIdKind::Mod` で分かれているところ) に
`Variant` の腕を足す。`Variant` はそこで終端なので、
その先にセグメントがあればエラーにする。

`import package::color::Color;` だけして `Color::Red` と書く形は
`DefIdKind::Ty` の既存の腕がそのまま処理するので、変更は要らない。

---

## 各段の実装

### 1. AST (`biwac_ast`)

```rust
pub enum TypeDef {
    Struct(StructDef),
    Enum(EnumDef),          // コメントアウトされているものを実装
    TypeAlias(TypeAlias),
    NativeTypeAlias(NativeTypeAlias),
}

pub struct EnumDef {
    pub id: Ident,
    pub def_id: OnceCell<TyDefId>,
    pub variants: Vec<VariantDecl>,
    pub genargs: Option<GenArgsDecl<GenDefId>>,
    pub attrs: Attrs,
}

pub struct VariantDecl {
    pub id: Ident,
    pub def_id: OnceCell<VariantDefId>,
    pub fields: VariantFieldsDecl,
}

pub enum VariantFieldsDecl {
    Unit,
    Tuple(Vec<TypRepr>),
    Struct(Vec<(Ident, TypRepr)>),
}
```

match 側:

```rust
pub struct MatchExpr {
    pub scrutinee: Box<Exprs>,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

pub struct MatchArm {
    pub pattern: Pattern,
    pub body: BlockExpr,       // 式形。文形は MatchStmt が BlockStmt を持つ
    pub span: Span,
}

pub enum Pattern {
    Wildcard(Span),
    Binding(Ident, OnceCell<VarId>),
    Variant {
        path: Path,
        fields: PatternFields,
        span: Span,
    },
}

pub enum PatternFields {
    Unit,
    Tuple(Vec<Pattern>),                     // 今回は Binding か Wildcard のみ
    Struct(Vec<(Ident, Pattern)>),
}
```

`MatchExpr` / `MatchStmt` は `IfExpr` / `IfStmt` の作りに揃える。

### 2. パーサ (`biwac_parser`, `biwac_novel_parser`)

**式のパーサは変えない。** `Foo::Bar(x)` は `FnCall` のまま、
`Foo::Bar { x = 1 }` は `Literal::Struct` のまま、`Foo::Bar` は `Variable` のままにする。
構文は曖昧でも名前は曖昧でないので、**分岐は lowering に置く**。

追加するのは、

- `consume_type_definition` の `KwEnum` 分岐
- `consume_match_expression_or_statement`
- `consume_pattern` (パターン位置なので `Foo::Bar { .. }` の `{` は曖昧でない)
- `consume_expression_or_statement` の `KwMatch` への dispatch

scrutinee は `consume_condition_expression` (構造体リテラル禁止) で読む。
名前が `condition` のままだと用途と合わないので `consume_scrutinee_expression`
などに改名するか、別名を足す。

**ノベルパーサの match は後回しにしてよい。**
scene の `{{ }}` の中は行指向で、`if` も
「`{` で終わる行 → 文の並び → `}` の行」という形でしか書けない。
match のアームは 1 行に収まらないので、行継続の判定
(`.`, `(`, `,` で終わったら継続) を拡張するか、
アームごとにブロックを強制する形に限定するかを決める必要がある。
まず通常コード側だけ通し、ノベル側は別に検討する。

### 3. 名前解決 (`biwac_name_resolver`)

#### 定義の収集 (`def_collector`)

- `TypeDef::Enum` に `TyDefId` を振り、`TyOrVal::Ty` として module tree に載せる。
  **struct と全く同じ扱いでよい**
- 各 variant に `VariantDefId` を振り、enum の `TyNameTree::children` に
  `AssocNameTreeItemKind::Variant` として載せる
- 同時に `VariantOwner` の表を作る

`TyNameTree` に印は付けない。名前解決の側で enum を struct と区別する必要が無い。

impl ブロックの収集は既存のまま。enum への `impl` は struct と同じ経路で通る。

#### パス解決

**enum のパス解決は struct と同じ経路をそのまま通る。**
`Color` は `ModuleNameTreeItem::Ty` として module tree に載っているので、
`resolve_path_in_module` → `resolve_path_in_ty` と辿る。
`Color::Red` も `Color::from_hex` も同じ `children` を引く。
違うのは返る `DefIdKind` が `Variant` か `Val` かだけである。

型エイリアス越し (`type C = Color; C::Red`) も、
`resolve_path_in_ty` が `alias_target` を辿る既存の仕組みで通る。

外部パッケージ側は `resolve_path_in_ext_ty` → `lookup_assoc` が
`ExternalChildKind::Variant` を返せるようにし、
`ext_child_ref_to_def_id_kind` に `Variant` の腕を足す。

#### lowering

ここが構文の曖昧性を解くところである。

| 入力                                                    | 判定              | 出力                   |
| ------------------------------------------------------- | ----------------- | ---------------------- |
| `lower_callee` が `DefIdKind::Variant`                  | tuple 形式の構築  | `Primary::VariantCtor` |
| `lower_literal` の `Literal::Struct` の path が variant | struct 形式の構築 | 同上                   |
| `lower_variable` が `DefIdKind::Variant`                | unit 形式の構築   | 同上                   |

`Callee::AssocFn` を足したときと同じ要領で、既存の分岐に腕を 1 つずつ足す。

**親 enum は `VariantOwner` の表から引く。** パスの途中のセグメントから
辿ろうとすると、import した `Red` 単体の形で辿れなくなる (前述)。

宣言の `shape` と書き方が食い違ったら (`Bar(X)` を `Bar { .. }` で作るなど)
名前解決の段でエラーにする。型推論まで持ち越さない。
`shape` は自パッケージなら HIR、外部なら `DiskVariantData` から引ける。

#### パターン内の束縛

アームごとに `VariableScope` を開き、`FnResolveCtx` が持つ
`next_var_id` から VarId を採番する
(スコープが独自にカウンタを持つと番号が衝突するのは以前直したとおり)。

同じアーム内での名前重複は `DuplicatedVariableName` で報告する。

### 4. HIR (`biwac_hir`)

```rust
pub enum TyDefKind {
    Struct(Box<StructDef>),
    Enum(Box<EnumDef>),               // コメントアウトを実装
    NativeTypeAlias(Box<NativeTypeAliasDef>),
}

pub struct EnumDef {
    pub name: Ident,
    /// 宣言順。添字がそのままタグの値になる。
    pub variants: Vec<VariantDef>,
    pub genargs: Vec<GenDefId>,
}

pub struct VariantDef {
    pub name: Ident,
    pub def_id: VariantDefId,
    pub shape: VariantShape,           // Unit / Tuple / Struct
    /// タプル形式なら `_0`, `_1` に正規化済み。宣言順。
    pub fields: Vec<(Ident, Ty)>,
}
```

`Hir` に `variant_owners: HashMap<VariantDefId, VariantOwner>` を足す。
`DefCollector` が作ったものをそのまま持ち上げる。
外部パッケージ分は `TyCtx::get_ty_impl` と同じ流儀で遅延ロードして混ぜる。

**enum は `TyDefKind` の腕なので、`DefinedTyImpl` の `vals`
(関連関数・メソッドの表) は struct と共通のまま使える。**
`impl Color { .. }` は今の経路で動く。

式側:

```rust
pub enum Primary {
    ...
    VariantCtor(VariantCtor),
    Match(MatchExpr),
}

pub struct VariantCtor {
    pub enum_def_id: TyDefId,
    pub variant: VariantDefId,
    pub index: u32,
    /// 宣言順に正規化した実引数。
    pub fields: Vec<(Ident, Expr)>,
    pub span: Span,
}
```

`alias_expansion` は式の中も歩くようになっている
(`Callee::AssocFn` の self 型のため)。
`VariantCtor` と `Match` の中も歩くように腕を足す。

### 5. 型推論 (`biwac_type_inferrer`)

#### 構築

`VariantCtor` の型は `Defined(enum_def_id, genargs)`。
genargs はフィールドの型と実引数を単一化して決める。
`Option::Some(x)` で `x: Image` なら `T := Image`。

`Character::new` に self 型を混ぜたのと同じ考え方で、
「宣言されたフィールドの型」と「渡された式の型」を順に `call_unify` すればよい。

エイリアス経由 (`type IntOpt = Option[Int]; IntOpt::Some(1)`) も、
`Callee::AssocFn` に入れた `call_site_self_ty_is_usable` と同じ判定で扱える。
`VariantCtor` にも呼び出し位置の型を持たせるかは実装時に決める。

#### match

- scrutinee の型は `Defined(enum, ..)` でなければならない
- 各アームのパターンのバリアントが、その enum のものであることを確認する
- 束縛変数の型は、単相化前のフィールドの型に enum の genargs を代入したもの
- 式形ならすべてのアームの型を単一化する。文形なら Void

#### 網羅性

ネストが無いので集合の被覆判定で済む。

- `_` があれば網羅
- 無ければ、出現したバリアントの集合が全バリアントと一致するか
- 同じバリアントが 2 回出たら「到達しないアーム」の警告 (エラーでもよい)

エラーは今回入れた `TyErrorReport` に乗せる。
不足しているバリアント名を並べて出す。

### 6. 依存メタデータ (`biwac_dependency_metadata`)

- `BIWAC_DEPENDENCY_METADATA_FORMAT_VERSION` を 5 → 6
- `DiskSymbolKind::Enum = 4`, `DiskSymbolKind::Variant = 5`
- `DiskEnumData { name, name_span, def_raw_code, def_span, genargs, variant_symbols, assoc_symbols }`
- `DiskVariantData { name, name_span, owner, index, shape, fields }`。
  `owner` は親 enum のシンボル番号、`index` は宣言順の添字 (= タグ)
- `SymbolBody` に 2 つの腕。現在 8 箇所で match している
- SVH: enum は「名前 + genargs + バリアント列 (順序が意味を持つので正準化しない)」。
  **バリアントの順序を変えるとタグが変わる**ので、順序は SVH に含めなければならない
- `symbol_mangling_info` に腕を足す (variant はマングル名を持たないが、
  診断で名前を引くのに要る)
- `get_ext_ty_impl` が enum を復元できるようにする。
  variant シンボルを辿って `TyDefKind::Enum` を組み立てる
- `lookup_assoc` が variant シンボルも返せるようにする
  (`ExternalChildKind::Variant` を足す)
- `lookup_child` は変更不要。enum 自体は `ExternalChildKind::Ty` のままである

### 7. MIR (`biwac_mir`, `biwac_mir_build`)

rustc と同じ形にする。

```rust
pub enum AggregateKind {
    Struct(TyDefId),
    Enum(TyDefId, u32),        // (enum, variant index)
}

pub enum Rvalue {
    Use(Operand),
    BinaryOp(BinOp, Operand, Operand),
    UnaryOp(UnOp, Operand),
    Aggregate(AggregateKind, Vec<(InternedIdent, Operand)>),   // 第 1 引数を差し替え
    Discriminant(Place),                                        // 追加
}

pub enum PlaceElem {
    Field(InternedIdent, Ty),
    Downcast(u32),             // 追加。バリアント番号
}
```

`Downcast` は必ず `Field` の直前に来る (`[Downcast(1), Field("_0", T)]`)。
`Field` が型を持っているので、backend は定義表を引かずに済む。

`match` の lowering は `if` とほぼ同じ:

1. scrutinee を local に評価する
2. `Discriminant` でタグを別 local に取る
3. `SwitchInt { discr, targets }` で各アームのブロックへ跳ぶ。
   `_` があればそれが `otherwise`、無ければ網羅済みなので
   最後のアームを `otherwise` にする
4. 各アームの先頭で、束縛ごとに
   `local = Use(Place { local: scrutinee, projection: [Downcast(i), Field(name)] })`
   を積む
5. 式形なら各アームの末尾で結果 local に代入し、合流ブロックへ `Goto`

`validate.rs` に「`Downcast` の直後は `Field` である」
「`Downcast` は enum の place にのみ現れる」の検査を足す。

### 8. MIR の codec

テキスト形式なので、`Aggregate` / `Discriminant` / `Downcast` の
encode と decode を足す。表記案:

```
_3 = Enum(ty#12, 1) { _0: _2 }
_4 = discriminant(_3)
_5 = ((_3 as variant#1)._0)
```

`encode.rs` / `decode.rs` の両方に対称に入れる。
`assets/tests/mir_fixture` に enum と match のフィクスチャを足して、
`round_trip` テストで往復を検証する。

### 9. 単相化 (`biwac_mir_transform`)

```rust
pub enum MonoTyDefKind {
    Struct { members: Vec<(InternedIdent, Ty)> },
    Enum { variants: Vec<MonoVariant> },        // 追加
    Native { code: String },
}

pub struct MonoVariant {
    pub name: InternedIdent,
    pub fields: Vec<(InternedIdent, Ty)>,       // 宣言順
}
```

`instantiate_ty` が enum のとき、全バリアントの全フィールドの型を
worklist に積む (どのバリアントが使われるか静的に絞らない。
`ref.cast` の対象型がすべて必要になるため)。

`Rvalue::Aggregate` の型置換に `AggregateKind::Enum` の腕を足す。
`PlaceElem::Downcast` は型を含まないので置換不要。

### 10. wasm codegen

- 型宣言: enum ごとに親型 1 つ + バリアント数だけ子型を rec グループに出す。
  名前は `$<mangled>.t{i}` と `$<mangled>.t{i}.v{n}`
- `wasm_ty` は enum に対して `(ref null $<parent>)` を返す
- `Aggregate(Enum(..))`: タグの定数を積み、フィールドを宣言順に積み、
  `struct.new $<child>`
- `Discriminant`: `struct.get $<parent> $tag`
- `Downcast` + `Field`: `ref.cast (ref $<child>)` → `struct.get $<child> $<field>`
- `SwitchInt` は既存のまま (`br_table` か if 連鎖)

現在の `aggregate_key` はメンバ名の集合から実体を絞っているが、
enum では `AggregateKind` が enum の `TyDefId` を持つので、
そちらから引ける。struct 側も同じように直せるなら直したい。

### 11. TypeScript codegen

TS 生成器は **HIR から直接**出力していて MIR を通らない。
wasm と実装が 2 系統になる点に注意。

- 型宣言: バリアントごとのオブジェクト型のユニオンとして `type` を出す
- `VariantCtor`: `{ tag: 1, _0: <expr> }` のオブジェクトリテラル
- `Match`: `switch (scrutinee.tag) { case 0: ... }`。
  式形は、既にある「ブロックを含む if 式」と同じ問題に当たる
  (`expression.rs` の `todo!()`)。
  一時変数に代入する形に落とす実装をここで入れることになる

`while` / ブロック文 / ブロックを含む if 式が
TS 生成器で `todo!()` のままなので、**tier 2 は enum と一緒にそこも埋めることになる**。
先に wasm を通し、TS は後追いにする。

---

## 実装の順序

各段でビルドが通り、テストが緑になる単位に切る。

1. **トークンと AST と パーサ**。enum 宣言と match をパースして AST を作るところまで。
   `biwac_parser` の単体テストで AST の形を検証する
2. **DefId と名前解決**。`VariantDefId` と `DefIdKind::Variant` の導入、
   variant の登録、`VariantOwner` 表、import の腕。
   enum は `DefIdKind::Ty` のままなので既存の型解決には手を入れない
3. **HIR と lowering**。variant 構築の 3 分岐と match の lowering。
   ここまでで「パースして HIR になる」
4. **型推論**。構築の型付け、match のアーム、網羅性検査
5. **MIR と codec**。`Downcast` / `Discriminant` / `AggregateKind`、
   `mir_fixture` の往復テスト
6. **単相化と wasm codegen**。ここで初めて動くものになる。
   `assets/tests/test1` に enum を足して `wasm_output` テストを通す
7. **std の `Option` を enum で書き直す**。native の `anyref` / `null` ハックを消す。
   `Option[Int]` が書けるようになったことを test1 で確認する
8. **TypeScript codegen**。合わせて `while` / ブロック文 / ブロック付き if 式の
   `todo!()` を埋める

6 まで来れば tier 1 として使える。7 が実際の受け入れ試験になる。

---

## 実装して変わったところ

方針から外れた点と、書いていなかった判断を記録しておく。

### バリアントの解決を型推論まで遅らせた

方針では lowering で親の enum とフィールドの並びを決めるつもりだったが、
**外部パッケージの enum は lowering の時点でまだ HIR に載っていない**。
`TyCtx` が `.biwameta` から遅延ロードする作りなので、
lowering には引く手段が無い。

そこで HIR の `VariantCtor` / `VariantPattern` に
`resolved: OnceCell<ResolvedVariant>` を持たせ、型推論が埋めることにした。
`MethodCall::def_id` と同じ流儀である。
自パッケージと外部で経路が分かれないのが利点である。

lowering が決めるのは「バリアントである」ことと「どう書かれたか」
(`shape` と、位置か名前か) までにとどめた。

### タプル形式のフィールド名はパーサが付ける

`_0`, `_1` を作るには interner が要るが、lowering は interner を持っていない。
パーサは持っているので、`VariantFieldsDecl::Tuple` の時点で
`Vec<(Ident, TypRepr)>` として名前を付けてしまう。

### `match` は 2 分岐の連鎖に落とす

MIR の `SwitchInt` は多分岐を表せるが、
**wasm 側の構造化変換 (`arch/wasm/structure.rs`) が 2 分岐しか扱えない**。
分岐表 (`br_table`) に落とす形へ広げるのはそれなりの手間なので、
`lower_match` が「タグと 1 つの値を比べる」形の連鎖を組むことにした。

あわせて、2 分岐の `SwitchInt` の意味を
「値 0 かどうか」から「その値と一致するか」に揃えた。
以前は `if` と `while` からしか作られず、値が必ず 0 だったので
比較を省いていた。

バリアントが増えたときの `br_table` 化は将来の最適化である。

### 集約の実体は代入先の型で決める

wasm の `aggregate_key` はフィールドの型から実体を絞っていたが、
**フィールドを持たないもの (unit バリアント) では絞れない**。
`Option[Position]::None` と `Option[CanvasObject]::None` が
どちらも「フィールドなし」で、先に見つかった方の子型を作ってしまう。

代入先の place の型を渡して、そこから実体を引くようにした。

### TypeScript は絞り込みに頼らない

判別可能なユニオンにしたので `if (c.__tag === 0)` で絞り込めるはずだが、
`let c: Color = Color::Rgb(..)` を出すと TypeScript が
**初期化式から `c` を `{ __tag: 1, .. }` に絞り込んでしまう**。
そのまま比較すると「重なりがない」と怒られる。

型の正しさは biwa 側で検査済みなので、生成コードは絞り込みに頼らない。

- タグの比較は `(c.__tag as number) === 0`
- payload の読み出しは `(c as any)._0`、
  受ける変数には推論結果の型を注釈する

`any` が出るのは読み出しの 1 箇所だけで、束縛された変数は正しい型を持つ。

### `.biwameta` はシンボルの採番順と本体の書き出し順が一致していなければならない

`push_body` は順に追加していくので、
Phase 2 で決めた `[struct][native type alias][enum][variant][assoc fn][top fn][mod]`
の順で本体を書かないと、シンボル番号が別の本体を指す。
実装中にここを踏んで、外部の enum が native type alias として読めてしまった。

### `std::panic::abort`

`Option::unwrap` の none 側で「戻らない」ことを表す必要が出た。
`!` 型も例外も無いので、戻り値がジェネリックな native 関数を置いた。
wasm では `unreachable`、TypeScript では `throw` になる。

---

## 落とし穴

**種別の分岐は `TyDefKind` に寄せる。** `DefIdKind` に `Enum` を作らないのは、
そこでの取りこぼしがコンパイルエラーにならず「型が見つからない」で
静かに落ちるためである。逆に `TyDefKind::Enum` を足すと網羅的な `match` が壊れ、
必要な箇所が全部エラーで出る。判定を書きたくなったら、
`DefIdKind` ではなく `TyDefId` から `TyDefKind` を引く側に置くこと。

**バリアントの順序が ABI である。** タグは宣言順の添字なので、
順序を入れ替えると生成物の意味が変わる。SVH に順序を含めること。
struct のメンバを名前順に正準化しているのと逆なので、間違えやすい。

**条件式の構造体リテラル禁止が enum にも効く。**
`if opt == Option::None { }` は書けない (`Option::None` は変数として読まれ、
`{` はブロックになる)。今の struct と同じ扱いだが、
enum では書きたくなる場面が増えるので、
専用の診断を出すか括弧を促すかを考えたほうがいい。

**バリアント名と関連関数名の衝突。** 同じ children マップに載るので
検査を足さないと後段で取り違える。

**`ref.cast` の失敗は trap になる。** 網羅性検査を通っていれば起きないはずだが、
`SwitchInt` の `otherwise` に落ちた先で誤った `Downcast` をすると
実行時に落ちる。MIR の validate で「`Downcast(i)` を含む place は、
その値のタグが `i` であるブロックにしか現れない」まで検査するのは重いので、
lowering 側で正しく組むことに依存する。

**ジェネリックな enum の単相化。** バリアントが使われていなくても
親型の子型は全部出す必要がある。`ref.cast` の対象型が定義されていないと
アセンブルできない。

**再帰的な enum。** `enum List[T] { Nil, Cons(T, List[T]) }` は
参照型なので WasmGC では自然に書けるが、単相化は止まらない
(`List[T]` の中に `List[T]` が出るだけなので実際には止まる。
`List[List[T]]` のような書き方をすると `INSTANCE_LIMIT` に当たる)。
現状の上限で守られる。

---

## 実装後の状態

- 両ターゲットで動く。`wat` でアセンブルでき、`wasmparser` の validate を通り、
  Node で実行して TypeScript 側と同じ値を返すことを確認した
- `std` の `Option` は enum で書き直した。
  native の `anyref` / `null` ハックは消え、**`Option[Int]` が書けるようになった**
- `Vec::get` と `Map::get` は Option の構築を biwa 側に移した。
  native は範囲・鍵の検査をしない取り出しだけを担う
- コンパイラのテストは 41 件すべて通る。
  `assets/tests` に自パッケージの enum、バリアントの直接 import、
  外部パッケージ (std) の enum を足してある

---

## 今回やらないこと

- `trait`
- ネストしたパターン、リテラル・範囲パターン、`|`、ガード、`..`
- `let` パターン、`if let` 相当
- ノベル `#` コード行での match (行継続の設計が要る)
- 明示的な判別子指定 (`Red = 1`)
- struct に対する分解パターン

---

## 参考にした確認結果

- WasmGC の部分型 (`sub` / `ref.cast` / `ref.test` / 親型経由の `struct.get`) は
  `wat 1.258` でアセンブルでき、`wasmparser 0.258` の validate を通り、
  Node v22.22.3 で正しく動く
- MIR の `SwitchInt` は既に存在し、コメントに「将来の `match` もこれになる」とある
- `TyDefKind::Enum` と `TypeDef::Enum` は AST / HIR ともにコメントアウトで場所が空けてある
- `TyDefKind::Struct` に触れている箇所は 18、`SymbolBody::Struct` は 8。
  どちらも網羅的な `match` なので、腕を足せばコンパイルエラーで洗い出せる
- `DefIdKind::Ty` に触れている箇所は 12 あるが、その多くは
  `let ... else { return None }` や `_ => None` の形で、
  腕を足しても**コンパイルエラーにならない**。ここに enum を分けてはいけない
- `.biwameta` の形式バージョンは現在 5 で、不一致は再取得を促す形になっている
