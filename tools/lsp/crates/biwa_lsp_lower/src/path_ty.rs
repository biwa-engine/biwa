use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{Path, PathSegment, PrimTyp, RetTypRepr, TypRepr, TypReprVal};
use biwac_base::{IdentInterner, ModId};
use biwac_span::Span;

use crate::cursor::{Children, SyntaxNode, intern_ident_token, node_span};
use crate::error::LowerError;

/// `IdentPath` ノード (`package::foo::Bar` のような列) を `biwac_ast::Path` に直す。
///
/// 実コンパイラの `consume_qualified_identifier` は `self` を segment として
/// 受け付けない (`self` は `Variable::SelfVar` として別扱いされる)。
/// biwa-lsp-lexer には `Self` 専用のトークンが無く `self` (小文字) しか
/// キーワードとして区別できないため、`self` が Path の先頭に来る形は
/// ここでは「識別子 "self" が書かれた」ものとして扱う (実コンパイラの文法上は
/// 本来ありえない位置なので、通常のコードでは通らない)。
pub(crate) fn lower_ident_path(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
) -> Option<Path> {
    let mut children = Children::of(node);
    let mut segments = Vec::new();

    let abs_header = if let Some(pkg_tok) = children.eat_token(SyntaxKind::KwPackage) {
        let span = crate::cursor::token_span(mod_id, &pkg_tok);
        // `::` は `expect` されている前提の構文なので、無ければそのまま諦める。
        children.eat_token(SyntaxKind::ColonColon);
        Some(biwac_ast::AbsolutePathHeader::Package(span))
    } else {
        None
    };

    // 先頭セグメント: `self` (KwSelf) または通常の識別子。
    if let Some(tok) = children
        .eat_token(SyntaxKind::Ident)
        .or_else(|| children.eat_token(SyntaxKind::KwSelf))
    {
        segments.push(PathSegment::from(intern_ident_token(
            mod_id, interner, &tok,
        )));
    }

    while children.eat_token(SyntaxKind::ColonColon).is_some() {
        if let Some(tok) = children.eat_token(SyntaxKind::Ident) {
            segments.push(PathSegment::from(intern_ident_token(
                mod_id, interner, &tok,
            )));
        } else {
            break;
        }
    }

    if segments.is_empty() && abs_header.is_none() {
        return None;
    }

    Some(Path::new(abs_header, segments))
}

/// `GenericsArgList` (`[T, Int]` のような、型引数の**指定**) を読む。
/// 宣言 (`GenericsArgDecl`, `[T, U]` で識別子しか書けないもの) とは別物。
fn lower_generics_arg_list(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Vec<TypRepr> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBracket);

    let mut out = Vec::new();
    while let Some(ty_node) = children.eat_node(SyntaxKind::TypeRepr) {
        if let Some(ty) = lower_type_repr(mod_id, interner, &ty_node, errors) {
            out.push(ty);
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    out
}

/// `TypeRepr` ノードを `biwac_ast::TypRepr` に直す。
///
/// 戻り値注釈 (`-> T`) の省略による void は、この関数の外
/// (呼び出し側で `->` の有無を見て `RetTypRepr::Void`/`RetTypRepr::Typ` を
/// 組み立てる形) で扱う。実コンパイラに `Void` というキーワードは無い。
pub(crate) fn lower_type_repr(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<TypRepr> {
    let mut children = Children::of(node);
    let span = node_span(mod_id, node);

    match children.peek_kind() {
        Some(SyntaxKind::KwUint) => {
            children.next_elem();
            Some(TypRepr {
                val: TypReprVal::Primitive(PrimTyp::Uint),
                span,
            })
        }
        Some(SyntaxKind::KwInt) => {
            children.next_elem();
            Some(TypRepr {
                val: TypReprVal::Primitive(PrimTyp::Int),
                span,
            })
        }
        Some(SyntaxKind::KwFloat) => {
            children.next_elem();
            Some(TypRepr {
                val: TypReprVal::Primitive(PrimTyp::Float),
                span,
            })
        }
        Some(SyntaxKind::KwBool) => {
            children.next_elem();
            Some(TypRepr {
                val: TypReprVal::Primitive(PrimTyp::Bool),
                span,
            })
        }
        Some(SyntaxKind::IdentPath) => {
            let path_node = children.next_node()?;
            let path = lower_ident_path(mod_id, interner, &path_node)?;
            let genargs = if let Some(list_node) = children.eat_node(SyntaxKind::GenericsArgList) {
                Some(lower_generics_arg_list(
                    mod_id, interner, &list_node, errors,
                ))
            } else {
                None
            };
            Some(TypRepr::new_def_typ(path, genargs))
        }
        _ => {
            errors.push(LowerError::new("expected a type", span));
            None
        }
    }
}

/// `(-> <type-repr>)?` を読んで `RetTypRepr` にする。省略時は void
/// (`args_span` の終端を指す 0 幅の span を使う。biwac_parser の
/// `consume_return_type` が引数リストの終端で作る span と同じ考え方)。
/// 呼び出し側は `Arrow` の手前まで読み進めた `Children` を渡すこと。
pub(crate) fn lower_optional_return_type(
    mod_id: ModId,
    interner: &mut IdentInterner,
    children: &mut Children,
    args_span: &Span,
    errors: &mut Vec<LowerError>,
) -> Option<RetTypRepr> {
    if children.eat_token(SyntaxKind::Arrow).is_some() {
        let ret_node = children.eat_node(SyntaxKind::TypeRepr)?;
        Some(RetTypRepr::Typ(lower_type_repr(
            mod_id, interner, &ret_node, errors,
        )?))
    } else {
        let end = args_span.end();
        Some(RetTypRepr::Void(Span::new(mod_id, end, end)))
    }
}
