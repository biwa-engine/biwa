use std::path::Path;

use biwac_base::{IdentInterner, SourceHolder};

use biwac_base::PackageKind;

use crate::{BiwacSourceParser, Pkg};

#[test]
fn test1() {
    // assets/tests/test1
    // 以下にbiwaのパッケージのディレクトリがあることを前提とする

    let mut srcs = SourceHolder::default();
    let mut interner = IdentInterner::default();
    let pkg_root_path = Path::new("../../assets/tests/test1");

    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root_path.to_path_buf()).unwrap();

    let pkg = Pkg::try_load::<BiwacSourceParser>(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root_path.to_path_buf(),
    )
    .unwrap();

    assert!(pkg.pkg_kind == PackageKind::Bin);

    // existence check of `collections` module
    assert!(
        pkg.root_module
            .children
            .contains_key(&interner.get_or_insert("collections"))
    );

    // existence check of `math` module
    let mod_math = pkg
        .root_module
        .children
        .get(&interner.get_or_insert("math"))
        .unwrap();

    // existence check of `math::pos` module
    assert!(
        mod_math
            .children
            .contains_key(&interner.get_or_insert("pos"))
    );

    // existence check of `math::line` module
    assert!(
        mod_math
            .children
            .contains_key(&interner.get_or_insert("line"))
    );
}

/// `mod` 宣言とファイルの突き合わせ (issue #8 の段階 1)。
mod mod_decl {
    use std::path::Path;

    use biwac_base::{IdentInterner, SourceHolder};

    use crate::{BiwacSourceParser, Pkg, PkgLoadError};

    /// フィクスチャを読み込み、結果を `check` に渡す。
    ///
    /// エラーは interner を借りたままなので、モジュール名は先に intern して `names` で渡す。
    fn load(
        name: &str,
        names: &[&str],
        check: impl FnOnce(Result<&Pkg, &[PkgLoadError]>, &[biwac_base::InternedIdent]),
    ) {
        let mut srcs = SourceHolder::default();
        let mut interner = IdentInterner::default();
        let ids: Vec<_> = names.iter().map(|n| interner.get_or_insert(n)).collect();
        let pkg_root_path = Path::new("../../assets/tests").join(name);
        let metadata =
            biwac_metadata_loader::try_load_package_metadata(pkg_root_path.clone()).unwrap();

        let result =
            Pkg::try_load::<BiwacSourceParser>(&metadata, &mut interner, &mut srcs, pkg_root_path);
        match &result {
            Ok(pkg) => check(Ok(pkg), &ids),
            Err(holder) => check(Err(&holder.errs), &ids),
        }
    }

    #[test]
    fn loads_declared_modules() {
        load("mod_tree", &["a", "b", "c"], |pkg, ids| {
            let pkg = pkg.unwrap_or_else(|e| panic!("load failed: {e:?}"));
            let a = &pkg.root_module.children[&ids[0]];
            assert!(a.children.contains_key(&ids[1]));
            assert!(pkg.root_module.children.contains_key(&ids[2]));
        });
    }

    #[test]
    fn rejects_undeclared_file() {
        load("mod_undeclared", &[], |r, _| {
            let errs = r.err().expect("must fail");
            assert!(
                matches!(
                    errs,
                    [PkgLoadError::UndeclaredModuleFile { path, declare_in: Some(parent) }]
                        if path == "src/forgotten.biwa" && parent == "src/lib.biwa"
                ),
                "{errs:?}"
            );
        });
    }

    #[test]
    fn rejects_file_in_orphan_directory() {
        load("mod_orphan_dir", &[], |r, _| {
            let errs = r.err().expect("must fail");
            assert!(
                matches!(
                    errs,
                    [PkgLoadError::UndeclaredModuleFile { path, declare_in: None }]
                        if path == "src/lost/inner.biwa"
                ),
                "{errs:?}"
            );
        });
    }

    #[test]
    fn rejects_declaration_without_file() {
        load("mod_missing_file", &[], |r, _| {
            let errs = r.err().expect("must fail");
            assert!(
                matches!(
                    errs,
                    [PkgLoadError::ModuleFileNotFound { name, expected, .. }]
                        if name == "nowhere" && expected == "src/nowhere.biwa"
                ),
                "{errs:?}"
            );
        });
    }

    #[test]
    fn rejects_duplicated_declaration() {
        load("mod_duplicated", &[], |r, _| {
            let errs = r.err().expect("must fail");
            assert!(
                matches!(
                    errs,
                    [PkgLoadError::DuplicatedModDecl { name, .. }] if name == "a"
                ),
                "{errs:?}"
            );
        });
    }
}
