use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{
    BinOperator, BinaryExpr, BoolLiteral, CallExpr, Exprs, FloatLiteral, FnLiteral, FnLiteralArg,
    IntegerLiteral, Literal, MatchExpr, MatchExprArm, MemberAccess, Primary, StringLiteral,
    StructLiteral, UnOperator, UnaryExpr, Variable,
};
use biwac_base::{IdentInterner, ModId};
use biwac_span::Span;

use crate::cursor::{
    Children, SyntaxElement, SyntaxNode, intern_ident_token, node_span, token_span,
};
use crate::error::LowerError;
use crate::path_ty::lower_type_repr;
use crate::path_ty::{lower_ident_path, path_is_usable_as_value};
use crate::pattern::lower_pattern;
use crate::stmt::{lower_block_expr_mandatory, lower_fn_body};

/// `0x..`/`0b..` の基数つき整数と、末尾 `u` を許す biwa-lsp-lexer の
/// `IntLiteral` 正規表現を読む。基数の対応が `0o` を欠くなど
/// biwac_lexer (`compiler/src/biwac_lexer/src/number.rs`) と完全には一致しないが、
/// CST に実際に出現しうる形に対して読めれば十分としている。
fn parse_int_literal_text(text: &str) -> Option<u64> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(bin) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        u64::from_str_radix(bin, 2).ok()
    } else {
        text.strip_suffix('u').unwrap_or(text).parse::<u64>().ok()
    }
}

fn lower_literal_node(
    mod_id: ModId,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Literal> {
    let mut children = Children::of(node);
    let elem = children.next_elem()?;
    let tok = match elem {
        rowan::NodeOrToken::Token(t) => t,
        rowan::NodeOrToken::Node(n) => {
            errors.push(LowerError::new("malformed literal", node_span(mod_id, &n)));
            return None;
        }
    };
    let span = token_span(mod_id, &tok);

    match tok.kind() {
        SyntaxKind::IntLiteral => match parse_int_literal_text(tok.text()) {
            Some(val) => Some(Literal::Integer(IntegerLiteral { val, span })),
            None => {
                errors.push(LowerError::new(
                    format!("cannot parse integer literal `{}`", tok.text()),
                    span,
                ));
                None
            }
        },
        SyntaxKind::FloatLiteral => match tok.text().parse::<f64>() {
            Ok(val) => Some(Literal::Float(FloatLiteral { val, span })),
            Err(_) => {
                errors.push(LowerError::new(
                    format!("cannot parse float literal `{}`", tok.text()),
                    span,
                ));
                None
            }
        },
        SyntaxKind::StringLiteral => {
            let text = tok.text();
            // レキサの正規表現 `"[^"\n]*"` は開閉の `"` を含めて切り出す。
            let inner = text
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(text);
            match biwac_base::unescape(inner) {
                Ok(val) => Some(Literal::String(StringLiteral { val, span })),
                Err(_) => {
                    errors.push(LowerError::new(
                        format!("invalid escape sequence in string literal `{text}`"),
                        span,
                    ));
                    None
                }
            }
        }
        SyntaxKind::TrueLiteral => Some(Literal::Bool(BoolLiteral { val: true, span })),
        SyntaxKind::FalseLiteral => Some(Literal::Bool(BoolLiteral { val: false, span })),
        SyntaxKind::NoneLiteral => {
            errors.push(LowerError::new(
                "`NONE` has no representation in the compiler AST yet (no `Option` literal)",
                span,
            ));
            None
        }
        _ => {
            errors.push(LowerError::new("unrecognized literal token", span));
            None
        }
    }
}

fn lower_struct_literal(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<StructLiteral> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let path_node = children.eat_node(SyntaxKind::IdentPath)?;
    let path = lower_ident_path(mod_id, interner, &path_node)?;
    children.eat_token(SyntaxKind::LBrace);

    let mut members = Vec::new();
    while let Some(field_node) = children.eat_node(SyntaxKind::StructLiteralField) {
        let mut fchildren = Children::of(&field_node);
        let Some(ident_tok) = fchildren.eat_token(SyntaxKind::Ident) else {
            break;
        };
        let ident = intern_ident_token(mod_id, interner, &ident_tok);
        fchildren.eat_token(SyntaxKind::Eq);
        if let Some(expr_node) = fchildren.next_node() {
            if let Some(expr) = lower_expr(mod_id, interner, &expr_node, errors) {
                members.push((ident, Box::new(expr)));
            }
        }
        children.eat_token(SyntaxKind::Comma);
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(StructLiteral {
        path,
        members,
        span,
    })
}

fn lower_ident_path_as_variable(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    let probe = Children::of(node);
    if let Some(rowan::NodeOrToken::Token(tok)) = probe.peek()
        && tok.kind() == SyntaxKind::KwSelf
    {
        return Some(Exprs::Primary(Primary::Variable(Variable::SelfVar(
            token_span(mod_id, &tok),
        ))));
    }

    let Some(path) = lower_ident_path(mod_id, interner, node).filter(path_is_usable_as_value)
    else {
        errors.push(LowerError::new(
            "malformed identifier path",
            node_span(mod_id, node),
        ));
        return None;
    };
    Some(Exprs::Primary(Primary::Variable(Variable::Path(path))))
}

pub(crate) fn lower_call_arg_list(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Vec<Exprs> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LParen);
    let mut args = Vec::new();
    while let Some(arg_node) = children.next_node() {
        if let Some(e) = lower_expr(mod_id, interner, &arg_node, errors) {
            args.push(e);
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RParen);
    args
}

/// `PostfixExpr` の先頭の子 (呼び出し・`.` の対象になっている式) を lower する。
fn lower_postfix_base(
    mod_id: ModId,
    interner: &mut IdentInterner,
    elem: &SyntaxElement,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    match elem {
        rowan::NodeOrToken::Node(n) => lower_expr(mod_id, interner, n, errors),
        rowan::NodeOrToken::Token(t) => {
            errors.push(LowerError::new(
                "unexpected token in postfix expression",
                token_span(mod_id, t),
            ));
            None
        }
    }
}

/// `PostfixExpr` ノードは checkpoint によるネストで左結合の後置演算子の連鎖
/// (`a.b.c()`, `foo(1, 2).bar()`, ...) を表す。1 つのノードは
/// 「その 1 段」の演算子 1 つだけを持ち、それより手前の連鎖は
/// 先頭の子として再帰的にネストしている。
///
/// 実コンパイラの AST でも呼び出しは `<式> ( <引数列> )` (`Primary::Call`) の 1 つの形なので、
/// そのまま対応させる。ただし CST は `.ident(args)` を 1 段にまとめているので、
/// それは「メンバアクセス `x.ident` の呼び出し」に開く。
fn lower_postfix_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let first = children.next_elem()?;

    let left = lower_postfix_base(mod_id, interner, &first, errors)?;

    if children.eat_token(SyntaxKind::Dot).is_some() {
        let member_tok = children.eat_token(SyntaxKind::Ident)?;
        let member = intern_ident_token(mod_id, interner, &member_tok);
        let member_access = Exprs::Primary(Primary::MemberAccess(MemberAccess {
            left: Box::new(left),
            member,
        }));

        if let Some(args_node) = children.eat_node(SyntaxKind::CallArgList) {
            let args = lower_call_arg_list(mod_id, interner, &args_node, errors);
            return Some(Exprs::Primary(Primary::Call(CallExpr {
                callee: Box::new(member_access),
                args,
                span,
            })));
        }
        return Some(member_access);
    }

    if let Some(args_node) = children.eat_node(SyntaxKind::CallArgList) {
        let args = lower_call_arg_list(mod_id, interner, &args_node, errors);
        return Some(Exprs::Primary(Primary::Call(CallExpr {
            callee: Box::new(left),
            args,
            span,
        })));
    }

    errors.push(LowerError::new(
        "unrecognized postfix expression shape",
        span,
    ));
    None
}

fn lower_unary_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let op_tok = match children.next_elem() {
        Some(rowan::NodeOrToken::Token(t)) => t,
        _ => {
            errors.push(LowerError::new("expected a unary operator", span));
            return None;
        }
    };
    let operand_node = children.next_node()?;
    let operand = lower_expr(mod_id, interner, &operand_node, errors)?;

    let op = match op_tok.kind() {
        SyntaxKind::Minus => UnOperator::Neg,
        SyntaxKind::Plus | SyntaxKind::Bang => {
            errors.push(LowerError::new(
                format!(
                    "unary `{}` has no representation in the compiler AST yet (only unary `-` exists)",
                    op_tok.text()
                ),
                token_span(mod_id, &op_tok),
            ));
            return None;
        }
        _ => {
            errors.push(LowerError::new("unexpected unary operator", span));
            return None;
        }
    };

    Some(Exprs::Unary(UnaryExpr {
        op,
        right: Box::new(operand),
        span,
    }))
}

fn lower_binary_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    let left_node = children.next_node()?;
    let left = lower_expr(mod_id, interner, &left_node, errors)?;

    let op_tok = match children.next_elem() {
        Some(rowan::NodeOrToken::Token(t)) => t,
        _ => {
            errors.push(LowerError::new("expected a binary operator", span));
            return None;
        }
    };

    let right_node = children.next_node()?;
    let right = lower_expr(mod_id, interner, &right_node, errors)?;

    let op = match op_tok.kind() {
        SyntaxKind::Plus => BinOperator::Add,
        SyntaxKind::Minus => BinOperator::Sub,
        SyntaxKind::Star => BinOperator::Mul,
        SyntaxKind::Slash => BinOperator::Div,
        SyntaxKind::Percent => BinOperator::Mod,
        SyntaxKind::Lt => BinOperator::Lt,
        SyntaxKind::Gt => BinOperator::Gt,
        SyntaxKind::LtEq => BinOperator::Le,
        SyntaxKind::GtEq => BinOperator::Ge,
        SyntaxKind::EqEq => BinOperator::Eq,
        SyntaxKind::BangEq => BinOperator::Ne,
        SyntaxKind::AmpAmp | SyntaxKind::PipePipe => {
            errors.push(LowerError::new(
                format!(
                    "logical `{}` has no representation in the compiler AST yet",
                    op_tok.text()
                ),
                token_span(mod_id, &op_tok),
            ));
            return None;
        }
        _ => {
            errors.push(LowerError::new("unexpected binary operator", span));
            return None;
        }
    };

    Some(Exprs::Binary(BinaryExpr {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }))
}

/// `IfExpr` ノード (`if a {A} else if b {B} else {C}` のように else-if の連鎖が
/// 1 つのノードにフラットに並ぶ) を、`biwac_ast::IfExpr` (else 節を 1 つしか
/// 持たない) へ変換する。
///
/// `else if` は `else { if .. {..} else {..} }` と意味的に等価であることを
/// 使って、末尾から畳み込んでネストした `IfExpr` を組み立てる。
/// 式位置の `if` は必ず値を返さねばならないので、各段の内側の `if` は
/// `BlockExpr{stmts: [], expr: <nested if>}` として素通しする。
pub(crate) fn lower_if_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<biwac_ast::IfExpr> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwIf);

    let mut arms: Vec<(Exprs, biwac_ast::BlockExpr)> = Vec::new();

    let final_else: biwac_ast::BlockExpr = loop {
        let cond_node = children.next_node()?;
        let cond = lower_expr(mod_id, interner, &cond_node, errors)?;
        let then_node = children.eat_node(SyntaxKind::BlockExpr)?;
        let then = lower_block_expr_mandatory(mod_id, interner, &then_node, errors)?;
        arms.push((cond, then));

        if children.eat_token(SyntaxKind::KwElse).is_none() {
            errors.push(LowerError::new(
                "`if` used as an expression must have an `else` branch",
                span.clone(),
            ));
            return None;
        }
        if children.eat_token(SyntaxKind::KwIf).is_some() {
            continue;
        }
        let else_node = children.eat_node(SyntaxKind::BlockExpr)?;
        break lower_block_expr_mandatory(mod_id, interner, &else_node, errors)?;
    };

    let mut iter = arms.into_iter().rev();
    let (last_cond, last_then) = iter.next()?;
    let mut acc = biwac_ast::IfExpr {
        cond: Box::new(last_cond),
        then: last_then,
        els: final_else,
        span: span.clone(),
    };
    for (cond, then) in iter {
        let wrapped_els = biwac_ast::BlockExpr {
            stmts: vec![],
            expr: Box::new(Exprs::Primary(Primary::IfExpr(acc))),
            span: span.clone(),
        };
        acc = biwac_ast::IfExpr {
            cond: Box::new(cond),
            then,
            els: wrapped_els,
            span: span.clone(),
        };
    }
    Some(acc)
}

/// `MatchExpr` (`docs/enum-and-match.md`)。すべてのアームが値を返す。
/// アーム本体は `{ .. }` (`BlockExpr`) か、`,` で終わる裸の式のどちらか。
fn lower_match_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<MatchExpr> {
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
        let Some(body_node) = arm_children.next_node() else {
            continue;
        };
        let body = if body_node.kind() == SyntaxKind::BlockExpr {
            lower_block_expr_mandatory(mod_id, interner, &body_node, errors)
        } else {
            lower_expr(mod_id, interner, &body_node, errors).map(|e| {
                let e_span = e.span();
                biwac_ast::BlockExpr {
                    stmts: vec![],
                    expr: Box::new(e),
                    span: e_span,
                }
            })
        };
        if let Some(body) = body {
            let arm_span = Span::merge(&pattern.span(), &body.span);
            arms.push(MatchExprArm {
                pattern,
                body,
                span: arm_span,
            });
        }
    }
    children.eat_token(SyntaxKind::RBrace);

    Some(MatchExpr {
        scrutinee: Box::new(scrutinee),
        arms,
        span,
    })
}

/// 式の位置に現れうる CST ノードを種類ごとに振り分ける。
///
/// biwa-lsp-parser の文法は式ノードを `PrimaryExpr` で包まない
/// (実際には一度も生成されない予約 kind)。後置演算子が 1 つも
/// 付かない式は `parse_primary` が直接作った kind (`Literal`, `IdentPath`,
/// `StructLiteral`, `ParenExpr`, `BlockExpr`, ...) のまま子として現れる。
/// `FnLiteral` ノード (`fn ( <引数> ,* ) ( -> <型> )? <ブロック>`) を無名関数にする。
fn lower_fn_literal(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<FnLiteral> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::KwFn);
    children.eat_token(SyntaxKind::LParen);

    let mut args = Vec::new();
    while let Some(arg_node) = children.eat_node(SyntaxKind::FnLiteralArg) {
        let mut arg_children = Children::of(&arg_node);
        let id_tok = arg_children.eat_token(SyntaxKind::Ident)?;
        let id = intern_ident_token(mod_id, interner, &id_tok);
        let typ = if arg_children.eat_token(SyntaxKind::Colon).is_some() {
            let ty_node = arg_children.eat_node(SyntaxKind::TypeRepr)?;
            Some(lower_type_repr(mod_id, interner, &ty_node, errors)?)
        } else {
            None
        };
        args.push(FnLiteralArg {
            id,
            typ,
            var_id: std::cell::OnceCell::new(),
        });
        children.eat_token(SyntaxKind::Comma);
    }
    children.eat_token(SyntaxKind::RParen);

    let rtype = if children.eat_token(SyntaxKind::Arrow).is_some() {
        let ty_node = children.eat_node(SyntaxKind::TypeRepr)?;
        Some(lower_type_repr(mod_id, interner, &ty_node, errors)?)
    } else {
        None
    };

    let body_node = children.eat_node(SyntaxKind::BlockStmt)?;
    let (stmts, expr, _) = lower_fn_body(mod_id, interner, &body_node, errors);

    Some(FnLiteral {
        args,
        rtype,
        stmts,
        expr: expr.map(Box::new),
        span,
    })
}

pub(crate) fn lower_expr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Exprs> {
    match node.kind() {
        SyntaxKind::BinaryExpr => lower_binary_expr(mod_id, interner, node, errors),
        SyntaxKind::UnaryExpr => lower_unary_expr(mod_id, interner, node, errors),
        SyntaxKind::PostfixExpr => lower_postfix_expr(mod_id, interner, node, errors),
        SyntaxKind::IfExpr => lower_if_expr(mod_id, interner, node, errors)
            .map(|e| Exprs::Primary(Primary::IfExpr(e))),
        SyntaxKind::BlockExpr => lower_block_expr_mandatory(mod_id, interner, node, errors)
            .map(|b| Exprs::Primary(Primary::Block(b))),
        SyntaxKind::MatchExpr => lower_match_expr(mod_id, interner, node, errors)
            .map(|m| Exprs::Primary(Primary::Match(m))),
        SyntaxKind::ParenExpr => {
            let mut children = Children::of(node);
            children.eat_token(SyntaxKind::LParen);
            let inner = children.next_node()?;
            lower_expr(mod_id, interner, &inner, errors)
        }
        SyntaxKind::Literal => {
            lower_literal_node(mod_id, node, errors).map(|l| Exprs::Primary(Primary::Literal(l)))
        }
        SyntaxKind::IdentPath => lower_ident_path_as_variable(mod_id, interner, node, errors),
        SyntaxKind::StructLiteral => lower_struct_literal(mod_id, interner, node, errors)
            .map(|s| Exprs::Primary(Primary::Literal(Literal::Struct(s)))),
        SyntaxKind::FnLiteral => lower_fn_literal(mod_id, interner, node, errors)
            .map(|f| Exprs::Primary(Primary::FnLiteral(f))),
        SyntaxKind::Error => {
            errors.push(LowerError::new(
                "cannot lower a syntax error into an expression",
                node_span(mod_id, node),
            ));
            None
        }
        _ => {
            errors.push(LowerError::new(
                "unexpected node in expression position",
                node_span(mod_id, node),
            ));
            None
        }
    }
}
