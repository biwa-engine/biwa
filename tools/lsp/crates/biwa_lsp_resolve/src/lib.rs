//! `biwac_name_resolver` を編集中のバッファに対して走らせるための土台。
//!
//! ディスク上のパッケージ全体を [`biwac_package_loader::Pkg`] でロードし、
//! 開いているファイルに対応するモジュールだけをエディタ上の生きた内容
//! (`biwa_lsp_lower` で lower した [`biwac_ast::ModAst`]) に差し替えてから、
//! `biwac_name_resolver::NameResolver` に通す。
//!
//! 名前解決はパッケージ単位でしか行えない (`biwac_name_resolver` のコメント参照)
//! ため、この関数はディスク上の他ファイルもまとめて読み直す。
//! エディタで開いているが保存していない他ファイルの変更は反映されない。

mod classify;
mod diagnostics;
mod package;

pub use classify::{Classification, ResolvedKind};
pub use diagnostics::Diagnostic;

use std::path::Path;

pub struct DocumentResolution {
    pub diagnostics: Vec<Diagnostic>,
    pub classifications: Vec<Classification>,
}

/// `doc_path` にあるファイルを名前解決する。
///
/// # 既知の制約
/// - `doc_path` の祖先ディレクトリに `biwa-package.json` が見つからなければ
///   `Err` になる (パッケージの外で単体のファイルとして開いている場合)。
/// - 依存パッケージは `<pkg_root>/.biwa_build/deps/<name>/.biwa_build/typescript/`
///   に事前にビルド済みでなければならない。
/// - 直接依存だけを見る (推移的依存の閉包は辿らない)。
/// - target は `TypeScript` に固定。
pub fn resolve_document(doc_path: &Path, doc_src: &str) -> Result<DocumentResolution, String> {
    let pkg_root = package::find_package_root(doc_path).ok_or_else(|| {
        format!(
            "no {} found above {}",
            biwac_base::METADATA_FILE_NAME,
            doc_path.display()
        )
    })?;

    let mut interner = biwac_base::IdentInterner::default();
    let mut srcs = biwac_base::SourceHolder::default();

    let metadata = biwac_metadata_loader::try_load_package_metadata(pkg_root.clone())
        .map_err(|e| format!("failed to load {}: {e:?}", biwac_base::METADATA_FILE_NAME))?;

    let external_packages = package::load_external_packages(&pkg_root, &metadata, &mut interner)?;

    let mut pkg = biwac_package_loader::Pkg::try_load(
        &metadata,
        &mut interner,
        &mut srcs,
        pkg_root.clone(),
    )
    .map_err(|e| format!("failed to load package sources: {:?}", e.errs))?;

    let target_modpath = package::doc_modpath(&pkg_root, doc_path).ok_or_else(|| {
        format!(
            "{} is not under {}/src",
            doc_path.display(),
            pkg_root.display()
        )
    })?;

    let doc_mod_id =
        package::substitute_module(&mut pkg, &target_modpath, &mut interner, doc_src)
            .ok_or_else(|| format!("module for {} not found in package tree", doc_path.display()))?;

    // 選択されていない arch の native を落としてから解決する
    // (同名の arch 違い native がシンボル衝突するため)。
    pkg.walk_modules_mut(|module| {
        biwac_attribute::retain_for_target(&mut module.ast, package::TARGET, &interner);
    });

    let pkg_name = interner.get_or_insert(metadata.metadata.name.value());

    let resolver =
        biwac_name_resolver::NameResolver::new(&metadata, external_packages, pkg_name, &mut pkg)
            .map_err(|e| format!("{e:?}"))?;

    // `try_resolve` は `&mut Pkg` を受け取るだけで所有権は返さない。
    // 失敗しても `pkg` の `OnceCell` はそこまで解決できた分だけ埋まった状態で
    // 手元に残るので、成功・失敗にかかわらず同じ `pkg` から分類できる
    // (診断だけがエラーの有無で変わる)。
    let diagnostics = match resolver.try_resolve(&mut interner) {
        Ok(_output) => Vec::new(),
        Err(errors) => diagnostics::extract(&errors, doc_mod_id, &interner),
    };

    let classifications = classify::classify(&pkg, doc_mod_id);

    Ok(DocumentResolution {
        diagnostics,
        classifications,
    })
}
