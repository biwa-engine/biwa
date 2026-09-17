use std::cell::OnceCell;

use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{
    AssignStmt, BlockExpr, BlockStmt, ExprStmt, Exprs, MatchStmt, MatchStmtArm, Stmt, TypDecl,
    VarDecl, WhileStmt,
};
use biwac_base::{IdentInterner, ModId};
use biwac_span::Span;

use crate::cursor::{Children, SyntaxNode, intern_ident_token, node_span};
use crate::error::LowerError;
use crate::expr::lower_expr;
use crate::path_ty::lower_type_repr;
use crate::pattern::lower_pattern;

fn is_stmt_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::VarDefStmt
            | SyntaxKind::IfStmt
            | SyntaxKind::WhileStmt
            | SyntaxKind::ForStmt
            | SyntaxKind::MatchStmt
            | SyntaxKind::ExprStmt
            | SyntaxKind::AssignStmt
            | SyntaxKind::BlockStmt
    )
}

struct BlockBody {
    stmts: Vec<Stmt>,
    tail: Option<Exprs>,
    span: Span,
}

/// `BlockStmt`/`BlockExpr` どちらの CST ノードも同じ形 (`{` 文* 末尾式? `}`) を
/// しているので、ここで一括して読む。どちらとして確定させるか
/// (末尾式が要るか、あってはいけないか) は呼び出し側 3 関数が決める。
fn lower_block_body(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> BlockBody {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBrace);

    let mut stmts = Vec::new();
    let mut tail: Option<Exprs> = None;

    loop {
        match children.peek_kind() {
            None | Some(SyntaxKind::RBrace) => break,
            Some(k) if is_stmt_kind(k) => {
                let Some(stmt_node) = children.next_node() else {
                    break;
                };
                if let Some(s) = lower_stmt(mod_id, interner, &stmt_node, errors) {
                    if tail.is_some() {
                        errors.push(LowerError::new(
                            "unreachable statement after a tail expression",
                            node_span(mod_id, &stmt_node),
                        ));
                    }
                    stmts.push(s);
                }
            }
            Some(_) => {
                if let Some(expr_node) = children.next_node() {
                    if tail.is_some() {
                        errors.push(LowerError::new(
                            "a block can only end with one tail expression",
                            node_span(mod_id, &expr_node),
                        ));
                    } else if let Some(e) = lower_expr(mod_id, interner, &expr_node, errors) {
                        tail = Some(e);
                    }
                } else {
                    // `}` でも文でもない孤立トークン (エラー回復の残骸)。読み飛ばす。
                    children.next_elem();
                }
            }
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    BlockBody { stmts, tail, span }
}

/// `if`/`while` の本体のように、末尾式を持たない (常に `;` で終わる文の列である)
/// 位置の `{ .. }`。実コンパイラの `consume_block_statement` に対応する。
pub(crate) fn lower_block_stmt_strict(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> BlockStmt {
    let body = lower_block_body(mod_id, interner, node, errors);
    if let Some(tail) = &body.tail {
        errors.push(LowerError::new(
            "a value-producing tail expression is not allowed here (only `fn`/`scene` bodies \
             and blocks used as expressions may end with one)",
            tail.span(),
        ));
    }
    BlockStmt {
        stmts: body.stmts,
        span: body.span,
    }
}

/// 関数・メソッド本体だけが許される「文の列 + 省略可能な末尾式」の形。
/// 実コンパイラの `ExprOrStmt` (`consume_block_expression_or_statement`) に対応する。
pub(crate) fn lower_fn_body(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> (Vec<Stmt>, Option<Exprs>, Span) {
    let body = lower_block_body(mod_id, interner, node, errors);
    (body.stmts, body.tail, body.span)
}

/// 式として使われる `{ .. }` (if 式の腕、裸のブロック式)。必ず末尾式が要る。
pub(crate) fn lower_block_expr_mandatory(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<BlockExpr> {
    let body = lower_block_body(mod_id, interner, node, errors);
    match body.tail {
        Some(tail) => Some(BlockExpr {
            stmts: body.stmts,
            expr: Box::new(tail),
            span: body.span,
        }),
        None => {
            errors.push(LowerError::new(
                "a block used as an expression must end with an expression (no trailing `;`)",
                body.span,
            ));
            None
        }
    }
}

fn lower_var_decl(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<VarDecl> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwLet);
    let id_tok = children.eat_token(SyntaxKind::Ident)?;
    let id = intern_ident_token(mod_id, interner, &id_tok);

    let typ = if children.eat_token(SyntaxKind::Colon).is_some() {
        let ty_node = children.eat_node(SyntaxKind::TypeRepr)?;
        TypDecl::Typ(lower_type_repr(mod_id, interner, &ty_node, errors)?)
    } else {
        TypDecl::Any
    };

    children.eat_token(SyntaxKind::Eq);
    let init_node = children.next_node()?;
    let init = lower_expr(mod_id, interner, &init_node, errors)?;
    children.eat_token(SyntaxKind::Semi);

    Some(VarDecl {
        typ,
        id,
        init,
        span,
        var_id: OnceCell::new(),
    })
}

fn lower_expr_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<ExprStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let expr_node = children.next_node()?;
    let expr = lower_expr(mod_id, interner, &expr_node, errors)?;
    Some(ExprStmt { expr, span })
}

fn lower_assign_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<AssignStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let dst_node = children.next_node()?;
    let dst_expr = lower_expr(mod_id, interner, &dst_node, errors)?;
    let Exprs::Primary(dst) = dst_expr else {
        errors.push(LowerError::new(
            "the left-hand side of an assignment must be a variable or field access",
            node_span(mod_id, &dst_node),
        ));
        return None;
    };
    children.eat_token(SyntaxKind::Eq);
    let src_node = children.next_node()?;
    let src = lower_expr(mod_id, interner, &src_node, errors)?;

    Some(AssignStmt { dst, src, span })
}

fn lower_while_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<WhileStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwWhile);
    let cond_node = children.next_node()?;
    let cond = lower_expr(mod_id, interner, &cond_node, errors)?;
    let block_node = children.eat_node(SyntaxKind::BlockStmt)?;
    let stmts = lower_block_stmt_strict(mod_id, interner, &block_node, errors);

    Some(WhileStmt { cond, stmts, span })
}

/// `if` 文 (値を返さない、`else` は省略可能) を、else-if の連鎖を
/// `BlockStmt{stmts: [Stmt::If(..)]}` へ畳み込みながら組み立てる。
/// 式位置の `if` ([`crate::expr::lower_if_expr`]) と同じ考え方で、
/// こちらは各段が値を返す必要がない分、素直に「1 文だけを持つブロック」に包める。
fn lower_if_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<biwac_ast::IfStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwIf);

    let mut arms: Vec<(Exprs, BlockStmt)> = Vec::new();
    let mut final_else: Option<BlockStmt> = None;

    loop {
        let cond_node = children.next_node()?;
        let cond = lower_expr(mod_id, interner, &cond_node, errors)?;
        let then_node = children.eat_node(SyntaxKind::BlockStmt)?;
        let then = lower_block_stmt_strict(mod_id, interner, &then_node, errors);
        arms.push((cond, then));

        if children.eat_token(SyntaxKind::KwElse).is_none() {
            break;
        }
        if children.eat_token(SyntaxKind::KwIf).is_some() {
            continue;
        }
        let else_node = children.eat_node(SyntaxKind::BlockStmt)?;
        final_else = Some(lower_block_stmt_strict(mod_id, interner, &else_node, errors));
        break;
    }

    let mut iter = arms.into_iter().rev();
    let (last_cond, last_then) = iter.next()?;
    let mut acc = biwac_ast::IfStmt {
        cond: last_cond,
        then: last_then,
        els: final_else,
        span: span.clone(),
    };
    for (cond, then) in iter {
        let wrapped_els = BlockStmt {
            stmts: vec![Stmt::If(acc)],
            span: span.clone(),
        };
        acc = biwac_ast::IfStmt {
            cond,
            then,
            els: Some(wrapped_els),
            span: span.clone(),
        };
    }
    Some(acc)
}

/// `MatchStmt` (`docs/enum-and-match.md`)。値を返さないので各アーム本体は
/// 常に `{ .. }` (`BlockStmt`) で、末尾式は持たない。
fn lower_match_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<MatchStmt> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwMatch);
    let scrutinee_node = children.next_node()?;
    let scrutinee = lower_expr(mod_id, interner, &scrutinee_node, errors)?;
    children.eat_token(SyntaxKind::LBrace);

    let mut arms = Vec::new();
    while let Some(arm_node) = children.eat_node(SyntaxKind::MatchArm) {
        let mut arm_children = Children::of(&arm_node);
        let Some(pattern) = arm_children
            .eat_node(SyntaxKind::Pattern)
            .and_then(|n| lower_pattern(mod_id, interner, &n, errors))
        else {
            continue;
        };
        arm_children.eat_token(SyntaxKind::FatArrow);
        let Some(body_node) = arm_children.eat_node(SyntaxKind::BlockStmt) else {
            continue;
        };
        let body = lower_block_stmt_strict(mod_id, interner, &body_node, errors);
        let arm_span = Span::merge(&pattern.span(), &body.span);
        arms.push(MatchStmtArm {
            pattern,
            body,
            span: arm_span,
        });
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(MatchStmt {
        scrutinee,
        arms,
        span,
    })
}

pub(crate) fn lower_stmt(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Stmt> {
    match node.kind() {
        SyntaxKind::VarDefStmt => lower_var_decl(mod_id, interner, node, errors).map(Stmt::VarDecl),
        SyntaxKind::IfStmt => lower_if_stmt(mod_id, interner, node, errors).map(Stmt::If),
        SyntaxKind::WhileStmt => lower_while_stmt(mod_id, interner, node, errors).map(Stmt::While),
        SyntaxKind::ForStmt => {
            errors.push(LowerError::new(
                "`for` loops have no representation in the compiler AST yet (only `while` exists)",
                node_span(mod_id, node),
            ));
            None
        }
        SyntaxKind::MatchStmt => lower_match_stmt(mod_id, interner, node, errors).map(Stmt::Match),
        SyntaxKind::ExprStmt => lower_expr_stmt(mod_id, interner, node, errors).map(Stmt::Expr),
        SyntaxKind::AssignStmt => lower_assign_stmt(mod_id, interner, node, errors).map(Stmt::Assign),
        SyntaxKind::BlockStmt => Some(Stmt::Block(lower_block_stmt_strict(
            mod_id, interner, node, errors,
        ))),
        _ => {
            errors.push(LowerError::new(
                "unexpected statement node",
                node_span(mod_id, node),
            ));
            None
        }
    }
}
