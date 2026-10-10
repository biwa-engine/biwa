//! `mod` 宣言されていないファイルがあっても解析が止まらないこと
//! (`biwac_package_loader::Pkg::try_load_tolerant`)。
//!
//! 実コンパイラは宣言されていないファイルをエラーにする。エディタでは、新しいファイルを
//! 作ってから宣言を書くまでの間にパッケージ全体の解析が止まると困るので、
//! 読めた分で続け、宣言されていないファイル自身にだけそのことを診断として出す。

use std::path::Path;

fn fixture_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/undeclared")
        .join(rel)
}

#[test]
fn other_files_are_still_analyzed() {
    let path = fixture_path("src/lib.biwa");
    let src = std::fs::read_to_string(&path).unwrap();
    if let Err(e) = biwa_lsp_resolve::resolve_document(&path, &src) {
        panic!("expected resolve_document to survive an undeclared file, got: {e}");
    }
}

#[test]
fn undeclared_file_gets_a_diagnostic() {
    let path = fixture_path("src/stray.biwa");
    let src = std::fs::read_to_string(&path).unwrap();
    let resolution = biwa_lsp_resolve::resolve_document(&path, &src)
        .unwrap_or_else(|e| panic!("expected a diagnostic, got Err: {e}"));
    assert!(
        resolution
            .diagnostics
            .iter()
            .any(|d| d.message.contains("not declared as a module")
                && d.message.contains("src/lib.biwa")),
        "{:?}",
        resolution
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}
