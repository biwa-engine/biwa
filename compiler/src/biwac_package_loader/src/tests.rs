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

/// 改行コードの違い (`\r\n` / `\r` / `\n`) で読み込み結果が変わらないこと。
///
/// 読み込み時に `\n` へ正規化する (`biwac_base::normalize_newlines`) ので、
/// どの改行で書かれたソースも LF で書いたものと同じ AST・同じ span になる。
/// 生ノベルテキストの行末の改行も `\n` になる (`\r` が本文に漏れない)。
mod newline {
    use std::path::{Path, PathBuf};

    use biwac_base::{IdentInterner, SourceHolder};

    use crate::{BiwacSourceParser, Pkg};

    /// 改行が字句解析・ノベル DSL の行分けに効く構文をひととおり含むソース (LF)。
    const SRC: &str = r#"// 行コメント
import std::game::Game;

struct State {}

type MyGame = Game[State];

fn add(x: Int, y: Int) -> Int {
  let s = "文字列"; // 行末のコメント
  return x + y;
}

[[native(arch="wasm")]]
fn native_add(x: Int, y: Int) -> Int {{
  local.get 0 ;; WAT のコメント
  local.get 1
  i32.add
}}

scene opening(g: MyGame) -> MyGame {{
  Hello, World!

  // ノベル DSL の行コメント
  こんにちは $(add(1, 2)) さん。>>
  地の文の "引用符" は文字列を開かない。 // コメント
  #let x = add(1,
    2)
  #if x > 2 {
    Nice!
  }
  #endscene g
}}
"#;

    /// 一時ディレクトリにパッケージを作り、`lib.biwa` に `src` を書く。
    struct TempPkg(PathBuf);

    impl TempPkg {
        fn new(label: &str, src: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "biwac_package_loader-newline-{}-{label}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("src")).unwrap();
            std::fs::write(
                root.join(biwac_base::METADATA_FILE_NAME),
                r#"{ "name": "newline", "version": "0.1.0", "dependencies": [] }"#,
            )
            .unwrap();
            std::fs::write(root.join("src/lib.biwa"), src).unwrap();
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempPkg {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 読み込んで、(保持されたソース, AST の Debug 表現) を返す。
    ///
    /// モジュールは `lib.biwa` 1 つだけなので、子モジュールの `HashMap` の
    /// 並び順で Debug 表現が揺れることはない。
    fn load(label: &str, src: &str) -> (String, String) {
        let pkg_dir = TempPkg::new(label, src);

        let mut srcs = SourceHolder::default();
        let mut interner = IdentInterner::default();
        let metadata =
            biwac_metadata_loader::try_load_package_metadata(pkg_dir.path().to_path_buf()).unwrap();
        let pkg = Pkg::try_load::<BiwacSourceParser>(
            &metadata,
            &mut interner,
            &mut srcs,
            pkg_dir.path().to_path_buf(),
        )
        .unwrap_or_else(|e| panic!("{label}: load failed: {:?}", e.errs));

        assert!(
            pkg.root_module.children.is_empty(),
            "the fixture must be a single module"
        );
        let held = srcs.mods.into_values().next().unwrap().src;
        (held, format!("{:?}", pkg.root_module))
    }

    fn check_same_as_lf(label: &str, src: &str) {
        let (lf_src, lf_ast) = load(&format!("{label}-lf"), SRC);
        let (src, ast) = load(label, src);

        assert!(
            !src.contains('\r'),
            "{label}: `\\r` must not survive loading"
        );
        assert_eq!(lf_src, src, "{label}: held source differs");
        assert_eq!(lf_ast, ast, "{label}: AST (including spans) differs");
    }

    #[test]
    fn fixture_has_novel_text_ending_with_lf() {
        // 比較の前提: LF 版で生ノベルテキストの行末が `\n` として AST に入っていること。
        let (_, ast) = load("novel-text", SRC);
        assert!(ast.contains(r#"Hello, World!\n"#), "{ast}");
        assert!(!ast.contains(r#"\r"#), "{ast}");
    }

    #[test]
    fn crlf_source_loads_like_lf() {
        check_same_as_lf("crlf", &SRC.replace('\n', "\r\n"));
    }

    #[test]
    fn cr_only_source_loads_like_lf() {
        check_same_as_lf("cr", &SRC.replace('\n', "\r"));
    }

    #[test]
    fn mixed_line_endings_load_like_lf() {
        // 行ごとに CRLF / LF / CR を順に混ぜる。
        //
        // CR の行の次に「空行 + LF」が来ると `\r\n` という 1 つの改行になり、
        // 行が 1 つ消えてしまう (正規化としてはそれが正しい)。
        // CR の次は必ず CRLF の行になるよう、この順に並べてある。
        let endings = ["\r\n", "\n", "\r"];
        let mixed: String = SRC
            .split_inclusive('\n')
            .enumerate()
            .map(|(i, line)| {
                let body = line.strip_suffix('\n').unwrap_or(line);
                let ending = if line.ends_with('\n') {
                    endings[i % endings.len()]
                } else {
                    ""
                };
                format!("{body}{ending}")
            })
            .collect();
        check_same_as_lf("mixed", &mixed);
    }
}
