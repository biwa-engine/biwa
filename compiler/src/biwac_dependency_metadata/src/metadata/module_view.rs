use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use biwac_base::{IdentInterner, InternedIdent, PackageId};
use biwac_span::{DefId, PackageLocalDefId, TyDefId, ValDefId, VariantDefId};

use super::{
    DepMetadata,
    body::SymbolBody,
    format::{
        DiskVisibility, REEXPORT_KIND_MOD, REEXPORT_KIND_TRAIT, REEXPORT_KIND_TY,
        REEXPORT_KIND_VAL, REEXPORT_KIND_VARIANT,
    },
};

/// 外部パッケージのモジュールのシンボルを名前で検索する統一インタフェース。
///
/// 自パッケージの ModuleNameTree と外部パッケージの DepMetadata 両方を
/// 名前解決コンテキストから差異なく扱えるようにするためのトレイト。
///
/// DefId の割り当ては DefCollector が一括で行う。
/// このトレイトは lookup のみを担い、割り当て済みの sym_idx を返す。
pub trait PackageModuleView: Send + Sync {
    /// 直接の子シンボルを名前で検索する。
    /// `interner` は InternedIdent → &str 変換に使用 (DepMetadata 実装側)。
    fn lookup_child(
        &self,
        name: InternedIdent,
        interner: &IdentInterner,
    ) -> Option<ExternalChildRef>;

    /// 子モジュール (kind == Mod) のビューを返す。
    fn get_module_view(&self, module_sym_idx: u32) -> Box<dyn PackageModuleView>;

    /// 型シンボルの assoc シンボルを名前で検索する (method/assoc-fn 解決用)。
    /// `local_ty_idx`: その型のシンボルインデックス (将来 enum 等にも対応)。
    fn lookup_assoc(
        &self,
        local_ty_idx: u32,
        name: InternedIdent,
        interner: &IdentInterner,
    ) -> Option<ExternalChildRef>;

    /// この view のパッケージ ID。
    fn pkg_id(&self) -> PackageId;

    /// 子シンボルをすべて返す (glob import 用)。見えないものも含む。
    fn list_children(&self, interner: &mut IdentInterner)
    -> Vec<(InternedIdent, ExternalChildRef)>;

    /// 型シンボルが enum ならそのバリアントをすべて返す (glob import 用)。enum でなければ `None`。
    fn list_variants(
        &self,
        local_ty_idx: u32,
        interner: &mut IdentInterner,
    ) -> Option<Vec<(InternedIdent, ExternalChildRef)>>;
}

/// 外部パッケージのシンボルへの参照。
#[derive(Debug, Clone, Copy)]
pub struct ExternalChildRef {
    /// シンボルを定義しているパッケージ。
    ///
    /// ふつうは引いた view のパッケージだが、re-export (`pub import`) は
    /// 別のパッケージのシンボルを指しうる。
    pub pkg_id: PackageId,
    /// `pkg_id` のパッケージ内でのシンボルインデックス (DiskSymbolIndex の値)。
    pub sym_idx: u32,
    pub kind: ExternalChildKind,
    /// 宣言の可視性。見えないものも表から消さずに返す
    /// (「見つからない」ではなく「見えない」と言えるようにするため)。
    pub vis: biwac_hir::Visibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChildKind {
    Mod,
    Ty,
    Val,
    Variant,
    Trait,
}

impl ExternalChildRef {
    fn def_id(&self) -> DefId {
        DefId::new(self.pkg_id, PackageLocalDefId::new(self.sym_idx))
    }

    pub fn as_ty_def_id(&self) -> TyDefId {
        TyDefId::new(self.def_id())
    }

    pub fn as_trait_def_id(&self) -> biwac_span::TraitDefId {
        biwac_span::TraitDefId::new(self.def_id())
    }

    pub fn as_val_def_id(&self) -> ValDefId {
        ValDefId::new(self.def_id())
    }

    pub fn as_variant_def_id(&self) -> VariantDefId {
        VariantDefId::new(self.def_id())
    }
}

/// DepMetadata の特定モジュールへの lazy view。
///
/// 各モジュールの子シンボル名索引を初回アクセス時に構築する (OnceLock)。
/// 以降は O(1) で名前検索できる。
/// Arc<DepMetadata> を保持するので 'static + Send + Sync。
pub struct DepMetadataModuleView {
    dep: Arc<DepMetadata>,
    module_sym_idx: u32,
    pkg_id: PackageId,
    /// 遅延構築: 子シンボル名 (raw string) → 参照 (re-export を含む)
    name_index: OnceLock<HashMap<String, ExternalChildRef>>,
}

impl DepMetadataModuleView {
    /// ルートモジュールの view を作成する。
    pub fn new_root(dep: Arc<DepMetadata>, pkg_id: PackageId) -> Self {
        let module_sym_idx = dep.root_sym_idx;
        Self {
            dep,
            module_sym_idx,
            pkg_id,
            name_index: OnceLock::new(),
        }
    }

    /// 特定シンボルインデックスのサブモジュール view を作成する (名前解決から使用)。
    pub fn new_for_sym_idx(dep: Arc<DepMetadata>, module_sym_idx: u32, pkg_id: PackageId) -> Self {
        Self {
            dep,
            module_sym_idx,
            pkg_id,
            name_index: OnceLock::new(),
        }
    }

    fn new_submodule(dep: Arc<DepMetadata>, module_sym_idx: u32, pkg_id: PackageId) -> Self {
        Self {
            dep,
            module_sym_idx,
            pkg_id,
            name_index: OnceLock::new(),
        }
    }

    /// モジュールの子シンボル名索引を構築する。
    /// 各子シンボルのボディを lazy decode して名前文字列を取得する。
    fn build_name_index(&self) -> HashMap<String, ExternalChildRef> {
        let mut map = HashMap::new();

        let body = match self.dep.get_symbol_body(self.module_sym_idx as usize) {
            Ok(b) => b,
            Err(_) => return map,
        };
        let SymbolBody::Mod(ref mod_data) = *body else {
            return map;
        };

        for &child_idx in &mod_data.children.0 {
            let hdr = match self.dep.sym_hdrs.get(child_idx.0 as usize) {
                Some(h) => h,
                None => continue,
            };
            let child_body = match self.dep.sym_bodies.get(child_idx.0 as usize, hdr) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let (name_offset, kind) = match child_body {
                SymbolBody::Fn(fn_data) => (fn_data.name, ExternalChildKind::Val),
                SymbolBody::Struct(struct_data) => (struct_data.name, ExternalChildKind::Ty),
                SymbolBody::Mod(mod_data) => (mod_data.name, ExternalChildKind::Mod),
                SymbolBody::NativeTypeAlias(alias) => (alias.name, ExternalChildKind::Ty),
                SymbolBody::TypeAlias(alias) => (alias.name, ExternalChildKind::Ty),
                SymbolBody::Enum(enum_data) => (enum_data.name, ExternalChildKind::Ty),
                SymbolBody::Trait(trait_data) => (trait_data.name, ExternalChildKind::Trait),
                // バリアントはモジュールの直下には載らない。enum の子である。
                // trait の項目も同様で、trait impl ブロックには名前が無い。
                SymbolBody::Variant(_) | SymbolBody::TraitAssoc(_) | SymbolBody::TraitImpl(_) => {
                    continue;
                }
            };
            let name_str = match self.dep.strings.get(name_offset) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };
            map.insert(
                name_str,
                ExternalChildRef {
                    pkg_id: self.pkg_id,
                    sym_idx: child_idx.0,
                    kind,
                    vis: self.dep.ext_visibility(self.pkg_id, child_idx.0),
                },
            );
        }

        // re-export。名前は定義と重ならない (名前解決が検査している) が、念のため定義を優先する。
        for r in &mod_data.reexports.0 {
            let Ok(name) = self.dep.strings.get(r.name) else {
                continue;
            };
            let kind = match r.kind {
                REEXPORT_KIND_MOD => ExternalChildKind::Mod,
                REEXPORT_KIND_TY => ExternalChildKind::Ty,
                REEXPORT_KIND_VAL => ExternalChildKind::Val,
                REEXPORT_KIND_VARIANT => ExternalChildKind::Variant,
                REEXPORT_KIND_TRAIT => ExternalChildKind::Trait,
                _ => continue,
            };
            let (pkg_id, sym_idx) = if r.external == 0 {
                (self.pkg_id, r.target)
            } else {
                match self.dep.resolve_ext_sym(r.target) {
                    Some(t) => t,
                    None => continue,
                }
            };
            // 見える範囲は re-export したモジュール (このモジュール) が基準である。
            let declared = DiskVisibility::try_from(r.vis)
                .map(biwac_hir::DeclaredVisibility::from)
                .unwrap_or(biwac_hir::DeclaredVisibility::Private);
            let vis = self
                .dep
                .visibility_in_module(self.pkg_id, self.module_sym_idx, declared);
            map.entry(name.to_string()).or_insert(ExternalChildRef {
                pkg_id,
                sym_idx,
                kind,
                vis,
            });
        }
        map
    }
}

impl PackageModuleView for DepMetadataModuleView {
    fn lookup_child(
        &self,
        name: InternedIdent,
        interner: &IdentInterner,
    ) -> Option<ExternalChildRef> {
        let name_str = interner.get_str(&name)?;
        let index = self.name_index.get_or_init(|| self.build_name_index());
        index.get(name_str).copied()
    }

    fn get_module_view(&self, module_sym_idx: u32) -> Box<dyn PackageModuleView> {
        Box::new(DepMetadataModuleView::new_submodule(
            Arc::clone(&self.dep),
            module_sym_idx,
            self.pkg_id,
        ))
    }

    fn lookup_assoc(
        &self,
        local_ty_idx: u32,
        name: InternedIdent,
        interner: &IdentInterner,
    ) -> Option<ExternalChildRef> {
        let name_str = interner.get_str(&name)?;
        let hdr = self.dep.sym_hdrs.get(local_ty_idx as usize)?;
        let body = self.dep.sym_bodies.get(local_ty_idx as usize, hdr).ok()?;
        // native type alias (`type Vec[T] = {{ ... }};`) も assoc fns を持つ。
        // struct だけを見ていると `Vec::new()` が外のパッケージから引けない。
        // enum はバリアントも子に持つ。関連関数と同じ表から引ける
        // (名前空間が型と値で分かれていないので `Color::Red` も `Color::from_hex` も同じ)。
        if let SymbolBody::Enum(ref enum_data) = *body {
            for &variant_sym_idx in &enum_data.variant_symbols.0 {
                let variant_hdr = self.dep.sym_hdrs.get(variant_sym_idx.0 as usize)?;
                let variant_body = self
                    .dep
                    .sym_bodies
                    .get(variant_sym_idx.0 as usize, variant_hdr)
                    .ok()?;
                let SymbolBody::Variant(ref variant_data) = *variant_body else {
                    continue;
                };
                if self.dep.strings.get(variant_data.name).unwrap_or("") == name_str {
                    return Some(ExternalChildRef {
                        pkg_id: self.pkg_id,
                        sym_idx: variant_sym_idx.0,
                        kind: ExternalChildKind::Variant,
                        vis: self.dep.ext_visibility(self.pkg_id, variant_sym_idx.0),
                    });
                }
            }
        }

        let assoc_symbols = match *body {
            SymbolBody::Struct(ref struct_data) => &struct_data.assoc_symbols.0,
            SymbolBody::NativeTypeAlias(ref alias_data) => &alias_data.assoc_symbols.0,
            SymbolBody::Enum(ref enum_data) => &enum_data.assoc_symbols.0,
            _ => return None,
        };
        for &assoc_sym_idx in assoc_symbols {
            let assoc_hdr = self.dep.sym_hdrs.get(assoc_sym_idx.0 as usize)?;
            let assoc_body = self
                .dep
                .sym_bodies
                .get(assoc_sym_idx.0 as usize, assoc_hdr)
                .ok()?;
            let SymbolBody::Fn(ref fn_data) = *assoc_body else {
                continue;
            };
            let assoc_name = self.dep.strings.get(fn_data.name).unwrap_or("");
            if assoc_name == name_str {
                return Some(ExternalChildRef {
                    pkg_id: self.pkg_id,
                    sym_idx: assoc_sym_idx.0,
                    kind: ExternalChildKind::Val,
                    vis: self.dep.ext_visibility(self.pkg_id, assoc_sym_idx.0),
                });
            }
        }
        None
    }

    fn pkg_id(&self) -> PackageId {
        self.pkg_id
    }

    fn list_children(
        &self,
        interner: &mut IdentInterner,
    ) -> Vec<(InternedIdent, ExternalChildRef)> {
        let index = self.name_index.get_or_init(|| self.build_name_index());
        // 名前順に並べる (glob の結果を決定論的にするため)。
        let mut names: Vec<(&String, &ExternalChildRef)> = index.iter().collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        names
            .into_iter()
            .map(|(name, r)| (interner.get_or_insert(name), *r))
            .collect()
    }

    fn list_variants(
        &self,
        local_ty_idx: u32,
        interner: &mut IdentInterner,
    ) -> Option<Vec<(InternedIdent, ExternalChildRef)>> {
        let hdr = self.dep.sym_hdrs.get(local_ty_idx as usize)?;
        let body = self.dep.sym_bodies.get(local_ty_idx as usize, hdr).ok()?;
        let SymbolBody::Enum(ref enum_data) = *body else {
            return None;
        };
        let mut out = Vec::new();
        for &variant_sym_idx in &enum_data.variant_symbols.0 {
            let variant_hdr = self.dep.sym_hdrs.get(variant_sym_idx.0 as usize)?;
            let variant_body = self
                .dep
                .sym_bodies
                .get(variant_sym_idx.0 as usize, variant_hdr)
                .ok()?;
            let SymbolBody::Variant(ref variant_data) = *variant_body else {
                continue;
            };
            let name = self.dep.strings.get(variant_data.name).unwrap_or("");
            out.push((
                interner.get_or_insert(name),
                ExternalChildRef {
                    pkg_id: self.pkg_id,
                    sym_idx: variant_sym_idx.0,
                    kind: ExternalChildKind::Variant,
                    vis: self.dep.ext_visibility(self.pkg_id, variant_sym_idx.0),
                },
            ));
        }
        Some(out)
    }
}
