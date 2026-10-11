use std::{cell::RefCell, collections::HashMap, sync::Arc};

use biwac_ast::PathSegment;
use biwac_base::{InternedIdent, ModId, PackageId};
use biwac_dependency_metadata::{DepMetadata, PackageModuleView};
use biwac_hir::{Ty, Visibility};
use biwac_span::{TraitDefId, TyDefId, ValDefId, VariantDefId};

use crate::{ResolveError, resolving::import_table::ModuleImports};

pub struct NameTree {
    pub(crate) self_pkg_name: InternedIdent,
    /// 自パッケージのみ保持 (型付きアクセス・ミューテーション用)。
    /// 外部パッケージは ext_pkg_views に格納する。
    pub(crate) packages: HashMap<InternedIdent, PackageNameTree>,
    /// 外部パッケージの lazy モジュール view (PackageModuleView トレイト経由)。
    pub(crate) ext_pkg_views: HashMap<InternedIdent, Arc<dyn PackageModuleView>>,
    /// PackageId → DepMetadata (型情報の lazy アクセス用)。
    pub(crate) ext_pkg_data: HashMap<PackageId, Arc<DepMetadata>>,
    /// 自パッケージのモジュールごとの import の表 (明示した import・glob・re-export)。
    /// def collection の Step 1 の直後に作る (`resolving::import_table`)。
    pub(crate) imports: HashMap<ModId, ModuleImports>,
}

impl std::fmt::Debug for NameTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameTree")
            .field("self_pkg_name", &self.self_pkg_name)
            .field("packages", &self.packages)
            .field(
                "ext_pkg_views",
                &self.ext_pkg_views.keys().collect::<Vec<_>>(),
            )
            .field(
                "ext_pkg_data",
                &self.ext_pkg_data.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct PackageNameTree {
    pub(crate) pkg_id: PackageId,
    pub(crate) root_module_tree: ModuleNameTree,
}

#[derive(Debug)]
pub struct ModuleNameTree {
    pub(crate) mod_id: ModId,
    /// 親モジュール。ルートモジュールなら `None`。`super::` の解決に使う。
    pub(crate) parent: Option<ModId>,
    /// 子の可視性。`children` と同じ名前を持つ。子モジュールは `mod` 宣言に書いたもの。
    pub(crate) vis: HashMap<InternedIdent, Visibility>,
    pub(crate) children: HashMap<InternedIdent, ModuleNameTreeItem>,
}

impl ModuleNameTree {
    /// 子 (`name`) の可視性。
    pub fn child_visibility(&self, name: InternedIdent) -> Option<Visibility> {
        self.vis.get(&name).copied()
    }
}

#[derive(Debug)]
pub struct TyNameTree {
    pub(crate) def_id: TyDefId,
    /// Associated items (fns, types) registered via impl blocks, keyed by item name.
    pub(crate) children: RefCell<HashMap<InternedIdent, AssocNameTree>>,
    /// If this entry is a type alias, stores the canonical (chain-followed) non-alias TyDefId.
    pub(crate) alias_target: RefCell<Option<TyDefId>>,
}

#[derive(Debug)]
pub enum ModuleNameTreeItem {
    Mod(ModuleNameTree),
    Ty(TyNameTree),
    Val(ValDefId),
    /// trait。
    ///
    /// 項目は載せない。trait の項目をパスから直接引く構文
    /// (`Gyao::gyao`) は今のところ無く、
    /// 実装は型の側 (`TyNameTree`) からしか引かないためである。
    Trait(TraitDefId),
}

#[derive(Debug)]
pub struct AssocNameTree {
    pub(crate) assocs: Vec<AssocNameTreeItem>,
}

#[derive(Debug, Clone)]
pub struct AssocNameTreeItem {
    pub genargs: Vec<Ty>,
    pub kind: AssocNameTreeItemKind,
    /// 可視性。関連 item は impl ブロックのあるモジュールが基準、variant は enum と同じ。
    pub vis: Visibility,
}

#[derive(Debug, Clone)]
pub enum AssocNameTreeItemKind {
    // TODO:
    // Ty(AssocNameTreeTyItem),
    Val(ValDefId),
    /// enum のバリアント。
    ///
    /// 関連関数と同じ children に載る。名前空間が型と値で分かれていないので、
    /// `Color::Red` も `Color::from_hex` も同じ表から一意に引ける。
    Variant(VariantDefId),
}

impl AssocNameTree {
    pub(crate) fn find_matched(
        &self,
        genargs: Option<&[Ty]>,
        segment: &PathSegment,
    ) -> Result<&AssocNameTreeItem, ResolveError> {
        match genargs {
            Some(genargs) => {
                for item in &self.assocs {
                    if item.genargs.len() == genargs.len()
                        && item
                            .genargs
                            .iter()
                            .zip(genargs)
                            .all(|(t1, t2)| t1.kind.is_duplicated_for_impl_genarg(&t2.kind))
                    {
                        return Ok(item);
                    }
                }

                Err(ResolveError::AssocItemNotFoundForGenArgs {
                    segment: segment.clone(),
                })
            }
            None => {
                if self.assocs.len() == 1 {
                    Ok(&self.assocs[0])
                } else {
                    Err(ResolveError::AmbiguousAssocItem {
                        segment: segment.clone(),
                    })
                }
            }
        }
    }

    pub(crate) fn register_assoc(
        &mut self,
        name: InternedIdent,
        new: AssocNameTreeItem,
    ) -> Result<(), ResolveError> {
        for item in &self.assocs {
            // バリアントは impl のジェネリック引数で分かれない。
            // 同じ名前に何かが既にあれば、それだけで衝突である
            // (`enum Foo { Bar }` と `impl Foo { fn Bar() }` など)。
            let variant_involved = matches!(item.kind, AssocNameTreeItemKind::Variant(_))
                || matches!(new.kind, AssocNameTreeItemKind::Variant(_));

            if variant_involved
                || (item.genargs.len() == new.genargs.len()
                    && item
                        .genargs
                        .iter()
                        .zip(&new.genargs)
                        .all(|(t1, t2)| t1.kind.is_duplicated_for_impl_genarg(&t2.kind)))
            {
                return Err(ResolveError::DuplicatedAssociatedItemForGenArgs {
                    name,
                    assoc1: item.clone(),
                    assoc2: new,
                });
            }
        }

        self.assocs.push(new);

        Ok(())
    }
}
