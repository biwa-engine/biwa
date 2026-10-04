use biwac_ast::Globals;
use biwac_base::IdentInterner;
use biwac_dependency_metadata::ExternalPackage;
use biwac_host_export::{HostExportError, HostExportTable};
use biwac_package_loader::Pkg;
use biwac_span::Span;

use crate::ResolveError;

// `[[host_export="..."]]` の回収。
//
// lang item の回収 (`lang_item_collector.rs`) と同じ構造である。
// DefCollector とは独立したパスであり、DefId は AST ノードの OnceCell に
// 格納済みなので、def collection の後に AST を再走査すれば
// 属性と DefId を同じノードから読める。
//
// 対象はトップレベル `fn` のみ (`biwac_attribute` の `Target::Fn` と対応する)。
//
// 依存パッケージが host export した関数も、依存 → 自パッケージの順に取り込む。
// std が export した関数を、std を使う playable package のビルドで
// 生成物から export するためである。取り込んだものは単相化の roots にも入るので、
// 自パッケージから呼ばれていなくても到達性で刈り取られない。
pub(crate) fn collect_host_exports(
    pkg: &Pkg,
    external_packages: &[ExternalPackage],
    interner: &IdentInterner,
) -> Result<HostExportTable, Vec<ResolveError>> {
    let mut table = HostExportTable::new();
    let mut errors = Vec::new();

    // 直接依存に限らず推移閉包すべてを見る。
    // 各パッケージの .biwameta には自分が定義した host export しか載らないので
    // (DepMetadata::new は自パッケージのシンボルしか書かない)、重複登録にはならない。
    // export 名の重複はパッケージをまたいでも許さない
    // (生成物の export は 1 つの名前空間を共有するため)。
    for p in external_packages {
        let exports = match p.meta.host_exports(p.pkg_id) {
            Ok(exports) => exports,
            Err(e) => {
                errors.push(ResolveError::HostExport(
                    HostExportError::BrokenDependencyMetadata {
                        package: interner.get_str(&p.ident).unwrap_or("?").to_string(),
                        reason: e.to_string(),
                    },
                ));
                continue;
            }
        };
        for (def_id, name) in exports {
            // 依存側は自身のビルド時に検証済みであり、ここには AST が無いので位置は持たない。
            if let Err(e) = table.insert(def_id, name.to_string(), Span::dummy()) {
                errors.push(ResolveError::HostExport(e));
            }
        }
    }

    pkg.walk_modules(|module| {
        for g in &module.ast.globals {
            let Globals::FnDef(f) = g else { continue };

            let Some((name, span)) = biwac_attribute::host_export_name(&f.attrs, interner) else {
                continue;
            };

            // DefId が未設定なのは def collection が失敗した場合のみで、
            // その場合は既に別のエラーが報告されている。
            let Some(def_id) = f.def_id.get().copied() else {
                continue;
            };

            if let Err(e) = table.insert(def_id, name.to_string(), span) {
                errors.push(ResolveError::HostExport(e));
            }
        }
    });

    if errors.is_empty() {
        Ok(table)
    } else {
        Err(errors)
    }
}
