//! 生成物と runtime の約束の版 (ABI) の照合。
//!
//! コンパイラは生成物の custom section `biwa.abi` に版を u32 (little endian) で書く
//! (`compiler/src/biwac_generator/src/arch/wasm.rs` の `ABI_VERSION`)。
//! 合わない生成物を走らせると、足りない import (LinkError) や trap といった分かりにくい形で失敗するので、
//! compile の前に照合して、ツールチェインが合っていないことをはっきり言う。
//!
//! wasmtime は custom section を読む API を持たないので、section の並びだけを自分で読む。

/// この runtime が走らせられる版。コンパイラの `ABI_VERSION` を上げたら合わせる
/// (`engine/nodejs/src/engine/vm/wasm/abi.ts` も)。
pub const ABI_VERSION: u32 = 3;

const ABI_SECTION: &str = "biwa.abi";

/// 生成物の版を確かめる。合わなければエラーの文を返す。
pub fn check(wasm: &[u8]) -> Result<(), String> {
    match read(wasm) {
        Some(version) if version == ABI_VERSION => Ok(()),
        Some(version) => Err(format!(
            "this game was built for Biwa ABI {version}, but this runtime supports ABI {ABI_VERSION}; \
             build the game and the app with the same Biwa toolchain"
        )),
        None => Err(format!(
            "this game has no Biwa ABI version (custom section `{ABI_SECTION}`); \
             it was built by an older Biwa toolchain, so rebuild it with the same toolchain as this runtime \
             (ABI {ABI_VERSION})"
        )),
    }
}

/// custom section `biwa.abi` の値。無い・壊れているときは `None`。
pub fn read(wasm: &[u8]) -> Option<u32> {
    let mut rest = wasm.strip_prefix(b"\0asm")?.get(4..)?;
    while !rest.is_empty() {
        let id = rest[0];
        let (size, n) = leb128(&rest[1..])?;
        let body = rest.get(1 + n..1 + n + size)?;
        rest = &rest[1 + n + size..];
        if id != 0 {
            continue;
        }
        let (name_len, n) = leb128(body)?;
        let name = body.get(n..n + name_len)?;
        if name == ABI_SECTION.as_bytes() {
            let data: [u8; 4] = body[n + name_len..].try_into().ok()?;
            return Some(u32::from_le_bytes(data));
        }
    }
    None
}

/// 符号なし LEB128 (32 bit まで)。値と読んだバイト数。
fn leb128(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value: u64 = 0;
    for (i, &b) in bytes.iter().enumerate().take(5) {
        value |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Some((usize::try_from(value).ok()?, i + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空のモジュールに custom section を足したもの。
    fn module_with(sections: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"\0asm\x01\0\0\0".to_vec();
        for (name, data) in sections {
            let mut body = vec![name.len() as u8];
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(data);
            out.push(0);
            out.push(body.len() as u8);
            out.extend(body);
        }
        out
    }

    #[test]
    fn reads_the_abi_section() {
        let wasm = module_with(&[("name", b"xx"), (ABI_SECTION, &ABI_VERSION.to_le_bytes())]);
        assert_eq!(read(&wasm), Some(ABI_VERSION));
        assert!(check(&wasm).is_ok());
    }

    #[test]
    fn rejects_other_versions_and_missing_sections() {
        let wasm = module_with(&[(ABI_SECTION, &(ABI_VERSION + 1).to_le_bytes())]);
        assert!(check(&wasm).unwrap_err().contains("built for Biwa ABI"));
        assert!(check(&module_with(&[])).unwrap_err().contains("no Biwa ABI version"));
        assert_eq!(read(b"not wasm"), None);
    }
}
