mod check;
mod error;
mod table;

pub use check::check;
pub use error::{EntrypointError, SignatureProblem};
pub use table::{ContractTy, Entrypoint, EntrypointKind, EntrypointRequirement, Entrypoints};

// この crate は「エントリポイント (ランタイムが名前を知っていて直接呼ぶもの)」の規約を持つ。
//
// いまは `fn main()` の 1 つだけで、playable package は定義しなければならない。
// 名前・種別 (関数か scene か)・シグネチャを検査し、解決結果 (`Entrypoints`) を返す。
//
// scene 一般のシグネチャ (`(Game[..]) -> Game[..]`) の規約は biwac_scene にある。
//
// どのターゲット言語でどんなシンボル名として公開されるかは
// codegen 側 (biwac_generator の各 arch) の規約であり、ここでは関知しない。
