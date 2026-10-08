use std::collections::HashMap;

use biwac_span::ValDefId;

/// そのシンボルが必須かどうか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneRequirement {
    /// playable package では必ず定義されていなければならない。
    RequiredInPlayable,

    /// 定義されていれば使われるが、無くてもよい。
    #[allow(dead_code)]
    Optional,
}

/// ランタイムが名前を知っているシンボルの種別。
///
/// 引数と戻り値の型はシンボルごとに表 ([`WellKnownSymbol::args`] /
/// [`WellKnownSymbol::ret`]) が決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellKnownKind {
    /// `scene`。generator として出力される。
    Scene,
    /// 普通の関数。
    Fn,
}

/// ランタイムとの規約に現れる型。`Void` (戻り値なし) 以外は lang item である。
///
/// ジェネリック引数に何が入るかは問わない (`Game[..]` の中身は開発者が決める)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractTy {
    /// lang item `game`。
    Game,
    /// 戻り値なし。
    Void,
}

impl ContractTy {
    /// 対応する lang item。`Void` には無い。
    pub fn lang_item(&self) -> Option<biwac_lang_item::LangItem> {
        match self {
            Self::Game => Some(biwac_lang_item::LangItem::Game),
            Self::Void => None,
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Self::Game => "`Game`",
            Self::Void => "nothing",
        }
    }
}

/// すべての scene が守るシグニチャ `(Game[..]) -> Game[..]` の引数。
///
/// 既知シンボルでない scene もこれに従う。
pub const SCENE_ARGS: &[ContractTy] = &[ContractTy::Game];
/// すべての scene が守るシグニチャの戻り値。
pub const SCENE_RET: ContractTy = ContractTy::Game;

impl WellKnownKind {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Scene => "a scene",
            Self::Fn => "a function",
        }
    }
}

// ランタイムが名前を知っていて直接呼ぶシンボルの一覧。
//
// biwac_lang_item の lang_item_table! や
// biwac_attribute の attribute_table! と同じ流儀で、
// 「どの名前が特別か」をこの表 1 箇所に集める。
//
// これらはルートモジュール (playable package なら main.biwa) に定義する。
// どのターゲット言語でどんなシンボル名になるかは codegen 側の規約であり、
// ここでは関知しない。
macro_rules! well_known_symbol_table {
    ( $( $variant:ident, $name:literal, $kind:expr, $args:expr, $ret:expr, $requirement:expr ; )* ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum WellKnownSymbol {
            $($variant,)*
        }

        impl WellKnownSymbol {
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

            pub fn kind(&self) -> WellKnownKind {
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

            pub fn requirement(&self) -> SceneRequirement {
                match self {
                    $(Self::$variant => $requirement,)*
                }
            }
        }
    };
}

well_known_symbol_table!(
    // ランタイムが起動時に呼ぶ唯一のエントリポイント。playable package の main.biwa に定義する。
    //
    // 中で `Window[S]` を組み立てて `show()` するのはゲーム側で、UI はすべてゲーム側が決める。
    // scene (`Window` の `main_scene`) も最初の `Game` の作り方 (`SceneStartButton` の `on_click`) も
    // 関数の値として UI に渡すので、ランタイムが名前で呼ぶものはこれだけでよい。
    // 引数も戻り値も持たないのは、`S` (ゲームの状態の型) をホストに見せないためである
    // (ホストは単相化された型を名指しできない)。
    //
    // 表の先頭が単相化のエントリ (wasm の `__biwa_entrypoint`) になる。
    Main, "main", WellKnownKind::Fn, &[], ContractTy::Void,
        SceneRequirement::RequiredInPlayable;

    // 将来ここにイベントハンドラ的なものが増える想定:
    // OnSave, "on_save", WellKnownKind::Fn, SceneRequirement::Optional;
);

/// 検査を通った既知シンボルの解決結果。
///
/// library package では空になる (エントリポイントを持つのは playable package だけ)。
#[derive(Debug, Clone, Default)]
pub struct WellKnownSymbols {
    scenes: HashMap<WellKnownSymbol, ValDefId>,
}

impl WellKnownSymbols {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, scene: WellKnownSymbol) -> Option<ValDefId> {
        self.scenes.get(&scene).copied()
    }

    /// この `ValDefId` が既知シンボルのいずれかであればそれを返す。
    pub fn find(&self, def_id: &ValDefId) -> Option<WellKnownSymbol> {
        self.scenes
            .iter()
            .find(|(_, v)| *v == def_id)
            .map(|(k, _)| *k)
    }

    pub(crate) fn set(&mut self, scene: WellKnownSymbol, def_id: ValDefId) {
        self.scenes.insert(scene, def_id);
    }
}
