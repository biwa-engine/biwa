/**
 * 生成物と runtime の約束の版 (ABI) の照合。
 *
 * コンパイラは生成物の custom section `biwa.abi` に版を u32 (little endian) で書く
 * (`compiler/src/biwac_generator/src/arch/wasm.rs` の `ABI_VERSION`)。
 * 合わない生成物を走らせると、足りない import (`LinkError`) や trap といった分かりにくい形で失敗するので、
 * compile の前に照合して、ツールチェインが合っていないことをはっきり言う。
 *
 * `WebAssembly.Module.customSections()` は使わず、section の並びを自分で読む
 * (`Module.imports()` が JavaScriptCore で投げることがあるのと同じ心配を避ける。`worker.ts` の `declaredImports`)。
 * ネイティブの runtime も同じ読み方をしている (`engine/desktop-rust/crates/vm/src/abi.rs`)。
 */

/** この runtime が走らせられる版。コンパイラの `ABI_VERSION` を上げたら合わせる。 */
export const ABI_VERSION = 3;

const ABI_SECTION = "biwa.abi";

/** 生成物の版を確かめる。合わなければ投げる。 */
export function checkAbi(wasm: ArrayBuffer): void {
  const version = readAbi(new Uint8Array(wasm));
  if (version === ABI_VERSION) {
    return;
  }
  if (version === null) {
    throw new Error(
      `this game has no Biwa ABI version (custom section \`${ABI_SECTION}\`); it was built by an older Biwa toolchain, ` +
        `so rebuild it with the same toolchain as this runtime (ABI ${ABI_VERSION})`,
    );
  }
  throw new Error(
    `this game was built for Biwa ABI ${version}, but this runtime supports ABI ${ABI_VERSION}; ` +
      "build the game and the app with the same Biwa toolchain",
  );
}

/** custom section `biwa.abi` の値。無い・壊れているときは `null`。 */
export function readAbi(wasm: Uint8Array): number | null {
  const magic = [0x00, 0x61, 0x73, 0x6d];
  if (wasm.length < 8 || magic.some((b, i) => wasm[i] !== b)) {
    return null;
  }
  const name = new TextEncoder().encode(ABI_SECTION);
  let pos = 8;
  while (pos < wasm.length) {
    const id = wasm[pos];
    const size = leb128(wasm, pos + 1);
    if (size === null) {
      return null;
    }
    const start = pos + 1 + size.length;
    const end = start + size.value;
    if (end > wasm.length) {
      return null;
    }
    pos = end;
    if (id !== 0) {
      continue;
    }
    const nameLen = leb128(wasm, start);
    if (nameLen === null) {
      return null;
    }
    const nameStart = start + nameLen.length;
    const dataStart = nameStart + nameLen.value;
    const same =
      nameLen.value === name.length && name.every((b, i) => wasm[nameStart + i] === b);
    if (!same) {
      continue;
    }
    if (end - dataStart !== 4) {
      return null;
    }
    return new DataView(wasm.buffer, wasm.byteOffset + dataStart, 4).getUint32(0, true);
  }
  return null;
}

/** 符号なし LEB128 (32 bit まで)。 */
function leb128(bytes: Uint8Array, pos: number): { value: number; length: number } | null {
  let value = 0;
  for (let i = 0; i < 5 && pos + i < bytes.length; i++) {
    const b = bytes[pos + i];
    value += (b & 0x7f) * 2 ** (7 * i);
    if ((b & 0x80) === 0) {
      return { value, length: i + 1 };
    }
  }
  return null;
}
