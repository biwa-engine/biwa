//! [`biwac_type_inferrer::TyErrorReport`] を LSP の `Diagnostic` に変換する。
//!
//! `TyErrorReport::print_error_message` (biwac_type_inferrer/src/error.rs) と
//! 同じ match を自前で持ち、使う情報 (主たる label の span とメッセージ) だけを
//! 取り出す。型の表示名は `TyErrorReport::names` (`TyNames::render`) を
//! そのまま使う (これは公開 API)。
//!
//! `TyCtx::infer()` は最初の 1 件で止まる (`Result<Hir, Box<TyErrorReport>>`)
//! ので、ここでは常に高々 1 件の診断を返す。

use biwac_base::{IdentInterner, InternedIdent, ModId};
use biwac_hir::FnTy;
use biwac_span::Span;
use biwac_type_inferrer::{TyError, TyErrorReport};

use crate::Diagnostic;

/// `doc_mod_id` を指すものだけを返す (通常は 0 か 1 件)。
pub(crate) fn extract(
    report: &TyErrorReport,
    doc_mod_id: ModId,
    interner: &IdentInterner,
) -> Vec<Diagnostic> {
    match to_diagnostic(report, interner) {
        Some((span, message)) if span.module() == doc_mod_id => vec![Diagnostic {
            start: span.begin(),
            end: span.end(),
            message,
        }],
        _ => Vec::new(),
    }
}

fn ident_str<'a>(interner: &'a IdentInterner, id: &InternedIdent) -> &'a str {
    interner.get_str(id).unwrap_or("<unknown>")
}

/// [`FnTy`] 自体は span を持たないので、引数か戻り値のものを借りる
/// (`biwac_type_inferrer::error::fn_ty_span` と同じ規則)。
fn fn_ty_span(fty: &FnTy) -> &Span {
    fty.args
        .first()
        .map(|a| &a.span)
        .unwrap_or(&fty.rty.as_ref().span)
}

fn to_diagnostic(report: &TyErrorReport, interner: &IdentInterner) -> Option<(Span, String)> {
    let names = &report.names;

    match &report.error {
        TyError::StructLiteralMemberConfliced { member1, member2 } => {
            let name = ident_str(interner, &member1.id);
            Some((
                member2.span.clone(),
                format!("Member `{name}` is assigned twice."),
            ))
        }

        TyError::StructLiteralAssignToInexsistentMember { def_id, member } => {
            let name = ident_str(interner, &member.id);
            let ty = names
                .tys
                .get(def_id)
                .cloned()
                .unwrap_or_else(|| "the struct".to_string());
            Some((
                member.span.clone(),
                format!("`{ty}` has no member `{name}`."),
            ))
        }

        TyError::StructLiteralMemberInsufficient {
            sliteral,
            insufficient_members,
        } => {
            let missing = insufficient_members
                .iter()
                .map(|m| format!("`{}`", ident_str(interner, m)))
                .collect::<Vec<_>>()
                .join(", ");
            Some((
                sliteral.span.clone(),
                format!("Struct literal is missing members: {missing} not assigned."),
            ))
        }

        TyError::InvalidStructLiteralOnAliasType { ty, sliteral } => Some((
            sliteral.span.clone(),
            format!(
                "Struct literal cannot be used for this type: `{}` is not a struct.",
                names.render(&ty.kind)
            ),
        )),

        TyError::StructNotHasMember { def_id, access } => {
            let name = ident_str(interner, &access.member.id);
            let ty = names
                .tys
                .get(def_id)
                .cloned()
                .unwrap_or_else(|| "the struct".to_string());
            Some((
                access.member.span.clone(),
                format!("`{ty}` has no member `{name}`."),
            ))
        }

        TyError::ExprNotHasMember { ty, access } => {
            let name = ident_str(interner, &access.member.id);
            let ty = names.render(&ty.kind);
            Some((
                access.member.span.clone(),
                format!("`{ty}` has no member `{name}`."),
            ))
        }

        TyError::InvalidBinaryOperationForType { ty, op, expr } => {
            let ty = names.render(&ty.kind);
            Some((
                expr.span(),
                format!("`{op}` cannot be applied to `{ty}`."),
            ))
        }

        TyError::InvalidUnaryOperationForType { ty, op, expr } => {
            let ty = names.render(&ty.kind);
            Some((
                expr.span(),
                format!("`{op}` cannot be applied to `{ty}`."),
            ))
        }

        TyError::InvalidAssignOperation { ass } => Some((
            ass.span.clone(),
            "This expression cannot be assigned to; only a variable or a struct member is assignable."
                .to_string(),
        )),

        TyError::FnArgLenMismatched(callee, caller) => Some((
            fn_ty_span(caller).clone(),
            format!(
                "This call takes {} argument(s), but {} were given.",
                callee.args.len(),
                caller.args.len()
            ),
        )),

        TyError::FnGenArgLenMismatched(callee, caller) => Some((
            fn_ty_span(caller).clone(),
            format!(
                "This call takes {} generic argument(s), but {} were given.",
                callee.genargs.len(),
                caller.genargs.len()
            ),
        )),

        TyError::TypeConfliced { t1, t2 } => {
            let n1 = names.render(&t1.kind);
            let n2 = names.render(&t2.kind);
            Some((
                t2.span.clone(),
                format!("Expected `{n1}`, but found `{n2}`."),
            ))
        }

        TyError::OccursCheckFailed { ty, .. } => Some((
            ty.span.clone(),
            format!(
                "This type would contain itself: inferred as `{}`.",
                names.render(&ty.kind)
            ),
        )),

        // span を持たない (呼び出し位置に紐づかない/文脈依存の) エラー。
        TyError::InsufficientContext => None,

        TyError::TypeNotInferable { ty } => {
            let rendered = names.render(&ty.kind);
            Some((
                ty.span.clone(),
                format!("The type of this expression cannot be determined: `{rendered}`."),
            ))
        }

        TyError::NotCallable { ty } => Some((
            ty.span.clone(),
            format!(
                "`{}` is not a function and cannot be called.",
                names.render(&ty.kind)
            ),
        )),

        TyError::TraitItemAsValue { span } => Some((
            span.clone(),
            "A trait item reached through a generic type cannot be used as a value yet."
                .to_string(),
        )),

        TyError::BoundMethodAsValue { ty, method } => {
            let name = ident_str(interner, &method.id);
            let ty = names.render(&ty.kind);
            Some((
                method.span.clone(),
                format!(
                    "The method `{name}` cannot be used as a value together with its receiver; use `{ty}::{name}`."
                ),
            ))
        }

        TyError::NotAMethod { ty, method } => {
            let name = ident_str(interner, &method.id);
            let ty = names.render(&ty.kind);
            Some((
                method.span.clone(),
                format!("`{name}` is not a method of `{ty}`; call it as `{ty}::{name}(..)`."),
            ))
        }

        TyError::SceneAsValue { span } => Some((
            span.clone(),
            "A scene cannot be used as a value yet.".to_string(),
        )),

        TyError::ReturnTypeRequired { rty } => {
            let ty = names.render(&rty.kind);
            Some((rty.span.clone(), format!("This function must return `{ty}`.")))
        }

        TyError::MethodNotFound { ty, method } => {
            let name = ident_str(interner, &method.id);
            let ty = names.render(&ty.kind);
            Some((
                method.span.clone(),
                format!("`{ty}` has no method `{name}`."),
            ))
        }

        TyError::MethodNotInScope { ty, method } => {
            let name = ident_str(interner, &method.id);
            let ty = names.render(&ty.kind);
            Some((
                method.span.clone(),
                format!(
                    "`{name}` is provided by a trait that is not in scope (used on `{ty}`); import the trait to make it visible."
                ),
            ))
        }

        TyError::TraitBoundNotSatisfied { ty, span, .. } => {
            let ty = names.render(&ty.kind);
            Some((
                span.clone(),
                format!("`{ty}` does not satisfy the required trait."),
            ))
        }

        TyError::AmbiguousMethod { ty, method } => {
            let name = ident_str(interner, &method.id);
            let ty = names.render(&ty.kind);
            Some((
                method.span.clone(),
                format!("`{name}` is ambiguous on `{ty}`."),
            ))
        }

        TyError::VariantShapeMismatched {
            declared,
            found,
            span,
        } => Some((
            span.clone(),
            format!("This variant is declared in {declared} form, but written in {found} form."),
        )),

        TyError::VariantFieldCountMismatched {
            expected,
            found,
            span,
        } => Some((
            span.clone(),
            format!("This variant takes {expected} field(s), but {found} were given."),
        )),

        TyError::VariantFieldNotFound { field } => {
            let name = ident_str(interner, &field.id);
            Some((
                field.span.clone(),
                format!("This variant has no field `{name}`."),
            ))
        }

        TyError::VariantFieldInsufficient { missing, span } => {
            let missing = missing
                .iter()
                .map(|m| format!("`{}`", ident_str(interner, m)))
                .collect::<Vec<_>>()
                .join(", ");
            Some((
                span.clone(),
                format!("Variant is missing fields: {missing} not given."),
            ))
        }

        TyError::MatchOnNonEnum { ty, span } => {
            let ty = names.render(&ty.kind);
            Some((span.clone(), format!("`match` needs an enum, but this is `{ty}`.")))
        }

        TyError::VariantOfAnotherEnum { ty, span } => {
            let ty = names.render(&ty.kind);
            Some((
                span.clone(),
                format!("This pattern is not a variant of `{ty}`."),
            ))
        }

        TyError::NonExhaustiveMatch { missing, span } => {
            let missing = missing
                .iter()
                .map(|m| format!("`{}`", ident_str(interner, m)))
                .collect::<Vec<_>>()
                .join(", ");
            Some((
                span.clone(),
                format!("This `match` is not exhaustive: {missing} not covered."),
            ))
        }

        TyError::UnreachableMatchArm { span } => Some((
            span.clone(),
            "This arm is never reached; an earlier arm already covers it.".to_string(),
        )),

        // パッケージ横断のメタ情報の欠如であり、特定のソース位置に紐づかない。
        TyError::MissingLangItem { .. } => None,
    }
}
