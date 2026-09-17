use std::cell::OnceCell;

use biwa_lsp_lexer::SyntaxKind;
use biwac_ast::{Ident, IdentPattern, Path, PathSegment, Pattern, PatternFields, VariantPattern};
use biwac_base::{IdentInterner, ModId};

use crate::cursor::{Children, SyntaxNode, intern_ident_token, node_span, token_span};
use crate::error::LowerError;
use crate::path_ty::lower_ident_path;

/// `Pattern` ノード (`_` / `Foo` / `Foo::Bar` / `Foo::Bar(..)` / `Foo::Bar { .. }`)
/// を `biwac_ast::Pattern` に直す。
///
/// 単独の識別子が束縛なのか unit バリアントなのかは構文からは決まらない
/// (biwac_parser と同じ)。名前解決の側で振り分ける前提で、ここでは
/// 「セグメントが1つで abs_header が無い」ものを暫定的に `Pattern::Ident` とする。
pub(crate) fn lower_pattern(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Option<Pattern> {
    let span = node_span(mod_id, node);
    let mut children = Children::of(node);

    match children.peek_kind() {
        Some(SyntaxKind::KwUnderscore) => {
            let tok = children.eat_token(SyntaxKind::KwUnderscore)?;
            Some(Pattern::Wildcard(token_span(mod_id, &tok)))
        }
        Some(SyntaxKind::IdentPath) => {
            let path_node = children.next_node()?;
            let path = lower_ident_path(mod_id, interner, &path_node)?;

            match children.peek_kind() {
                Some(SyntaxKind::PatternTupleFields) => {
                    let fields_node = children.eat_node(SyntaxKind::PatternTupleFields)?;
                    let pats = lower_pattern_tuple_fields(mod_id, interner, &fields_node, errors);
                    Some(Pattern::Variant(VariantPattern {
                        path,
                        fields: PatternFields::Tuple(pats),
                        span,
                    }))
                }
                Some(SyntaxKind::PatternStructFields) => {
                    let fields_node = children.eat_node(SyntaxKind::PatternStructFields)?;
                    let fields =
                        lower_pattern_struct_fields(mod_id, interner, &fields_node, errors);
                    Some(Pattern::Variant(VariantPattern {
                        path,
                        fields: PatternFields::Struct(fields),
                        span,
                    }))
                }
                _ => {
                    if path.abs_header.is_none() && path.segments.len() == 1 {
                        Some(Pattern::Ident(IdentPattern {
                            path,
                            var_id: OnceCell::new(),
                        }))
                    } else {
                        Some(Pattern::Variant(VariantPattern {
                            path,
                            fields: PatternFields::Unit,
                            span,
                        }))
                    }
                }
            }
        }
        _ => {
            errors.push(LowerError::new("expected a pattern", span));
            None
        }
    }
}

fn lower_pattern_tuple_fields(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Vec<Pattern> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LParen);

    let mut pats = Vec::new();
    while let Some(pat_node) = children.eat_node(SyntaxKind::Pattern) {
        if let Some(p) = lower_pattern(mod_id, interner, &pat_node, errors) {
            pats.push(p);
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RParen);
    pats
}

/// `{ name = n, alpha }`。`{ alpha }` は `{ alpha = alpha }` の省略形として展開する。
fn lower_pattern_struct_fields(
    mod_id: ModId,
    interner: &mut IdentInterner,
    node: &SyntaxNode,
    errors: &mut Vec<LowerError>,
) -> Vec<(Ident, Pattern)> {
    let mut children = Children::of(node);
    children.eat_token(SyntaxKind::LBrace);

    let mut fields = Vec::new();
    while let Some(field_node) = children.eat_node(SyntaxKind::PatternField) {
        let mut field_children = Children::of(&field_node);
        if let Some(id_tok) = field_children.eat_token(SyntaxKind::Ident) {
            let name = intern_ident_token(mod_id, interner, &id_tok);
            let pattern = if field_children.eat_token(SyntaxKind::Eq).is_some() {
                field_children
                    .next_node()
                    .and_then(|n| lower_pattern(mod_id, interner, &n, errors))
            } else {
                Some(Pattern::Ident(IdentPattern {
                    path: Path::new(None, vec![PathSegment::from(name.clone())]),
                    var_id: OnceCell::new(),
                }))
            };
            if let Some(p) = pattern {
                fields.push((name, p));
            }
        }
        if children.eat_token(SyntaxKind::Comma).is_none() {
            break;
        }
    }
    children.eat_token(SyntaxKind::RBrace);
    fields
}
