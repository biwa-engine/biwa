//! `biwac_name_resolver` / `biwac_type_inferrer` を編集中のバッファに対して
//! 走らせるための土台。
//!
//! ディスク上のパッケージ全体を [`biwac_package_loader::Pkg`] でロードし、
//! 開いているファイルに対応するモジュールだけをエディタ上の生きた内容
//! (`biwa_lsp_lower` で lower した [`biwac_ast::ModAst`]) に差し替えてから、
//! `biwac_name_resolver::NameResolver` に通す。名前解決が成功したら、
//! その `Hir` をそのまま `biwac_type_inferrer::TyCtx` に渡して型推論も行う
//! (依存パッケージのメタデータは名前解決のために既に読み込み済みなので、
//! ほぼ結線するだけでよい — `biwac_driver` の
//! `load_analyze_and_codegen_single_package` と同じ配線)。
//!
//! ロードには [`package::LspSourceParser`] (biwa-lsp 自身の lossless /
//! エラー耐性パーサ) を `biwac_package_loader::SourceParser` として差し込む。
//! 実コンパイラの `biwac_lexer`/`biwac_parser` は構文エラーで即失敗するため、
//! それをそのまま使うとパッケージ内のどこか 1 箇所の構文エラーだけで
//! 名前解決そのものが始まらなくなる。`LspSourceParser` は常に salvage された
//! `ModAst` を返すので、開いていないファイルに構文エラーがあっても
//! パッケージ全体のロード・名前解決は続けられる。
//!
//! 名前解決はパッケージ単位でしか行えない (`biwac_name_resolver` のコメント参照)
//! ため、この関数はディスク上の他ファイルもまとめて読み直す。
//! エディタで開いているが保存していない他ファイルの変更は反映されない。

mod classify;
mod diagnostics;
mod package;
mod type_diagnostics;

pub use classify::{Classification, ResolvedKind};
pub use diagnostics::Diagnostic;

use std::path::Path;
use std::sync::Arc;

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
/// - target は `Wasm` に固定 (`package::TARGET` 参照)。
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

    // `LspSourceParser` (biwa-lsp 自身の lossless / エラー耐性パーサ) を使う。
    // 実コンパイラの `biwac_lexer`/`biwac_parser` (既定の `BiwacSourceParser`)
    // だと構文エラーのあるファイルが 1 つでもあるとパッケージ全体のロードが
    // 失敗し、名前解決そのものが始まらなくなってしまう。
    // モジュール木の形の誤り (宣言されていないファイルなど) では止めず、
    // 読めた分で解析を続ける (`try_load_tolerant`)。誤りは開いているファイルの
    // 診断として出す (`loader_diagnostics`)。
    let (mut pkg, load_errors) = biwac_package_loader::Pkg::try_load_tolerant::<
        package::LspSourceParser,
    >(&metadata, &mut interner, &mut srcs, pkg_root.clone())
    .map_err(|e| format!("failed to load package sources: {:?}", e.errs))?;

    let target_modpath = package::doc_modpath(&pkg_root, doc_path).ok_or_else(|| {
        format!(
            "{} is not under {}/src",
            doc_path.display(),
            pkg_root.display()
        )
    })?;

    let Some(doc_mod_id) =
        package::substitute_module(&mut pkg, &target_modpath, &mut interner, doc_src)
    else {
        // 開いているファイルがどこからも `mod` 宣言されていなければ、
        // モジュール木に無いので解析できない。そのことだけを診断として出す。
        return match package::undeclared_message(&pkg_root, doc_path, &load_errors) {
            Some(message) => Ok(DocumentResolution {
                diagnostics: vec![Diagnostic {
                    start: 0,
                    end: 0,
                    message,
                }],
                classifications: Vec::new(),
            }),
            None => Err(format!(
                "module for {} not found in package tree",
                doc_path.display()
            )),
        };
    };
    let loader_diagnostics = package::loader_diagnostics(&load_errors, doc_mod_id);

    // 選択されていない arch の native を落としてから解決する
    // (同名の arch 違い native がシンボル衝突するため)。
    pkg.walk_modules_mut(|module| {
        biwac_attribute::retain_for_target(&mut module.ast, package::TARGET, &interner);
    });

    let pkg_name = interner.get_or_insert(metadata.metadata.name.value());

    // 型推論 (`biwac_type_inferrer::TyCtx`) が要る依存パッケージのメタデータ。
    // `external_packages` は次で `NameResolver::new` に消費されるので、
    // ここで先に (`biwac_driver` の `load_analyze_and_codegen_single_package`
    // と同じ形で) 控えておく。名前で引けるかを問わない型推論では
    // `direct` の区別を落として推移閉包すべてを渡す。
    let ext_pkgs_for_ty: Vec<(
        biwac_base::PackageId,
        Arc<biwac_dependency_metadata::DepMetadata>,
    )> = external_packages
        .iter()
        .map(|p| (p.pkg_id, Arc::clone(&p.meta)))
        .collect();

    let resolver =
        biwac_name_resolver::NameResolver::new(&metadata, external_packages, pkg_name, &mut pkg)
            .map_err(|e| format!("{e:?}"))?;

    // `try_resolve` は `&mut Pkg` を受け取るだけで所有権は返さない。
    // 失敗しても `pkg` の `OnceCell` はそこまで解決できた分だけ埋まった状態で
    // 手元に残るので、成功・失敗にかかわらず同じ `pkg` から分類できる
    // (診断だけがエラーの有無で変わる)。
    //
    // メソッド呼び出しの分類 (`.foo()` が実際どのメソッドか) だけは
    // `biwac_ast`/`Pkg` 側に対応する情報が無く、型推論が埋める
    // `biwac_hir::Call::target` からしか分からないので、
    // 型推論に成功したときだけ別枠で集めて `classifications` に混ぜる。
    let (diagnostics, method_classifications) = match resolver.try_resolve(&mut interner) {
        Ok(biwac_name_resolver::ResolveOutput {
            hir, lang_items, ..
        }) => {
            // 名前解決が成功したら、その Hir をそのまま型推論に渡す。
            // 依存パッケージはすでに名前解決のために読み込み済みなので、
            // ここは実質つなぐだけでよい (`biwac_driver` と同じ結線)。
            match biwac_type_inferrer::TyCtx::new(hir, lang_items, ext_pkgs_for_ty, &mut interner)
                .infer()
            {
                Ok(hir) => {
                    let methods = classify::classify_resolved_methods(&hir, doc_mod_id);
                    (Vec::new(), methods)
                }
                Err(e) => (
                    type_diagnostics::extract(&e, doc_mod_id, &interner),
                    Vec::new(),
                ),
            }
        }
        Err(errors) => (
            diagnostics::extract(&errors, doc_mod_id, &interner),
            Vec::new(),
        ),
    };

    let mut diagnostics = diagnostics;
    diagnostics.extend(loader_diagnostics);

    let mut classifications = classify::classify(&pkg, doc_mod_id);
    classifications.extend(method_classifications);
    classifications.sort_by_key(|c| c.start);

    Ok(DocumentResolution {
        diagnostics,
        classifications,
    })
}
