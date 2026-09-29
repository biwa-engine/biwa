use biwac_ast::Globals;
use biwac_base::IdentInterner;
use biwac_host_export::HostExportTable;
use biwac_package_loader::Pkg;

use crate::ResolveError;

// `[[host_export="..."]]` の回収。
//
// lang item の回収 (`lang_item_collector.rs`) と同じ構造である。
// DefCollector とは独立したパスであり、DefId は AST ノードの OnceCell に
// 格納済みなので、def collection の後に AST を再走査すれば
// 属性と DefId を同じノードから読める。
//
// 対象は自パッケージのトップレベル `fn` のみ (`biwac_attribute` の
// `Target::Fn` と対応する)。依存パッケージ由来の host_export を
// 取り込む仕組みはまだ無い (`biwac_host_export` のコメント参照)。
pub(crate) fn collect_host_exports(
    pkg: &Pkg,
    interner: &IdentInterner,
) -> Result<HostExportTable, Vec<ResolveError>> {
    let mut table = HostExportTable::new();
    let mut errors = Vec::new();

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
