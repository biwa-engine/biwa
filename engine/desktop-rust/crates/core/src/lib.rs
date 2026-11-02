//! Biwa のネイティブのプレイヤーの、エンジンの論理。
//!
//! 外部の crate に依存しない。ウィンドウ・GPU・文字の整形・wasm ランタイムはどれも
//! ここが決めたインターフェース (型とトレイト) の向こう側にいる。
//!
//! - [`syscall`]: ゲーム (wasm) からエンジンへの依頼
//! - [`engine`]: それを受けて状態を変え、1 フレームの描画の指示を組み立てる
//! - [`content`]: Message Window の中身と文字送り
//! - [`canvas`]: canvas オブジェクトと遷移 ([`transition`] はその番号と曲線)
//! - [`image`]: 画像の番号と読み込みの依頼
//! - [`ui`]: UI Element の木 (段 5 までは scene を始めるのに要る分と、出力先の箱だけ)
//! - [`text`]: 文字の整形とラスタライズのインターフェース
//! - [`draw`]: 描画の指示

pub mod canvas;
pub mod content;
pub mod draw;
pub mod engine;
pub mod geometry;
pub mod image;
pub mod syscall;
pub mod text;
pub mod transition;
pub mod ui;
