//! `tests/fixtures/minipkg` は natives/attributes を使わない最小限パッケージ。
//!
//! # なぜここで「識別子が解決される」ところまで検証しないか
//!
//! `biwac_name_resolver::NameResolver::try_resolve` は `Game`/`Character`/
//! `String` などの lang item がどこかに (自パッケージか依存先に) 定義されて
//! いることを、コードがそれらを実際に使っているかにかかわらず要求する
//! (`biwac_lang_item` 参照)。`std` はそれらを自分で定義しているので依存無しで
//! 成立するが、その定義は `[[lang = "..."]]` 属性つきの native 型エイリアスで
//! 書かれており、`biwa_lsp_lower` はまだ属性・native 実装を lowering できない
//! (既知の非対応)。そのため「依存無しで完結する biwa パッケージ」を
//! 名前解決までフルに通すテスト用フィクスチャは今のところ作れない
//! (実際の利用では `std` は `.biwameta` 済みの依存として読み込まれるので
//! この制約に当たらない — `/home/coder/test1` で手動確認済み)。
//!
//! ここでは `biwac_package_loader::Pkg::try_load` に `LspSourceParser` を
//! 差し込んだときの「他ファイルの構文エラーでパッケージ全体のロードが
//! 落ちない」という直接の効果だけを、`src/package.rs` 側のユニットテスト
//! (`try_load_with_lsp_source_parser_survives_a_syntax_error_in_another_module`)
//! で検証している。ここでは `resolve_document` を通しで呼んでも
//! (中身が lang item エラーで終わるとしても) トップレベルの `Err` には
//! ならないことだけを確認する。

use std::path::Path;

fn fixture_path(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/minipkg")
        .join(rel)
}

#[test]
fn resolve_document_does_not_hard_fail_when_another_module_has_a_syntax_error() {
    let path = fixture_path("src/lib.biwa");
    let src = std::fs::read_to_string(&path).unwrap();

    // 実コンパイラの `biwac_lexer`/`biwac_parser` (`BiwacSourceParser`) を
    // そのまま使っていた頃は、`src/broken.biwa` の構文エラーだけで
    // `Pkg::try_load` が `Err` になり、`resolve_document` も
    // "failed to load package sources" で即失敗していた。
    if let Err(e) = biwa_lsp_resolve::resolve_document(&path, &src) {
        panic!("expected resolve_document to survive a syntax error in another module, got: {e}");
    }
}
