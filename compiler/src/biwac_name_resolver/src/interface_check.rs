//! private-in-public の検査 (`docs/symbol-visibility-impl-status.md` §5.2)。
//!
//! 項目の実効可視性を V とすると、その**インターフェース**に現れる型・trait は
//! どれも実効可視性が V 以上でなければならない。
//! `pub fn make() -> Priv` のように、使う側から名前を書けない型の値が外に出るのを定義側で止める。
//!
//! 実効可視性には re-export (`pub import`) の経路も数える (`docs/useful-import-patterns-impl-status.md` §4.5)。
//!
//! 名前解決が済んだ AST の上で行う。HIR では型エイリアスが右辺に展開されていて、
//! `pub fn f() -> PrivAlias` の `PrivAlias` が見えなくなっているためである。

use std::collections::HashMap;

use biwac_ast::{
    ArgDeclList, Globals, Ident, ImplBlock, PathSegmentResolution, RetTypRepr, TraitItemArgs,
    TypRepr, TypReprVal, TypeDef, VariantFieldsDecl, symbols::globals::GenArgsDecl,
};
use biwac_base::ModId;
use biwac_hir::VisibilityScope;
use biwac_package_loader::{LoadedModule, Pkg};
use biwac_span::{DefIdKind, TraitDefId, TyDefId, ValDefId};

use crate::{
    ResolveError,
    resolving::import_table::ModuleImports,
    visibility::{ModuleParents, ScopeOps},
};

/// 実効可視性。項目に届く経路ごとの見える範囲の和である。
///
/// 1 本の経路の範囲は、経路上の各段 (モジュール・項目) の見える範囲の共通部分で、
/// どの段の範囲もその項目自身を含む部分木なので、最も狭い 1 つになる。
/// 定義の経路に、re-export (`pub import`) の経路の分だけ範囲を足す (和)。
#[derive(Debug, Clone)]
pub struct EffectiveVisibility {
    ranges: Vec<VisibilityScope>,
}

impl EffectiveVisibility {
    fn empty() -> Self {
        Self { ranges: Vec::new() }
    }

    fn public() -> Self {
        Self {
            ranges: vec![VisibilityScope::Public],
        }
    }

    /// 見える範囲のうち、最も広いもの (エラーの文面に使う)。空なら `None`。
    pub fn widest(&self) -> Option<VisibilityScope> {
        self.ranges.first().copied()
    }
}

/// 実効可視性どうしの演算。範囲 1 つずつの演算 ([`ScopeOps`]) を経路の和に広げたもの。
struct Scopes<'a> {
    ops: ScopeOps<'a>,
}

impl Scopes<'_> {
    fn covers(&self, outer: VisibilityScope, inner: VisibilityScope) -> bool {
        self.ops.covers(outer, inner)
    }

    fn meet(&self, a: VisibilityScope, b: VisibilityScope) -> Option<VisibilityScope> {
        self.ops.meet(a, b)
    }

    /// 2 つの実効可視性の共通部分 (経路ごとの範囲の組ごとの共通部分の和)。
    fn meet_all(&self, a: &EffectiveVisibility, b: &EffectiveVisibility) -> EffectiveVisibility {
        let mut ranges = Vec::new();
        for x in &a.ranges {
            for y in &b.ranges {
                if let Some(r) = self.meet(*x, *y) {
                    ranges.push(r);
                }
            }
        }
        EffectiveVisibility { ranges }
    }

    /// 経路を 1 段延ばす: 各経路の範囲と、次の段の範囲 `next` の共通部分。
    fn through(&self, eff: &EffectiveVisibility, next: VisibilityScope) -> EffectiveVisibility {
        self.meet_all(eff, &EffectiveVisibility { ranges: vec![next] })
    }

    /// 和 (経路を足す)。他の範囲に含まれる範囲は落とす。
    fn union(&self, a: &EffectiveVisibility, b: &EffectiveVisibility) -> EffectiveVisibility {
        let mut ranges: Vec<VisibilityScope> = Vec::new();
        for r in a.ranges.iter().chain(&b.ranges) {
            if ranges.iter().any(|x| self.covers(*x, *r)) {
                continue;
            }
            ranges.retain(|x| !self.covers(*r, *x));
            ranges.push(*r);
        }
        EffectiveVisibility { ranges }
    }

    /// 同じ範囲か。
    fn same(&self, a: &EffectiveVisibility, b: &EffectiveVisibility) -> bool {
        self.covers_all(a, b) && self.covers_all(b, a)
    }

    /// `outer` が `inner` を含むか (`inner` のどの範囲も、`outer` のどれかの範囲に含まれる)。
    fn covers_all(&self, outer: &EffectiveVisibility, inner: &EffectiveVisibility) -> bool {
        inner
            .ranges
            .iter()
            .all(|i| outer.ranges.iter().any(|o| self.covers(*o, *i)))
    }
}

/// 自パッケージのモジュール・型・trait の実効可視性。
struct Effective {
    modules: HashMap<ModId, EffectiveVisibility>,
    tys: HashMap<TyDefId, EffectiveVisibility>,
    traits: HashMap<TraitDefId, EffectiveVisibility>,
    /// 関数・native 関数・scene。
    vals: HashMap<ValDefId, EffectiveVisibility>,
}

/// re-export の経路で足す範囲の持ち主 (自パッケージのもの)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Mod(ModId),
    Ty(TyDefId),
    Trait(TraitDefId),
    Val(ValDefId),
}

impl Key {
    fn of(kind: &DefIdKind) -> Option<Self> {
        match kind {
            DefIdKind::Mod(id) if id.is_self_pkg() => Some(Self::Mod(*id)),
            DefIdKind::Ty(id) if id.pkg().is_self() => Some(Self::Ty(*id)),
            DefIdKind::Trait(id) if id.pkg().is_self() => Some(Self::Trait(*id)),
            DefIdKind::Val(id) if id.pkg().is_self() => Some(Self::Val(*id)),
            _ => None,
        }
    }
}

/// 定義の経路に、re-export の経路 `extra` を足した実効可視性。
fn effective_visibilities(
    pkg: &Pkg,
    imports: &HashMap<ModId, ModuleImports>,
    parents: &ModuleParents,
    scopes: &Scopes,
) -> Effective {
    // re-export の経路は、モジュールの実効可視性が決まらないと決まらず、
    // re-export されたモジュールの子にも経路が延びるので、増えなくなるまで繰り返す。
    // 範囲は有限なので止まる。
    let mut extra: HashMap<Key, EffectiveVisibility> = HashMap::new();
    loop {
        let mut eff = Effective {
            modules: HashMap::new(),
            tys: HashMap::new(),
            traits: HashMap::new(),
            vals: HashMap::new(),
        };
        collect_modules(
            &pkg.root_module,
            EffectiveVisibility::public(),
            parents,
            scopes,
            &extra,
            &mut eff,
        );
        pkg.walk_modules(|module| collect_defs(module, parents, scopes, &extra, &mut eff));

        // モジュール M の import の表の名前 x が T を指すなら、「M ∩ x の可視性」を T に足す。
        let mut next = extra.clone();
        let mut mod_ids: Vec<&ModId> = imports.keys().collect();
        mod_ids.sort();
        for mod_id in mod_ids {
            let Some(module_eff) = eff.modules.get(mod_id) else {
                continue;
            };
            let table = &imports[mod_id];
            for entry in table.explicit.values().chain(table.glob.values()) {
                let Some(key) = Key::of(&entry.kind) else {
                    continue;
                };
                let path = scopes.through(module_eff, entry.vis.scope);
                let current = next
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(EffectiveVisibility::empty);
                next.insert(key, scopes.union(&current, &path));
            }
        }

        let unchanged = next.len() == extra.len()
            && next
                .iter()
                .all(|(k, v)| extra.get(k).is_some_and(|e| scopes.same(e, v)));
        if unchanged {
            return eff;
        }
        extra = next;
    }
}

impl Effective {
    /// 型・trait の実効可視性。依存パッケージ・組み込みのものは `pub` として扱う
    /// (シグネチャにそれを書けたなら、名前解決がその場所から見えることを確かめてある)。
    fn of(&self, kind: &DefIdKind) -> Option<EffectiveVisibility> {
        match kind {
            DefIdKind::Ty(id) if id.pkg().is_self() => self.tys.get(id).cloned(),
            DefIdKind::Trait(id) if id.pkg().is_self() => self.traits.get(id).cloned(),
            DefIdKind::Ty(_) | DefIdKind::Trait(_) => Some(EffectiveVisibility::public()),
            // ジェネリック引数などは可視性を持たない。
            _ => None,
        }
    }
}

/// パッケージ全体の private-in-public を検査する。
pub(crate) fn check(pkg: &Pkg, imports: &HashMap<ModId, ModuleImports>) -> Vec<ResolveError> {
    let parents = ModuleParents::of(pkg);
    let parent_map = parents.to_map();
    let scopes = Scopes {
        ops: ScopeOps {
            parents: &parent_map,
        },
    };

    let eff = effective_visibilities(pkg, imports, &parents, &scopes);

    let mut checker = Checker {
        scopes: &scopes,
        eff: &eff,
        errors: Vec::new(),
    };
    pkg.walk_modules(|module| checker.check_module(module, &parents));
    checker.errors
}

/// モジュールの実効可視性。ルートは `pub`、子は `mod` 宣言の可視性と親の共通部分。
fn collect_modules(
    module: &LoadedModule,
    module_eff: EffectiveVisibility,
    parents: &ModuleParents,
    scopes: &Scopes,
    extra: &HashMap<Key, EffectiveVisibility>,
    eff: &mut Effective,
) {
    for g in &module.ast.globals {
        let Globals::Mod(decl) = g else { continue };
        if let Some(child) = module.children.get(&decl.id.id) {
            let declared = parents.resolve(&decl.vis, module.mod_id).scope;
            let child_eff = with_extra(
                scopes,
                scopes.through(&module_eff, declared),
                extra,
                Key::Mod(child.mod_id),
            );
            collect_modules(child, child_eff, parents, scopes, extra, eff);
        }
    }
    eff.modules.insert(module.mod_id, module_eff);
}

/// 経路の和に、re-export の経路 (`extra[key]`) を足す。
fn with_extra(
    scopes: &Scopes,
    eff: EffectiveVisibility,
    extra: &HashMap<Key, EffectiveVisibility>,
    key: Key,
) -> EffectiveVisibility {
    match extra.get(&key) {
        Some(e) => scopes.union(&eff, e),
        None => eff,
    }
}

/// 型・trait・関数の実効可視性。宣言の可視性とモジュールの共通部分 (と re-export の経路)。
fn collect_defs(
    module: &LoadedModule,
    parents: &ModuleParents,
    scopes: &Scopes,
    extra: &HashMap<Key, EffectiveVisibility>,
    eff: &mut Effective,
) {
    let module_eff = eff.modules[&module.mod_id].clone();
    let item = |vis: &biwac_ast::Visibility, key: Key| {
        with_extra(
            scopes,
            scopes.through(&module_eff, parents.resolve(vis, module.mod_id).scope),
            extra,
            key,
        )
    };
    for g in &module.ast.globals {
        match g {
            Globals::TypeDef(t) => {
                let (def_id, vis) = match t {
                    TypeDef::Struct(s) => (s.def_id.get(), &s.vis),
                    TypeDef::Enum(e) => (e.def_id.get(), &e.vis),
                    TypeDef::TypeAlias(a) => (a.def_id.get(), &a.vis),
                    TypeDef::NativeTypeAlias(a) => (a.def_id.get(), &a.vis),
                };
                if let Some(def_id) = def_id {
                    eff.tys.insert(*def_id, item(vis, Key::Ty(*def_id)));
                }
            }
            Globals::TraitDef(t) => {
                if let Some(def_id) = t.def_id.get() {
                    eff.traits
                        .insert(*def_id, item(&t.vis, Key::Trait(*def_id)));
                }
            }
            Globals::FnDef(f) => {
                if let Some(def_id) = f.def_id.get() {
                    eff.vals.insert(*def_id, item(&f.vis, Key::Val(*def_id)));
                }
            }
            Globals::NativeFnDef(f) => {
                if let Some(def_id) = f.def_id.get() {
                    eff.vals.insert(*def_id, item(&f.vis, Key::Val(*def_id)));
                }
            }
            Globals::NovelScene(sc) => {
                if let Some(def_id) = sc.def_id.get() {
                    eff.vals.insert(*def_id, item(&sc.vis, Key::Val(*def_id)));
                }
            }
            _ => {}
        }
    }
}

struct Checker<'a> {
    scopes: &'a Scopes<'a>,
    eff: &'a Effective,
    errors: Vec<ResolveError>,
}

/// インターフェースの持ち主 (エラーの文面で「何の」インターフェースかを言う)。
struct Owner<'a> {
    name: &'a Ident,
    eff: EffectiveVisibility,
}

impl Checker<'_> {
    /// 関数・scene の実効可視性 (re-export の経路を含む)。表に無ければ `fallback`。
    fn val_eff(
        &self,
        def_id: Option<&ValDefId>,
        fallback: impl FnOnce() -> EffectiveVisibility,
    ) -> EffectiveVisibility {
        def_id
            .and_then(|id| self.eff.vals.get(id).cloned())
            .unwrap_or_else(fallback)
    }

    fn check_module(&mut self, module: &LoadedModule, parents: &ModuleParents) {
        let module_eff = self.eff.modules[&module.mod_id].clone();
        let home = module.mod_id;
        let through = |scopes: &Scopes, base: &EffectiveVisibility, vis: &biwac_ast::Visibility| {
            scopes.through(base, parents.resolve(vis, home).scope)
        };

        for g in &module.ast.globals {
            match g {
                Globals::FnDef(f) => {
                    let owner = Owner {
                        name: &f.id,
                        eff: self
                            .val_eff(f.def_id.get(), || through(self.scopes, &module_eff, &f.vis)),
                    };
                    self.check_signature(&owner, &f.args, &f.rtype, &f.genargs);
                }
                Globals::NativeFnDef(f) => {
                    let owner = Owner {
                        name: &f.id,
                        eff: self
                            .val_eff(f.def_id.get(), || through(self.scopes, &module_eff, &f.vis)),
                    };
                    self.check_signature(&owner, &f.args, &f.rtype, &f.genargs);
                }
                Globals::NovelScene(s) => {
                    let owner = Owner {
                        name: &s.id,
                        eff: self
                            .val_eff(s.def_id.get(), || through(self.scopes, &module_eff, &s.vis)),
                    };
                    self.check_signature::<biwac_span::LocalGenDefId>(
                        &owner, &s.args, &s.rtype, &None,
                    );
                }
                Globals::TypeDef(TypeDef::Struct(s)) => {
                    let Some(struct_eff) = s.def_id.get().and_then(|id| self.eff.tys.get(id))
                    else {
                        continue;
                    };
                    // メンバは、メンバの可視性と struct の共通部分。
                    for m in &s.members {
                        let owner = Owner {
                            name: &m.id,
                            eff: through(self.scopes, struct_eff, &m.vis),
                        };
                        self.check_ty(&owner, &m.typ);
                    }
                }
                Globals::TypeDef(TypeDef::Enum(e)) => {
                    let Some(enum_eff) = e.def_id.get().and_then(|id| self.eff.tys.get(id)) else {
                        continue;
                    };
                    // variant のフィールドは常に enum と同じ可視性になる。
                    let owner = Owner {
                        name: &e.id,
                        eff: enum_eff.clone(),
                    };
                    for v in &e.variants {
                        match &v.fields {
                            VariantFieldsDecl::Unit => {}
                            VariantFieldsDecl::Tuple(fields)
                            | VariantFieldsDecl::Struct(fields) => {
                                for (_, typ) in fields {
                                    self.check_ty(&owner, typ);
                                }
                            }
                        }
                    }
                }
                Globals::TypeDef(TypeDef::TypeAlias(a)) => {
                    let Some(alias_eff) = a.def_id.get().and_then(|id| self.eff.tys.get(id)) else {
                        continue;
                    };
                    let owner = Owner {
                        name: &a.ident,
                        eff: alias_eff.clone(),
                    };
                    self.check_ty(&owner, &a.right);
                }
                Globals::TraitDef(t) => {
                    let Some(trait_eff) = t.def_id.get().and_then(|id| self.eff.traits.get(id))
                    else {
                        continue;
                    };
                    let owner = Owner {
                        name: &t.id,
                        eff: trait_eff.clone(),
                    };
                    self.check_bounds(&owner, &t.genargs);
                    // trait の項目は常に trait と同じ可視性になる。
                    for item in &t.items {
                        let owner = Owner {
                            name: &item.id,
                            eff: trait_eff.clone(),
                        };
                        let args = match &item.args {
                            TraitItemArgs::Assoc(a) => &a.args,
                            TraitItemArgs::Method(a) => &a.args,
                        };
                        for arg in args {
                            self.check_ty(&owner, &arg.typ);
                        }
                        self.check_ret(&owner, &item.rtype);
                        self.check_bounds(&owner, &item.genargs);
                    }
                }
                Globals::ImplBlock(b) => self.check_impl(b, parents, home),
                Globals::TypeDef(TypeDef::NativeTypeAlias(_))
                | Globals::Import(_)
                | Globals::Mod(_)
                | Globals::VarDecl(_)
                | Globals::NativeCode(_) => {}
            }
        }
    }

    /// impl ブロックの項目。
    ///
    /// - inherent impl の項目: 項目の可視性 (impl ブロックのモジュールが基準) と型の共通部分。
    /// - trait impl の項目: trait と型の共通部分 (impl の実効可視性。項目には可視性を書けない)。
    fn check_impl(&mut self, b: &ImplBlock, parents: &ModuleParents, home: ModId) {
        let self_eff = self.eff_of_ty(&b.self_typ);
        let impl_eff = match &b.trait_typ {
            Some(trait_typ) => {
                let trait_eff = self.eff_of_ty(trait_typ);
                self.scopes.meet_all(&self_eff, &trait_eff)
            }
            None => self_eff,
        };

        let item_eff = |vis: &biwac_ast::Visibility| match &b.trait_typ {
            Some(_) => impl_eff.clone(),
            None => self
                .scopes
                .through(&impl_eff, parents.resolve(vis, home).scope),
        };

        for f in &b.assoc_fns {
            let owner = Owner {
                name: &f.id,
                eff: item_eff(&f.vis),
            };
            self.check_signature(&owner, &f.args, &f.rtype, &f.genargs);
            self.check_bounds(&owner, &b.genargs_decl);
        }
        for f in &b.native_assoc_fns {
            let owner = Owner {
                name: &f.id,
                eff: item_eff(&f.vis),
            };
            self.check_signature(&owner, &f.args, &f.rtype, &f.genargs);
            self.check_bounds(&owner, &b.genargs_decl);
        }
        for m in &b.methods {
            let owner = Owner {
                name: &m.id,
                eff: item_eff(&m.vis),
            };
            for arg in &m.args.args {
                self.check_ty(&owner, &arg.typ);
            }
            self.check_ret(&owner, &m.rtype);
            self.check_bounds(&owner, &m.genargs);
            self.check_bounds(&owner, &b.genargs_decl);
        }
        for m in &b.native_methods {
            let owner = Owner {
                name: &m.id,
                eff: item_eff(&m.vis),
            };
            for arg in &m.args.args {
                self.check_ty(&owner, &arg.typ);
            }
            self.check_ret(&owner, &m.rtype);
            self.check_bounds(&owner, &m.genargs);
            self.check_bounds(&owner, &b.genargs_decl);
        }
    }

    /// 型 (impl の対象・trait) の実効可視性。プリミティブや `Self` など、可視性を持たないものは `pub`。
    fn eff_of_ty(&self, typ: &TypRepr) -> EffectiveVisibility {
        match &typ.val {
            TypReprVal::Defined(d) => d
                .path
                .segments
                .last()
                .and_then(|s| match s.resolved_id.get() {
                    Some(PathSegmentResolution::Ok(kind)) => self.eff.of(kind),
                    _ => None,
                })
                .unwrap_or_else(EffectiveVisibility::public),
            _ => EffectiveVisibility::public(),
        }
    }

    fn check_signature<I>(
        &mut self,
        owner: &Owner,
        args: &ArgDeclList,
        rtype: &RetTypRepr,
        genargs: &Option<GenArgsDecl<I>>,
    ) {
        for arg in &args.args {
            self.check_ty(owner, &arg.typ);
        }
        self.check_ret(owner, rtype);
        self.check_bounds(owner, genargs);
    }

    fn check_ret(&mut self, owner: &Owner, rtype: &RetTypRepr) {
        if let RetTypRepr::Typ(t) = rtype {
            self.check_ty(owner, t);
        }
    }

    fn check_bounds<I>(&mut self, owner: &Owner, genargs: &Option<GenArgsDecl<I>>) {
        for g in genargs.iter().flat_map(|g| &g.genargs) {
            for bound in &g.bounds {
                self.check_ty(owner, bound);
            }
        }
    }

    /// 型の中に現れる型・trait (型引数・関数型の引数と戻り値まで辿る) が、持ち主以上に見えるか。
    fn check_ty(&mut self, owner: &Owner, typ: &TypRepr) {
        match &typ.val {
            TypReprVal::Primitive(_) | TypReprVal::SelfTyp => {}
            TypReprVal::Fn(f) => {
                for a in &f.args {
                    self.check_ty(owner, a);
                }
                if let Some(r) = &f.rty {
                    self.check_ty(owner, r);
                }
            }
            TypReprVal::Defined(d) => {
                if let Some(segment) = d.path.segments.last()
                    && let Some(PathSegmentResolution::Ok(kind)) = segment.resolved_id.get()
                    && let Some(used_eff) = self.eff.of(kind)
                    && !self.scopes.covers_all(&used_eff, &owner.eff)
                {
                    self.errors.push(ResolveError::PrivateInPublic {
                        item: owner.name.clone(),
                        item_scope: owner.eff.widest(),
                        used: segment.ident.clone(),
                        used_scope: used_eff.widest(),
                        span: d.path.span(),
                    });
                }
                for g in d.genargs.iter().flatten() {
                    self.check_ty(owner, g);
                }
            }
        }
    }
}
