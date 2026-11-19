//! コード生成のターゲット。
//!
//! この表がターゲットに関する語彙の唯一の情報源である。
//! `[[native(arch = "...")]]` の arch 名も、成果物の置き場所も、ここから引く。
//!
//! いまのターゲットは wasm だけである (TypeScript のターゲットは issue #23 で廃止した)。
//! それでもターゲットという語彙を残しておくのは、`[[native(arch = "...")]]` の arch 名と
//! 成果物の置き場所 (`.biwa_build/wasm/`) がこれに結び付いているからである。

use std::fmt;

macro_rules! target_table {
    ( $( $variant:ident, $name:literal, $ext:literal ; )* ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Target {
            $($variant,)*
        }

        impl Target {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// `--target` に渡す名前であり、`[[native(arch = "...")]]` の arch 名でもある。
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

            /// 成果物の拡張子。
            pub fn bin_extension(&self) -> &'static str {
                match self {
                    $(Self::$variant => $ext,)*
                }
            }
        }
    };
}

target_table!(
    // VM や別プロセスに隔離して動かす形。
    // エンジンの機能は同期的なホスト関数の呼び出しで使う。
    // 単相化がプログラム全体の操作なので、生成物は playable パッケージに 1 つだけ出る。
    Wasm, "wasm", "wasm";
);

impl Target {
    /// このターゲットの成果物と中間生成物を入れるディレクトリ名。
    ///
    /// `arch` の切り落としでシンボルの集合がターゲットごとに変わるので、
    /// `.biwameta` も `.biwamir` もターゲット依存である。
    /// ターゲットを切り替えても互いのキャッシュを壊さないよう、
    /// すべてこの下に入れる。
    pub fn build_subdir(&self) -> &'static str {
        self.name()
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// 選択肢を人に見せる形に並べる。
pub fn describe_targets(targets: &[Target]) -> String {
    targets
        .iter()
        .map(|t| format!("`{}`", t.name()))
        .collect::<Vec<_>>()
        .join(", ")
}
