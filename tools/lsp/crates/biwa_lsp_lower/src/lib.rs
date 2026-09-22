//! CST (`biwa-lsp-parser` の rowan ベースの構文木) を、コンパイラ本体
//! (`biwac_ast`) が使う AST へ変換する層。
//!
//! # なぜこの crate があるか
//!
//! `biwa-lsp-parser` は lossless (コメント等のトリビアを保持する) かつ
//! エラー耐性 (壊れた入力でも部分木を返す) な CST を構築する。
//! これはエディタでの編集体験には必須だが、biwac_parser が作る
//! `biwac_ast::ModAst` とは形が違う。
//!
//! 一方、名前解決 (`biwac_name_resolver`) や型推論 (`biwac_type_inferrer`) は
//! `biwac_ast` にしか依存しておらず、パーサ内部の実装には依存していない。
//! そのため、CST から `biwac_ast` 相当の値を組み立てさえすれば、
//! これらの意味解析をそのまま LSP から呼び出せる。
//!
//! この crate はその「組み立て」を行う。rust-analyzer の `hir::lower` に近い
//! 役割で、表現できない構文要素に出会っても `Result` で止まらず、
//! その要素だけを [`LowerError`] として記録しつつ残りの木を組み立て続ける
//! (salvage 方針)。
//!
//! # 既知の非対応 (biwa-lsp-parser の文法が biwac の文法と噛み合っていない箇所)
//!
//! この lowering を書く過程で、biwa-lsp-parser の CST 文法が biwac の実際の
//! 文法と食い違っている箇所がいくつか見つかった (biwa-lsp-parser が独自に
//! 文法を実装していることの帰結)。lowering ではこれらを埋め合わせようとはせず、
//! [`LowerError`] を出して該当要素を落とす:
//!
//! - `import .. as ..` (biwac にエイリアス構文が無い)
//! - 単項 `+`/`!` (biwac には単項 `-` しか無い)
//! - 論理演算子 `&&`/`||` を式の中置演算子として使うこと
//!   (biwac_ast::BinOperator に対応するバリアントが無い。`&&` そのものは
//!   ジェネリクスの制限列 `T: A && B` としては使える)
//! - `NONE` リテラル (`biwac_ast::Literal` に Option 相当のバリアントが無い)
//! - `for .. in ..` ループ (biwac_ast::Stmt に `For` が無い。`while` のみ)
//! - ネイティブ実装 (`{{ .. }}`) と属性 (`[[ .. ]]`) 全般
//! - パターンのネスト、リテラル/範囲パターン、`|` による選択、ガード、`..`
//!   によるフィールド省略 (biwac 自身がまだ入れていない範囲。
//!   `docs/enum-and-match.md` の「今回やらないこと」と同じ)
//! - `scene` の本体 (novel モード) のうち `@` 行 (キャラクター指定)。
//!   `#`/地の文/`$` 埋め込み式は `NovelStmt` へ構造化される
//!   (`crate::novel` 参照) が、`@` 行は実コンパイラ (`biwac_novel_parser`)
//!   自身もまだ `NovelStmt` へ変換する文法を持たない
//!   (`symbols/statements.rs` の `CharaCommand => todo!()`) ため読み捨てる。
//! - ブロック直下、素のまま置かれた `if`/`match` を tail 式として使うこと
//!   (`fn f() -> Int { if c {1} else {2} }`)。`parse_statement_or_expr` が
//!   `if`/`match` を常に文形 (`IfStmt`/`MatchStmt`) としてパースしてしまい、
//!   それが式として使われる最後の要素かどうかを判定しないため、CST 側に
//!   `IfExpr`/`MatchExpr` が現れない。`let r = if c {1} else {2}; r` のように
//!   一度束縛すれば式として読める。
//!
//! これらは biwa-lsp-parser 側の文法を拡張すれば解消する。lowering 側の
//! 制約ではない。

mod cursor;
mod error;
mod expr;
mod globals;
mod novel;
mod path_ty;
mod pattern;
mod stmt;

pub use error::LowerError;
pub use globals::lower_module;
