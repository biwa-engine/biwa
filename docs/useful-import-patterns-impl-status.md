# glob import と re-export (issue #18) 実装方針・状況

issue #18 「[feature] Useful `import` patterns」の実装方針と進み具合のメモ。
可視性 (issue #8) の上に乗る機能なので、用語と規則は `docs/symbol-visibility-impl-status.md` (以下「可視性メモ」) に従う。

状況: **§5 の全段階を実装済み**。

## 0. スコープ

- glob import: `import foo::bar::*;`
- re-export: `pub import foo::bar::baz;` (`pub(super)` / `pub(package)` も)
- 両者の組み合わせ: `pub import foo::bar::*;`

## 1. 決まったこと

1. **glob import は、`*` の位置の見えるシンボルをすべて import する。**
   - 「見える」は、import を書いたモジュールから見えるか (可視性メモ §5.3)。
   - 対象はモジュールの子 (定義と、そのモジュールの import が作った名前) と、enum の variant。
2. **import する側のモジュールでは、glob で入る名前と、そのモジュールで使える他の名前が重複してはならない。**
   - 他の名前 = そのモジュールの定義・明示した import・他の glob で入る名前。
3. **re-export は、シンボルの定義に付いた可視性の範囲を超えられない** (可視性メモ §1 の 6。Rust と同じ)。
   - 明示した `pub import a::b;` で、書いた可視性が `b` の可視性の範囲を超えればエラー。
   - 祖先のモジュールによる頭打ちは超えられる (private なモジュールの中の `pub` なものを上で公開できる)。
4. **glob の re-export (`pub import foo::bar::*;`) は、各シンボルを「元の可視性」と「書いた可視性」の狭い方で re-export する** (Rust と同じ。§2)。
   見える子に `pub` より狭いものが混ざっていてもエラーにしない。
5. **glob の re-export で、書いた可視性まで広げられるシンボルが 1 つも無ければエラーにする**
   (Rust は警告。Biwa には警告の仕組みが無く、書いた意図と結果が食い違っているので止める)。

## 2. Rust ではどうなっているか (調査)

手元の rustc 1.98.1 で確かめた。

- `pub use m::*;` は、`m` の見える子を、それぞれ「元の可視性」と `pub` の狭い方で re-export する。エラーにはならない。
  - `m` に `pub fn a` / `pub(crate) fn b` / `pub(super) fn c` / private な `fn d` を置いてルートで `pub use m::*;` すると、
    クレートの中からは `crate::a()` / `crate::b()` / `crate::c()` が呼べる。外のクレートからは `lib::a()` は呼べるが `lib::b()` は「`b` is private」。
  - `d` は glob の位置から見えないので、取り込まれない (使うと not found)。
- 明示した `pub use m::b;` (`b` は `pub(crate)`) はエラー (E0364「only public within the crate, and cannot be re-exported outside」)。
- glob で `pub` にできるものが 1 つも無いとき (`pub use m::*;` で中身が全部 `pub(crate)`) は警告
  (「glob import doesn't reexport anything with visibility `pub` because no imported item is public enough」)。
- 名前の重複は Rust の方が緩い: 明示した項目・import は glob の名前を隠し、glob どうしの衝突は使ったときだけエラーになる。
  Biwa は §1 の 2 のとおり、重複そのものをエラーにする。

## 3. 今のコードの状態 (調査)

- `ImportDecl { path, span }`。パーサは `pub import` を「not supported yet」で拒否する。`*` は読めない。
- 名前解決: `ModuleResolveCtx::new` が import を「最後のセグメントの名前 → パス」の表にし、モジュールの子との重複を検査する。
  パスの先頭がその名前ならパスを解決して、その先を辿る。import の解決結果はそのモジュールの中でしか使われず、
  **他のモジュールからは見えない** (モジュールの子にならない)。
  - `prepare_trait_scope` が import をすべて解決する (import した trait をスコープに入れるため) ので、使われない import も解決はされる。
- `ModuleResolveCtx` は本番の名前解決のほか、def collection の Step 2 (型エイリアスの右辺)・Step 3 (impl)・Step 4 (trait impl) でも作られる。
  そこで書かれたパスも import を経由しうる。
- `.biwameta` のモジュールのシンボル (`DiskModData`) は子のシンボル番号の列しか持たず、re-export を表せない。

## 4. 方針

### 4.1 構文

- `<visibility>? "import" <path> ( "::" "*" )? ";"`。`ImportDecl` に `vis: Visibility` と `glob: bool` を足す。
- `super::*` / `package::a::*` のようにヘッダの直後の `*` も書ける。パス全体が `*` だけ (`import *;`) は書けない。
- LSP の文法・lowering も同じ。

### 4.2 import の表 (名前解決)

- def collection の Step 1 (名前の木を作る) の直後に、モジュールごとの **import の表** (名前 → 指すもの・可視性・宣言の位置) を作る。
  Step 2 以降はどれもこの表を使う。
  - 明示した import も glob も表に入れる。表はそのモジュールの外からも引ける (re-export のため)。
  - 表は不動点で作る: 空から始めて、すべての import を「名前の木 + 今の表」の上で解決し直し、表が増えなくなるまで繰り返す。
    re-export の連鎖や、re-export されたものの glob も、これで順序によらずに埋まる。表は増える一方なので必ず止まる。
  - 解決は副作用の無い探索で行う (パスのセグメントの `resolved_id` には書かない。書くと「まだ見つからない」を「無い」と確定させてしまう)。
  - 最後まで解決できない明示した import は、これまでどおり本番の名前解決でパスを解決するときにエラーになる。
- 表の可視性 (他のモジュールから引いたときの可視性):
  - 明示した import: 書いた可視性 (何も書かなければ private = そのモジュールとその子孫)。
  - glob で入った名前: 書いた可視性と元の可視性の狭い方。
- 引き方:
  - パスの先頭: モジュールの子 → 表 (明示した import・glob) → パッケージ名。
  - 途中のモジュールの中: 子が無ければ、そのモジュールの表を引く。見えるかは表の可視性で判定する (可視性メモ §5.3 と同じ)。
- 依存パッケージの re-export も同じく引けるようにする (§4.4)。

### 4.3 検査

- 名前の重複 (§1 の 2): 表の名前どうし・表の名前とモジュールの子。glob で入った名前が同じものを指すだけなら重複としない
  (別々の glob から同じ項目が入るのは、名前が 1 つのものを指していて曖昧さが無いため)。
- re-export の範囲 (§1 の 3): 明示した import の書いた可視性が、指すものの可視性の範囲を超えればエラー。
  指すものが re-export なら、その re-export の可視性で比べる。
- glob の re-export で何も広げられない (§1 の 5): エラー。
- `*` を付けられるのはモジュールと enum だけ (enum は variant を取り込む)。それ以外の型・値は glob の対象にならないのでエラー。

### 4.4 `.biwameta`

- モジュールのシンボルに re-export の表 (名前・指すシンボル・可視性) を足す。指すシンボルは他のパッケージのものでもよいので、
  型の参照と同じく自パッケージのシンボル番号か外部シンボルの表 (`ext_syms`) で指す。版を上げる。
- 依存する側の `PackageModuleView::lookup_child` は子が無ければ re-export の表を引く。glob のために子の一覧も引けるようにする。
- `ExternalChildRef` が他のパッケージのシンボルを指しうるので、パッケージも持たせる。

### 4.5 private-in-public (可視性メモ §5.2)

- 実効可視性は「届く経路ごとの範囲の和」。re-export の経路を足す:
  モジュール M の表の名前 x が項目 T を指すなら、「M の実効可視性 ∩ x の可視性」を T の実効可視性に足す。
  re-export されたモジュールの子にも同じ経路が延びるので、モジュール・項目・re-export を辺とするグラフの上の不動点として求める。

## 5. 段階

1. 構文 (AST・パーサ・LSP)。
2. import の表と、名前解決での引き方 (自パッケージ)。§4.3 の検査。
3. `.biwameta` の re-export と、依存パッケージの re-export・glob。
4. private-in-public に re-export の経路を足す。
5. フィクスチャ・テスト。

## 6. 進み具合

### 6.1 段階 1: 構文 (実装済み)

- `ImportDecl { vis, path, glob, span }`。パーサは `import` のパスだけ最後に `::*` を読む (`consume_import_path`)。
  `super::*` / `package::*` のセグメントの無いパスのために、`Path::span` はセグメントが無ければヘッダの位置を返すようにした。
  型や式のパスに `*` は書けない。
- LSP: 文法 (`parse_path(p, allow_glob)`。`*` は IdentPath の中の `Star` トークン)・lowering。

### 6.2 段階 2: import の表と名前解決 (実装済み)

- `biwac_name_resolver::resolving::import_table`。`NameTree::imports` (モジュール → `ModuleImports { explicit, glob }`)。
  def collection の Step 1 の直後に不動点で作る (§4.2)。glob で依存パッケージの子の名前を intern するので、`DefCollector::collect` は interner を可変で借りるようにした。
- 名前解決:
  - パスの先頭: モジュールの子 → 明示した import (今までどおりパスを解決する) → glob の表 → パッケージ名。
  - 途中のモジュール: 子が無ければ、そのモジュールの表 (明示した import・glob) を引く。見えるかは表の可視性で判定する
    (private な import が作った名前は、そのモジュールとその子孫からしか見えない)。
  - 引いた名前の先は、明示した import と同じ関数 (`resolve_rest`) で辿る (以前は import の分岐の中に同じ処理があった)。
  - glob で入った trait も、そのモジュールの trait のスコープに入る。
- 検査 (§4.3): `ReexportBeyondVisibility` / `GlobReexportsNothing` / `GlobImportOfNonContainer` /
  `ImportedNameConflict` (glob どうし) / `GlobImportShadowed` (glob と定義・明示した import)。
  明示した import どうし・明示した import と定義の重複は、以前からの `DuplicatedSymbolName` / `DuplicatedSymbolAndDefIdName`。
- 範囲の演算 (含む・共通部分) は `visibility::ScopeOps` にまとめ、private-in-public と共有した。
  自パッケージのルートモジュールの部分木はパッケージ全体と同じ範囲として扱う (`pub(package)` と比べるため)。
- private-in-public (§4.5) も同時に入れた: 実効可視性に re-export の経路を足し、不動点で求める。関数・scene の実効可視性にも経路を数える。
- フィクスチャ: `imp_ok` (正例)、`imp_errors` (5 種の誤り)、`imp_invisible` (private な import は他のモジュールから見えない)。
  glob のパス (`*` の手前) も本番の名前解決で解決し、途中のセグメントが見えるかを確かめる (`import b::secret::*;` で `secret` が見えなければ `InvisibleItem`)。

### 6.3 段階 3: `.biwameta` の re-export (実装済み)

- `DiskModData::reexports` (`DiskReexport { name, kind, external, target, vis }`)。版 14 → 15。SVH にも入れる。
  指すものが自パッケージなら `external = 0` でシンボル番号、他のパッケージなら `external = 1` で `ext_syms` の番号。
  - 書くのは **可視性を書いた import (`pub` / `pub(package)` / `pub(super)`) が作った名前**だけ。HIR の `Hir::reexports` で運ぶ。
    - `pub` でないものも書く: 依存する側が「見つからない」ではなく「見えない」と言えるようにするため (定義のシンボルと同じ方針)。
    - 可視性を書かない import は書かない: re-export ではなく、書くと private な import を変えただけで SVH が変わり、依存する側が作り直しになるため。
- 依存する側: `DepMetadataModuleView` の名前の索引に re-export も入れる (定義が優先)。見える範囲は re-export したモジュールを基準に組み立てる
  (`DepMetadata::visibility_in_module`)。
  - `ExternalChildRef` に `pkg_id` (シンボルを定義したパッケージ) を足し、`as_*_def_id` は引数を取らなくした。
  - 名前解決で依存パッケージのモジュール・型の中を辿るときは、指すものの `pkg_id` の view を作り直す (`resolve_rest`)。
    以前は「引いた view のパッケージ」で辿っていたが、re-export は別のパッケージを指しうる。
  - glob のために `PackageModuleView::list_children` / `list_variants` を足した (依存パッケージのモジュールと enum の glob)。
- フィクスチャ: `imp_dep` (private なモジュールの中のものの glob の re-export、別のパッケージ `vis_dep` の関数の re-export、モジュールの re-export)、
  `imp_dep_user` (それらを名前・glob で使う。名前解決と型推論が通る)、`imp_dep_user_ng` (`pub(package)` の re-export は外から「見えない」)。

### 6.4 段階 4: private-in-public (実装済み。段階 2 と同時)

- `interface_check::effective_visibilities`: 定義の経路に、import の表の各名前の経路 (「モジュールの実効可視性 ∩ 名前の可視性」) を足し、
  増えなくなるまで繰り返す。re-export されたモジュールの子にも経路が延びる。関数・scene にも経路を数える。
- `imp_ok` の `pub(package) fn via_reexport() -> shapes::Deep` が、re-export の経路を数えて初めて通ることで確かめている。

### 6.5 実装しながら決めたこと

- **同じものを指すだけの名前の重なりはエラーにしない**: 別々の glob から同じ項目が入る、明示した import と glob が同じ項目を入れる場合。
  名前は 1 つのものを指していて曖昧さが無いため。別のものを指すなら §1 の 2 のとおりエラー。
- **glob の対象はモジュールと enum** (enum は variant を取り込む)。struct などの関連関数は取り込まない (Rust も関連 item は import できない)。
- **re-export の範囲の比べ方**: 指すものが re-export なら、その re-export の可視性で比べる (連鎖しても定義の可視性を超えない)。
- **自パッケージのルートモジュールの部分木は `pub(package)` と同じ範囲**として扱う (`ScopeOps::covers`)。
- **import の表を作る段階 (Step 1 の直後) では、型の関連関数は名前の木にまだ載っていない**ので、`import Foo::new;` のような関連関数の import は
  表に入らない。自分のモジュールの中では以前どおりパスを解決して使えるが、他のモジュールから re-export としては引けない。
  型エイリアス越しの variant の import (`import Alias::Variant;`) も同じ。

### 6.6 確認

- compiler 140 件、LSP 100 件。
- std と `~/test1` を強制再ビルドし、ブラウザで Link と scene の開始まで動くことを確かめた (std は glob・re-export をまだ使っていない)。

