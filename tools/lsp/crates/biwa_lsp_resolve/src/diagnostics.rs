//! [`biwac_name_resolver::ResolveError`] を LSP の `Diagnostic` 相当の
//! `(Span, message)` に変換する。
//!
//! `ResolveError::print_error_message` は ariadne でソース抜粋つきの表示を
//! 直接標準エラーに書き出す副作用しか持たず、構造化されたメッセージを
//! 返す API が無い。そのため、そこと同じ match を自前で持ち、
//! 使う情報(主たる label の span とメッセージ)だけを取り出す。
//! (`error.rs` を直接読みながら書いた。あちら側が変わったらここも合わせる)

use biwac_ast::{Path, PathSegment, PathSegmentResolution};
use biwac_base::{IdentInterner, ModId};
use biwac_name_resolver::ResolveError;
use biwac_span::{DefIdKind, Span};

pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

/// `doc_mod_id` を指す診断だけを取り出す。
///
/// 名前解決はパッケージ全体を 1 回で解決するため、開いていない他ファイルの
/// エラーも `errors` に混ざりうる。`textDocument/publishDiagnostics` は
/// URI ごとなので、ここで自分のファイル分だけに絞る。
pub(crate) fn extract(
    errors: &[ResolveError],
    doc_mod_id: ModId,
    interner: &IdentInterner,
) -> Vec<Diagnostic> {
    errors
        .iter()
        .filter_map(|e| to_diagnostic(e, interner))
        .filter(|d| d.mod_id == doc_mod_id)
        .map(|d| Diagnostic {
            start: d.span.begin(),
            end: d.span.end(),
            message: d.message,
        })
        .collect()
}

struct RawDiagnostic {
    span: Span,
    mod_id: ModId,
    message: String,
}

fn raw(span: Span, message: String) -> Option<RawDiagnostic> {
    let mod_id = span.module();
    Some(RawDiagnostic {
        span,
        mod_id,
        message,
    })
}

fn name<'a>(interner: &'a IdentInterner, id: &biwac_base::InternedIdent) -> &'a str {
    interner.get_str(id).unwrap_or("<unknown>")
}

fn first_unresolved(path: &Path) -> Option<&PathSegment> {
    path.segments
        .iter()
        .find(|s| !matches!(s.resolved_id.get(), Some(PathSegmentResolution::Ok(_))))
}

fn last_segment(path: &Path) -> &PathSegment {
    path.segments
        .last()
        .expect("compiler bug: path with no segment")
}

fn def_id_kind_name(kind: &DefIdKind) -> &'static str {
    match kind {
        DefIdKind::Package(_) => "a package",
        DefIdKind::Mod(_) => "a module",
        DefIdKind::Ty(_) => "a type",
        DefIdKind::Variant(_) => "an enum variant",
        DefIdKind::Val(_) => "a value",
        DefIdKind::Gen(_) | DefIdKind::LocalGen(_) => "a generic parameter",
        DefIdKind::Var(_) => "a variable",
        DefIdKind::Trait(_) => "a trait",
        DefIdKind::TraitAssoc(_) => "a trait item",
    }
}

/// パスを要求する箇所に見つかった種別が食い違ったときの定型メッセージ。
fn expected_ty_but(interner: &IdentInterner, path: &Path, found: &str) -> Option<RawDiagnostic> {
    let segment = last_segment(path);
    let n = name(interner, &segment.ident.id);
    raw(
        segment.ident.span.clone(),
        format!("Type expected here, but `{n}` is {found}."),
    )
}

fn to_diagnostic(err: &ResolveError, interner: &IdentInterner) -> Option<RawDiagnostic> {
    use ResolveError as E;
    match err {
        E::LangItem(e) => {
            let span = e.span()?.clone();
            raw(span, e.message())
        }

        // 依存パッケージ由来の重複など、位置を持たないものは出さない。
        E::HostExport(e) => {
            let span = e.span()?.clone();
            raw(span, e.message())
        }

        E::DuplicatedSymbolAndModuleName {
            name: n,
            symbol_span,
            ..
        } => {
            let n = name(interner, n);
            raw(
                symbol_span.clone(),
                format!("`{n}` is defined twice (a module already takes that name)."),
            )
        }

        E::DuplicatedSymbolName { name: n, span2, .. } => {
            let n = name(interner, n);
            raw(span2.clone(), format!("`{n}` is defined twice."))
        }

        E::DuplicatedSymbolAndDefIdName {
            name: n,
            span,
            def_id_kind,
        } => {
            let n = name(interner, n);
            raw(
                span.clone(),
                format!(
                    "`{n}` is defined twice ({} with the same name already exists).",
                    def_id_kind_name(def_id_kind)
                ),
            )
        }

        E::DuplicatedStructMember { name: n, span2, .. } => {
            let n = name(interner, n);
            raw(
                span2.clone(),
                format!("Struct member `{n}` is declared twice."),
            )
        }

        E::DuplicatedAssociatedItemForGenArgs { .. } => {
            // AssocNameTreeItem は span を持たないので、位置を示せる label が無い。
            // ここでは LSP 診断として出しようがないので諦める。
            None
        }

        E::UnexpectedSelfType { span } => raw(
            span.clone(),
            "`Self` is not usable here; write the type name instead.".to_string(),
        ),
        E::UnexpectedSelfVariable { span } => raw(
            span.clone(),
            "`self` is only available in a method.".to_string(),
        ),

        E::IdentNotFound { ident } => {
            let n = name(interner, &ident.id);
            raw(
                ident.span.clone(),
                format!("`{n}` is not found in current scope."),
            )
        }

        E::PathResolutionFailed { path } => match first_unresolved(path) {
            Some(segment) => {
                let n = name(interner, &segment.ident.id);
                raw(segment.ident.span.clone(), format!("`{n}` is not found."))
            }
            None => {
                let segment = last_segment(path);
                let n = name(interner, &segment.ident.id);
                raw(
                    segment.ident.span.clone(),
                    format!("`{n}` does not resolve to what is needed here."),
                )
            }
        },

        E::GenericTypeWithGenArgs { path, .. } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!(
                    "`{n}` is a generic parameter, not a generic type; it cannot take generic arguments."
                ),
            )
        }

        E::TypeNotFoundPackageFound { path, .. } => expected_ty_but(interner, path, "a package"),
        E::TypeNotFoundModuleFound { path, .. } => expected_ty_but(interner, path, "a module"),
        E::TypeNotFoundValueFound { path, .. } => expected_ty_but(interner, path, "a value"),
        E::TypeNotFoundVariableFound { path, .. } => expected_ty_but(interner, path, "a variable"),

        E::ValueNotFoundTypeFound { path, .. } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("Value expected here, but `{n}` is a type."),
            )
        }

        E::DuplicatedGenName { name: n, span2, .. }
        | E::DuplicatedLocalGenName { name: n, span2, .. } => {
            let n = name(interner, n);
            raw(
                span2.clone(),
                format!("Generic parameter `{n}` is declared twice."),
            )
        }

        E::DuplicatedVariableName { id, var2, .. } => {
            let n = name(interner, id);
            raw(var2.clone(), format!("Variable `{n}` is declared twice."))
        }

        E::CyclingTypeAlias {
            detected_position, ..
        } => raw(
            (**detected_position).clone(),
            "Type alias refers to itself; expanding it never terminates.".to_string(),
        ),

        E::VariantExpected { path } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("`{n}` is not an enum variant."),
            )
        }

        E::NestedPatternUnsupported { span } => raw(
            span.clone(),
            "Nested patterns are not supported yet; only a binding or `_` can appear here."
                .to_string(),
        ),

        E::AssocItemNotFoundForGenArgs { segment } | E::AmbiguousAssocItem { segment } => {
            let n = name(interner, &segment.ident.id);
            let msg = if matches!(err, E::AmbiguousAssocItem { .. }) {
                format!("`{n}` is ambiguous for these generic arguments.")
            } else {
                format!("No impl provides `{n}` for these generic arguments.")
            };
            raw(segment.ident.span.clone(), msg)
        }

        E::TypeNotFoundTraitFound { path, .. } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("`{n}` is a trait, not a type."),
            )
        }
        E::ValueNotFoundTraitFound { path, .. } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("`{n}` is a trait, not a value."),
            )
        }
        E::TraitExpected { path } => {
            let segment = last_segment(path);
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("`{n}` is not a trait; only a trait can be written after `:` here."),
            )
        }

        E::DuplicatedTraitItem { name: n, span2, .. } => {
            let n = name(interner, n);
            raw(
                span2.clone(),
                format!("`{n}` is declared twice in this trait."),
            )
        }

        E::ForeignTraitImpl { span } => raw(
            span.clone(),
            "Neither the trait nor the type is defined in this package; this impl is not allowed."
                .to_string(),
        ),

        E::DuplicatedTraitImpl { span2, .. } => raw(
            span2.clone(),
            "This trait is implemented twice for the same type.".to_string(),
        ),

        E::TraitImplNameConflict { name: n, span } => {
            let n = name(interner, n);
            raw(
                span.clone(),
                format!("`{n}` is already defined on this type."),
            )
        }

        E::MissingTraitItem { name: n, span } => {
            let n = name(interner, n);
            raw(span.clone(), format!("`{n}` is not implemented."))
        }
        E::UnknownTraitItem { name: n, span } => {
            let n = name(interner, n);
            raw(span.clone(), format!("The trait does not declare `{n}`."))
        }
        E::TraitItemSignatureMismatch {
            name: n,
            span,
            detail,
            ..
        } => {
            let n = name(interner, n);
            raw(
                span.clone(),
                format!("The signature of `{n}` does not match the trait: {detail}"),
            )
        }

        E::TraitBoundOnTypeDefUnsupported { span } => raw(
            span.clone(),
            "A trait bound cannot be written on a type definition yet.".to_string(),
        ),

        E::TraitAssocNotFound { segment } => {
            let n = name(interner, &segment.ident.id);
            raw(segment.ident.span.clone(), format!("`{n}` is not found."))
        }
        E::AmbiguousTraitAssoc { segment, .. } => {
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!("`{n}` is ambiguous; more than one trait in scope provides it."),
            )
        }
        E::TraitNotInScope { segment, .. } => {
            let n = name(interner, &segment.ident.id);
            raw(
                segment.ident.span.clone(),
                format!(
                    "`{n}` is provided by a trait that is not in scope; import the trait to use it."
                ),
            )
        }
    }
}
