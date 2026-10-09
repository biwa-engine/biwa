use biwac_base::IdentInterner;
use biwac_hir::{FnSignature, Hir, Ty, TyKind, ValDefKind};
use biwac_lang_item::{LangItem, LangItemTable};
use biwac_span::TyDefId;

use crate::{SceneError, SignatureProblem};

/// 自パッケージのすべての scene が `(Game[..]) -> Game[..]` であることを検査する。
///
/// 名前解決の後・型推論の前に走らせる。
/// scene のシグネチャは名前解決の時点で確定しており、型推論を待つ必要がない。
///
/// 型 alias は lowering の最後で展開済みなので、
/// `type MyGame = Game[MyState]` 越しに書かれていても `Game` として見える。
pub fn check(
    hir: &Hir,
    lang_items: &LangItemTable,
    interner: &IdentInterner,
) -> Result<(), Vec<SceneError>> {
    // lang item `game` が無いパッケージ (std をビルドする前など) では
    // 型を照合しようがないので、型の検査だけを諦める (`check_signature` 参照)。
    // lang item の欠落自体は名前解決のパスが報告している。
    let game = lang_items.get(&LangItem::Game).map(TyDefId::new);

    let mut errors = Vec::new();
    for (def_id, val) in &hir.vals {
        if !def_id.pkg().is_self() {
            continue;
        }
        if let ValDefKind::NovelScene(scene) = val {
            check_signature(&scene.signature, &scene.name, game, interner, &mut errors);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// scene のシグニチャ `(Game[..]) -> Game[..]` を検査する。
///
/// ジェネリック引数に何が入るかは問わない。
///
/// `game` (lang item の型) が引けないとき (std をビルドする前など) は型の照合を飛ばす。
/// 引数の個数とレシーバの有無は lang item に依らないので常に見る。
fn check_signature(
    sig: &FnSignature,
    name_ident: &biwac_hir::Ident,
    game: Option<TyDefId>,
    interner: &IdentInterner,
    errors: &mut Vec<SceneError>,
) {
    let name = interner.get_str(&name_ident.id).unwrap_or("").to_string();

    let mut push = |reason| {
        errors.push(SceneError::InvalidSignature {
            name: name.clone(),
            reason,
            span: name_ident.span.clone(),
        })
    };

    if sig.has_self {
        push(SignatureProblem::HasReceiver);
    }

    let args = sig.explicit_args();
    if args.len() == 1 {
        if is_game(&args[0].ty, game) == Some(false) {
            push(SignatureProblem::ArgType { index: 0 });
        }
    } else {
        push(SignatureProblem::ArgCount { found: args.len() });
    }

    if is_game(&sig.rty, game) == Some(false) {
        push(SignatureProblem::ReturnType);
    }
}

/// `ty` が `Game[..]` か。lang item が引けず判定できなければ `None`。
fn is_game(ty: &Ty, game: Option<TyDefId>) -> Option<bool> {
    let game = game?;
    Some(matches!(&ty.kind, TyKind::Defined(dt) if dt.def_id == game))
}
