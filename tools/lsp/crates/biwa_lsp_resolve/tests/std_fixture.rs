//! `compiler/assets/tests/std` は `biwac_name_resolver` 自身のテストが使っている
//! 依存無しの固定フィクスチャ (`no_std`, `dependencies: []`)。外部依存のビルドを
//! 要らないので、`biwa_lsp_resolve` の統合テストにそのまま使える。

use std::path::Path;

#[test]
fn resolves_the_compiler_test_fixture_std_package_cleanly() {
    let path = Path::new("../../../../compiler/assets/tests/std/src/lib.biwa");
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));

    let result = biwa_lsp_resolve::resolve_document(path, &src)
        .unwrap_or_else(|e| panic!("resolve_document failed: {e}"));

    let messages: Vec<&str> = result.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(messages.is_empty(), "unexpected diagnostics: {messages:?}");
}

#[test]
fn flags_an_unresolved_identifier_injected_into_the_live_buffer() {
    let path = Path::new("../../../../compiler/assets/tests/std/src/lib.biwa");
    let disk_src = std::fs::read_to_string(path).unwrap();
    let live_src = format!("{disk_src}\nfn __lsp_test_only() {{ __totally_undefined_name_xyz; }}\n");

    let result = biwa_lsp_resolve::resolve_document(path, &live_src)
        .unwrap_or_else(|e| panic!("resolve_document failed: {e}"));

    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("__totally_undefined_name_xyz")),
        "expected an unresolved-identifier diagnostic, got: {:?}",
        result.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
