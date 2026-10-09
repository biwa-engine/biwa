use std::collections::HashMap;

use biwac_span::ValDefId;

/// そのエントリポイントが必須かどうか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointRequirement {
    /// playable package では必ず定義されていなければならない。
    RequiredInPlayable,

    /// 定義されていれば使われるが、無くてもよい。
    #[allow(dead_code)]
    Optional,
}

/// エントリポイントの種別 (関数か scene か)。
///
/// 引数と戻り値の型はエントリポイントごとに表 ([`Entrypoint::args`] /
/// [`Entrypoint::ret`]) が決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointKind {
    /// `scene`。generator として出力される。
    Scene,
    /// 普通の関数。
    Fn,
}

/// エントリポイントのシグネチャに現れる型。
///
/// 今のエントリポイント (`fn main()`) は引数も戻り値も持たないので `Void` しか無い。
/// lang item の型 (`Game[..]` など) を取るエントリポイントを足すときは、ここに足して
/// lang item を引いて照合する (以前は `Game` と `GameWindow` があった)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractTy {
    /// 戻り値なし。
    Void,
}

impl ContractTy {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Void => "nothing",
        }
    }
}

impl EntrypointKind {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Scene => "a scene",
            Self::Fn => "a function",
        }
    }
}

// エントリポイント (ランタイムが名前を知っていて直接呼ぶもの) の一覧。
//
// biwac_lang_item の lang_item_table! や
// biwac_attribute の attribute_table! と同じ流儀で、
// 「どの名前が特別か」をこの表 1 箇所に集める。
//
// これらはルートモジュール (playable package なら main.biwa) に定義する。
// どのターゲット言語でどんなシンボル名になるかは codegen 側の規約であり、
// ここでは関知しない。
macro_rules! entrypoint_table {
    ( $( $variant:ident, $name:literal, $kind:expr, $args:expr, $ret:expr, $requirement:expr ; )* ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Entrypoint {
            $($variant,)*
        }

        impl Entrypoint {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            pub fn name(&self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)*
                    _ => None,
                }
            }

            pub fn kind(&self) -> EntrypointKind {
                match self {
                    $(Self::$variant => $kind,)*
                }
            }

            /// 期待する引数の型 (順序どおり)。
            pub fn args(&self) -> &'static [ContractTy] {
                match self {
                    $(Self::$variant => $args,)*
                }
            }

            /// 期待する戻り値の型。
            pub fn ret(&self) -> ContractTy {
                match self {
                    $(Self::$variant => $ret,)*
                }
            }

            pub fn requirement(&self) -> EntrypointRequirement {
                match self {
                    $(Self::$variant => $requirement,)*
                }
            }
        }
    };
}

entrypoint_table!(
    // ランタイムが起動時に呼ぶ唯一のエントリポイント。playable package の main.biwa に定義する。
    //
    // 中で `Window[S]` を組み立てて `show()` するのはゲーム側で、UI はすべてゲーム側が決める。
    // scene (`Window` の `main_scene`) も最初の `Game` の作り方 (`SceneStartButton` の `on_click`) も
    // 関数の値として UI に渡すので、ランタイムが名前で呼ぶものはこれだけでよい。
    // 引数も戻り値も持たないのは、`S` (ゲームの状態の型) をホストに見せないためである
    // (ホストは単相化された型を名指しできない)。
    //
    // 表の先頭が単相化のエントリ (wasm の `__biwa_entrypoint`) になる。
    Main, "main", EntrypointKind::Fn, &[], ContractTy::Void,
        EntrypointRequirement::RequiredInPlayable;

    // 将来ここにイベントハンドラ的なものが増える想定:
    // OnSave, "on_save", EntrypointKind::Fn, EntrypointRequirement::Optional;
);

/// 検査を通ったエントリポイントの解決結果。
///
/// library package では空になる (エントリポイントを持つのは playable package だけ)。
#[derive(Debug, Clone, Default)]
pub struct Entrypoints {
    entries: HashMap<Entrypoint, ValDefId>,
}

impl Entrypoints {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, entrypoint: Entrypoint) -> Option<ValDefId> {
        self.entries.get(&entrypoint).copied()
    }

    /// この `ValDefId` がエントリポイントのいずれかであればそれを返す。
    pub fn find(&self, def_id: &ValDefId) -> Option<Entrypoint> {
        self.entries
            .iter()
            .find(|(_, v)| *v == def_id)
            .map(|(k, _)| *k)
    }

    pub(crate) fn set(&mut self, entrypoint: Entrypoint, def_id: ValDefId) {
        self.entries.insert(entrypoint, def_id);
    }
}
