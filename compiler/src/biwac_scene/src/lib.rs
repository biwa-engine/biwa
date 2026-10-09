mod check;
mod error;

pub use check::check;
pub use error::{SceneError, SignatureProblem};

// この crate は「scene が守るべき規約」を持つ。
//
// すべての scene は lang item `game` のみを引数に取り `game` を返す (`(Game[..]) -> Game[..]`)。
// scene はストーリーの一区切りであり、ゲームの状態を受け取って返すためである。
// ジェネリック引数に何が入るかは問わない (`Game[..]` の中身は開発者が決める)。
//
// ランタイムが名前で呼ぶエントリポイント (`fn main()`) の規約は biwac_entrypoint にある。
