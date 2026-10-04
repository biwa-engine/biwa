use biwac_base::{IdentInterner, ModId, PackageKind};
use biwac_hir::{FnSignature, Hir, Ty, TyKind, ValDefKind};
use biwac_lang_item::LangItemTable;
use biwac_span::TyDefId;

use crate::{
    ContractTy, SCENE_ARGS, SCENE_RET, SceneError, SceneRequirement, SignatureProblem,
    WellKnownKind, WellKnownSymbol, WellKnownSymbols,
};

/// scene の規約を検査し、既知 scene の解決結果を返す。
///
/// 名前解決の後・型推論の前に走らせる。
/// scene のシグネチャは名前解決の時点で確定しており、型推論を待つ必要がない。
///
/// 型 alias は lowering の最後で展開済みなので、
/// `type MyGame = Game[A, B]` 越しに書かれていても `Game` として見える。
pub fn check(
    hir: &Hir,
    lang_items: &LangItemTable,
    pkg_kind: PackageKind,
    root_mod_id: ModId,
    interner: &IdentInterner,
) -> Result<WellKnownSymbols, Vec<SceneError>> {
    let mut errors = Vec::new();
    let mut found = WellKnownSymbols::new();

    // 規約に現れる型の lang item が無いパッケージ (std をビルドする前など) では
    // シグネチャを照合しようがないので検査を諦める (`check_signature` 参照)。
    // lang item の欠落自体は名前解決のパスが報告している。
    let contract_ty = |ty: ContractTy| lang_items.get(&ty.lang_item()).map(TyDefId::new);

    for (def_id, val) in &hir.vals {
        if !def_id.pkg().is_self() {
            continue;
        }

        // scene はすべて `(Game[..]) -> Game[..]` でなければならない。
        if let ValDefKind::NovelScene(scene) = val {
            check_signature(
                &scene.signature,
                &scene.name,
                SCENE_ARGS,
                SCENE_RET,
                &contract_ty,
                interner,
                &mut errors,
            );
        }

        // ルートモジュールに書かれた既知の名前だけがランタイムから呼ばれる。
        let (name_ident, kind) = match val {
            ValDefKind::NovelScene(s) => (&s.name, WellKnownKind::Scene),
            ValDefKind::Fn(f) => (&f.name, WellKnownKind::Fn),
            ValDefKind::Native(f) => (&f.name, WellKnownKind::Fn),
        };
        if name_ident.span.module() != root_mod_id {
            continue;
        }
        let Some(name) = interner.get_str(&name_ident.id) else {
            continue;
        };
        let Some(well_known) = WellKnownSymbol::from_name(name) else {
            continue;
        };
        if well_known.kind() != kind {
            // 種別が違うものは登録しない。
            // 下の「必須なのに無い」の検査が理由を添えて報告する。
            continue;
        }

        // scene 以外の既知シンボルはここでシグニチャを見る。
        // 期待する型はシンボルごとに表が持っている。
        if kind == WellKnownKind::Fn
            && let ValDefKind::Fn(f) = val
        {
            check_signature(
                &f.signature,
                &f.name,
                well_known.args(),
                well_known.ret(),
                &contract_ty,
                interner,
                &mut errors,
            );
        }

        found.set(well_known, *def_id);
    }

    // エントリポイントを持つのは playable package だけ。
    // library package では既知の名前も普通の scene として扱う。
    if !pkg_kind.is_playable() {
        return finish(WellKnownSymbols::new(), errors);
    }

    for &well_known in WellKnownSymbol::ALL {
        if well_known.requirement() != SceneRequirement::RequiredInPlayable {
            continue;
        }
        if found.get(well_known).is_some() {
            continue;
        }

        // 同名の値が scene でない形で定義されていないかを見て、
        // 「無い」のか「scene でない」のかを区別して報告する。
        match val_named_with_other_kind(hir, well_known, root_mod_id, interner) {
            Some(span) => errors.push(SceneError::EntryPointNotScene {
                scene: well_known,
                span,
            }),
            None => errors.push(SceneError::MissingEntryPoint { scene: well_known }),
        }
    }

    finish(found, errors)
}

fn finish(
    found: WellKnownSymbols,
    errors: Vec<SceneError>,
) -> Result<WellKnownSymbols, Vec<SceneError>> {
    if errors.is_empty() {
        Ok(found)
    } else {
        Err(errors)
    }
}

/// ランタイムが呼ぶシンボルのシグニチャを検査する。
///
/// - scene は `(Game[..]) -> Game[..]`
/// - 既知の関数は表 (`WellKnownSymbol::args` / `ret`) のとおり
///   (`on_new_game` は `(GameWindow) -> Game[..]`)
///
/// ジェネリック引数に何が入るかは問わない。
///
/// 期待する型の lang item が引けないもの (std をビルドする前など) は照合を飛ばす。
/// 引数の個数とレシーバの有無は lang item に依らないので常に見る。
fn check_signature(
    sig: &FnSignature,
    name_ident: &biwac_hir::Ident,
    expected_args: &'static [ContractTy],
    expected_ret: ContractTy,
    contract_ty: &impl Fn(ContractTy) -> Option<TyDefId>,
    interner: &IdentInterner,
    errors: &mut Vec<SceneError>,
) {
    let name = interner.get_str(&name_ident.id).unwrap_or("").to_string();

    let mut push = |reason| {
        errors.push(SceneError::InvalidSceneSignature {
            scene: name.clone(),
            expected_args,
            expected_ret,
            reason,
            span: name_ident.span.clone(),
        })
    };

    if sig.self_ty.is_some() {
        push(SignatureProblem::HasReceiver);
    }

    if sig.args.len() == expected_args.len() {
        for (index, (arg, expected)) in sig.args.iter().zip(expected_args).enumerate() {
            if is_contract_ty(&arg.ty, *expected, contract_ty) == Some(false) {
                push(SignatureProblem::ArgType {
                    index,
                    expected: *expected,
                });
            }
        }
    } else {
        push(SignatureProblem::ArgCount {
            found: sig.args.len(),
            expected: expected_args.len(),
        });
    }

    if is_contract_ty(&sig.rty, expected_ret, contract_ty) == Some(false) {
        push(SignatureProblem::ReturnType {
            expected: expected_ret,
        });
    }
}

/// `ty` が規約の型 `expected` か。lang item が引けず判定できなければ `None`。
fn is_contract_ty(
    ty: &Ty,
    expected: ContractTy,
    contract_ty: &impl Fn(ContractTy) -> Option<TyDefId>,
) -> Option<bool> {
    let def_id = contract_ty(expected)?;
    Some(matches!(&ty.kind, TyKind::Defined(dt) if dt.def_id == def_id))
}

/// ルートモジュールに同じ名前の値があるが、期待した種別でない場合にその span を返す。
///
/// 「無い」のか「種別が違う」のかを分けて報告するために使う。
fn val_named_with_other_kind(
    hir: &Hir,
    well_known: WellKnownSymbol,
    root_mod_id: ModId,
    interner: &IdentInterner,
) -> Option<biwac_span::Span> {
    hir.vals
        .iter()
        .filter(|(def_id, _)| def_id.pkg().is_self())
        .find_map(|(_, val)| {
            let (ident, kind) = match val {
                ValDefKind::Fn(f) => (&f.name, WellKnownKind::Fn),
                ValDefKind::Native(f) => (&f.name, WellKnownKind::Fn),
                ValDefKind::NovelScene(s) => (&s.name, WellKnownKind::Scene),
            };
            if kind == well_known.kind() {
                return None;
            }

            (ident.span.module() == root_mod_id
                && interner.get_str(&ident.id) == Some(well_known.name()))
            .then(|| ident.span.clone())
        })
}
