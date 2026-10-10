//! 宣言の可視性を「見える範囲」に直す (`docs/symbol-visibility-impl-status.md` §6.3)。
//!
//! 見える範囲は宣言したモジュール (`home`) とその親から決まる。
//! 宣言の span がそのモジュールの `ModId` を持っているので、親の表だけを用意すればよい。
//! まだ検査には使っていない (段階 3 以降)。

use std::collections::HashMap;

use biwac_ast::Globals;
use biwac_base::{ModId, PackageId};
use biwac_hir::{DeclaredVisibility, Visibility};
use biwac_package_loader::{LoadedModule, Pkg};
use biwac_span::Span;

use crate::ResolveError;

/// 自パッケージのモジュール → 親モジュール。
pub(crate) struct ModuleParents {
    parents: HashMap<ModId, ModId>,
}

impl ModuleParents {
    pub(crate) fn of(pkg: &Pkg) -> Self {
        fn walk(module: &LoadedModule, parents: &mut HashMap<ModId, ModId>) {
            for (_, child) in module.children_ordered() {
                parents.insert(child.mod_id, module.mod_id);
                walk(child, parents);
            }
        }
        let mut parents = HashMap::new();
        walk(&pkg.root_module, &mut parents);
        Self { parents }
    }

    /// `home` で宣言された可視性を見える範囲に直す。
    pub(crate) fn resolve(&self, vis: &biwac_ast::Visibility, home: ModId) -> Visibility {
        resolve_in_self(
            DeclaredVisibility::from(vis),
            home,
            self.parents.get(&home).copied(),
        )
    }

    /// モジュール → 親モジュールの表そのもの。HIR に渡して、型推論の可視性の判定に使わせる。
    pub(crate) fn to_map(&self) -> HashMap<ModId, ModId> {
        self.parents.clone()
    }

    /// 自パッケージのモジュールの可視性 (`mod` 宣言に書いたもの)。ルートモジュールは載らない。
    pub(crate) fn mod_visibilities(&self, pkg: &Pkg) -> HashMap<ModId, Visibility> {
        let mut out = HashMap::new();
        pkg.walk_modules(|module| {
            for g in &module.ast.globals {
                let Globals::Mod(decl) = g else { continue };
                if let Some(child) = module.children.get(&decl.id.id) {
                    out.insert(child.mod_id, self.resolve(&decl.vis, module.mod_id));
                }
            }
        });
        out
    }
}

/// 自パッケージの宣言の可視性を見える範囲に直す。
///
/// ルートモジュールの `pub(super)` は [`check_super_in_root`] がエラーにする。
/// ここでは続きの解析のために、何も書かなかったのと同じ扱いにする。
pub(crate) fn resolve_in_self(
    declared: DeclaredVisibility,
    home: ModId,
    parent: Option<ModId>,
) -> Visibility {
    Visibility::resolve(declared, PackageId::SELF_PACKAGE, home, parent).unwrap_or_else(|| {
        Visibility::resolve(
            DeclaredVisibility::Private,
            PackageId::SELF_PACKAGE,
            home,
            None,
        )
        .expect("private always resolves")
    })
}

/// ルートモジュールに書かれた `pub(super)` をエラーにする。ルートモジュールには親が無い。
pub(crate) fn check_super_in_root(pkg: &Pkg) -> Vec<ResolveError> {
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |vis: &biwac_ast::Visibility| {
        if let biwac_ast::Visibility::Super(span) = vis {
            spans.push(span.clone());
        }
    };
    for g in &pkg.root_module.ast.globals {
        match g {
            Globals::Mod(m) => push(&m.vis),
            Globals::FnDef(f) => push(&f.vis),
            Globals::NativeFnDef(f) => push(&f.vis),
            Globals::NovelScene(s) => push(&s.vis),
            Globals::TraitDef(t) => push(&t.vis),
            Globals::TypeDef(t) => match t {
                biwac_ast::TypeDef::Struct(s) => {
                    push(&s.vis);
                    s.members.iter().for_each(|m| push(&m.vis));
                }
                biwac_ast::TypeDef::Enum(e) => push(&e.vis),
                biwac_ast::TypeDef::TypeAlias(a) => push(&a.vis),
                biwac_ast::TypeDef::NativeTypeAlias(a) => push(&a.vis),
            },
            Globals::ImplBlock(b) => {
                b.assoc_fns.iter().for_each(|f| push(&f.vis));
                b.methods.iter().for_each(|m| push(&m.vis));
                b.native_assoc_fns.iter().for_each(|f| push(&f.vis));
                b.native_methods.iter().for_each(|m| push(&m.vis));
            }
            Globals::Import(_) | Globals::VarDecl(_) | Globals::NativeCode(_) => {}
        }
    }
    spans
        .into_iter()
        .map(|span| ResolveError::SuperVisibilityInRoot { span })
        .collect()
}
