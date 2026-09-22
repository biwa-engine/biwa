//! `scene` の本体 (`NovelModeBody`、および `#if` の中の `BlockStmt`-shaped
//! ノベルブロック) を `biwac_ast::symbols::novel::NovelStmt` へ直す。
//!
//! # 設計
//!
//! `#let`/式文/代入文は通常コードの文と全く同じ `biwac_ast` の型
//! (`VarDecl`/`ExprStmt`/`AssignStmt`) を使う (`biwac_ast::symbols::novel` の
//! `use` 参照)。CST の形も `;` が無い以外は通常コードの
//! `VarDefStmt`/`ExprStmt`/`AssignStmt` と同じに揃えてあるので、
//! `crate::stmt` の対応する lowering 関数をそのまま呼べる。
//! `#if`/`#endscene`/地の文/埋め込み式だけがノベル専用の型を持つので、
//! ここで新しく書く。
//!
//! 実コンパイラ (`biwac_novel_parser`) との既知の違い:
//! - `@` 行 (`NovelStmt` に対応する variant が無い) は読み捨てる。
//!   実コンパイラ自身もまだ `NovelStmt` へ変換する文法を持たない
//!   (`symbols/statements.rs` の `CharaCommand => todo!()`)。
//! - `NovelContent::Text` は `\$` の unescape をしない (トークンのテキストを
//!   そのまま使う)。名前解決はテキスト内容を見ないので、この段では実害が無い。
//! - 複数行にまたがる地の文を 1 つの `Text` に融合したり、`>>` の手前のテキストと
//!   後ろのテキストを地続きとして扱ったりはしない。行ごと・トークンごとに
//!   別々の `NovelStmt::ContentPush` になる (name resolution には影響しない)。

use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{NovelBlockStmt, NovelContent, NovelEndSceneStmt, NovelFlush, NovelIfStmt, NovelStmt};
use biwac_base::{IdentInterner, ModId};

use crate::cursor::{Children, SyntaxNode, node_span, token_span};
use crate::error::LowerError;
use crate::expr::lower_expr;
use crate::stmt::{lower_assign_stmt, lower_expr_stmt, lower_var_decl};

/// `NovelModeBody` (`{`/`}` を持たない) と `#if` の本体 (`BlockStmt`-shaped、
/// `{`/`}` を持つ) の両方を受け付ける。前者は単に `LBrace`/`RBrace` が
/// 無いだけで `eat_token` は何もしない。
pub(crate) fn lower_novel_stmts(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Vec<NovelStmt> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBrace);

    let mut out = Vec::new();

    loop {
        match children.peek_kind() {
            None | Some(SyntaxKind::RBrace) | Some(SyntaxKind::DoubleRBrace) => break,

            Some(SyntaxKind::NovelCommandLine) => {
                let n = children.next_node().expect("peeked");
                if let Some(stmt) = lower_novel_command_line(mod_id, interner, &n, errors) {
                    out.push(stmt);
                }
            }

            // `@` 行。実コンパイラもまだ NovelStmt へ変換する文法を持たない。
            Some(SyntaxKind::NovelCharaLine) => {
                children.next_node();
            }

            Some(SyntaxKind::NovelEmbeddedExpr) => {
                let n = children.next_node().expect("peeked");
                if let Some(content) = lower_novel_embedded_expr(mod_id, interner, &n, errors) {
                    out.push(NovelStmt::ContentPush(content));
                }
            }

            // `NovelText`/`NovelWait` はノードを介さず直接トークンとして並ぶ。
            _ => match children.next_elem() {
                Some(rowan::NodeOrToken::Token(tok)) if tok.kind() == SyntaxKind::NovelText => {
                    out.push(NovelStmt::ContentPush(NovelContent::Text {
                        text: tok.text().to_string(),
                        span: token_span(mod_id, &tok),
                    }));
                }
                Some(rowan::NodeOrToken::Token(tok)) if tok.kind() == SyntaxKind::NovelWait => {
                    out.push(NovelStmt::ContentFlushAndWait(NovelFlush {
                        span: token_span(mod_id, &tok),
                    }));
                }
                // `Error` ノードなど、字句・構文解析が既に報告済みの残骸。読み飛ばす。
                Some(_) => {}
                None => break,
            },
        }
    }

    children.eat_token(SyntaxKind::RBrace);
    out
}

fn lower_novel_command_line(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<NovelStmt> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::NovelHash);
    let inner = children.next_node()?;

    match inner.kind() {
        SyntaxKind::VarDefStmt => {
            lower_var_decl(mod_id, interner, &inner, errors).map(NovelStmt::VarDecl)
        }
        SyntaxKind::ExprStmt => {
            lower_expr_stmt(mod_id, interner, &inner, errors).map(NovelStmt::Expr)
        }
        SyntaxKind::AssignStmt => {
            lower_assign_stmt(mod_id, interner, &inner, errors).map(NovelStmt::Assign)
        }
        SyntaxKind::IfStmt => {
            lower_novel_if_stmt(mod_id, interner, &inner, errors).map(NovelStmt::If)
        }
        SyntaxKind::NovelEndSceneStmt => {
            lower_novel_end_scene_stmt(mod_id, interner, &inner, errors)
                .map(NovelStmt::NovelEndScene)
        }
        _ => {
            errors.push(LowerError::new(
                "unrecognized novel `#` command",
                node_span(mod_id, &inner),
            ));
            None
        }
    }
}

fn lower_novel_if_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<NovelIfStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwIf);

    let cond_node = children.next_node()?;
    let cond = lower_expr(mod_id, interner, &cond_node, errors)?;

    let then_node = children.eat_node(SyntaxKind::BlockStmt)?;
    let then_span = node_span(mod_id, &then_node);
    let then_stmts = lower_novel_stmts(mod_id, interner, &then_node, errors);

    Some(NovelIfStmt {
        cond,
        then: NovelBlockStmt {
            stmts: then_stmts,
            span: then_span,
        },
        // 実コンパイラもまだ `else` を持たない (`biwac_novel_parser` 参照)。
        els: None,
        span,
    })
}

fn lower_novel_end_scene_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<NovelEndSceneStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwEndScene);
    let expr_node = children.next_node()?;
    let expr = lower_expr(mod_id, interner, &expr_node, errors)?;
    Some(NovelEndSceneStmt { expr, span })
}

fn lower_novel_embedded_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<NovelContent> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::NovelDollar);
    let expr_node = children.next_node()?;
    let expr = lower_expr(mod_id, interner, &expr_node, errors)?;
    Some(NovelContent::Expr { expr, span })
}
