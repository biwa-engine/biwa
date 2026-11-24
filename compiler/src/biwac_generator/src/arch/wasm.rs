//! WASM バックエンド。
//!
//! 入力は単相化済みの [`biwac_mir::MonoMir`] である。
//! ジェネリクスは消えており、到達可能な実体だけが並んでいる。

pub mod emit;
pub mod structure;

pub use emit::{WasmError, emit};

/// 生成物と runtime の間の約束 (syscall の import の名前と引数、UI の property の番号、
/// `biwa:runtime` の import、export の名前など) の版。
///
/// 生成物の custom section [`ABI_SECTION`] に u32 (little endian) で書き、
/// runtime (`engine/desktop-rust/crates/vm`、`engine/nodejs/src/engine/vm/wasm/worker.ts`) が
/// 読み込むときに照合する。合わなければ、分かりにくい失敗 (LinkError や trap) の前に止まる。
///
/// **約束を変えたら上げ、runtime の側も合わせる。** std の `[[native(arch="wasm")]]` の import 宣言を
/// 変えたときもここである (std はツールチェインの一部なので、コンパイラと同じ版で配られる)。
pub const ABI_VERSION: u32 = 3;

/// [`ABI_VERSION`] を書く custom section の名前。
pub const ABI_SECTION: &str = "biwa.abi";

/// wasm バイナリの [`ABI_SECTION`] を読む。無い・壊れているときは `None`。
pub fn abi_version(binary: &[u8]) -> Option<u32> {
    for payload in wasmparser::Parser::new(0).parse_all(binary) {
        if let Ok(wasmparser::Payload::CustomSection(section)) = payload
            && section.name() == ABI_SECTION
        {
            let bytes: [u8; 4] = section.data().try_into().ok()?;
            return Some(u32::from_le_bytes(bytes));
        }
    }
    None
}

/// WAT を wasm バイナリにする。
///
/// アセンブルしたあと検証まで通す。
/// `wat` は「符号化できるか」しか見ないので、
/// 型が合っていない出力を黙って通してしまう。
/// 壊れた wasm を書き出して実行時に初めて気づくより、ここで止めるほうがよい。
pub fn assemble(wat: &str) -> Result<Vec<u8>, String> {
    let binary = wat::parse_str(wat).map_err(|e| e.to_string())?;

    // GC / 参照型を使うので、既定より広い機能集合で検証する。
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&binary)
        .map_err(|e| format!("the generated wasm is invalid: {e}"))?;

    Ok(binary)
}
