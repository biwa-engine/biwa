use std::{path::Path, sync::Arc};

use biwac_base::{BiwacError, ErrorContext};
use biwac_dependency_metadata::ExternalPackage;

use crate::NameResolver;

/// Loads a .biwameta file from a built dependency's build directory.
fn load_dep_metadata(dep_root: &Path, dep_name: &str) -> biwac_dependency_metadata::DepMetadata {
    let meta_path = dep_root
        .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
        .join(format!("{}.biwameta", dep_name));
    let data = std::fs::read(&meta_path).unwrap();
    biwac_dependency_metadata::DepMetadata::decode_file(&data).unwrap()
}

#[test]
fn test1() {
    // assets/tests/test1
    // 以下にbiwaのパッケージのディレクトリがあることを前提とする

    let mut srcs = biwac_base::SourceHolder::default();
    let mut interner = biwac_base::IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests/std");
    let pkg_name = interner.get_or_insert("std");

    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root_path.to_path_buf()).unwrap();

    // let build_dir_path = pkg_root_path.join(Path::new(biwac_base::BIWA_BUILD_DIRECTORY_NAME));

    // 依存パッケージは <root>/.biwa_build/deps/<name>/ に取得済みである前提。
    // std は依存を持たないので、実際にはここは使われない。
    let packages_dir = biwac_base::dependencies_dir(pkg_root_path);

    let root_dep_names: Vec<String> = metadata
        .metadata
        .dependencies
        .iter()
        .map(|d| d.name.value().to_string())
        .collect();

    // std は依存を持たないので、ここは常に空になる。
    // driver と違って推移閉包は辿らず、直接依存だけを見る簡易版である。
    let external_packages: Vec<ExternalPackage> = root_dep_names
        .iter()
        .map(|dep_name| {
            let dep_root = packages_dir.join(dep_name);
            let dep_metadata =
                biwac_metadata_loader::try_load_package_metadata(dep_root.clone()).unwrap();
            let dep_meta = load_dep_metadata(&dep_root, dep_name);
            ExternalPackage {
                ident: interner.get_or_insert(dep_name),
                // PackageId は (name, version) から導出される。driver と同じ規則。
                pkg_id: biwac_span::PackageHashId::new(
                    &dep_metadata.metadata.name,
                    &dep_metadata.metadata.version,
                )
                .as_package_id(),
                meta: Arc::new(dep_meta),
                direct: true,
            }
        })
        .collect();

    let mut pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path.to_path_buf(),
    )
    .unwrap();

    // driver と同じく、選択されていない arch の native を落としてから名前解決する。
    // std は同じ名前で arch 違いの native を並べているので、
    // 落とさずに渡すとシンボルが衝突する。
    pkg.walk_modules_mut(|module| {
        biwac_attribute::retain_for_target(
            &mut module.ast,
            biwac_base::Target::TypeScript,
            &interner,
        );
    });

    let _hir = NameResolver::new(&metadata, external_packages, pkg_name, &mut pkg)
        .unwrap()
        .try_resolve(&mut interner)
        .map_err(|errors| {
            for e in errors {
                e.print_error_message(&ErrorContext {
                    metadata: &metadata,
                    srcs: &srcs,
                    interner: &interner,
                });
            }
        })
        .unwrap();
}

/// 名前の表に可視性が載ること (issue #8 の段階 2)。
///
/// モジュールの子は `ModuleNameTree::child_visibility`、関連 item と variant は
/// `AssocNameTreeItem::vis` に載る。見える範囲は宣言したモジュール (関連 item は impl ブロックのモジュール)
/// とその親から決まる。
#[test]
fn name_tree_carries_visibility() {
    use biwac_base::PackageId;
    use biwac_hir::{DeclaredVisibility as D, VisibilityScope as S};

    use crate::{ModuleNameTreeItem, resolving::def_collector::DefCollector};

    let mut srcs = biwac_base::SourceHolder::default();
    let mut interner = biwac_base::IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests/mod_tree");
    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root_path.to_path_buf()).unwrap();
    let pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path.to_path_buf(),
    )
    .unwrap_or_else(|_| panic!("failed to load mod_tree"));
    let names = [
        "mod_tree", "a", "c", "top", "Pair", "Choice", "Left", "new", "sum", "from_a",
    ]
    .map(|n| (n, interner.get_or_insert(n)));
    let id = |n: &str| names.iter().find(|(k, _)| *k == n).unwrap().1;

    let name_tree = DefCollector::new()
        .collect(id("mod_tree"), &pkg, Vec::new(), &mut interner)
        .unwrap_or_else(|e| panic!("def collection failed: {e:?}"));
    let root = &name_tree.packages[&id("mod_tree")].root_module_tree;
    let root_mod = root.mod_id;
    let vis = |n: &str| {
        let v = root.child_visibility(id(n)).unwrap();
        (v.declared, v.scope)
    };

    assert_eq!(vis("a"), (D::Public, S::Public));
    assert_eq!(vis("c"), (D::Private, S::Module(root_mod)));
    assert_eq!(vis("top"), (D::Public, S::Public));
    assert_eq!(
        vis("Pair"),
        (D::Package, S::Package(PackageId::SELF_PACKAGE))
    );

    // `a` の中の `pub(super) fn from_a` は、親 (ルート) が範囲になる。
    let Some(ModuleNameTreeItem::Mod(a)) = root.children.get(&id("a")) else {
        panic!("`a` is not a module")
    };
    let from_a = a.child_visibility(id("from_a")).unwrap();
    assert_eq!(
        (from_a.declared, from_a.scope),
        (D::Super, S::Module(root_mod))
    );

    // 関連 item と variant。
    let assoc = |ty: &str, n: &str| {
        let Some(ModuleNameTreeItem::Ty(t)) = root.children.get(&id(ty)) else {
            panic!("`{ty}` is not a type")
        };
        let v = t.children.borrow()[&id(n)].assocs[0].vis;
        (v.declared, v.scope)
    };
    assert_eq!(assoc("Pair", "new"), (D::Public, S::Public));
    assert_eq!(
        assoc("Pair", "sum"),
        (D::Package, S::Package(PackageId::SELF_PACKAGE))
    );
    assert_eq!(assoc("Choice", "Left"), (D::Public, S::Public));
}

/// パッケージの中の可視性の違反が、それぞれ `InvisibleItem` になること (issue #8 の段階 3)。
///
/// `vis_errors` には、見えるもの (`// OK`) と見えないもの (`// NG`) を並べてある。
/// 報告されるのは見えないものだけで、それぞれ 1 度だけである
/// (import が private を指すときは、その import の側で 1 度)。
#[test]
fn reports_invisible_items() {
    let mut srcs = biwac_base::SourceHolder::default();
    let mut interner = biwac_base::IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests/vis_errors");
    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root_path.to_path_buf()).unwrap();
    let mut pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path.to_path_buf(),
    )
    .unwrap_or_else(|_| panic!("failed to load vis_errors"));
    let pkg_name = interner.get_or_insert("vis_errors");

    let errors = match NameResolver::new(&metadata, Vec::new(), pkg_name, &mut pkg)
        .unwrap()
        .try_resolve(&mut interner)
    {
        Ok(_) => panic!("vis_errors must be rejected"),
        Err(errors) => errors,
    };

    let mut invisible: Vec<&str> = errors
        .iter()
        .map(|e| match e {
            crate::ResolveError::InvisibleItem { segment, .. } => {
                interner.get_str(&segment.ident.id).unwrap()
            }
            e => panic!("unexpected error: {e:?}"),
        })
        .collect();
    invisible.sort();
    assert_eq!(invisible, ["Secret", "b", "hidden_fn", "secret"]);
}

/// private-in-public: 項目のインターフェースに項目より見えない型・trait があれば
/// `PrivateInPublic` になること (issue #8 の段階 5)。
///
/// `vis_interface` には違反 (`// NG`) と通るもの (`// OK`) を並べてある。
/// 型エイリアス (HIR では右辺に展開されて消える) とジェネリック引数の制限 (trait) も見ること、
/// 祖先のモジュールによる頭打ち (`ok_capped`) を数えることを確かめる。
#[test]
fn reports_private_in_public() {
    let mut srcs = biwac_base::SourceHolder::default();
    let mut interner = biwac_base::IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests/vis_interface");
    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root_path.to_path_buf()).unwrap();
    let mut pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path.to_path_buf(),
    )
    .unwrap_or_else(|_| panic!("failed to load vis_interface"));
    let pkg_name = interner.get_or_insert("vis_interface");

    let errors = match NameResolver::new(&metadata, Vec::new(), pkg_name, &mut pkg)
        .unwrap()
        .try_resolve(&mut interner)
    {
        Ok(_) => panic!("vis_interface must be rejected"),
        Err(errors) => errors,
    };

    let mut pairs: Vec<(&str, &str)> = errors
        .iter()
        .map(|e| match e {
            crate::ResolveError::PrivateInPublic { item, used, .. } => (
                interner.get_str(&item.id).unwrap(),
                interner.get_str(&used.id).unwrap(),
            ),
            e => panic!("unexpected error: {e:?}"),
        })
        .collect();
    pairs.sort();
    assert_eq!(
        pairs,
        [
            ("Choice", "Priv"),
            ("field", "Priv"),
            ("ng_alias", "PrivAlias"),
            ("ng_arg", "Priv"),
            ("ng_bound", "PrivTrait"),
            ("ng_method", "Priv"),
            ("ng_nested", "Priv"),
            ("ng_ret", "Priv"),
        ]
    );
}

/// 依存の無いフィクスチャを読み込んで名前解決し、エラーを `check` に渡す (成功なら空)。
fn resolve_errors_of(
    name: &str,
    check: impl FnOnce(&[crate::ResolveError], &biwac_base::IdentInterner),
) {
    let mut srcs = biwac_base::SourceHolder::default();
    let mut interner = biwac_base::IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests").join(name);
    let metadata = biwac_metadata_loader::try_load_package_metadata(pkg_root_path.clone()).unwrap();
    let mut pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path,
    )
    .unwrap_or_else(|_| panic!("failed to load {name}"));
    let pkg_name = interner.get_or_insert(name);
    let errors = match NameResolver::new(&metadata, Vec::new(), pkg_name, &mut pkg)
        .unwrap()
        .try_resolve(&mut interner)
    {
        Ok(_) => Vec::new(),
        Err(errors) => errors,
    };
    check(&errors, &interner);
}

/// glob import と re-export の正例が名前解決を通ること (issue #18)。
///
/// private なモジュールの中のものの re-export、`pub` と `pub(package)` の混ざった glob の re-export、
/// 明示した import と glob の重なり (同じものを指す)、re-export の連鎖、enum の glob、`super::*`、
/// 他のモジュールの re-export をパスの途中で引くこと、re-export の経路を数えた private-in-public。
#[test]
fn glob_and_reexport_resolve() {
    resolve_errors_of("imp_ok", |errors, _| {
        assert!(errors.is_empty(), "{errors:?}");
    });
}

/// glob import と re-export の誤りがそれぞれ報告されること (issue #18)。
#[test]
fn reports_import_pattern_errors() {
    use crate::ResolveError as E;
    resolve_errors_of("imp_errors", |errors, interner| {
        let name = |id| interner.get_str(id).unwrap();
        let mut kinds: Vec<String> = errors
            .iter()
            .map(|e| match e {
                E::ReexportBeyondVisibility { name: n, .. } => {
                    format!("beyond {}", name(&n.id))
                }
                E::GlobReexportsNothing { .. } => "nothing".to_string(),
                E::GlobImportOfNonContainer { .. } => "non-container".to_string(),
                E::ImportedNameConflict { name: n, .. } => format!("conflict {}", name(n)),
                E::GlobImportShadowed { name: n, .. } => format!("shadowed {}", name(n)),
                e => panic!("unexpected error: {e:?}"),
            })
            .collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                "beyond pkg_only",
                "conflict value",
                "non-container",
                "nothing",
                "shadowed local"
            ]
        );
    });
}

/// import が作った名前は、その import の可視性で他のモジュールから見える (private な import は見えない)。
/// glob import のパスの途中も、見えるかを確かめる。
#[test]
fn private_import_is_invisible_from_other_modules() {
    resolve_errors_of("imp_invisible", |errors, interner| {
        let names: Vec<&str> = errors
            .iter()
            .map(|e| match e {
                crate::ResolveError::InvisibleItem { segment, .. } => {
                    interner.get_str(&segment.ident.id).unwrap()
                }
                e => panic!("unexpected error: {e:?}"),
            })
            .collect();
        let mut names = names;
        names.sort();
        assert_eq!(names, ["kept", "secret"]);
    });
}
