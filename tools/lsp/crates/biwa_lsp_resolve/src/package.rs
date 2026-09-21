//! ディスク上のパッケージ (`biwa-package.json` があるディレクトリ) を見つけ、
//! `biwac_package_loader` でロードしたうえで、編集中のバッファ 1 つ分だけ
//! CST 由来の AST に差し替える。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use biwac_base::{IdentInterner, MetadataHolder, ModId, ModPath, Target};
use biwac_dependency_metadata::{DepMetadata, ExternalPackage};
use biwac_package_loader::{LoadedModule, Pkg};

/// `doc_path` の祖先ディレクトリを遡って `biwa-package.json` を探す。
pub(crate) fn find_package_root(doc_path: &Path) -> Option<PathBuf> {
    let mut dir = doc_path.parent()?;
    loop {
        if dir.join(biwac_base::METADATA_FILE_NAME).is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// `doc_path` の、パッケージルート `<pkg_root>/src/` からの相対 [`ModPath`]。
pub(crate) fn doc_modpath(pkg_root: &Path, doc_path: &Path) -> Option<ModPath> {
    let src_dir = pkg_root.join("src");
    let rel = doc_path.strip_prefix(&src_dir).ok()?;
    let rel_str = rel.to_str()?.replace('\\', "/");
    ModPath::from_file_name(&rel_str)
}

/// このモジュールが読み込む対象の target。
///
/// # 既知の制約
/// LSP には `--target` 相当の指定が無いので `TypeScript` に固定している。
/// wasm ターゲット向けの `[[native(arch = "wasm")]]` しか持たない関数は
/// ここでは「native 実装が無い」扱いで落ち、意図しない診断が出ることがある。
pub(crate) const TARGET: Target = Target::TypeScript;

/// 直接依存パッケージのメタデータを読み込む。
///
/// # 既知の制約
/// 直接依存だけを見て、推移的依存の閉包は辿らない (`biwac_driver` はここで
/// 依存グラフを解決するが、その機能は driver 内部に閉じていて外から呼べない)。
/// 直接依存のシグニチャが間接依存の型を参照していると解決に失敗しうる。
pub(crate) fn load_external_packages(
    pkg_root: &Path,
    metadata: &MetadataHolder,
    interner: &mut IdentInterner,
) -> Result<Vec<ExternalPackage>, String> {
    let packages_dir = biwac_base::dependencies_dir(pkg_root);

    metadata
        .metadata
        .dependencies
        .iter()
        .map(|dep| {
            let dep_name = dep.name.value().to_string();
            let dep_root = packages_dir.join(&dep_name);

            let dep_pkg_metadata = biwac_metadata_loader::try_load_package_metadata(
                dep_root.clone(),
            )
            .map_err(|e| format!("failed to load metadata for dependency `{dep_name}`: {e:?}"))?;

            let meta_path = dep_root
                .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
                .join(TARGET.build_subdir())
                .join(format!("{dep_name}.biwameta"));
            let data = std::fs::read(&meta_path).map_err(|e| {
                format!(
                    "dependency `{dep_name}` does not look built yet (missing {}): {e}",
                    meta_path.display()
                )
            })?;
            let dep_meta = DepMetadata::decode_file(&data).map_err(|e| {
                format!("failed to decode metadata for dependency `{dep_name}`: {e:?}")
            })?;

            Ok(ExternalPackage {
                ident: interner.get_or_insert(&dep_name),
                pkg_id: biwac_span::PackageHashId::new(
                    &dep_pkg_metadata.metadata.name,
                    &dep_pkg_metadata.metadata.version,
                )
                .as_package_id(),
                meta: Arc::new(dep_meta),
                direct: true,
            })
        })
        .collect()
}

/// `pkg` の中から `target_modpath` に一致するモジュールを探し、その AST を
/// `doc_src` (エディタ上の生きた内容) を lower した結果に差し替える。
///
/// 見つかったモジュールの [`ModId`] を返す。見つからなければ `None`
/// (ディスク上のツリーに存在しない新規ファイルを開いた場合など)。
pub(crate) fn substitute_module(
    pkg: &mut Pkg,
    target_modpath: &ModPath,
    interner: &mut IdentInterner,
    doc_src: &str,
) -> Option<ModId> {
    let mod_id = find_mod_id(&pkg.root_module, target_modpath)?;

    let parsed = biwa_lsp_parser::parse(doc_src);
    let (ast, _lower_errors) =
        biwa_lsp_lower::lower_module(mod_id, target_modpath.clone(), interner, &parsed.syntax());

    replace_ast(&mut pkg.root_module, mod_id, ast);

    Some(mod_id)
}

fn find_mod_id(module: &LoadedModule, target: &ModPath) -> Option<ModId> {
    if &module.ast.modpath == target {
        return Some(module.mod_id);
    }
    module.children.values().find_map(|c| find_mod_id(c, target))
}

/// `ast` を消費して該当モジュールへ挿し込む。木を下る途中は `Some` を積み戻し続け、
/// 挿し込めたら `None` を返して上に伝える (再帰の各段で消費済みかを判定するため)。
fn replace_ast(
    module: &mut LoadedModule,
    mod_id: ModId,
    ast: biwac_ast::ModAst,
) -> Option<biwac_ast::ModAst> {
    if module.mod_id == mod_id {
        module.ast = ast;
        return None;
    }

    let mut ast = Some(ast);
    for child in module.children.values_mut() {
        if let Some(a) = ast.take() {
            ast = replace_ast(child, mod_id, a);
        } else {
            break;
        }
    }
    ast
}
