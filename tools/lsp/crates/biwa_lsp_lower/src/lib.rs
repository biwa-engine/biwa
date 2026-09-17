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
//! - 明示的な `Void` 型キーワード (biwac は `->` 省略で Void を表す。
//!   `biwa-lsp-lexer` に `KwVoid` トークンがあるのはこの CST 独自の拡張)
//! - 単項 `+`/`!` (biwac には単項 `-` しか無い)
//! - 論理演算子 `&&`/`||` を式の中置演算子として使うこと
//!   (biwac_ast::BinOperator に対応するバリアントが無い。`&&` そのものは
//!   ジェネリクスの制限列 `T: A && B` としては使える)
//! - `NONE` リテラル (`biwac_ast::Literal` に Option 相当のバリアントが無い)
//! - `for .. in ..` ループ (biwac_ast::Stmt に `For` が無い。`while` のみ)
//! - `type X = ..;` の右辺 (biwa-lsp-parser がまだ `=` 以降を読まない)
//! - ネイティブ実装 (`{{ .. }}`) と属性 (`[[ .. ]]`) 全般
//! - パターンのネスト、リテラル/範囲パターン、`|` による選択、ガード、`..`
//!   によるフィールド省略 (biwac 自身がまだ入れていない範囲。
//!   `docs/enum-and-match.md` の「今回やらないこと」と同じ)
//! - `scene` の本体 (novel モード) の構造化 (`#`/`@` 行を文の列として組む、
//!   `NovelIfStmt` 相当の CST ノードを作る、など)。字句解析の段階では
//!   `#` コマンドの複数行への継続 (`(`/`[`/`,`/`.`/`::` で終わったら次行へ)
//!   と `$` 埋め込み式 (`$(expr)` / `$ident(..)...` の呼び出しで終わる連鎖) は
//!   `biwa_lsp_lexer::lexer::lex_novel_segment` が正しく認識し、中身を通常
//!   コードのトークン列として切り出す。ここから `biwac_ast::NovelStmt` へ
//!   構造化する文法が biwa-lsp-parser にまだ無く、常に空の本体として
//!   salvage される (`docs/enum-and-match.md` が言う「ノベル `#` コード行での
//!   match」と同じ理由で、行継続を含む文の並びを組む設計がまだ無いため)。
//! - `Self` (大文字) 型/パス (biwa-lsp-lexer に専用トークンが無く、
//!   ただの識別子 `Self` として読まれる)
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
mod path_ty;
mod pattern;
mod stmt;

pub use error::LowerError;
pub use globals::lower_module;
