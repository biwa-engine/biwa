//! import の表 (`docs/useful-import-patterns-impl-status.md` §4.2)。
//!
//! モジュールごとに、import が作った名前 (明示した import と glob) を「名前 → 指すもの・可視性」の表にする。
//! 表はそのモジュールの外からも引ける。`pub import` (re-export) はこれで他のモジュールから見える。
//!
//! 表は def collection の Step 1 (名前の木) の直後に、不動点で作る。
//! 空から始めて、すべての import を「名前の木 + 今の表」の上で解決し直し、表が増えなくなるまで繰り返す。
//! re-export の連鎖や、re-export されたものの glob も、これで import の順序によらずに埋まる。
//! 表は増える一方なので必ず止まる。
//!
//! 解決は副作用の無い探索で行う。パスのセグメントの `resolved_id` (一度しか書けない) には書かない。
//! 書くと「まだ見つからない」を「無い」と確定させてしまうからである。
//! 明示した import のパスは、本番の名前解決が改めて解決する (そこで可視性も確かめる)。

use std::collections::HashMap;

use biwac_ast::{AbsolutePathHeader, Globals, ImportDecl, Path};
use biwac_base::{IdentInterner, InternedIdent, ModId, PackageId};
use biwac_dependency_metadata::{DepMetadataModuleView, PackageModuleView};
use biwac_hir::{DeclaredVisibility, Visibility, VisibilityScope};
use biwac_package_loader::{LoadedModule, Pkg};
use biwac_span::{DefIdKind, Span, TyDefId};

use crate::{
    ModuleNameTree, NameTree, ResolveError, TyNameTree,
    name_tree::AssocNameTreeItemKind,
    resolving::{
        context::module_level::{ext_child_ref_to_def_id_kind, module_item_to_def_id_kind},
        def_collector::{collect_mod_trees, collect_ty_trees},
    },
    visibility::{ModuleParents, ScopeOps},
};

/// import が作った名前 1 つ。
#[derive(Debug, Clone)]
pub struct ImportEntry {
    pub kind: DefIdKind,
    /// 他のモジュールから引いたときの可視性。
    ///
    /// 明示した import は書いた可視性 (何も書かなければ private)。
    /// glob で入った名前は、書いた可視性と元の可視性の狭い方。
    pub vis: Visibility,
    /// import 宣言の位置 (エラーに使う)。
    pub span: Span,
}

/// 1 つのモジュールの import の表。
#[derive(Debug, Default)]
pub struct ModuleImports {
    /// 明示した import (`import a::b;`)。名前はパスの最後のセグメント。
    pub explicit: HashMap<InternedIdent, ImportEntry>,
    /// glob (`import a::*;`) で入った名前。
    pub glob: HashMap<InternedIdent, ImportEntry>,
}

impl ModuleImports {
    /// 名前を引く。明示した import が glob より先である。
    pub fn get(&self, name: InternedIdent) -> Option<&ImportEntry> {
        self.explicit.get(&name).or_else(|| self.glob.get(&name))
    }
}

/// 探索で辿り着いたもの。
#[derive(Debug, Clone)]
struct Found {
    kind: DefIdKind,
    vis: Visibility,
}

/// 探索の途中で居る場所 (子を引ける入れ物)。
enum Place<'t> {
    Mod(&'t ModuleNameTree),
    Ty(&'t TyNameTree),
    ExtMod(PackageId, u32),
    ExtTy(PackageId, u32),
}

fn public() -> Visibility {
    Visibility {
        declared: DeclaredVisibility::Public,
        scope: VisibilityScope::Public,
    }
}

struct Builder<'t> {
    name_tree: &'t NameTree,
    mod_index: HashMap<ModId, &'t ModuleNameTree>,
    ty_index: HashMap<TyDefId, &'t TyNameTree>,
    parents: &'t ModuleParents,
    parent_map: HashMap<ModId, ModId>,
    tables: HashMap<ModId, ModuleImports>,
}

/// パッケージのすべてのモジュールの import の表を作り、§4.3 の検査をする。
pub(crate) fn build(
    pkg: &Pkg,
    name_tree: &NameTree,
    interner: &mut IdentInterner,
) -> (HashMap<ModId, ModuleImports>, Vec<ResolveError>) {
    let root = &name_tree.packages[&name_tree.self_pkg_name].root_module_tree;
    let mut mod_index = HashMap::new();
    collect_mod_trees(root, &mut mod_index);
    let mut ty_index = HashMap::new();
    collect_ty_trees(root, &mut ty_index);
    let parents = ModuleParents::of(pkg);

    fn collect_imports<'p>(m: &'p LoadedModule, out: &mut Vec<(ModId, Vec<&'p ImportDecl>)>) {
        let imports = m
            .ast
            .globals
            .iter()
            .filter_map(|g| match g {
                Globals::Import(i) => Some(i),
                _ => None,
            })
            .collect();
        out.push((m.mod_id, imports));
        for (_, child) in m.children_ordered() {
            collect_imports(child, out);
        }
    }
    let mut modules: Vec<(ModId, Vec<&ImportDecl>)> = Vec::new();
    collect_imports(&pkg.root_module, &mut modules);
    modules.sort_by_key(|(id, _)| *id);

    let mut b = Builder {
        name_tree,
        tables: mod_index
            .keys()
            .map(|id| (*id, ModuleImports::default()))
            .collect(),
        mod_index,
        ty_index,
        parent_map: parents.to_map(),
        parents: &parents,
    };

    // glob どうしで同じ名前が別のものを指したもの (モジュール, 名前, 先の宣言, 後の宣言)。
    let mut glob_conflicts: Vec<(ModId, InternedIdent, Span, Span)> = Vec::new();

    loop {
        let mut changed = false;
        for (mod_id, imports) in &modules {
            for import in imports {
                let declared = b.parents.resolve(&import.vis, *mod_id);
                if import.glob {
                    let Some(Ok(members)) = b.glob_members(*mod_id, &import.path, interner) else {
                        continue;
                    };
                    for (name, found) in members {
                        let Some(vis) = b.narrower(declared, found.vis) else {
                            continue;
                        };
                        let table = b.tables.get_mut(mod_id).unwrap();
                        match table.glob.get(&name) {
                            None => {
                                table.glob.insert(
                                    name,
                                    ImportEntry {
                                        kind: found.kind,
                                        vis,
                                        span: import.span.clone(),
                                    },
                                );
                                changed = true;
                            }
                            Some(e) if e.kind != found.kind => {
                                let c = (*mod_id, name, e.span.clone(), import.span.clone());
                                if !glob_conflicts.iter().any(|x| x.0 == c.0 && x.1 == c.1) {
                                    glob_conflicts.push(c);
                                }
                            }
                            Some(_) => {}
                        }
                    }
                } else {
                    let Some(last) = import.path.segments.last() else {
                        continue;
                    };
                    if b.tables[mod_id].explicit.contains_key(&last.ident.id) {
                        continue;
                    }
                    if let Some(found) = b.resolve(*mod_id, &import.path, interner) {
                        b.tables.get_mut(mod_id).unwrap().explicit.insert(
                            last.ident.id,
                            ImportEntry {
                                kind: found.kind,
                                vis: declared,
                                span: import.span.clone(),
                            },
                        );
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut errors = Vec::new();
    for (mod_id, imports) in &modules {
        b.check_module(*mod_id, imports, interner, &mut errors);
    }
    for (_, name, span1, span2) in glob_conflicts {
        errors.push(ResolveError::ImportedNameConflict { name, span1, span2 });
    }

    (b.tables, errors)
}

impl<'t> Builder<'t> {
    fn scope_ops(&self) -> ScopeOps<'_> {
        ScopeOps {
            parents: &self.parent_map,
        }
    }

    fn is_visible_from(&self, vis: &Visibility, from: ModId) -> bool {
        vis.is_visible_from(from, |m| self.parent_map.get(&m).copied())
    }

    /// glob の re-export の可視性: 書いた可視性 `declared` と元の可視性 `item` の狭い方。
    /// 書かれた形は、範囲を決めた方のものを残す。
    fn narrower(&self, declared: Visibility, item: Visibility) -> Option<Visibility> {
        let scope = self.scope_ops().meet(declared.scope, item.scope)?;
        Some(if scope == declared.scope {
            declared
        } else {
            item
        })
    }

    fn ext_view(&self, pkg_id: PackageId, sym_idx: u32) -> Option<DepMetadataModuleView> {
        let dep = self.name_tree.ext_pkg_data.get(&pkg_id)?;
        Some(DepMetadataModuleView::new_for_sym_idx(
            std::sync::Arc::clone(dep),
            sym_idx,
            pkg_id,
        ))
    }

    /// `place` の子 `name`。自パッケージのモジュールなら、定義が無ければ import の表を引く。
    fn child(&self, place: &Place, name: InternedIdent, interner: &IdentInterner) -> Option<Found> {
        match place {
            Place::Mod(m) => match m.children.get(&name) {
                Some(item) => Some(Found {
                    kind: module_item_to_def_id_kind(item),
                    vis: m.child_visibility(name).unwrap_or_else(public),
                }),
                None => self.tables.get(&m.mod_id)?.get(name).map(|e| Found {
                    kind: e.kind.clone(),
                    vis: e.vis,
                }),
            },
            Place::Ty(t) => {
                let children = t.children.borrow();
                let item = children.get(&name)?.assocs.first()?;
                Some(Found {
                    kind: match item.kind {
                        AssocNameTreeItemKind::Val(id) => DefIdKind::Val(id),
                        AssocNameTreeItemKind::Variant(id) => DefIdKind::Variant(id),
                    },
                    vis: item.vis,
                })
            }
            Place::ExtMod(pkg_id, sym_idx) => {
                let r = self
                    .ext_view(*pkg_id, *sym_idx)?
                    .lookup_child(name, interner)?;
                Some(Found {
                    kind: ext_child_ref_to_def_id_kind(&r),
                    vis: r.vis,
                })
            }
            Place::ExtTy(pkg_id, sym_idx) => {
                let r = self
                    .ext_view(*pkg_id, *sym_idx)?
                    .lookup_assoc(*sym_idx, name, interner)?;
                Some(Found {
                    kind: ext_child_ref_to_def_id_kind(&r),
                    vis: r.vis,
                })
            }
        }
    }

    /// `kind` の中に入れるなら、その場所。
    fn place_of(&self, kind: &DefIdKind) -> Option<Place<'t>> {
        match kind {
            DefIdKind::Mod(id) if id.is_self_pkg() => self.mod_index.get(id).map(|m| Place::Mod(m)),
            DefIdKind::Mod(id) => Some(Place::ExtMod(
                PackageId::new(id.pkg_id_bits()),
                id.sym_idx(),
            )),
            DefIdKind::Ty(id) if id.pkg().is_self() => self.ty_index.get(id).map(|t| Place::Ty(t)),
            DefIdKind::Ty(id) => Some(Place::ExtTy(id.pkg(), id.local_idx())),
            DefIdKind::Package(pkg_id) if pkg_id.is_self() => Some(Place::Mod(self.root())),
            DefIdKind::Package(pkg_id) => {
                let dep = self.name_tree.ext_pkg_data.get(pkg_id)?;
                Some(Place::ExtMod(*pkg_id, dep.root_sym_idx))
            }
            _ => None,
        }
    }

    fn root(&self) -> &'t ModuleNameTree {
        &self.name_tree.packages[&self.name_tree.self_pkg_name].root_module_tree
    }

    /// パスの先頭の場所 (中に入れないものなら `None`) と、そこから辿るセグメントの位置。
    /// ヘッダの無いパスなら、先頭のセグメントを引いた結果も返す。
    fn start(
        &self,
        from: ModId,
        path: &Path,
        interner: &IdentInterner,
    ) -> Option<(Option<Place<'t>>, usize, Option<Found>)> {
        match &path.abs_header {
            Some(AbsolutePathHeader::Package(_)) => Some((Some(Place::Mod(self.root())), 0, None)),
            Some(AbsolutePathHeader::Super { depth, .. }) => {
                let mut m = from;
                for _ in 0..*depth {
                    m = *self.parent_map.get(&m)?;
                }
                Some((Some(Place::Mod(self.mod_index.get(&m)?)), 0, None))
            }
            Some(AbsolutePathHeader::SelfTyp(_)) => None,
            None => {
                let first = path.segments.first()?.ident.id;
                let found = self
                    .child(&Place::Mod(self.mod_index.get(&from)?), first, interner)
                    .or_else(|| {
                        // パッケージ名 (自分と直接依存)。
                        if self.name_tree.packages.contains_key(&first) {
                            Some(Found {
                                kind: DefIdKind::Package(PackageId::SELF_PACKAGE),
                                vis: public(),
                            })
                        } else {
                            self.name_tree.ext_pkg_views.get(&first).map(|v| Found {
                                kind: DefIdKind::Package(v.pkg_id()),
                                vis: public(),
                            })
                        }
                    })?;
                Some((self.place_of(&found.kind), 1, Some(found)))
            }
        }
    }

    /// 明示した import のパスが指すもの。
    fn resolve(&self, from: ModId, path: &Path, interner: &IdentInterner) -> Option<Found> {
        let (mut place, next, mut found) = self.start(from, path, interner)?;
        for segment in path.segments.iter().skip(next) {
            let f = self.child(place.as_ref()?, segment.ident.id, interner)?;
            place = self.place_of(&f.kind);
            found = Some(f);
        }
        found
    }

    /// glob import の `*` の位置の、`from` から見える子。
    ///
    /// パスが解決できなければ `None` (本番の名前解決がエラーにする)。
    /// モジュールでも enum でもなければ `Some(Err(()))`。
    fn glob_members(
        &self,
        from: ModId,
        path: &Path,
        interner: &mut IdentInterner,
    ) -> Option<Result<Vec<(InternedIdent, Found)>, ()>> {
        let place = if path.segments.is_empty() {
            self.start(from, path, interner)?.0?
        } else {
            let found = self.resolve(from, path, interner)?;
            match self.place_of(&found.kind) {
                Some(place) => place,
                None => return Some(Err(())),
            }
        };

        let members: Vec<(InternedIdent, Found)> = match place {
            Place::Mod(m) => {
                let mut members: Vec<(InternedIdent, Found)> = m
                    .children
                    .iter()
                    .map(|(name, item)| {
                        (
                            *name,
                            Found {
                                kind: module_item_to_def_id_kind(item),
                                vis: m.child_visibility(*name).unwrap_or_else(public),
                            },
                        )
                    })
                    .collect();
                if let Some(table) = self.tables.get(&m.mod_id) {
                    for (name, e) in table.explicit.iter().chain(&table.glob) {
                        if !members.iter().any(|(n, _)| n == name) {
                            members.push((
                                *name,
                                Found {
                                    kind: e.kind.clone(),
                                    vis: e.vis,
                                },
                            ));
                        }
                    }
                }
                members
            }
            Place::Ty(t) => {
                let variants: Vec<(InternedIdent, Found)> = t
                    .children
                    .borrow()
                    .iter()
                    .filter_map(|(name, assoc)| {
                        let item = assoc.assocs.first()?;
                        match item.kind {
                            AssocNameTreeItemKind::Variant(id) => Some((
                                *name,
                                Found {
                                    kind: DefIdKind::Variant(id),
                                    vis: item.vis,
                                },
                            )),
                            AssocNameTreeItemKind::Val(_) => None,
                        }
                    })
                    .collect();
                if variants.is_empty() {
                    return Some(Err(()));
                }
                variants
            }
            Place::ExtMod(pkg_id, sym_idx) => self
                .ext_view(pkg_id, sym_idx)?
                .list_children(interner)
                .into_iter()
                .map(|(name, r)| {
                    (
                        name,
                        Found {
                            kind: ext_child_ref_to_def_id_kind(&r),
                            vis: r.vis,
                        },
                    )
                })
                .collect(),
            Place::ExtTy(pkg_id, sym_idx) => {
                match self
                    .ext_view(pkg_id, sym_idx)?
                    .list_variants(sym_idx, interner)
                {
                    Some(variants) => variants
                        .into_iter()
                        .map(|(name, r)| {
                            (
                                name,
                                Found {
                                    kind: ext_child_ref_to_def_id_kind(&r),
                                    vis: r.vis,
                                },
                            )
                        })
                        .collect(),
                    None => return Some(Err(())),
                }
            }
        };

        let mut visible: Vec<(InternedIdent, Found)> = members
            .into_iter()
            .filter(|(_, f)| self.is_visible_from(&f.vis, from))
            .collect();
        // 表への入れ方 (重複の報告の向きなど) を決定論的にするため、名前順に並べる。
        visible.sort_by(|(a, _), (b, _)| interner.get_str(a).cmp(&interner.get_str(b)));
        Some(Ok(visible))
    }

    /// 不動点の後の検査 (§4.3)。
    fn check_module(
        &self,
        mod_id: ModId,
        imports: &[&ImportDecl],
        interner: &mut IdentInterner,
        errors: &mut Vec<ResolveError>,
    ) {
        let module = self.mod_index[&mod_id];
        let table = &self.tables[&mod_id];

        for import in imports {
            let declared = self.parents.resolve(&import.vis, mod_id);
            if import.glob {
                match self.glob_members(mod_id, &import.path, interner) {
                    None => {}
                    Some(Err(())) => errors.push(ResolveError::GlobImportOfNonContainer {
                        span: import.span.clone(),
                    }),
                    // 書いた可視性まで広げられるものが 1 つも無い re-export。
                    Some(Ok(members)) if declared.declared != DeclaredVisibility::Private => {
                        let widened = members.iter().any(|(_, f)| {
                            self.narrower(declared, f.vis).map(|v| v.scope) == Some(declared.scope)
                        });
                        if !widened {
                            errors.push(ResolveError::GlobReexportsNothing {
                                span: import.span.clone(),
                                vis: declared,
                            });
                        }
                    }
                    Some(Ok(_)) => {}
                }
            } else if declared.declared != DeclaredVisibility::Private
                && let Some(last) = import.path.segments.last()
                && let Some(found) = self.resolve(mod_id, &import.path, interner)
                && !self.scope_ops().covers(found.vis.scope, declared.scope)
            {
                // re-export は、指すものの可視性の範囲を超えられない。
                errors.push(ResolveError::ReexportBeyondVisibility {
                    name: last.ident.clone(),
                    span: import.span.clone(),
                    declared,
                    target: found.vis,
                });
            }
        }

        // glob で入った名前は、そのモジュールの定義・明示した import と重複してはならない
        // (同じものを指すだけなら重複としない)。
        let mut names: Vec<(&InternedIdent, &ImportEntry)> = table.glob.iter().collect();
        names.sort_by(|(a, _), (b, _)| interner.get_str(a).cmp(&interner.get_str(b)));
        for (name, entry) in names {
            let other = module
                .children
                .get(name)
                .map(|item| (module_item_to_def_id_kind(item), None))
                .or_else(|| {
                    table
                        .explicit
                        .get(name)
                        .map(|e| (e.kind.clone(), Some(e.span.clone())))
                });
            if let Some((kind, other_span)) = other
                && kind != entry.kind
            {
                errors.push(ResolveError::GlobImportShadowed {
                    name: *name,
                    glob_span: entry.span.clone(),
                    other_span,
                });
            }
        }
    }
}
