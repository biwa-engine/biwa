use crate::language::BiwaLanguage;
use crate::parser::Parser;
use biwa_lsp_lexer::SyntaxKind;
use rowan::GreenNode;

pub struct ParseResult {
    pub green_node: GreenNode,
    pub errors: Vec<String>,
}

impl ParseResult {
    pub fn syntax(&self) -> rowan::SyntaxNode<BiwaLanguage> {
        rowan::SyntaxNode::new_root(self.green_node.clone())
    }
}

// ── Root ─────────────────────────────────────────────────────────────────────

pub fn parse_root(p: &mut Parser) {
    p.start_node(SyntaxKind::Root);
    loop {
        p.skip_trivia();
        if p.current() == SyntaxKind::Eof {
            break;
        }
        parse_global_symbol(p);
    }
    p.finish_node();
}

fn parse_global_symbol(p: &mut Parser) {
    match p.current_non_trivia() {
        SyntaxKind::KwImport => parse_import_decl(p),
        SyntaxKind::KwFn => parse_function_def(p),
        SyntaxKind::KwStruct => parse_struct_def(p),
        SyntaxKind::KwType => parse_type_alias_def(p),
        SyntaxKind::KwImpl => parse_impl_block(p),
        SyntaxKind::KwScene => parse_scene_def(p),
        _ => {
            // エラー回復: 不明なトークンを Error ノードとして消費
            p.start_node(SyntaxKind::Error);
            p.skip_trivia();
            p.bump();
            p.finish_node();
            p.errors.push("unexpected token at top-level".into());
        }
    }
}

// ── import ───────────────────────────────────────────────────────────────────

fn parse_import_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::ImportDecl);
    p.skip_trivia();
    p.expect(SyntaxKind::KwImport);
    parse_identifier_path(p);
    // optional `as identifier`
    if p.at(SyntaxKind::KwAs) {
        p.skip_trivia();
        p.bump(); // as
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
    }
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

// ── function def ─────────────────────────────────────────────────────────────

pub(crate) fn parse_function_def(p: &mut Parser) {
    p.start_node(SyntaxKind::FunctionDef);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFn);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_function_arg_decl(p);
    p.expect(SyntaxKind::Arrow);
    parse_type_repr(p);
    parse_block(p);
    p.finish_node();
}

fn parse_method_def(p: &mut Parser) {
    p.start_node(SyntaxKind::MethodDef);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFn);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_method_arg_decl(p);
    p.expect(SyntaxKind::Arrow);
    parse_type_repr(p);
    parse_block(p);
    p.finish_node();
}

fn parse_function_arg_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::FunctionArgDecl);
    p.expect(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        p.expect(SyntaxKind::Colon);
        parse_type_repr(p);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

fn parse_method_arg_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::MethodArgDecl);
    p.expect(SyntaxKind::LParen);
    p.skip_trivia();
    p.expect(SyntaxKind::KwSelf);
    if p.at(SyntaxKind::Comma) {
        p.skip_trivia();
        p.bump();
    }
    while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        p.expect(SyntaxKind::Colon);
        parse_type_repr(p);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

// ── struct def ───────────────────────────────────────────────────────────────

fn parse_struct_def(p: &mut Parser) {
    p.start_node(SyntaxKind::StructDef);
    p.skip_trivia();
    p.expect(SyntaxKind::KwStruct);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        p.expect(SyntaxKind::Colon);
        parse_type_repr(p);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

// ── type alias ───────────────────────────────────────────────────────────────

fn parse_type_alias_def(p: &mut Parser) {
    p.start_node(SyntaxKind::TypeAliasDef);
    p.skip_trivia();
    p.expect(SyntaxKind::KwType);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

// ── impl block ───────────────────────────────────────────────────────────────

fn parse_impl_block(p: &mut Parser) {
    p.start_node(SyntaxKind::ImplBlock);
    p.skip_trivia();
    p.expect(SyntaxKind::KwImpl);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_type_repr(p);
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        // method or function
        if p.current_non_trivia() == SyntaxKind::KwFn {
            // peek ahead: after `fn name [generics?]` comes `(self` → method
            // simpler: try to detect `self` as first arg
            // We parse as function_def first; if it has `self` it's a method
            // For now, use parse_method_def heuristic based on content
            // Actually we need to look ahead. Let's just always try method first.
            if is_method_def(p) {
                parse_method_def(p);
            } else {
                parse_function_def(p);
            }
        } else {
            p.start_node(SyntaxKind::Error);
            p.skip_trivia();
            p.bump();
            p.finish_node();
        }
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

/// `fn name [generics?] ( self ...` かどうかをルックアヘッドで判定。
fn is_method_def(p: &mut Parser) -> bool {
    // 現在位置から `fn`, whitespace, ident, optional `[...]`, `(`, trivia, `self` を探す
    let mut i = p.pos;
    let tokens = &p.tokens as *const _;
    let tokens: &Vec<biwa_lsp_lexer::Token> = unsafe { &*tokens };

    // skip trivia
    while i < tokens.len() && is_trivia(tokens[i].kind) {
        i += 1;
    }
    if i >= tokens.len() || tokens[i].kind != SyntaxKind::KwFn {
        return false;
    }
    i += 1;
    while i < tokens.len() && is_trivia(tokens[i].kind) {
        i += 1;
    }
    // ident
    if i >= tokens.len() || tokens[i].kind != SyntaxKind::Ident {
        return false;
    }
    i += 1;
    while i < tokens.len() && is_trivia(tokens[i].kind) {
        i += 1;
    }
    // optional generics
    if i < tokens.len() && tokens[i].kind == SyntaxKind::LBracket {
        let mut depth = 1;
        i += 1;
        while i < tokens.len() && depth > 0 {
            match tokens[i].kind {
                SyntaxKind::LBracket => depth += 1,
                SyntaxKind::RBracket => depth -= 1,
                _ => {}
            }
            i += 1;
        }
    }
    while i < tokens.len() && is_trivia(tokens[i].kind) {
        i += 1;
    }
    // (
    if i >= tokens.len() || tokens[i].kind != SyntaxKind::LParen {
        return false;
    }
    i += 1;
    while i < tokens.len() && is_trivia(tokens[i].kind) {
        i += 1;
    }
    // self?
    i < tokens.len() && tokens[i].kind == SyntaxKind::KwSelf
}

fn is_trivia(k: SyntaxKind) -> bool {
    matches!(
        k,
        SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::LineComment
    )
}

// ── scene def ────────────────────────────────────────────────────────────────

fn parse_scene_def(p: &mut Parser) {
    p.start_node(SyntaxKind::SceneDef);
    p.skip_trivia();
    p.expect(SyntaxKind::KwScene);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    parse_function_arg_decl(p);
    p.expect(SyntaxKind::Arrow);
    parse_type_repr(p);
    // novel mode: {{ ... }}
    p.expect(SyntaxKind::DoubleLBrace);
    parse_novel_mode_body(p);
    p.expect(SyntaxKind::DoubleRBrace);
    p.finish_node();
}

fn parse_novel_mode_body(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelModeBody);
    while !p.at(SyntaxKind::DoubleRBrace) && p.current() != SyntaxKind::Eof {
        match p.current() {
            SyntaxKind::NovelText
            | SyntaxKind::NovelAt
            | SyntaxKind::NovelHash
            | SyntaxKind::NovelDollarBrace
            | SyntaxKind::NovelCloseBrace
            | SyntaxKind::Newline
            | SyntaxKind::Whitespace
            | SyntaxKind::LineComment => p.bump(),
            _ => {
                p.start_node(SyntaxKind::Error);
                p.bump();
                p.finish_node();
            }
        }
    }
    p.finish_node();
}

// ── generics ─────────────────────────────────────────────────────────────────

fn parse_generics_arg_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::GenericsArgDecl);
    p.skip_trivia();
    p.expect(SyntaxKind::LBracket);
    while !p.at(SyntaxKind::RBracket) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RBracket);
    p.finish_node();
}

// ── type repr ────────────────────────────────────────────────────────────────

fn parse_type_repr(p: &mut Parser) {
    p.start_node(SyntaxKind::TypeRepr);
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::KwVoid
        | SyntaxKind::KwInt
        | SyntaxKind::KwUint
        | SyntaxKind::KwFloat
        | SyntaxKind::KwBool => {
            p.skip_trivia();
            p.bump();
        }
        _ => {
            // identifier path
            parse_identifier_path(p);
            // optional generics
            if p.at(SyntaxKind::LBracket) {
                parse_generics_arg_list(p);
            }
        }
    }
    p.finish_node();
}

fn parse_generics_arg_list(p: &mut Parser) {
    p.start_node(SyntaxKind::GenericsArgList);
    p.skip_trivia();
    p.expect(SyntaxKind::LBracket);
    while !p.at(SyntaxKind::RBracket) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_type_repr(p);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RBracket);
    p.finish_node();
}

// ── identifier path ──────────────────────────────────────────────────────────

fn parse_identifier_path(p: &mut Parser) {
    p.start_node(SyntaxKind::IdentPath);
    p.skip_trivia();
    // optional `package::`
    if p.current_non_trivia() == SyntaxKind::KwPackage {
        p.skip_trivia();
        p.bump();
        p.expect(SyntaxKind::ColonColon);
    }
    // `self` も識別子として許可
    if p.current_non_trivia() == SyntaxKind::KwSelf {
        p.skip_trivia();
        p.bump();
    } else {
        p.expect(SyntaxKind::Ident);
    }
    // `:: ident` の繰り返し
    while p.at(SyntaxKind::ColonColon) {
        p.skip_trivia();
        p.bump(); // ::
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
    }
    p.finish_node();
}

// ── block (statement list) ───────────────────────────────────────────────────

fn parse_block(p: &mut Parser) {
    p.start_node(SyntaxKind::BlockStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_statement_or_expr(p);
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

// ── statement / expression (simplified) ──────────────────────────────────────

fn parse_statement_or_expr(p: &mut Parser) {
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::LBrace => parse_block(p),
        SyntaxKind::KwLet => parse_var_def_stmt(p),
        SyntaxKind::KwIf => parse_if_stmt(p),
        SyntaxKind::KwWhile => parse_while_stmt(p),
        SyntaxKind::KwFor => parse_for_stmt(p),
        SyntaxKind::Eof | SyntaxKind::RBrace => {}
        _ => parse_expr_or_assign_stmt(p),
    }
}

fn parse_var_def_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::VarDefStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwLet);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::Colon) {
        p.skip_trivia();
        p.bump();
        parse_type_repr(p);
    }
    p.expect(SyntaxKind::Eq);
    parse_expression(p);
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

fn parse_if_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::IfStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwIf);
    parse_expression(p);
    parse_block(p);
    while p.at(SyntaxKind::KwElse) {
        p.skip_trivia();
        p.bump(); // else
        if p.at(SyntaxKind::KwIf) {
            p.skip_trivia();
            p.bump(); // if
            parse_expression(p);
            parse_block(p);
        } else {
            parse_block(p);
            break;
        }
    }
    p.finish_node();
}

fn parse_while_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::WhileStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwWhile);
    parse_expression(p);
    parse_block(p);
    p.finish_node();
}

fn parse_for_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::ForStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFor);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    p.expect(SyntaxKind::KwIn);
    parse_expression(p);
    parse_block(p);
    p.finish_node();
}

fn parse_expr_or_assign_stmt(p: &mut Parser) {
    // 式をパースした後、`=` が来たら代入文、`;` が来たら式文、
    // どちらでもなければ block expression として扱う
    let checkpoint = p.builder.checkpoint();
    parse_expression(p);

    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::Eq => {
            p.builder
                .start_node_at(checkpoint, SyntaxKind::AssignStmt.into());
            p.skip_trivia();
            p.bump(); // =
            parse_expression(p);
            p.expect(SyntaxKind::Semi);
            p.finish_node();
        }
        SyntaxKind::Semi => {
            p.builder
                .start_node_at(checkpoint, SyntaxKind::ExprStmt.into());
            p.skip_trivia();
            p.bump(); // ;
            p.finish_node();
        }
        _ => {
            // 末尾式 (block expression の返し値): ノードを巻かないでそのまま
        }
    }
}

// ── expression (Pratt / recursive descent) ───────────────────────────────────

pub(crate) fn parse_expression(p: &mut Parser) {
    p.skip_trivia();
    if p.current_non_trivia() == SyntaxKind::KwIf {
        parse_if_expr(p);
    } else {
        parse_logical_or(p);
    }
}

fn parse_if_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::IfExpr);
    p.skip_trivia();
    p.expect(SyntaxKind::KwIf);
    parse_expression(p);
    parse_block_expr(p);
    while p.at(SyntaxKind::KwElse) {
        p.skip_trivia();
        p.bump();
        if p.at(SyntaxKind::KwIf) {
            p.skip_trivia();
            p.bump();
            parse_expression(p);
            parse_block_expr(p);
        } else {
            parse_block_expr(p);
            break;
        }
    }
    p.finish_node();
}

fn parse_block_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::BlockExpr);
    p.skip_trivia();
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_statement_or_expr(p);
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

macro_rules! parse_binary {
    ($name:ident, $next:ident, $($op:ident),+) => {
        fn $name(p: &mut Parser) {
            let checkpoint = p.builder.checkpoint();
            $next(p);
            p.skip_trivia();
            while matches!(p.current_non_trivia(), $(SyntaxKind::$op)|+) {
                p.builder.start_node_at(checkpoint, SyntaxKind::BinaryExpr.into());
                p.skip_trivia();
                p.bump();
                $next(p);
                p.finish_node();
                p.skip_trivia();
            }
        }
    };
}

parse_binary!(parse_logical_or, parse_logical_and, PipePipe);
parse_binary!(parse_logical_and, parse_relational, AmpAmp);
parse_binary!(
    parse_relational,
    parse_additive,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    BangEq
);
parse_binary!(parse_additive, parse_multiplicative, Plus, Minus);
parse_binary!(parse_multiplicative, parse_unary, Star, Slash, Percent);

fn parse_unary(p: &mut Parser) {
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::Plus | SyntaxKind::Minus | SyntaxKind::Bang => {
            p.start_node(SyntaxKind::UnaryExpr);
            p.skip_trivia();
            p.bump();
            parse_postfix(p);
            p.finish_node();
        }
        _ => parse_postfix(p),
    }
}

fn parse_postfix(p: &mut Parser) {
    let checkpoint = p.builder.checkpoint();
    parse_primary(p);
    loop {
        p.skip_trivia();
        match p.current_non_trivia() {
            SyntaxKind::LParen => {
                p.builder
                    .start_node_at(checkpoint, SyntaxKind::PostfixExpr.into());
                parse_call_arg_list(p);
                p.finish_node();
            }
            SyntaxKind::Dot => {
                p.builder
                    .start_node_at(checkpoint, SyntaxKind::PostfixExpr.into());
                p.skip_trivia();
                p.bump(); // .
                p.skip_trivia();
                p.expect(SyntaxKind::Ident);
                if p.at(SyntaxKind::LParen) {
                    parse_call_arg_list(p);
                }
                p.finish_node();
            }
            _ => break,
        }
    }
}

fn parse_call_arg_list(p: &mut Parser) {
    p.start_node(SyntaxKind::CallArgList);
    p.skip_trivia();
    p.expect(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_expression(p);
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    p.finish_node();
}

fn parse_primary(p: &mut Parser) {
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::LParen => {
            p.start_node(SyntaxKind::ParenExpr);
            p.skip_trivia();
            p.bump();
            parse_expression(p);
            p.expect(SyntaxKind::RParen);
            p.finish_node();
        }
        SyntaxKind::LBrace => {
            parse_block_expr(p);
        }
        SyntaxKind::IntLiteral
        | SyntaxKind::FloatLiteral
        | SyntaxKind::StringLiteral
        | SyntaxKind::TrueLiteral
        | SyntaxKind::FalseLiteral
        | SyntaxKind::NoneLiteral => {
            p.start_node(SyntaxKind::Literal);
            p.skip_trivia();
            p.bump();
            p.finish_node();
        }
        SyntaxKind::Ident | SyntaxKind::KwPackage | SyntaxKind::KwSelf => {
            // struct literal か ident path かを判定
            // ident path の直後に `{` が来れば struct literal
            let checkpoint = p.builder.checkpoint();
            parse_identifier_path(p);
            if p.at(SyntaxKind::LBrace) {
                p.builder
                    .start_node_at(checkpoint, SyntaxKind::StructLiteral.into());
                parse_struct_literal_fields(p);
                p.finish_node();
            }
            // else: IdentPath ノードのみ
        }
        _ => {
            p.start_node(SyntaxKind::Error);
            if p.current() != SyntaxKind::Eof {
                p.skip_trivia();
                p.bump();
            }
            p.finish_node();
            p.errors.push("expected expression".to_string());
        }
    }
}

fn parse_struct_literal_fields(p: &mut Parser) {
    p.skip_trivia();
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        p.start_node(SyntaxKind::StructLiteralField);
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        p.expect(SyntaxKind::Eq);
        parse_expression(p);
        p.finish_node();
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    fn no_errors(src: &str) {
        let r = parse(src);
        assert!(
            r.errors.is_empty(),
            "unexpected errors: {:?}\nsrc: {}",
            r.errors,
            src
        );
    }

    #[test]
    fn parse_import() {
        no_errors("import foo::bar::Baz;");
        no_errors("import foo::Bar as B;");
    }

    #[test]
    fn parse_fn() {
        no_errors("fn add(x: Int, y: Int) -> Int { x }");
        no_errors("fn nothing() -> Void {}");
    }

    #[test]
    fn parse_struct() {
        no_errors("struct Point { x: Int, y: Int, }");
        no_errors("struct Box[T] { inner: T }");
    }

    #[test]
    fn parse_impl_with_method() {
        no_errors(
            r#"
impl Point {
  fn new(x: Int, y: Int) -> Point {
    Point { x = x, y = y }
  }
  fn scale(self, n: Int) -> Int {
    self.x * n
  }
}
        "#,
        );
    }

    #[test]
    fn parse_scene() {
        no_errors("scene s(g: MyGame) -> MyGame {{\n  Hello!\n}}\n");
        no_errors("scene s(g: MyGame) -> MyGame {{\n  @biwa\n  こんにちは\n}}\n");
    }

    #[test]
    fn parse_expressions() {
        no_errors("fn f() -> Int { 1 + 2 * 3 }");
        no_errors("fn f() -> Bool { TRUE && FALSE || TRUE }");
        no_errors("fn f(x: Int) -> Int { -x }");
    }

    #[test]
    fn parse_statements() {
        no_errors(
            r#"
fn f() -> Int {
  let x = 1;
  let y: Int = 2;
  x = 3;
  if x > 0 { x } else { y }
}
        "#,
        );
    }

    #[test]
    fn parse_full_example() {
        no_errors(
            r#"
import bar::Bar;

fn add(x: Int, y: Int) -> Int {
  x + y
}

struct Foo[T] {
  a: Int,
  b: Bar,
  c: T,
}

impl[T] Foo[T] {
  fn new(b: Bar, c: T) -> Self {
    Self {
      a = 0,
      b = b,
      c = c,
    }
  }

  fn set_c(self, c: T) -> Void {
    self.c = c;
  }
}
        "#,
        );
    }
}
