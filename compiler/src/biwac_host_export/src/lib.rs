use std::collections::HashMap;

use biwac_span::{Span, ValDefId};

// host export は「生成物から、指定した名前でホスト (エンジン) が直接呼べる
// 関数」の登録簿である。
//
// `[[lang="..."]]` が「コンパイラ自身が呼ぶ」ためのものであるのに対し、
// `[[host_export="..."]]` は「ホスト側が呼ぶ」ためのもの。
// `__biwa_entrypoint` (`fn main()`、biwac_scene の WellKnownSymbol)
// が固定名・固定シグネチャの特別扱いなのに対し、こちらは属性で好きな関数を
// 好きな名前で export できる汎用の仕組みである。
//
// 対象は現状トップレベルの `fn` のみ。
//
// 依存パッケージ (例えば std) が host_export した関数も、依存元のビルドで
// export される。`.biwameta` のシンボルヘッダに host export のフラグが、
// fn のボディに export 名が載っており、依存元は名前解決の段階で
// 推移閉包すべての依存からそれを読み戻してこの表に加える
// (lang item と同じ経路)。表の `ValDefId` が自パッケージのものか
// 依存のものかは `def_id.pkg().is_self()` で区別できる。

#[derive(Debug, Clone, Default)]
pub struct HostExportTable {
    by_def: HashMap<ValDefId, String>,
}

#[derive(Debug)]
pub enum HostExportError {
    /// 同じ export 名が 2 つの定義に使われた。
    ///
    /// 同一の定義に `[[host_export="..."]]` が 2 回付いた場合は
    /// biwac_attribute の検証パス (`AttrError::DuplicatedAttribute`) が
    /// 別に検出するので、ここに来るのは必ず異なる定義同士である。
    ///
    /// 依存パッケージ由来の登録は位置を持たないので、`span` はダミーになりうる。
    DuplicatedName {
        name: String,
        previous: ValDefId,
        span: Span,
    },
    /// 依存パッケージの `.biwameta` から host export を読み戻せなかった。
    BrokenDependencyMetadata { package: String, reason: String },
}

impl HostExportError {
    /// ソース上の位置。依存パッケージ由来で位置を持たない場合は `None`。
    pub fn span(&self) -> Option<&Span> {
        match self {
            Self::DuplicatedName { span, .. } if !span.is_dummy() => Some(span),
            Self::DuplicatedName { .. } | Self::BrokenDependencyMetadata { .. } => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::DuplicatedName { name, .. } => {
                format!("host export name \"{name}\" is used by more than one function")
            }
            Self::BrokenDependencyMetadata { package, reason } => {
                format!("failed to read host exports of the dependency `{package}`: {reason}")
            }
        }
    }

    /// 補足。重複の相手が依存パッケージにあるときにそれを伝える。
    pub fn note(&self) -> Option<String> {
        match self {
            Self::DuplicatedName { previous, .. } if !previous.pkg().is_self() => Some(
                "the other function is defined in a dependency package; \
                 host export names must be unique across the whole program"
                    .to_string(),
            ),
            _ => None,
        }
    }
}

impl HostExportTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, def_id: ValDefId) -> Option<&str> {
        self.by_def.get(&def_id).map(String::as_str)
    }

    /// export 名の順に返す。
    ///
    /// 単相化の roots や生成物の export の並びがビルドごとに揺れないよう、
    /// `HashMap` の順序をそのまま見せない。
    pub fn iter(&self) -> impl Iterator<Item = (ValDefId, &str)> + '_ {
        let mut entries: Vec<(ValDefId, &str)> = self
            .by_def
            .iter()
            .map(|(def_id, name)| (*def_id, name.as_str()))
            .collect();
        entries.sort_by(|a, b| a.1.cmp(b.1));
        entries.into_iter()
    }

    pub fn is_empty(&self) -> bool {
        self.by_def.is_empty()
    }

    /// export 名の重複を検証しつつ登録する。
    pub fn insert(
        &mut self,
        def_id: ValDefId,
        name: String,
        span: Span,
    ) -> Result<(), HostExportError> {
        let previous = self
            .by_def
            .iter()
            .find(|(_, existing)| **existing == name)
            .map(|(def_id, _)| *def_id);

        if let Some(previous) = previous {
            return Err(HostExportError::DuplicatedName {
                name,
                previous,
                span,
            });
        }

        self.by_def.insert(def_id, name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biwac_span::{DefId, PackageLocalDefId};

    fn def(n: u32) -> ValDefId {
        ValDefId::new(DefId::new_in_self_pkg(PackageLocalDefId::new(n)))
    }

    #[test]
    fn distinct_names_are_ok() {
        let mut table = HostExportTable::new();
        table
            .insert(def(0), "foo".to_string(), Span::dummy())
            .unwrap();
        table
            .insert(def(1), "bar".to_string(), Span::dummy())
            .unwrap();

        assert_eq!(table.get(def(0)), Some("foo"));
        assert_eq!(table.get(def(1)), Some("bar"));
    }

    #[test]
    fn duplicated_name_is_an_error() {
        let mut table = HostExportTable::new();
        table
            .insert(def(0), "foo".to_string(), Span::dummy())
            .unwrap();

        let err = table
            .insert(def(1), "foo".to_string(), Span::dummy())
            .unwrap_err();

        match err {
            HostExportError::DuplicatedName { name, previous, .. } => {
                assert_eq!(name, "foo");
                assert_eq!(previous, def(0));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn iter_is_ordered_by_export_name() {
        let mut table = HostExportTable::new();
        table
            .insert(def(0), "zeta".to_string(), Span::dummy())
            .unwrap();
        table
            .insert(def(1), "alpha".to_string(), Span::dummy())
            .unwrap();
        table
            .insert(def(2), "mid".to_string(), Span::dummy())
            .unwrap();

        let names: Vec<&str> = table.iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn same_def_can_be_reinserted_under_a_different_name_without_colliding_with_itself() {
        // `by_def` はキーが def_id なので、同じ def_id への再登録は
        // 単純な上書きになる (同一定義への属性重複は biwac_attribute が防ぐので、
        // ここに来ること自体は想定していないが、少なくとも自分自身との
        // 重複として誤検出しないことは確認しておく)。
        let mut table = HostExportTable::new();
        table
            .insert(def(0), "foo".to_string(), Span::dummy())
            .unwrap();
        table
            .insert(def(0), "bar".to_string(), Span::dummy())
            .unwrap();
        assert_eq!(table.get(def(0)), Some("bar"));
    }
}
