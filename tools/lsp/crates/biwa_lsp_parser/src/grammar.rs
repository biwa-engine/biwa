use crate::language::BiwaLanguage;
use crate::parser::{ParseError, Parser};
use biwa_lsp_lexer::SyntaxKind;
use rowan::GreenNode;

pub struct ParseResult {
    pub green_node: GreenNode,
    pub errors: Vec<ParseError>,
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
    // 可視性 (`pub` ..) は各宣言ノードの先頭の子として読むので、その先で振り分ける。
    match kind_after_visibility(p) {
        SyntaxKind::KwImport => parse_import_decl(p),
        SyntaxKind::KwMod => parse_mod_decl(p),
        SyntaxKind::KwFn => parse_function_def(p),
        SyntaxKind::KwStruct => parse_struct_def(p),
        SyntaxKind::KwEnum => parse_enum_def(p),
        SyntaxKind::KwTrait => parse_trait_def(p),
        SyntaxKind::KwType => parse_type_alias_def(p),
        SyntaxKind::KwImpl => parse_impl_block(p),
        SyntaxKind::KwScene => parse_scene_def(p),
        _ => {
            // エラー回復: 不明なトークンを Error ノードとして消費
            p.skip_trivia();
            p.push_error("unexpected token at top-level");
            p.start_node(SyntaxKind::Error);
            p.bump();
            p.finish_node();
        }
    }
}

// ── visibility ───────────────────────────────────────────────────────────────
// `docs/symbol-visibility-impl-status.md`。biwac_parser と同じく、書けない場所でも
// 読んだうえでエラーにする (`reject_visibility`)。

/// 可視性があれば `Visibility` ノードとして読む。無ければ何もしない。
///
/// <visibility> ::= "pub" ( "(" ( "super" | "package" ) ")" )?
/// 読んだら `true`。
fn parse_opt_visibility(p: &mut Parser) -> bool {
    if p.current_non_trivia() != SyntaxKind::KwPub {
        return false;
    }
    p.skip_trivia();
    p.start_node(SyntaxKind::Visibility);
    p.bump(); // pub
    if p.at(SyntaxKind::LParen) {
        p.skip_trivia();
        p.bump(); // (
        p.skip_trivia();
        match p.current() {
            SyntaxKind::KwSuper | SyntaxKind::KwPackage => p.bump(),
            _ => p.push_error("expected `super` or `package` after `pub(`"),
        }
        p.expect(SyntaxKind::RParen);
    }
    p.finish_node();
    true
}

/// 可視性を書けない場所。書かれていれば読んだうえでエラーにする。
fn reject_visibility(p: &mut Parser, place: &str) {
    let begin = p.pos;
    if parse_opt_visibility(p) {
        let start = p.tokens[begin..]
            .iter()
            .find(|t| t.kind == SyntaxKind::KwPub)
            .map(|t| t.start)
            .unwrap_or(0);
        let end = p.tokens[p.pos - 1].end;
        p.errors.push(crate::parser::ParseError {
            message: format!("a visibility is not allowed on {place}"),
            start,
            end,
        });
    }
}

/// 可視性 (`pub` / `pub(..)`) を読み飛ばした先のトークンの kind。
/// 宣言の種類を、可視性ごとそのノードに入れる前に決めるための先読み。
fn kind_after_visibility(p: &Parser) -> SyntaxKind {
    let tokens = &p.tokens;
    let mut i = p.pos;
    let skip = |i: &mut usize| {
        while *i < tokens.len() && is_trivia(tokens[*i].kind) {
            *i += 1;
        }
    };
    skip(&mut i);
    if i < tokens.len() && tokens[i].kind == SyntaxKind::KwPub {
        i += 1;
        skip(&mut i);
        if i < tokens.len() && tokens[i].kind == SyntaxKind::LParen {
            while i < tokens.len() && tokens[i].kind != SyntaxKind::RParen {
                i += 1;
            }
            i += 1;
            skip(&mut i);
        }
    }
    tokens.get(i).map(|t| t.kind).unwrap_or(SyntaxKind::Eof)
}

// ── mod ──────────────────────────────────────────────────────────────────────

/// `<visibility>? mod <ident> ;`
fn parse_mod_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::ModDecl);
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwMod);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

// ── import ───────────────────────────────────────────────────────────────────

fn parse_import_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::ImportDecl);
    // 可視性を書けば re-export になる (`docs/useful-import-patterns-impl-status.md`)。
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwImport);
    // 最後に `::*` (glob) を書ける。`*` は IdentPath ノードの中の `Star` トークンになる。
    parse_path(p, true);
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
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFn);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_function_arg_decl(p);
    parse_optional_return_type(p);
    parse_block(p);
    p.finish_node();
}

/// `(-> <type-repr>)?`。省略時は void を表す。実コンパイラに `Void` という
/// キーワードは無く、`->` そのものを省略することで戻り値なしを表す
/// (`fn f() { .. }` と `fn f() -> Int { .. }` の対比)。
fn parse_optional_return_type(p: &mut Parser) {
    if p.at(SyntaxKind::Arrow) {
        p.skip_trivia();
        p.bump();
        parse_type_repr(p);
    }
}

fn parse_method_def(p: &mut Parser) {
    p.start_node(SyntaxKind::MethodDef);
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFn);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_method_arg_decl(p);
    parse_optional_return_type(p);
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
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwStruct);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        // メンバの可視性は、メンバ名の直前の `Visibility` ノードになる。
        parse_opt_visibility(p);
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

// ── enum def ─────────────────────────────────────────────────────────────────
// `docs/enum-and-match.md`

fn parse_enum_def(p: &mut Parser) {
    p.start_node(SyntaxKind::EnumDef);
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwEnum);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_variant_decl(p);
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

/// `Red` (unit) / `Rgb(Int, Int, Int)` (tuple) / `Named { name: String }` (struct)
fn parse_variant_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::VariantDecl);
    // variant (とそのフィールド) は常に enum と同じ可視性になる。
    reject_visibility(
        p,
        "an enum variant (it has the same visibility as the enum)",
    );
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LParen) {
        p.skip_trivia();
        p.bump(); // (
        while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
            reject_visibility(
                p,
                "a field of an enum variant (it has the same visibility as the enum)",
            );
            parse_type_repr(p);
            if p.at(SyntaxKind::Comma) {
                p.skip_trivia();
                p.bump();
            } else {
                break;
            }
        }
        p.expect(SyntaxKind::RParen);
    } else if p.at(SyntaxKind::LBrace) {
        p.skip_trivia();
        p.bump(); // {
        while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
            reject_visibility(
                p,
                "a field of an enum variant (it has the same visibility as the enum)",
            );
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
    }
    p.finish_node();
}

// ── trait def ────────────────────────────────────────────────────────────────
// `docs/trait.md`

fn parse_trait_def(p: &mut Parser) {
    p.start_node(SyntaxKind::TraitDef);
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwTrait);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        if kind_after_visibility(p) == SyntaxKind::KwFn {
            parse_trait_item_decl(p);
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

/// trait の項目。本体を持たず `;` で終わる。`self` を取ればメソッド形式、
/// 取らなければ関連関数形式になる (`is_method_def` で先読みして判定)。
fn parse_trait_item_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::TraitItemDecl);
    // trait の項目は trait と同じ可視性になる。
    reject_visibility(
        p,
        "an item of a trait (it has the same visibility as the trait)",
    );
    // `fn` の手前で先読みする。関数/メソッド定義の判定と同じ仕組み。
    let is_method = is_method_def(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwFn);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    if is_method {
        parse_method_arg_decl(p);
    } else {
        parse_function_arg_decl(p);
    }
    parse_optional_return_type(p);
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

// ── type alias ───────────────────────────────────────────────────────────────

fn parse_type_alias_def(p: &mut Parser) {
    p.start_node(SyntaxKind::TypeAliasDef);
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwType);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    p.expect(SyntaxKind::Eq);
    parse_type_repr(p);
    p.expect(SyntaxKind::Semi);
    p.finish_node();
}

// ── impl block ───────────────────────────────────────────────────────────────

fn parse_impl_block(p: &mut Parser) {
    p.start_node(SyntaxKind::ImplBlock);
    // impl ブロックそのものは可視性を持たない (項目が個別に持つ)。
    reject_visibility(p, "an impl block (put it on each item instead)");
    p.skip_trivia();
    p.expect(SyntaxKind::KwImpl);
    if p.at(SyntaxKind::LBracket) {
        parse_generics_arg_decl(p);
    }
    parse_type_repr(p);
    // `impl Ty: Trait { .. }` (`docs/trait.md`)。直後が `{` か `:` かの
    // 1トークンで決まるので曖昧さは無い。
    let is_trait_impl = p.at(SyntaxKind::Colon);
    if is_trait_impl {
        p.skip_trivia();
        p.bump(); // :
        parse_type_repr(p);
    }
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        // method or function
        if kind_after_visibility(p) == SyntaxKind::KwFn {
            // trait impl の項目は trait と同じ可視性になる。可視性は
            // 関数/メソッドのノードの中で読むので、ここではエラーだけ積む。
            if is_trait_impl && p.current_non_trivia() == SyntaxKind::KwPub {
                p.push_error(
                    "a visibility is not allowed on an item of a trait impl \
                     (it has the same visibility as the trait)",
                );
            }
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
    // 可視性 (`pub` / `pub(..)`) を飛ばす。
    if i < tokens.len() && tokens[i].kind == SyntaxKind::KwPub {
        i += 1;
        while i < tokens.len() && is_trivia(tokens[i].kind) {
            i += 1;
        }
        if i < tokens.len() && tokens[i].kind == SyntaxKind::LParen {
            while i < tokens.len() && tokens[i].kind != SyntaxKind::RParen {
                i += 1;
            }
            i += 1;
            while i < tokens.len() && is_trivia(tokens[i].kind) {
                i += 1;
            }
        }
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
    parse_opt_visibility(p);
    p.skip_trivia();
    p.expect(SyntaxKind::KwScene);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    parse_function_arg_decl(p);
    parse_optional_return_type(p);
    // novel mode: {{ ... }}
    p.expect(SyntaxKind::DoubleLBrace);
    parse_novel_mode_body(p);
    p.expect(SyntaxKind::DoubleRBrace);
    p.finish_node();
}

/// ノベルモードの本体。`#` コマンドと `$` 埋め込み式の中身は、字句解析の段階
/// (`biwa_lsp_lexer::lex_novel_segment`) で既に通常コードのトークン列として
/// 切り出されている (`#` コマンドの複数行への継続も、境界となるトークンを
/// またいでレクサ側で解決済み)。ここではその境界だけを見て文の列に組み立てる。
fn parse_novel_mode_body(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelModeBody);
    parse_novel_stmt_seq(p);
    p.finish_node();
}

/// `}}` (scene 全体の終端) か `}` (`#if` ブロックの終端) か Eof で止まるまで、
/// ノベル文の並びを読む。終端トークン自体は呼び出し側が消費する。
fn parse_novel_stmt_seq(p: &mut Parser) {
    loop {
        p.skip_trivia();
        match p.current_non_trivia() {
            SyntaxKind::DoubleRBrace | SyntaxKind::RBrace | SyntaxKind::Eof => return,
            SyntaxKind::NovelHash => parse_novel_command_line(p),
            SyntaxKind::NovelAt => parse_novel_chara_line(p),
            SyntaxKind::NovelText | SyntaxKind::NovelDollar | SyntaxKind::NovelWait => {
                parse_novel_content_element(p)
            }
            _ => {
                // 字句解析が `Error` として弾いたトークン、または想定外の
                // 通常コードトークン (境界判定の食い違い)。1 つ読み捨てて進む。
                p.push_error("unexpected token in novel mode body");
                p.start_node(SyntaxKind::Error);
                p.skip_trivia();
                p.bump();
                p.finish_node();
            }
        }
    }
}

/// 生の地の文 (`NovelText`)・待ちコマンド (`NovelWait`)・埋め込み式
/// (`NovelDollar` に続く式) のうち 1 要素を読む。
fn parse_novel_content_element(p: &mut Parser) {
    match p.current_non_trivia() {
        SyntaxKind::NovelDollar => parse_novel_embedded_expr(p),
        _ => {
            // NovelText / NovelWait はそのままのトークンとして置く。
            p.skip_trivia();
            p.bump();
        }
    }
}

/// 次の (`Whitespace`/`LineComment` を飛ばした先の) トークンが、間に改行を
/// 挟まず同じ行にあるか。`current_non_trivia` は改行も読み飛ばしてしまうので、
/// ノベルモードで「1 行分だけ」を境界にしたい場面ではこちらを使う。
fn next_is_on_same_line(p: &Parser) -> bool {
    let mut i = 0;
    loop {
        match p.nth(i) {
            SyntaxKind::Whitespace | SyntaxKind::LineComment => i += 1,
            SyntaxKind::Newline => return false,
            _ => return true,
        }
    }
}

/// `$` に続く埋め込み式。中身は通常コードの式と同じ文法で読む
/// (境界は字句解析側で「呼び出しで終わる」まで測ってあるので、ここでは
/// 普通に式をパースするだけで自然とその境界で止まる)。
fn parse_novel_embedded_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelEmbeddedExpr);
    p.skip_trivia();
    p.bump(); // $
    parse_delimited_expression(p);
    p.finish_node();
}

/// `@` に続くキャラクター指定行。中身は地の文と同じ形 (`NovelText`/`$`/`>>`)
/// だが、実コンパイラ (`biwac_novel_parser`) 側もまだこれを `NovelStmt` へ
/// 変換する文法を持たない (`CharaCommand => todo!()`) ので、lowering では
/// 中身を読み捨てる。ここでは CST 上で崩さず読めれば十分。
fn parse_novel_chara_line(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelCharaLine);
    p.skip_trivia();
    p.bump(); // @
    // `current_non_trivia` は改行も読み飛ばすので、それだけで次の要素を
    // 判定すると次の行の地の文まで `@` 行に取り込んでしまう。1 行分だけに
    // 留めるため、間に改行を挟まないことも確認する。
    while next_is_on_same_line(p)
        && matches!(
            p.current_non_trivia(),
            SyntaxKind::NovelText | SyntaxKind::NovelDollar | SyntaxKind::NovelWait
        )
    {
        parse_novel_content_element(p);
    }
    p.finish_node();
}

/// `#` に続く 1 つの論理コマンド。`if`/`let`/`endscene`/それ以外 (式文 or 代入文)
/// に分岐する。実コンパイラの `consume_statements` の `GeneralCommand` 腕に対応する。
fn parse_novel_command_line(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelCommandLine);
    p.skip_trivia();
    p.bump(); // #
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::KwIf => parse_novel_if_stmt(p),
        SyntaxKind::KwLet => parse_novel_let_stmt(p),
        SyntaxKind::KwEndScene => parse_novel_end_scene_stmt(p),
        _ => parse_novel_expr_or_assign_stmt(p),
    }
    p.finish_node();
}

/// ノベル `#if <cond> { .. }`。実コンパイラと同じく `else` は無い
/// (`biwac_novel_parser::symbols::statements::if_stmt` も `els` を作らない)。
/// 本体の閉じは `}}` ではなく単独の `}` 行 (`biwa_lsp_lexer` が `RBrace` として
/// 認識する)。
fn parse_novel_if_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::IfStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwIf);
    parse_condition_expression(p);
    parse_novel_block_stmt(p);
    p.finish_node();
}

/// `#if` の本体。`{` `}` の内側は再びノベル文の並び。
fn parse_novel_block_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::BlockStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::LBrace);
    parse_novel_stmt_seq(p);
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

/// ノベル `#let <ident> (":" <type>)? "=" <expr>`。`;` は無い
/// (行境界がそのまま文の終わりになる)。
fn parse_novel_let_stmt(p: &mut Parser) {
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
    p.finish_node();
}

/// ノベル `#endscene <expr>`。
fn parse_novel_end_scene_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::NovelEndSceneStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwEndScene);
    parse_expression(p);
    p.finish_node();
}

/// ノベル `#` コマンドの式文・代入文。通常コードの `parse_expr_or_assign_stmt`
/// と同じ checkpoint パターンだが、`;` は無い (行境界が終わりを決める)。
fn parse_novel_expr_or_assign_stmt(p: &mut Parser) {
    let checkpoint = p.builder.checkpoint();
    parse_expression(p);
    p.skip_trivia();
    if p.current_non_trivia() == SyntaxKind::Eq {
        p.builder
            .start_node_at(checkpoint, SyntaxKind::AssignStmt.into());
        p.skip_trivia();
        p.bump(); // =
        parse_expression(p);
        p.finish_node();
    } else {
        p.builder
            .start_node_at(checkpoint, SyntaxKind::ExprStmt.into());
        p.finish_node();
    }
}

// ── generics ─────────────────────────────────────────────────────────────────

fn parse_generics_arg_decl(p: &mut Parser) {
    p.start_node(SyntaxKind::GenericsArgDecl);
    p.skip_trivia();
    p.expect(SyntaxKind::LBracket);
    while !p.at(SyntaxKind::RBracket) && p.current_non_trivia() != SyntaxKind::Eof {
        p.skip_trivia();
        parse_generics_arg_item(p);
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

/// `T` または `T: A && B` (`docs/trait.md` の第2段)。
fn parse_generics_arg_item(p: &mut Parser) {
    p.start_node(SyntaxKind::GenericsArgItem);
    p.skip_trivia();
    p.expect(SyntaxKind::Ident);
    if p.at(SyntaxKind::Colon) {
        p.skip_trivia();
        p.bump(); // :
        parse_type_repr(p);
        while p.at(SyntaxKind::AmpAmp) {
            p.skip_trivia();
            p.bump(); // &&
            parse_type_repr(p);
        }
    }
    p.finish_node();
}

// ── type repr ────────────────────────────────────────────────────────────────

fn parse_type_repr(p: &mut Parser) {
    p.start_node(SyntaxKind::TypeRepr);
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::KwInt | SyntaxKind::KwUint | SyntaxKind::KwFloat | SyntaxKind::KwBool => {
            p.skip_trivia();
            p.bump();
        }
        // 関数型 `fn(A, B) -> C` / `fn(A)`。量化子 (`fn[T](..)`) は書けない。
        // 子は TypeRepr ノードと区切りのトークンのまま並べる (lowering が `fn` で見分ける)。
        SyntaxKind::KwFn => {
            p.skip_trivia();
            p.bump(); // fn
            p.expect(SyntaxKind::LParen);
            while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
                parse_type_repr(p);
                if p.at(SyntaxKind::Comma) {
                    p.skip_trivia();
                    p.bump();
                } else {
                    break;
                }
            }
            p.expect(SyntaxKind::RParen);
            if p.at(SyntaxKind::Arrow) {
                p.skip_trivia();
                p.bump(); // ->
                parse_type_repr(p);
            }
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
    parse_path(p, false);
}

/// `::` の直後が `*` (glob) なら読んで `true`。`allow_glob` でなければ読まない。
fn eat_glob(p: &mut Parser, allow_glob: bool) -> bool {
    if allow_glob && p.at(SyntaxKind::Star) {
        p.skip_trivia();
        p.bump(); // *
        true
    } else {
        false
    }
}

/// パス。`allow_glob` なら最後に `::*` を書ける (import だけ)。
fn parse_path(p: &mut Parser, allow_glob: bool) {
    p.start_node(SyntaxKind::IdentPath);
    p.skip_trivia();
    // optional `package::`
    if p.current_non_trivia() == SyntaxKind::KwPackage {
        p.skip_trivia();
        p.bump();
        p.expect(SyntaxKind::ColonColon);
        if eat_glob(p, allow_glob) {
            p.finish_node();
            return;
        }
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
    } else if p.current_non_trivia() == SyntaxKind::KwSuper {
        // `super::` (`super::super::..`)。後には必ず識別子 (import なら `*` でもよい) が来る。
        while p.current_non_trivia() == SyntaxKind::KwSuper {
            p.skip_trivia();
            p.bump();
            p.expect(SyntaxKind::ColonColon);
        }
        if eat_glob(p, allow_glob) {
            p.finish_node();
            return;
        }
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
    } else if p.current_non_trivia() == SyntaxKind::KwSelfType {
        // `Self` (型)。単独 (型位置) でも `Self::foo` (式位置) でもよいので、
        // `package` と違い `::` は無くてもエラーにしない。
        p.skip_trivia();
        p.bump();
        if p.at(SyntaxKind::ColonColon) {
            p.skip_trivia();
            p.bump();
            p.skip_trivia();
            p.expect(SyntaxKind::Ident);
        }
    } else if p.current_non_trivia() == SyntaxKind::KwSelf {
        // `self` も識別子として許可
        p.skip_trivia();
        p.bump();
    } else {
        p.expect(SyntaxKind::Ident);
    }
    // `:: ident` の繰り返し
    while p.at(SyntaxKind::ColonColon) {
        p.skip_trivia();
        p.bump(); // ::
        if eat_glob(p, allow_glob) {
            break;
        }
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
        SyntaxKind::KwMatch => parse_match_stmt(p),
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
    parse_condition_expression(p);
    parse_block(p);
    while p.at(SyntaxKind::KwElse) {
        p.skip_trivia();
        p.bump(); // else
        if p.at(SyntaxKind::KwIf) {
            p.skip_trivia();
            p.bump(); // if
            parse_condition_expression(p);
            parse_block(p);
        } else {
            parse_block(p);
            break;
        }
    }
    p.finish_node();
}

// ── match / pattern ──────────────────────────────────────────────────────────
// `docs/enum-and-match.md`
//
// `if` と同じく、この文法では「文位置の match」と「式位置の match」を
// 呼び出し位置 (statement vs expression) で分けている。本来は最初のアームの
// 本体の形 (ブロックか裸の式か) で決まる (`ExprOrStmt`) が、`if` の側が
// 既にその単純化を採っているので揃えてある (`parse_if_stmt` / `parse_if_expr`
// と同じ限界。`fn f() -> T { match .. { .. } }` のように tail 式として
// 裸で置く形は式として読まれない)。

fn parse_match_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::MatchStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwMatch);
    parse_condition_expression(p);
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_match_arm_stmt(p);
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

fn parse_match_arm_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::MatchArm);
    parse_pattern(p);
    p.skip_trivia();
    p.expect(SyntaxKind::FatArrow);
    parse_block(p);
    if p.at(SyntaxKind::Comma) {
        p.skip_trivia();
        p.bump();
    }
    p.finish_node();
}

fn parse_match_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::MatchExpr);
    p.skip_trivia();
    p.expect(SyntaxKind::KwMatch);
    parse_condition_expression(p);
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_match_arm_expr(p);
    }
    p.expect(SyntaxKind::RBrace);
    p.finish_node();
}

/// アームの本体。`{ .. }` か、`,` で終わる裸の式 (`docs/enum-and-match.md`)。
fn parse_match_arm_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::MatchArm);
    parse_pattern(p);
    p.skip_trivia();
    p.expect(SyntaxKind::FatArrow);
    if p.at(SyntaxKind::LBrace) {
        parse_block_expr(p);
    } else {
        parse_expression(p);
    }
    if p.at(SyntaxKind::Comma) {
        p.skip_trivia();
        p.bump();
    }
    p.finish_node();
}

// <pattern> ::= "_" | <identifier> | <qualified-identifier> ( "(" <pattern>* ")" | "{" .. "}" )?
fn parse_pattern(p: &mut Parser) {
    p.start_node(SyntaxKind::Pattern);
    p.skip_trivia();
    match p.current_non_trivia() {
        SyntaxKind::KwUnderscore => {
            p.skip_trivia();
            p.bump();
        }
        SyntaxKind::Ident | SyntaxKind::KwPackage | SyntaxKind::KwSuper => {
            parse_identifier_path(p);
            if p.at(SyntaxKind::LParen) {
                parse_pattern_tuple_fields(p);
            } else if p.at(SyntaxKind::LBrace) {
                parse_pattern_struct_fields(p);
            }
        }
        _ => {
            p.skip_trivia();
            p.push_error("expected a pattern");
            p.start_node(SyntaxKind::Error);
            if p.current() != SyntaxKind::Eof {
                p.bump();
            }
            p.finish_node();
        }
    }
    p.finish_node();
}

/// `Rgb(a, b, c)`
fn parse_pattern_tuple_fields(p: &mut Parser) {
    p.start_node(SyntaxKind::PatternTupleFields);
    p.skip_trivia();
    p.expect(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
        parse_pattern(p);
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

/// `Named { name = n, alpha }`。`{ alpha }` は `{ alpha = alpha }` の省略形
/// (lowering 側で展開する)。
fn parse_pattern_struct_fields(p: &mut Parser) {
    p.start_node(SyntaxKind::PatternStructFields);
    p.skip_trivia();
    p.expect(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && p.current_non_trivia() != SyntaxKind::Eof {
        p.start_node(SyntaxKind::PatternField);
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        if p.at(SyntaxKind::Eq) {
            p.skip_trivia();
            p.bump();
            parse_pattern(p);
        }
        p.finish_node();
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

fn parse_while_stmt(p: &mut Parser) {
    p.start_node(SyntaxKind::WhileStmt);
    p.skip_trivia();
    p.expect(SyntaxKind::KwWhile);
    parse_condition_expression(p);
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
    parse_condition_expression(p);
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
    } else if p.current_non_trivia() == SyntaxKind::KwMatch {
        parse_match_expr(p);
    } else {
        parse_logical_or(p);
    }
}

/// `if`/`while`/`for .. in`/`match` の対象式。構造体リテラルは認めない。
fn parse_condition_expression(p: &mut Parser) {
    let saved = std::mem::replace(&mut p.no_struct_literal, true);
    parse_expression(p);
    p.no_struct_literal = saved;
}

/// `(` `)` や引数リストの内側で読む式。制限はここで解ける。
fn parse_delimited_expression(p: &mut Parser) {
    let saved = std::mem::replace(&mut p.no_struct_literal, false);
    parse_expression(p);
    p.no_struct_literal = saved;
}

fn parse_if_expr(p: &mut Parser) {
    p.start_node(SyntaxKind::IfExpr);
    p.skip_trivia();
    p.expect(SyntaxKind::KwIf);
    parse_condition_expression(p);
    parse_block_expr(p);
    while p.at(SyntaxKind::KwElse) {
        p.skip_trivia();
        p.bump();
        if p.at(SyntaxKind::KwIf) {
            p.skip_trivia();
            p.bump();
            parse_condition_expression(p);
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
        parse_delimited_expression(p);
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
            parse_delimited_expression(p);
            p.expect(SyntaxKind::RParen);
            p.finish_node();
        }
        SyntaxKind::LBrace => {
            parse_block_expr(p);
        }
        // 無名関数。型の位置の `fn(A) -> B` とは、式の位置に現れることで区別される。
        SyntaxKind::KwFn => parse_fn_literal(p),
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
        SyntaxKind::Ident
        | SyntaxKind::KwPackage
        | SyntaxKind::KwSuper
        | SyntaxKind::KwSelf
        | SyntaxKind::KwSelfType => {
            // struct literal か ident path かを判定
            // ident path の直後に `{` が来れば struct literal
            let checkpoint = p.builder.checkpoint();
            parse_identifier_path(p);
            if p.at(SyntaxKind::LBrace) && !p.no_struct_literal {
                p.builder
                    .start_node_at(checkpoint, SyntaxKind::StructLiteral.into());
                parse_struct_literal_fields(p);
                p.finish_node();
            }
            // else: IdentPath ノードのみ
        }
        _ => {
            p.skip_trivia();
            p.push_error("expected expression");
            p.start_node(SyntaxKind::Error);
            if p.current() != SyntaxKind::Eof {
                p.bump();
            }
            p.finish_node();
        }
    }
}

/// `fn ( <引数> ,* ) ( -> <型> )? <ブロック>`。引数の型は省略できる。
fn parse_fn_literal(p: &mut Parser) {
    p.start_node(SyntaxKind::FnLiteral);
    p.skip_trivia();
    p.bump(); // fn
    p.expect(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && p.current_non_trivia() != SyntaxKind::Eof {
        p.start_node(SyntaxKind::FnLiteralArg);
        p.skip_trivia();
        p.expect(SyntaxKind::Ident);
        if p.at(SyntaxKind::Colon) {
            p.skip_trivia();
            p.bump(); // :
            parse_type_repr(p);
        }
        p.finish_node();
        if p.at(SyntaxKind::Comma) {
            p.skip_trivia();
            p.bump();
        } else {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    if p.at(SyntaxKind::Arrow) {
        p.skip_trivia();
        p.bump(); // ->
        parse_type_repr(p);
    }
    // 本体の中では構造体リテラルを書いてよい (条件式の中に書かれていても)。
    let saved = p.no_struct_literal;
    p.no_struct_literal = false;
    parse_block(p);
    p.no_struct_literal = saved;
    p.finish_node();
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
    fn parse_errors_carry_the_offending_token_span() {
        // `let x = ;` の `;` (式が無い位置) を指すはず。
        let src = "fn f() -> Int { let x = ; 1 }";
        let r = parse(src);
        assert!(!r.errors.is_empty(), "expected at least one error");
        let e = &r.errors[0];
        let semi = src.find(';').unwrap();
        assert_eq!(
            (e.start, e.end),
            (semi, semi + 1),
            "error span should point at `;`, got {:?} (text: {:?})",
            e,
            &src[e.start..e.end.min(src.len())]
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
        // `->` を省略すると戻り値なし (void)。実コンパイラに `Void`
        // というキーワードは無い (`docs/lexical.md` 参照はしていないが
        // biwac_parser の `consume_return_type` と同じ規則)。
        no_errors("fn nothing() {}");
        // `Void` は今やただの識別子として読める (未解決の型参照になるだけで、
        // 構文上のエラーにはならない)。
        no_errors("fn nothing_named_void() -> Void {}");
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

    #[test]
    fn parse_type_alias() {
        no_errors("type MyInt = Int;");
        no_errors("type MyGame = Game[MyGameState];");
        no_errors("type Boxed[T] = Box[T];");
    }

    #[test]
    fn parse_fn_literal() {
        no_errors("fn f() -> Int { let g = fn(x, y: Int) -> Int { x + y }; g(1, 2) }");
        no_errors("fn f() { apply(fn(x) { x * 2 }, 3); }");
        no_errors("fn f() -> Int { fn(x) { let y = x; y }(1) }");
    }

    #[test]
    fn parse_fn_type() {
        no_errors("fn apply(f: fn(Int, Bool) -> Int, x: Int) -> Int { x }");
        no_errors("struct Button { on_click: fn(Event), }");
        no_errors("fn pick() -> fn(Int) -> fn() { f }");
        no_errors("type Mapper[T, U] = fn(T) -> U;");
    }

    #[test]
    fn parse_enum() {
        no_errors(
            r#"
enum Color {
  Red,
  Rgb(Int, Int, Int),
  Named { name: String, alpha: Int },
}

enum Option[T] {
  None,
  Some(T),
}
        "#,
        );
    }

    #[test]
    fn parse_match_statement_and_expression() {
        no_errors(
            r#"
fn describe(c: Color) -> Int {
  match c {
    Color::Red => { }
    Color::Rgb(r, g, b) => { }
    Color::Named { name = n, alpha } => { }
    _ => { }
  }
  let n = match c {
    Color::Red => 0,
    Color::Rgb(r, g, b) => r,
    _ => 0,
  };
  n
}
        "#,
        );
    }

    #[test]
    fn parse_trait_and_impl_with_trait() {
        no_errors(
            r#"
trait Gyao {
  fn gyao(self) -> Int;
  fn guee(aaa: Int) -> Self;
}

impl Nyoee: Gyao {
  fn gyao(self) -> Int { 1 }
  fn guee(aaa: Int) -> Self { Nyoee { x = aaa } }
}
        "#,
        );
    }

    #[test]
    fn parse_generics_bounds() {
        no_errors("struct Bbb[T: Gyao] { t: T }");
        no_errors("fn f[T: A && B](t: T) -> Int { 1 }");
    }

    #[test]
    fn parse_scene_hash_command_with_args() {
        no_errors("scene s(g: G) -> G {{\n#play_se(se1, 2)\n}}\n");
    }

    #[test]
    fn parse_scene_hash_command_continuation() {
        no_errors("scene s(g: G) -> G {{\n#play_se(\n  se1,\n  2,\n)\n}}\n");
    }

    #[test]
    fn parse_scene_embedded_expression() {
        no_errors(
            r#"scene s(g: G) -> G {{
Hello! $blue(bold("a"))!
}}
"#,
        );
        no_errors("scene s(g: G) -> G {{\n$(player.hp)\n}}\n");
    }

    #[test]
    fn parse_scene_let_stmt() {
        no_errors("scene s(g: G) -> G {{\n#let x = 1\n}}\n");
        no_errors("scene s(g: G) -> G {{\n#let x: Int = 1\n}}\n");
    }

    #[test]
    fn parse_scene_assign_stmt() {
        no_errors("scene s(g: G) -> G {{\n#let x = 1\n#x = 2\n}}\n");
    }

    #[test]
    fn parse_scene_end_scene_stmt() {
        no_errors("scene s(g: G) -> G {{\n#endscene g\n}}\n");
    }

    #[test]
    fn parse_scene_if_stmt() {
        no_errors("scene s(g: G) -> G {{\n#if x > 2 {\ntext\n}\n}}\n");
    }

    #[test]
    fn parse_scene_if_stmt_with_nested_command() {
        no_errors("scene s(g: G) -> G {{\n#if x > 2 {\n#let y = 1\ntext $y()\n}\n}}\n");
    }

    #[test]
    fn parse_scene_novel_command_line_node_shape() {
        // `#play_se(se1)` は NovelCommandLine[NovelHash, ExprStmt[..]] に
        // 構造化されるはず (以前はトークンの塊のままだった)。
        use biwa_lsp_lexer::SyntaxKind;
        let result = parse("scene s(g: G) -> G {{\n#play_se(se1)\n}}\n");
        let root = result.syntax();
        let kinds: Vec<SyntaxKind> = root.descendants().map(|n| n.kind()).collect();
        assert!(
            kinds.contains(&SyntaxKind::NovelCommandLine),
            "expected a NovelCommandLine node, got {kinds:?}"
        );
        assert!(
            kinds.contains(&SyntaxKind::ExprStmt),
            "expected the command body to be an ExprStmt, got {kinds:?}"
        );
    }

    /// CST の再構成テキストが元のソースと一致すること (lossless)。
    ///
    /// 回帰テスト: ノベルモードで `#`/`@`/`}}` 行の行頭インデントが
    /// トークン化されず、それ以降の CST の全ノードの位置がずれて
    /// 壊れるバグがあった (biwa_lsp_lexer 側の修正と対で確認する)。
    fn assert_lossless(src: &str) {
        let result = parse(src);
        let tree_text = result.syntax().text().to_string();
        assert_eq!(
            tree_text, src,
            "parsed tree must reproduce the source exactly"
        );
    }

    #[test]
    fn scene_with_indented_novel_lines_is_lossless() {
        assert_lossless(
            r#"scene s(g: G) -> G {{
    #let x = f(
        1,
        2, // continues
    )
    @biwa
    text $foo(1).bar() more >>
    }}
"#,
        );
    }

    #[test]
    fn scene_with_if_block_and_end_scene_is_lossless() {
        assert_lossless(
            "scene s(g: G) -> G {{\n    #let x = add(1, 2)\n    #if x > 2 {\n         yes\n    }\n    #endscene g\n}}\n",
        );
    }
}
