use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{Path, PathSegment, PrimTyp, RetTypRepr, TypRepr, TypReprVal};
use biwac_base::{IdentInterner, ModId};
use biwac_span::Span;

use crate::cursor::{Children, SyntaxNode, intern_ident_token, node_span};
use crate::error::LowerError;

/// `IdentPath` ノード (`package::foo::Bar` や `Self::new` のような列) を
/// `biwac_ast::Path` に直す。
///
/// 実コンパイラの `consume_qualified_identifier` は `self` を segment として
/// 受け付けない (`self` は `Variable::SelfVar` として別扱いされる)。
/// biwa-lsp-lexer でも `self` (小文字) は独立したトークンだが、キーワードを
/// 識別子として許して salvage する既存の方針に合わせ、ここでは
/// 「識別子として書かれた」ものとして扱う (実コンパイラの文法上は本来
/// ありえない位置なので、通常のコードでは通らない)。
///
/// `Self` (大文字) は `package` と同じく先頭の path header
/// (`AbsolutePathHeader::SelfTyp`) として扱う。型位置の単独 `Self`
/// (`-> Self`, `x: Self`) はこの関数を呼ぶ前に `lower_type_repr` 側で
/// `TypReprVal::SelfTyp` として弾くので、ここに来るのは `Self::foo` /
/// `Self { .. }` の形だけを想定している。
///
/// **注意**: `Self { .. }` (構造体リテラル) は実コンパイラでも
/// segments が空のまま (`Path{header: Some(SelfTyp), segments: []}`) が
/// 正しい形なので、ここでは弾かない。一方 `Path::span()` は
/// `segments.last().unwrap()` を呼ぶため、式の値として使う経路
/// (`Variable::Path`/`FnCall`) でこの形が渡るとパニックしうる。
/// そちらは呼び出し側 (`lower_ident_path_as_variable` 等) で
/// `segments.is_empty()` を追加でチェックして弾いている。
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
    } else if let Some(self_ty_tok) = children.eat_token(SyntaxKind::KwSelfType) {
        let span = crate::cursor::token_span(mod_id, &self_ty_tok);
        // `Self` 単独 (`::` が続かない) はここでは segments が空のまま
        // 下の判定で `None` になる。型位置はこの関数に来ない前提なので、
        // 式位置での裸の `Self` (本来ありえない書き方) が安全に捨てられる。
        children.eat_token(SyntaxKind::ColonColon);
        Some(biwac_ast::AbsolutePathHeader::SelfTyp(
            biwac_ast::SelfTypHeader::new(span),
        ))
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

/// 式の値として使う位置 (`Variable::Path`/`FnCall`) で `path` を使ってよいか。
///
/// `segments` が空なのに `abs_header` だけある形 (`package` 単独や、
/// `Self::` が入力途中で途切れた形) は `Path::span()` の
/// `segments.last().unwrap()` がパニックする。`Self { .. }` (構造体リテラル)
/// はこの形が正しいので、値としての経路だけここでチェックする。
pub(crate) fn path_is_usable_as_value(path: &Path) -> bool {
    !path.segments.is_empty()
}

/// `IdentPath` ノードが `Self` 単独 (続きが無い) かどうかを見る。
/// 型位置の `Self` を `TypReprVal::SelfTyp` として特別扱いするための判定。
fn is_bare_self_type(node: &SyntaxNode) -> bool {
    let mut children = Children::of(node);
    if children.eat_token(SyntaxKind::KwSelfType).is_none() {
        return false;
    }
    children.peek_kind().is_none()
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
            if is_bare_self_type(&path_node) {
                return Some(TypRepr {
                    val: TypReprVal::SelfTyp,
                    span,
                });
            }
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
