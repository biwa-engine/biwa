use biwac_base::{IdentInterner, ModId, PackageKind};
use biwac_hir::{FnSignature, Hir, Ty, TyKind, ValDefKind};

use crate::{
    ContractTy, Entrypoint, EntrypointError, EntrypointKind, EntrypointRequirement, Entrypoints,
    SignatureProblem,
};

/// エントリポイントを検査し、その解決結果を返す。
///
/// 名前解決の後・型推論の前に走らせる。
/// シグネチャは名前解決の時点で確定しており、型推論を待つ必要がない。
pub fn check(
    hir: &Hir,
    pkg_kind: PackageKind,
    root_mod_id: ModId,
    interner: &IdentInterner,
) -> Result<Entrypoints, Vec<EntrypointError>> {
    let mut errors = Vec::new();
    let mut found = Entrypoints::new();

    for (def_id, val) in &hir.vals {
        if !def_id.pkg().is_self() {
            continue;
        }

        // ルートモジュールに書かれたエントリポイントの名前だけがランタイムから呼ばれる。
        let (name_ident, kind) = match val {
            ValDefKind::NovelScene(s) => (&s.name, EntrypointKind::Scene),
            ValDefKind::Fn(f) => (&f.name, EntrypointKind::Fn),
            ValDefKind::Native(f) => (&f.name, EntrypointKind::Fn),
        };
        if name_ident.span.module() != root_mod_id {
            continue;
        }
        let Some(name) = interner.get_str(&name_ident.id) else {
            continue;
        };
        let Some(entrypoint) = Entrypoint::from_name(name) else {
            continue;
        };
        if entrypoint.kind() != kind {
            // 種別が違うものは登録しない。
            // 下の「必須なのに無い」の検査が理由を添えて報告する。
            continue;
        }

        // 関数のエントリポイントはここでシグニチャを見る。
        // 期待する型はエントリポイントごとに表が持っている。
        if kind == EntrypointKind::Fn
            && let ValDefKind::Fn(f) = val
        {
            check_signature(
                &f.signature,
                &f.name,
                entrypoint.args(),
                entrypoint.ret(),
                interner,
                &mut errors,
            );
        }

        found.set(entrypoint, *def_id);
    }

    // エントリポイントを持つのは playable package だけ。
    // library package ではエントリポイントの名前も普通の関数・scene として扱う。
    if !pkg_kind.is_playable() {
        return finish(Entrypoints::new(), errors);
    }

    for &entrypoint in Entrypoint::ALL {
        if entrypoint.requirement() != EntrypointRequirement::RequiredInPlayable {
            continue;
        }
        if found.get(entrypoint).is_some() {
            continue;
        }

        // 同名の値が scene でない形で定義されていないかを見て、
        // 「無い」のか「scene でない」のかを区別して報告する。
        match val_named_with_other_kind(hir, entrypoint, root_mod_id, interner) {
            Some(span) => errors.push(EntrypointError::EntrypointWrongKind { entrypoint, span }),
            None => errors.push(EntrypointError::MissingEntrypoint { entrypoint }),
        }
    }

    finish(found, errors)
}

fn finish(
    found: Entrypoints,
    errors: Vec<EntrypointError>,
) -> Result<Entrypoints, Vec<EntrypointError>> {
    if errors.is_empty() {
        Ok(found)
    } else {
        Err(errors)
    }
}

/// エントリポイントのシグニチャを表 (`Entrypoint::args` / `ret`) のとおりか検査する
/// (`main` は `()`、引数も戻り値も無い)。
fn check_signature(
    sig: &FnSignature,
    name_ident: &biwac_hir::Ident,
    expected_args: &'static [ContractTy],
    expected_ret: ContractTy,
    interner: &IdentInterner,
    errors: &mut Vec<EntrypointError>,
) {
    let name = interner.get_str(&name_ident.id).unwrap_or("").to_string();

    let mut push = |reason| {
        errors.push(EntrypointError::InvalidSignature {
            name: name.clone(),
            expected_args,
            expected_ret,
            reason,
            span: name_ident.span.clone(),
        })
    };

    if sig.has_self {
        push(SignatureProblem::HasReceiver);
    }

    if sig.explicit_args().len() == expected_args.len() {
        for (index, (arg, expected)) in sig.explicit_args().iter().zip(expected_args).enumerate() {
            if !is_contract_ty(&arg.ty, *expected) {
                push(SignatureProblem::ArgType {
                    index,
                    expected: *expected,
                });
            }
        }
    } else {
        push(SignatureProblem::ArgCount {
            found: sig.explicit_args().len(),
            expected: expected_args.len(),
        });
    }

    if !is_contract_ty(&sig.rty, expected_ret) {
        push(SignatureProblem::ReturnType {
            expected: expected_ret,
        });
    }
}

/// `ty` が規約の型 `expected` か。
fn is_contract_ty(ty: &Ty, expected: ContractTy) -> bool {
    match expected {
        ContractTy::Void => matches!(ty.kind, TyKind::Void),
    }
}

/// ルートモジュールに同じ名前の値があるが、期待した種別でない場合にその span を返す。
///
/// 「無い」のか「種別が違う」のかを分けて報告するために使う。
fn val_named_with_other_kind(
    hir: &Hir,
    entrypoint: Entrypoint,
    root_mod_id: ModId,
    interner: &IdentInterner,
) -> Option<biwac_span::Span> {
    hir.vals
        .iter()
        .filter(|(def_id, _)| def_id.pkg().is_self())
        .find_map(|(_, val)| {
            let (ident, kind) = match val {
                ValDefKind::Fn(f) => (&f.name, EntrypointKind::Fn),
                ValDefKind::Native(f) => (&f.name, EntrypointKind::Fn),
                ValDefKind::NovelScene(s) => (&s.name, EntrypointKind::Scene),
            };
            if kind == entrypoint.kind() {
                return None;
            }

            (ident.span.module() == root_mod_id
                && interner.get_str(&ident.id) == Some(entrypoint.name()))
            .then(|| ident.span.clone())
        })
}
