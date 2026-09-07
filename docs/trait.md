# trait

`docs/biwa-lang-type-system.md` の「型の振る舞い trait」を実装するための方針。

`biwac_hir` / `biwac_name_resolver` / 新設の `biwac_trait_solver` に置かれたメモとモックコードを
出発点にして、既存のコードベースに落とし込める形まで具体化したものである。
モックと結論が違うところは、理由を添えて **モックとの差分** に書いた。

enum のときと同じく、まず全体を決めてから段を切る。

## 決めたこと

|                             |                                                                                                             |
| --------------------------- | ----------------------------------------------------------------------------------------------------------- |
| 段階                        | **第 1 段: 具体型に対する trait** → **第 2 段: ジェネリクスの trait 制限**。第 1 段だけで両ターゲットが動く |
| `DefIdKind`                 | 増えるのは `Trait(TraitDefId)`。第 2 段で `TraitAssoc(TraitAssocDefId)` を足す                              |
| trait impl の項目の置き場所 | 型の直接 impl と**同じ** `DefinedTyImpl::vals`。`trait_of: Option<TraitDefId>` で区別する                   |
| 名前ツリー                  | trait impl の項目は `TyNameTree::children` に**載せない**。import 必須の規則がそこで効く                    |
| 名前の衝突                  | ある型に対する関連名 (直接 impl・trait impl・バリアント) は**すべて一意**でなければならない                 |
| blanket impl                | 入れない (`impl[T] T: Foo` は禁止)。孤児則だけで重複検査が閉じるのはこの制限のおかげである                  |
| 既定実装                    | 入れない。trait の項目はシグニチャだけ                                                                      |
| trait solver の入口         | Rust の trait `TraitEnv` を受け取る。呼ぶ側 (name resolver / type inferrer) がそれぞれ実装する              |
| 第 2 段の解決先             | wasm は単相化で決める。TypeScript は witness (辞書) を引数で渡す                                            |
| `.biwameta`                 | 版を 6 → 7。`Trait` / `TraitAssoc` / `TraitImpl` のシンボル種別が増える                                     |

## 言語仕様の確認

`docs/biwa-lang-type-system.md` に書かれていることを、実装が判断すべき形に直しておく。

```biwa
trait Gyao {
  fn gyao(self) -> Gyoe;

  fn guee(aaa: Aaa) -> Self;
}

impl Nyoee: Gyao {
  fn gyao(self) -> Gyoe { Gyoe::new(self.nyoe.len()) }
  fn guee(aaa: Aaa) -> Self { Nyoee { nyoe = aaa.xxx() } }
}
```

- trait の項目は、`self` を取るメソッドか、戻り値に `Self` を含む関連関数のいずれかである
- impl できるのは、**trait 自身か対象の型のいずれかが自パッケージで定義されている**場合 (孤児則)
- trait 経由の関連関数・メソッドを使う箇所では、その trait が `import` されていなければならない
- 直接の impl に見つからなかったときに**初めて** trait を探す

最後の 2 つが効いている先は 2 箇所しかない。

- `<type>::foo()` — パスの解決。名前解決の段で決まる (biwa のパス解決は型推論とブートストラップしない)
- `<expr>.foo()` — メソッドの解決。型推論の段で決まる

どちらも「直接の実装を探す」→「無ければ trait へ」という同じ順序なので、
2 段目を `biwac_trait_solver` に切り出す、というのがメモの方針である。これはそのまま採る。

## 中心の判断

### 1. 段を 2 つに割る

trait の機能は、**呼び先が静的に決まるかどうか**で難易度がはっきり分かれる。

```biwa
// 第 1 段。呼び先は Nyoee::gyao で確定する
let g = nyoee.gyao();

// 第 2 段。T が何か分からないと呼び先が決まらない
impl[T: Gyao] Bbb[T] {
  fn new(aaa: Aaa) -> Self {
    let t = T::guee(aaa);
    let g = t.gyao();
    ...
  }
}
```

第 1 段は、**バックエンドに一切手を入れなくてよい**。
trait impl の中の関数は、名前解決の見え方が違うだけで、実体は今までの関連関数と何も変わらない。
MIR も `Callee::Direct` のままで、単相化も codegen もそのまま通る。

第 2 段は、呼び先が未確定のまま MIR まで運ぶ必要があり、
wasm と TypeScript で解き方が変わる (後述)。ここが工数のほとんどを占める。

第 1 段だけでも `Display` や `Eq` のような「型ごとに実装を差し替える」用途は満たせるので、
先に動かして std を書き換えてから第 2 段に進むのがよい。

### 2. trait impl の項目は、型の直接 impl と同じ場所に置く

置き場所の候補は 3 つあった。

| 案                                                                                | 問題                                                                                                                                         |
| --------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `Hir::vals` に入れる                                                              | `get_value_signature` の外部パッケージ経路が `impl_self_ty` を持たないトップレベル関数を前提にしている。`.biwameta` の読み書きも分岐が増える |
| `DefinedTyImpl` に新しい `trait_impls: Vec<TyTraitImpl>` を作り、実体もそこに持つ | シグニチャ取得・マングリング・TS の出力・メタデータの書き出しがすべて二重化する。触る箇所が広い                                              |
| **`DefinedTyImpl::vals` に入れ、印で区別する**                                    | 印を付け忘れると import 無しで見えてしまう                                                                                                   |

3 つめを採る。`TyValImplGenargsContentPair` に 1 フィールド増やすだけで済む。

```rust
pub struct TyValImplGenargsContentPair {
    pub impl_block_genargs: HashMap<InternedIdent, (LocalGenDefId, Span)>,
    pub genargs: Vec<Ty>,
    pub val_content: AssocValDefKind,
    /// trait impl の項目なら、その trait。
    /// 直接の impl なら `None`。
    ///
    /// 直接の探索 (`TyCtx::get_method_def_id`) はこれが `None` のものだけを見る。
    /// trait 越しの探索はスコープにある trait と突き合わせる。
    pub trait_of: Option<TraitDefId>,
}
```

これで以下が**全部そのまま動く**。

- `TyCtx::get_value_signature` — `assoc_val_map` 経由の既存の経路に乗る
- `Mangler::get_value_ident` / `impl_self_ty_of_assoc` — 同上
- TypeScript の出力 — `hir.tys` を歩いて `vals` を出しているだけなので変更なし
- `.biwameta` の assoc fn の書き出し — `trait_of` を 1 つ足すだけ
- MIR の構築と単相化 — 関連関数と区別が要らない

代わりに、**`trait_of` を渡し忘れると trait の項目が import 無しで見えてしまう**。
これは enum のときに `DefIdKind` で警戒したのと同じ「黙って通る」種類の失敗である。
そこで `register_impl_val` の引数はデフォルト値を持たせず、
呼び出し側 6 箇所すべてに明示的に書かせる (省略できないようにする)。

### 3. ある型に対する関連名はすべて一意

Rust は `<T as Trait>::foo` で曖昧さを解けるが、biwa は
「型と値で名前空間を分けない単一の木」という前提で名前解決を組んである。
その前提を崩さないために、**1 つの型にぶら下がる名前は、
直接 impl・trait impl・enum のバリアントを通じて一意**とする。

```biwa
impl Foo { fn bar(self) -> Int { .. } }
impl Foo: Baz { fn bar(self) -> Int { .. } }   // エラー: Foo に bar が 2 つ
```

同一パッケージ内であればこれは検査できる (`AssocNameTree::register_assoc` と同じ理屈)。
パッケージを跨ぐと検査できない — A が `String` に `foo` を生やし、B も `String` に `foo` を生やし、
C が両方 import する、という状況は起こりうる。これは C 側で
「`foo` が 2 つの trait から来ていて絞れない」という**使用箇所のエラー**にする。
`solve_method` が候補を 1 つに絞れなかった場合そのままエラーになるので、追加の仕組みは要らない。

将来 `[T as Gyao]::guee()` 構文 (型システムの文書のコメントに既に出ている) を入れれば
この制限は緩められる。今回は入れない。

### 4. blanket impl を入れない

`impl[T] T: Foo { .. }` を禁じる。理由は重複検査が閉じるからである。

孤児則により、`impl Ty: Trait` を書けるのは `Ty` のパッケージか `Trait` のパッケージだけである。
`Ty` のパッケージが `Trait` のパッケージを知っているか、その逆かのどちらかは必ず成り立つ…
とは**限らない**。A が `trait Foo` を、B が `struct Bar` を定義し、
両方が `impl Bar: Foo` を書ける状況は無い (B は A に依存していれば書けるが、A は B を知らない)。
つまり **重複しうる impl は必ず同じパッケージの中にある**。同一パッケージ内なら全部見えるので検査できる。

blanket impl を許すと、A の `impl[T] T: Foo` と B の `impl Bar: Foo` が衝突しうる。
A は B を知らないので検査できない。Rust がここでコヒーレンス規則の重い機械を必要とする理由である。
今回は入れない。

### 5. trait solver は `TraitEnv` を受け取る

モックの署名はこうなっている。

```rust
pub fn solve_assoc(
    ty: &Ty,
    assoc: &InternedIdent,
    trait_impls: &HashMap<TyDefId, TyTraitImplList>,
    gen_satisfying_traits: &HashMap<GenDefId, TraitCondList>,
    local_gen_satisfying_traits: &HashMap<LocalGenDefId, TraitCondList>,
) -> Result<ValDefId, TraitSolveError>
```

これは 2 つ問題がある。

1. **外部パッケージに届かない。** 呼ぶ側は 2 つあり、
   name resolver は `NameTree::ext_pkg_data` から、type inferrer は `TyCtx` の遅延キャッシュから
   `.biwameta` を引く。既に出来上がった `HashMap` を渡す形だと、
   呼ぶ前に全部読み込んでおくしかない。enum のときに「外部の enum は lowering の時点で HIR に載っていない」で
   踏んだのと同じ形である
2. **スコープにある trait が引数に無い。** import 規則の判断ができない

そこで、solver 側に環境のインタフェースを置く。

```rust
// biwac_trait_solver

/// solver が環境に問い合わせること。
/// 自パッケージと外部パッケージの区別は実装側が吸収する。
pub trait TraitEnv {
    /// この型に対する trait impl。自パッケージ・外部パッケージを問わない。
    fn trait_impls_of(&self, ty: TyDefId) -> Vec<TraitImplRef<'_>>;

    /// trait の宣言 (項目の名前とシグニチャ)。
    fn trait_def(&self, def_id: TraitDefId) -> Option<&TraitDef>;

    /// このモジュールで使える trait。宣言と import の和。
    fn traits_in_scope(&self, module: ModId) -> &[TraitDefId];

    /// ジェネリック引数に付いた制限 (第 2 段)。
    fn bounds_of_gen(&self, def_id: GenDefId) -> &[TraitCond];
    fn bounds_of_local_gen(&self, def_id: LocalGenDefId) -> &[TraitCond];
}
```

`biwac_trait_solver` の依存は `biwac_base` / `biwac_span` / `biwac_hir` のまま据え置ける
(`biwac_dependency_metadata` を足さずに済む)。
name resolver と type inferrer がそれぞれ `TraitEnv` を実装する。
`biwac_trait_solver` は name resolver に依存してはならない (循環する)。

### 6. `DefIdKind` に増えるのは `Trait` だけ (第 1 段)

enum のときは「`Ty` に含めるべきで、増やすべきでない」だった。
今回は逆で、**`Trait` は増やさなければならない**。trait はパスの解決先になるが、
型でも値でもないので `Ty` にも `Val` にも入らないからである。

```biwa
import package::gyao::Gyao;   // パスが trait に解決される
impl Nyoee: Gyao { .. }       //           〃
struct Foo[T: Gyao] { .. }    //           〃 (第 2 段)
```

`DefIdKind` を触っている箇所は **`biwac_name_resolver` の 10 ファイル・56 箇所しかない**
(他のクレートには 1 箇所も出てこない)。網羅マッチは
`lowering/mod.rs` の `ty_def_id_kind_from_def_id_kind` で、ここは
「型が要る位置に trait が来た」を専用のエラーにする場所なので、
コンパイルエラーで気づける。残りは `Ok(_) => None` 系だが、
そこに trait が流れ込んだ場合は「パスが解決できない」ではなく
「trait は値として使えない」と言いたいので、下の一覧の箇所には明示的な腕を書く。

| 場所                                                  | trait が来たときに出したいエラー                      |
| ----------------------------------------------------- | ----------------------------------------------------- |
| `lowering/mod.rs::ty_def_id_kind_from_def_id_kind`    | `TypeNotFoundTraitFound` (網羅マッチなので必ず気づく) |
| `lowering/expressions.rs::callee_from_path`           | `TraitIsNotCallable`                                  |
| `lowering/expressions.rs` の `Primary::Variable` 分岐 | 同上                                                  |
| `resolving/symbols/patterns.rs`                       | `VariantExpected` に寄せる                            |

`TraitAssocDefId` は第 2 段で追加する。

## DefId の設計

### `TraitDefId`

既にモックで `impl_typed_def_id!(TraitDefId)` が入っている。そのまま使う。

### `TraitAssocDefId` (第 2 段)

trait の項目そのものに振る id。
`T::guee()` のように**具体の実装がまだ決まらない**呼び先を表すために要る。

モックのコメントはこう問うている。

> DefIdKind に 具体の ValDefId までは未解決だが TraitDefId は分かっているという状態のマークを増やすべき?
> trait 定義で 各 val に TraitAssocDefId を振ってしまい、それをもたせるというのが良いか

後者を採る。`VariantDefId` とまったく同じ形になる。

- 親の trait と項目の添字は id からは分からないので、逆引き表を持つ
  (`Hir::trait_assoc_owners: HashMap<TraitAssocDefId, TraitAssocOwner>`、
  外部パッケージは `.biwameta` から遅延で引く)
- 呼び先を運ぶ側は `TraitAssocDefId` 1 つだけ持てばよい

```rust
pub struct TraitAssocOwner {
    pub trait_def_id: TraitDefId,
    /// 宣言順の添字。
    pub index: u32,
}
```

`Callee` はこうなる。`AssocFn` と形を揃えてある。

```rust
pub enum Callee {
    Var(VarId),
    Fn(ValDefId),
    AssocFn { def_id: ValDefId, self_ty: Ty },
    /// trait 越しで、まだ実装が決まっていない呼び出し (`T::guee(..)`)。
    ///
    /// `self_ty` は呼び出し位置に書かれた型で、
    /// 第 2 段では `TyKind::Gen` / `TyKind::LocGen` になる。
    /// 単相化 (wasm) または witness (TypeScript) がここで実装を決める。
    TraitAssocFn { assoc: TraitAssocDefId, self_ty: Ty },
}
```

メソッド側も `MethodCall::def_id: OnceCell<ValDefId>` では足りなくなるので、

```rust
pub enum MethodTarget {
    Direct(ValDefId),
    Trait { assoc: TraitAssocDefId, self_ty: Ty },
}
```

に変える。第 1 段の間は `Direct` しか作らないので、
`MethodTarget` を先に導入しておいて第 2 段で腕を足す形にすると差分が小さい。

## 名前解決 (`biwac_name_resolver`)

### 収集の順序

`def_collector.rs` に置かれたメモはこうなっている。

```rust
// TODO: 一番最初に trait に対して DefId だけ振る
// 型の定義にも trait は出現する
// ( `struct Foo[T: A] {...}` のように `trait A` でジェネリック引数を制限する場合)
```

**この前パスは要らない。** `trait` は `Globals` の一種として
`collect_in_module` の同じ走査で `struct` や `enum` と並べて採番すればよい。

制限に書かれた `A` が解決されるのは Step 2 以降で、
そのとき全シンボルの `DefId` は既に振り終わっている。
`struct Foo { x: Bar }` の `Bar` が後方の宣言でもよいのと同じ理屈である。
`DefCollector` 自身は制限の中身を見ないので、順序を作る必要が無い。

**モックとの差分。** 前パスを足すと採番順が変わり、
`.biwameta` のシンボル索引が動く。理由が無いのに動かすのは避けたい。

### trait impl の収集 — Step 4

`collect()` の現在の 3 段のあとに 1 段足す。

```
Step 1: 非 impl シンボルの採番 + モジュールの名前ツリー   ← trait と trait の項目もここ
Step 2: 型エイリアスの右辺の解決
Step 3: impl ブロックのシンボルを canonical な型の下に収集
Step 4: trait impl ブロックの収集                          ← 新規 (collect_trait_impls_in_module)
```

Step 3 と Step 4 を分けるのは、Step 4 の名前衝突検査が
Step 3 の結果 (その型の直接 impl の名前一覧) を必要とするからである。

`ImplBlock` に trait が書かれていれば Step 3 は素通りし、Step 4 が拾う。

```rust
fn collect_trait_impls_in_module(&mut self, ..) -> Result<(), Vec<ResolveError>> {
    // impl_block.trait_repr を解決 → TraitDefId と trait_genargs
    // impl_block.self_typ を解決  → canonical TyDefId と ty_genargs
    //
    // 1. 孤児則
    // 2. 重複
    // 3. 名前の衝突
    // 4. 項目が trait の宣言と一致しているか
    // 5. 項目に ValDefId を振り、trait impl 索引に登録する
}
```

#### 1. 孤児則

```rust
if !ty_def_id.pkg().is_self() && !trait_def_id.pkg().is_self() {
    errors.push(ResolveError::ForeignTraitImpl { .. });
}
```

プリミティブ型への impl が std だけ許されるのは既存の規則
(`collect_impls_in_module` に同じ趣旨の TODO が既にある) なので、
そちらの検査と同じ場所・同じ条件で書く。

#### 2. 重複

同じ `(canonical TyDefId, TraitDefId)` の組で、
`ty_genargs` と `trait_genargs` が重なる impl が 2 つあればエラー。
判定は既存の `TyKind::is_duplicated_for_impl_genarg` をそのまま使う
(`Gen` / `LocGen` はどちらか一方にでも出ていれば重複、という現行の扱いで足りる)。

#### 3. 名前の衝突

判断 3 のとおり、その型にぶら下がる関連名は全部一意である。
`TyNameTree::children` を引いて、既に何かがあればエラーにする。
`AssocNameTree::register_assoc` を再利用したいところだが、
**trait impl の項目を `children` に登録してはならない** (import 規則が効かなくなる) ので、
検査だけ行って登録はしない。

#### 4. 項目の一致

trait が宣言した項目がすべて実装されているか、
余分な項目が無いか、シグニチャが一致しているかを見る。
シグニチャの一致は「`Self` を impl 対象の型に、
trait のジェネリック引数を `trait_genargs` に置き換えたうえで等しいこと」である。
置換は既存の `Ty::embody_by_gen_ty_id` が使える。

### スコープにある trait

モジュールごとに以下の和を持つ。

- そのモジュールで宣言された trait
- そのモジュールの `import` が trait に解決されたもの

`ModuleResolveCtx` は既に `imports: HashMap<InternedIdent, &Path>` を持っているので、
解決してから `DefIdKind::Trait` のものを拾えばよい。

型推論の段ではモジュールの文脈が失われているので、HIR に持ち越す。

```rust
pub struct Hir {
    ...
    /// モジュールごとの、使える trait。
    ///
    /// 型推論のメソッド解決が import 規則を判断するのに要る。
    /// 自パッケージのモジュールだけでよい
    /// (外部パッケージのコードは既に解決済みである)。
    pub trait_scopes: HashMap<ModId, Vec<TraitDefId>>,
}
```

推論の側は、いま見ている関数の `FnDef::name.span.module()` で `ModId` を得る。

### パスの解決 — フォールバック

`resolve_path_in_ty` (`resolving/context/module_level.rs`) の
`children.get(&segment.ident.id)` が空振りしたときに solver を呼ぶ。

```rust
match children.get(&segment.ident.id) {
    Some(assoc_tree) => { /* 既存 */ }
    None => {
        // ここで初めて trait を探す
        match trait_solver::solve_assoc(&self_ty, &segment.ident.id, env) {
            Ok(def_id) => { /* DefIdKind::Val として書き込む */ }
            Err(e) => { /* 解決失敗 */ }
        }
    }
}
```

外部パッケージの型を辿る `resolve_path_in_ext_ty` も同じ扱いにする。
`lookup_assoc` が `None` を返したらフォールバックする。

第 1 段では、返るのは常に**具体の `ValDefId`** である。
第 2 段で `T::guee()` の形が入ってきたら
`DefIdKind::TraitAssoc(TraitAssocDefId)` も返りうるようになる。

## HIR (`biwac_hir`)

モックの `TraitDef` / `TraitCond` / `TraitCondList` / `TyTraitImpl` / `TyTraitImplList` を土台に、
置き場所の判断 (判断 2) に合わせて整理する。

```rust
pub struct TraitDef {
    pub name: Ident,
    /// 宣言順。添字が `TraitAssocOwner::index` になるので並べ替えてはならない。
    pub items: Vec<TraitItemDef>,
    pub genargs: Vec<GenDefId>,
}

pub struct TraitItemDef {
    pub name: Ident,
    pub def_id: TraitAssocDefId,
    pub signature: FnSignature,
}

/// ある型に対する 1 つの trait impl。
///
/// 項目の実体は `DefinedTyImpl::vals` にあり、ここは索引でしかない。
pub struct TyTraitImpl {
    pub trait_def_id: TraitDefId,
    pub impl_block_genargs: HashMap<InternedIdent, (LocalGenDefId, Span)>,
    pub ty_genargs: Vec<Ty>,
    pub trait_genargs: Vec<Ty>,
    pub vals: HashMap<InternedIdent, ValDefId>,
}
```

`Hir` に増えるもの。

```rust
pub struct Hir {
    ...
    pub traits: HashMap<TraitDefId, TraitDef>,
    pub trait_assoc_owners: HashMap<TraitAssocDefId, TraitAssocOwner>,   // 第 2 段
    pub trait_scopes: HashMap<ModId, Vec<TraitDefId>>,
}
```

`DefinedTyImpl` に増えるもの。

```rust
pub struct DefinedTyImpl {
    pub ty_content: Option<TyDefKind>,
    pub vals: HashMap<InternedIdent, TyValImplList>,
    /// この型に対する trait impl の索引。
    pub trait_impls: Vec<TyTraitImpl>,
}
```

`trait_assoc_owners` を `Hir::new` の中で `traits` から作るのは、
`variant_owners` を `tys` から作っているのと同じにする (外から渡さない)。

**モックとの差分。** モックは `TyTraitImpl::vals: HashMap<InternedIdent, ValDefId>` に
「実体は Hir.vals に格納?」というコメントが付いていた。
実体は `DefinedTyImpl::vals` に置き、ここは索引に留める (判断 2)。

**モックとの差分。** `TraitDef::fns: HashMap<InternedIdent, FnSignature>` を
`items: Vec<TraitItemDef>` に変えた。`TraitAssocDefId` を振る以上、
宣言順が `.biwameta` に出るので `HashMap` では順序が定まらない。
enum のバリアントで踏んだのと同じ形である。

## trait solver (`biwac_trait_solver`)

### `solve_assoc` / `solve_method`

やることは 2 つとも同じで、入口が違うだけである。

```rust
pub fn solve_assoc<E: TraitEnv>(
    self_ty: &Ty,
    name: InternedIdent,
    module: ModId,
    env: &E,
) -> Result<Solved, TraitSolveError>;

pub fn solve_method<E: TraitEnv>(
    receiver_ty: &Ty,
    name: InternedIdent,
    module: ModId,
    env: &E,
) -> Result<Solved, TraitSolveError>;

pub enum Solved {
    /// 実装が確定した。
    Impl(ValDefId),
    /// 型がジェネリック引数なので、実装は単相化まで決まらない (第 2 段)。
    Deferred(TraitAssocDefId),
}
```

手順はメモに書かれたとおり。

1. `self_ty` が満たす trait を集める
   - 具体の型 → `env.trait_impls_of(def_id)`
   - `Gen` / `LocGen` → `env.bounds_of_gen` / `bounds_of_local_gen` (第 2 段)
2. `env.traits_in_scope(module)` と `TraitDefId` で積を取る
3. 各 trait の項目名から `name` に一致するものを集める
4. `ty_genargs` が `self_ty` に適合するものに絞る
5. ちょうど 1 つ残れば成功。0 なら `NotFound`、2 つ以上なら `Ambiguous`

4 の適合判定は、既存の直接 impl の探索
(`TyCtx::get_method_def_id`) と同じく `is_duplicated_for_impl_genarg` を使う。
重複が禁じられているので、重なるものは高々 1 つしかない。

`Gen` / `LocGen` の枝が `Solved::Deferred` を返すのは、
モックのコメントが指摘しているとおり
「名前解決・型推論としては問題ないと返せれば十分」だからである。

### `TyKind::Fn` と `TyKind::Infer`

モックは `todo!()` で置いてある。分けて扱う。

- `TyKind::Infer` — solver に来る前に呼ぶ側で弾く。
  型推論は `TyError::InsufficientContext` を既に持っており、そちらの語彙で言うべきである。
  solver 側は `debug_assert` に留める
- `TyKind::Fn` — 関数型への impl は今回入れない。`TraitSolveError::NotImplementable` を返す

### `TraitSolveError`

```rust
pub enum TraitSolveError {
    /// スコープにある trait のどれにも見つからなかった。
    NotFound,
    /// 複数の trait が同じ名前を提供していて絞れない。
    Ambiguous { candidates: Vec<TraitDefId> },
    /// スコープに無い trait には実装があった。import を促す。
    NotInScope { candidates: Vec<TraitDefId> },
    /// この型には trait を実装できない。
    NotImplementable,
}
```

`NotInScope` を分けておくと
「`Gyao` を import すると `gyao` が使えます」と言えるので、入れておく価値がある。

## 型推論 (`biwac_type_inferrer`)

### メソッド解決のフォールバック

`TyCtx::get_method_def_id` は現在こう終わっている。

```rust
match matched.as_slice() {
    [id] => Ok(*id),
    [] => Err(not_found()),
    _ => panic!("compiler bug: duplicated associated implementation registered"),
}
```

`[]` の枝を solver へのフォールバックにする。あわせて、
候補を集めるループが `trait_of.is_none()` のものだけを見るようにする
(そうしないと import 無しで trait の項目が直接見えてしまう)。

`TyCtx` が `TraitEnv` を実装する。外部パッケージの trait impl は
`get_ty_impl` と同じ遅延ロードの仕組みに乗せる。

### 制限の検査 (第 2 段)

`impl[T: Gyao] Bbb[T]` に対して `Bbb::new[Nyoee](..)` と呼んだとき、
`Nyoee` が `Gyao` を満たすかを確かめる必要がある。

これは `call_unify` で `LocalGenDefId → Ty` の割り当てが決まった直後に行える。
既に `CallCtx::gen_assigns` として持っているので、そこを見て
制限のある引数それぞれについて `solve` を回す。

### witness の記録 (第 2 段)

制限を満たすと分かっただけでは足りず、**どの impl が満たしたのか**を残す必要がある。
wasm は単相化でそれを使い、TypeScript は実行時に渡す。

```rust
pub enum ImplSource {
    /// 具体の impl が満たした。
    Impl { ty: TyDefId, trait_def_id: TraitDefId },
    /// 呼び出し元自身の制限が満たした (自分の witness をそのまま渡す)。
    Param { param: LocalGenDefId, index: usize },
}

pub struct FnDef {
    ...
    /// 呼び出し式ごとの、呼び先の各制限を満たした根拠。
    /// `call_genargs` と同じ並び。
    pub call_witnesses: HashMap<ExprId, Vec<ImplSource>>,
}
```

`call_genargs` の隣に置くのは意図的である。
どちらも「推論の途中でしか計算されない、単相化が要る情報」で、寿命も使い道も同じである。
rustc の `ImplSource::{UserDefined, Param}` と同じ形にしてある。

## 依存メタデータ (`biwac_dependency_metadata`)

版を **6 → 7** に上げる。既存の `.biwa_build` は全部無効になる。

### シンボル種別

```rust
pub enum DiskSymbolKind {
    Mod = 0, Struct = 1, Fn = 2, NativeTypeAlias = 3, Enum = 4, Variant = 5,
    Trait = 6,
    /// trait が宣言した項目。`TraitAssocDefId` を組み立てるのに
    /// シンボル番号が要るので、独立したシンボルにする (Variant と同じ理由)。
    TraitAssoc = 7,
    /// 1 つの trait impl ブロック。
    TraitImpl = 8,
}
```

```rust
pub struct DiskTraitData {
    pub name, pub name_span, pub def_raw_code, pub def_span,
    pub genargs: DiskVec<DiskGenArg>,
    pub item_symbols: DiskVec<DiskSymbolIndex>,
}

pub struct DiskTraitAssocData {
    pub owner: DiskSymbolIndex,
    pub index: u32,
    pub fn_data: DiskFnData,
}

pub struct DiskTraitImplData {
    pub self_ty: DiskTy,                         // ty_genargs を含む
    pub trait_sym: DiskSymbolIndex,
    pub trait_genargs: DiskVec<DiskTy>,
    pub item_symbols: DiskVec<DiskSymbolIndex>,  // 実体は Fn シンボル
}
```

`DiskFnData` には 1 つ足す。既存の `impl_self_ty: DiskVec<DiskTy>` と同じ
「0 個か 1 個で Option を表す」書き方に揃える。

```rust
    /// trait impl の項目なら、その trait のシンボル。直接の impl なら空。
    pub trait_of: DiskVec<DiskSymbolIndex>,
```

### trait impl をどう見つけるか

型のシンボルから辿れるとは限らない。
A が `trait Foo` を定義して B の `String` に実装した場合、
その impl は A のメタデータにあり、B の `String` シンボルからは参照できない。

なので、ファイルヘッダに**パッケージ内の全 `TraitImpl` シンボルの一覧**を持たせ、
読み込み側は直接依存の分をまとめて先読みして
`HashMap<TyDefId, Vec<TyTraitImpl>>` を組む。

`.biwameta` は基本的に遅延読みだが、trait impl はパッケージあたり数が少ないので
先読みしてよい。むしろ「フォールバックのたびに全依存を走査する」方が高くつく。

### 採番順に注意

enum のときに踏んだところである。
Phase 2 のシンボル採番順と、Phase 4 の `push_body` の呼び出し順は
**一致していなければならない**。今の順は

```
[struct] [native type alias] [enum] [variant] [assoc fn] [top fn] [mod]
```

で、ここに `trait` / `trait assoc` / `trait impl` を挿す。
**両方の場所に、同じ位置に**入れること。片方だけ直すと
シンボル番号が別の本体を指し、外部の enum が native type alias として読めるような壊れ方をする。

## マングリング (`biwac_generator/src/mangle.rs`)

trait impl の項目は、対象の型の下にぶら下がるだけでは**衝突しうる**。

```
package a:  trait Foo { fn bar(self) -> Int; }   impl std::String: Foo { .. }
package b:  trait Baz { fn bar(self) -> Int; }   impl std::String: Baz { .. }
```

どちらも `_ZN3std6String3barE` になってしまう。trait を成分として挟む。

```
<path>  ::= <pkg> <mod>* <name>                     トップレベル関数 / 型定義
          | <owner> <name>                          関連関数 / メソッド
          | <owner> "R" <trait-path> <name>         trait impl の項目
```

`R` は今の文法で未使用の大文字で、名前の長さ前置 (数字始まり) とも
型タグ (`i` `f` `b` `v` `T` `N` `F` `I` `E`) とも衝突しない。

`trait_of` を見るだけで分岐できるので、`get_value_mangled` に 1 本枝を足すだけである。

## バックエンド

### 第 1 段 — 変更なし

trait impl の中の関数は、名前解決の見え方が違うだけで、
HIR から先は今までの関連関数とまったく同じである。

- MIR は `Callee::Direct`
- 単相化はそのまま通る
- wasm も TypeScript も `hir.tys` / MonoMir を歩くだけなので手を入れる必要がない

マングル名だけが変わる (上記)。

### 第 2 段 — wasm

単相化で解く。rustc の `Instance::resolve` と同じ形である。

`monomorphize.rs` の `subst_terminator` に腕を足す。

```rust
Callee::TraitAssocFn { assoc, self_ty } => {
    // subst で self_ty を具体化する
    let concrete = self.subst_ty(self_ty, subst);
    // trait impl 表から実装を引く
    let def_id = self.resolve_trait_assoc(*assoc, &concrete)?;
    self.fn_worklist.push(InstanceKey::new(def_id, genargs.clone()));
    Callee::Direct { def_id, genargs }
}
```

`subst` は呼び出し元の実体化で決まっているので、この時点で `self_ty` は必ず具体である
(具体でなければ既存の `UnresolvedGenericArg` と同じ扱いにする)。
`call_witnesses` を使えば表を引かずに済むが、
`subst` から引き直す方が経路が 1 本で済むので、まずは引き直す形でよい。

**実行時のコストはゼロ。** 出てくるのは普通の直接呼び出しである。

### 第 2 段 — TypeScript

TypeScript ターゲットは HIR から直接出していて、**単相化しない**。
ジェネリック関数は 1 回だけ出力され、`T` は TS のジェネリクスになる。
実行時に `T` は消えているので、`T::guee()` の呼び先は型からは決められない。

そこで witness (辞書) を引数で渡す。GHC の辞書渡しと同じである。

```ts
// trait Gyao の形
type Gyao$w<Self> = {
  gyao: (self: Self) => Gyoe;
  guee: (aaa: Aaa) => Self;
};

// impl Nyoee: Gyao ごとに 1 つの定数
const Gyao$for$_ZN3pkg5NyoeeE: Gyao$w<_ZN3pkg5NyoeeE> = {
  gyao: _ZN3pkg5NyoeeER..3gyaoE,
  guee: _ZN3pkg5NyoeeER..3gueeE,
};

// 制限のある関数は witness を先頭の引数で受ける
function _ZN3pkg3Bbb3newE<T>(__w0: Gyao$w<T>, aaa: Aaa): Bbb<T> {
  const t = __w0.guee(aaa);
  const g = __w0.gyao(t);
  ...
}

// 呼び出し側は call_witnesses から witness を決めて渡す
_ZN3pkg3Bbb3newE(Gyao$for$_ZN3pkg5NyoeeE, aaa);
```

`ImplSource::Param` の場合は、自分が受け取った `__wN` をそのまま前に渡す。

- 引数の並びは「制限の宣言順」で固定する。両ターゲットで同じ順序を使う必要はない
  (wasm には witness 引数が無い) が、TS の中では定義と呼び出しで一致していなければならない
- witness 定数は trait impl ごとに 1 つ。impl がジェネリック (`impl[T] Vec[T]: Iter[T]`) なら
  witness も関数にする (`Iter$for$Vec = <T>() => ({...})`)

**代案: TypeScript も単相化する。** 出力が素直になり witness の機械が要らなくなるが、
HIR から単相化する仕組みが無いので新規に作ることになるうえ、
生成物のサイズが増え、TS 側のジェネリクスを保つという今の設計と衝突する。採らない。

## 構文とトークン

### trait 宣言

```
"trait" <ident> <genargs-decl>? "{" ( <fn-signature> ";" )* "}"
```

項目は本体を持たない。`;` で終わる。

### `impl Ty: Trait`

```
"impl" <genargs-decl>? <typ> ( ":" <typ> )? "{" <impl-item>* "}"
```

現在の `consume_impl_block` は genargs のあと `self_typ` を読んですぐ `{` を求めているので、
そこに `:` の省略可能な枝を足すだけである。曖昧さは無い。

### ジェネリクスの制限 (第 2 段)

```
<genarg-decl-item> ::= <ident> ( ":" <trait-cond-list> )?
<trait-cond-list>  ::= <typ> ( "&&" <typ> )*
```

型システムの文書の記法 (`impl[T, U: A[T] && B[T]]`) に合わせる。

### トークンの追加

| トークン     | 綴り    | 備考                                                                                                    |
| ------------ | ------- | ------------------------------------------------------------------------------------------------------- |
| `KwTrait`    | `trait` | キーワード表に足す                                                                                      |
| `MarkAndAnd` | `&&`    | 第 2 段。`lexer.rs` の 2 文字表に `('&', '&')` を足す。`&` (`MarkAmpersand`) より先に見るので順序に注意 |

ノベルパーサ (`#` コード行) は今回も触らない。

## 各段の実装

### 第 1 段

| #   | 段                             | 内容                                                                                                                                               |
| --- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `biwac_lexer`                  | `KwTrait`                                                                                                                                          |
| 2   | `biwac_span`                   | `DefIdKind::Trait`。`TraitDefId` は導入済み                                                                                                        |
| 3   | `biwac_ast`                    | `Globals::TraitDef`、`TraitDef` / `TraitItemDecl`、`ImplBlock::trait_repr: Option<TypRepr>`                                                        |
| 4   | `biwac_parser`                 | `trait` ブロック、`impl .. : ..`                                                                                                                   |
| 5   | `biwac_hir`                    | `TraitDef` / `TraitItemDef` / `TyTraitImpl`、`DefinedTyImpl::trait_impls`、`TyValImplGenargsContentPair::trait_of`、`Hir::traits` / `trait_scopes` |
| 6   | `biwac_trait_solver`           | `TraitEnv`、`solve_assoc` / `solve_method` (具体型のみ)、`TraitSolveError`                                                                         |
| 7   | `biwac_name_resolver`          | trait の収集、Step 4、孤児則 / 重複 / 名前衝突 / 項目一致の検査、trait スコープ、パス解決のフォールバック、`TraitEnv` の実装                       |
| 8   | `biwac_type_inferrer`          | `get_method_def_id` のフォールバック、`TyCtx` の `TraitEnv` 実装                                                                                   |
| 9   | `biwac_dependency_metadata`    | `Trait` / `TraitAssoc` / `TraitImpl`、`DiskFnData::trait_of`、trait impl 索引、版 7                                                                |
| 10  | `biwac_generator`              | マングル名に trait 成分                                                                                                                            |
| 11  | `library/std` + `assets/tests` | 試験と std での利用                                                                                                                                |

### 第 2 段

| #   | 段                              | 内容                                                                                                                             |
| --- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `biwac_lexer`                   | `MarkAndAnd`                                                                                                                     |
| 2   | `biwac_span`                    | `TraitAssocDefId`、`DefIdKind::TraitAssoc`                                                                                       |
| 3   | `biwac_ast` / `biwac_parser`    | `GenArgDeclItem::bounds`                                                                                                         |
| 4   | `biwac_hir`                     | `TraitCond` / `TraitCondList`、`Hir::trait_assoc_owners`、`Callee::TraitAssocFn`、`MethodTarget::Trait`、`FnDef::call_witnesses` |
| 5   | `biwac_trait_solver`            | `Gen` / `LocGen` の枝、`Solved::Deferred`                                                                                        |
| 6   | `biwac_name_resolver`           | 制限の解決、`T::foo()` のフォールバック                                                                                          |
| 7   | `biwac_type_inferrer`           | 呼び出し位置での制限の検査、`ImplSource` の記録                                                                                  |
| 8   | `biwac_mir` / `biwac_mir_build` | `Callee::TraitAssoc`、codec、`validate`                                                                                          |
| 9   | `biwac_mir_transform`           | 単相化で実装を決める                                                                                                             |
| 10  | `biwac_generator` (TS)          | witness 型 / witness 定数 / 引数渡し                                                                                             |
| 11  | 試験                            | 両ターゲットで同じ値が出ることを確認する                                                                                         |

## 実装して変わったところ (第 1 段)

方針から外した判断と、実装して初めて分かったことを残す。

### `TraitEnv` は 2 つのメソッドで足りた

方針では `trait_def` / `traits_in_scope(module)` / `bounds_of_*` を並べていたが、
第 1 段で実際に要ったのは 2 つだけだった。

```rust
pub trait TraitEnv {
    fn trait_impls_of(&self, ty: TyDefId) -> Vec<TyTraitImpl>;
    fn traits_in_scope(&self) -> &[TraitDefId];
}
```

- `trait_def` は要らない。項目名の照合は `TyTraitImpl::vals` を引くだけで済む
- `traits_in_scope` から `module: ModId` を外した。
  環境をモジュール 1 つ分に閉じて作れば、solver が引数で受け取る必要が無い。
  名前解決はいま辿っているモジュール、型推論はいま推論している関数のモジュールに対して作る

### パスの解決は特殊化で絞らない

`solve_assoc` は `ty_genargs` を見ない。パスに型引数を書く構文がまだ無いためで、
直接の impl の探索 (`AssocNameTree::find_matched(None, ..)`) と同じ扱いに揃えた。
`solve_method` はレシーバの型が推論済みなので、そちらは特殊化まで見て絞る。

### trait の `Self` は暗黙のジェネリック引数

宣言の中の `Self` はまだ何の型でもないので、`TyKind::Gen` として扱う。
`TraitDef::self_gen: GenDefId` を持たせ、impl の検査で
`Ty::embody_by_gen_ty_id` に `self_gen -> 実装対象の型` を渡して置き換える。
trait のジェネリック引数の置き換えと同じ経路に乗るので、専用の仕組みが要らない。

### `.biwameta` は trait への参照を `DiskTy` で運ぶ

方針では `trait_sym: DiskSymbolIndex` にしていたが、これでは足りない。
**実装した trait が別パッケージのことがある** ためで、
シンボル索引はそのファイル内でしか意味を持たない。

trait は型ではないが、参照の運び方は型とまったく同じ
(自パッケージならシンボル索引、外部なら `ext_syms` 経由) なので、
`DiskTy` をそのまま使うことにした。ジェネリック引数も一緒に運べる。
`DiskFnData::trait_of` も同じ理由で `DiskVec<DiskTy>` である。

### `DiskFnData` にレシーバの有無を足した

`.biwameta` は「その関数が `self` を取るか」を記録していなかった。
`impl_self_ty` は関連関数にも入る (impl の対象型を運ぶため) ので、
それだけでは区別できない。
既存のコメントが「レシーバの有無で分岐したい用途が出たら印を足すこと」と
書いていたとおり、trait の宣言と実装の突き合わせが最初の用途になった。
`has_self: u32` を足してある。

### trait のシンボルは末尾に採番する

方針では触れていなかったが、`[struct][native type alias][enum][variant][assoc fn][top fn][mod]`
の **後ろ** に `[trait][trait assoc][trait impl]` を足した。
既存のシンボルの番号がまったく動かないので、
enum のときに踏んだ「採番順と本体の書き出し順のずれ」の危険が新しい 3 種だけに閉じる。
数の突き合わせを `debug_assert` で残してある。

### `assoc_val_map` は型ではなく値のパッケージで絞る

`Hir::new` が `assoc_val_map` を組むとき、
これまでは「所属する型が自パッケージか」で絞っていた。
**外部パッケージの型に trait を実装できる** ようになったので、この基準では取りこぼす。
関連アイテム自身の `ValDefId` で絞るように直した
(`.biwameta` の書き出しは元から同じ基準だった)。

同じ理由で、`register_impl_val` は対象の型が `tys` に無ければ
中身の無い入れ物 (`ty_content: None`) を作る。型の定義は依存メタデータの側にある。

### 型の位置に trait が来たら名前解決の段で弾く

`ty_kind_from_typ_repr` は解決に失敗した型を黙って `TyKind::Infer` に潰す。
そのため lowering で `TypeNotFoundTraitFound` を返しても捨てられてしまい、
「MIR が壊れている (コンパイラのバグ)」という無関係な診断まで流れてしまった。
`ResolveCtx::resolve_typ` の側で弾くようにしてある。

その代わり `impl Foo: Bar` の右辺はこの検査を通してはならないので、
`resolve_trait_typ` を別に用意した
(ジェネリック引数の側は普通の型なので `resolve_typ` に回す)。

### シグニチャの検査は lowering (Pass 2.5)

def collection の段では trait の項目の型がまだ解決されていないので、
検査は lowering まで遅らせる。
外部パッケージの trait も `.biwameta` から復元して同じ経路で検査したいので、
`lower()` に依存パッケージと interner を渡すようにした
(`NameResolver::try_resolve` が `&mut IdentInterner` を取るようになっている)。

### Step 3 は trait impl を素通りする

直接の impl を集める Step 3 で trait impl まで拾ってしまうと、
項目が `TyNameTree::children` に載って import 無しで見えるようになる。
`impl_block.trait_typ.is_some()` で明示的に飛ばし、Step 4 だけが扱う。

### 型エイリアスに対する impl は lowering で展開する

trait とは無関係の既存のバグを踏んだので、あわせて直した。

```biwa
type CharacterBiwa = Character[BiwaCharacterProps];

impl CharacterBiwa { .. }                       // これも壊れていた
impl CharacterBiwa: CharacterBiwaBehavior { .. } // 今回踏んだ形
```

名前解決はエイリアスを**型の位置では潰さない**
(潰すと `type C = Character[P]` の `[P]` が失われる) ので、
`ImplCollector::impl_self_tys` にはエイリアス自身の `TyDefId` が入る。
それをそのまま実装の置き場所にすると、関連関数が
「`.biwameta` のシンボルにならない型」にぶら下がることになり、
メタデータの書き出しで
`compiler bug: self-package type ... is missing from the symbol table`
で落ちる (リリースビルドでは黙ってシンボル 0 番に化ける)。

`alias_expansion` (Pass 5) は式やシグニチャの中は展開するが、
**`hir.tys` のキーは書き換えない**ので、これだけでは直らない。
`lower_impl_block` の入口で `expand_ty` を通し、
展開後の型の下に置くようにした。エイリアスの型引数もこれで拾える。

あわせて `TyTraitImpl` の索引を組む場所も lowering に移した。
def collection の段ではエイリアスの右辺の型引数がまだ解決されておらず、
`ty_genargs` が空になってしまうためである
(def collection 側の索引はパス解決のフォールバック専用に残してある。
そちらは特殊化で絞らないので影響しない)。

### 見つけた既存の壊れ

`library/std/src/collections.biwa` が `package::types::option::Option` を import していたが、
`option.biwa` は `src/option.biwa` にある (他のモジュールはすべて `package::option::` を使っていた)。
enum 化のときの取りこぼしで、`test/test1` がビルドできない状態だった。
`package::option::` に直してある。

## 実装後の状態 (第 1 段)

動くもの。

```biwa
trait Describe {
  fn describe(self) -> Int;
  fn from_code(code: Int) -> Self;
}

impl Pos: greeter::Describe { .. }        // 外部の trait + 自分の型
impl greeter::Box[Int]: Level { .. }      // 自分の trait + 外部の型
impl Shade: Level { .. }                  // どちらも自分
```

- trait の宣言、`impl Ty: Trait`、ジェネリックな型への trait impl
- trait 越しの関連関数 (`Ty::foo()`) とメソッド (`x.foo()`)
- 使う箇所で trait を `import` していることの要求
- 孤児則、impl の重複、関連名の衝突、実装漏れ、宣言に無い項目、シグニチャ不一致の検査
- パッケージを跨いだ trait / trait impl (`.biwameta` 版 7)

検証。

- コンパイラのテスト 41 件すべて通過、`cargo fmt` 済み
- `assets/tests` に 4 通りの impl (自/自、自 trait + 外部の型、外部 trait + 自の型、
  型エイリアス経由) と、型エイリアスに対する普通の impl を足し、
  両ターゲットで同じ値 (2227) を返すことを Node (wasm) と tsx (TypeScript) で確認
- 生成物のマングル名に trait 成分が入っていることを確認
  (`_ZN7greeter3BoxIiER5test15Level6level2E`)
- 診断 9 種をそれぞれ確認

バックエンドには一切手を入れていない (マングリングを除く)。
trait impl の中の関数は、名前解決の見え方が違うだけで、
HIR から先は今までの関連関数とまったく同じである。

## 落とし穴

- **`.biwameta` の採番順と `push_body` の順。** 3 種類のシンボルが増えるので、
  Phase 2 と Phase 4 の両方に、同じ位置に挿すこと (enum で踏んだ)
- **`trait_of` の付け忘れ。** 付け忘れると import 無しで見えてしまい、
  コンパイルは通る。`register_impl_val` の引数を省略不可にして、
  呼び出し側 6 箇所すべてに書かせる
- **`TyNameTree::children` に trait impl の項目を登録しない。** 登録すると import 規則が死ぬ。
  名前衝突の検査だけを `children` に対して行い、登録はしない
- **`resolve_path` は失敗時に `OnceCell` に印を書き込む。** trait のフォールバックを試す前に
  本物のパスで解決を試すと、失敗が焼き付いてやり直せない。
  パターンの `IdentPattern::resolve` で使ったのと同じく、複製したパスで先に試すか、
  `children` の空振りを確認してからフォールバックする (後者が素直)
- **外部パッケージの trait impl。** enum のときと同じ形の罠がある。
  lowering の時点では外部の型は HIR に載っていない。
  `TraitEnv` 越しにしか触らないようにして、経路を 1 本にする
- **`Hir::trait_scopes` は自パッケージだけでよい。**
  外部パッケージのコードは既に解決済みなので、そのモジュールの trait スコープは要らない
- **マングル名の衝突。** trait 成分を忘れると、
  別々のパッケージが同じ型に同じ名前を生やしたときに黙って同じ関数になる

## 今回やらないこと

- 既定実装 (trait の項目に本体を書く)
- 関連型 (`type Item;`)
- blanket impl (`impl[T] T: Foo`)
- `[T as Gyao]::guee()` の明示的な曖昧さ解消構文
- `dyn` 相当 (動的ディスパッチ)。biwa の trait は常に静的に解決される
- スーパートレイト (`trait A: B`)
- 演算子オーバーロード (`Add` などの lang item 化)
- ノベル `#` コード行での trait 関連の構文
