// WebView の機能を順に調べて表に並べる。
// 結果は「結果を JSON でコピー」で取り出せる (検証の報告に貼る用)。

const rows = document.getElementById("results");
const report = {};

function group(title) {
  const tr = document.createElement("tr");
  tr.className = "group";
  tr.innerHTML = `<td colspan="3"></td>`;
  tr.firstChild.textContent = title;
  rows.appendChild(tr);
}

/** status: true = 必要なものが動く, false = 動かない, null = 情報のみ */
function row(key, status, value) {
  report[key] = { status, value };
  const tr = document.createElement("tr");
  const label = status === true ? "OK" : status === false ? "NG" : "info";
  const cls = status === true ? "ok" : status === false ? "ng" : "info";
  tr.innerHTML = `<td class="k"></td><td class="${cls}">${label}</td><td class="v"></td>`;
  tr.children[0].textContent = key;
  tr.children[2].textContent =
    typeof value === "string" ? value : JSON.stringify(value);
  rows.appendChild(tr);
}

async function attempt(key, fn) {
  try {
    const [status, value] = await fn();
    row(key, status, value);
  } catch (e) {
    row(key, false, String(e && e.message ? e.message : e));
  }
}

// --- 環境 ---------------------------------------------------------------

function environment() {
  group("環境");
  row("userAgent", null, navigator.userAgent);
  row("location", null, location.href);
  row(
    "isSecureContext",
    window.isSecureContext,
    String(window.isSecureContext),
  );
  row("devicePixelRatio", null, String(window.devicePixelRatio));
  row("screen", null, `${screen.width}x${screen.height}`);
  row("languages", null, navigator.languages.join(", "));
}

// --- Wasm の実行に要るもの -----------------------------------------------

/**
 * (type $s (struct (field i32)))
 * (func (export "f") (param externref) (result i32)
 *   (struct.get $s 0 (ref.cast (ref $s) (struct.new $s (i32.const 7)))))
 */
const WASM_GC_MODULE = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x0a, 0x02, 0x5f, 0x01,
  0x7f, 0x00, 0x60, 0x01, 0x6f, 0x01, 0x7f, 0x03, 0x02, 0x01, 0x01, 0x07, 0x05,
  0x01, 0x01, 0x66, 0x00, 0x00, 0x0a, 0x10, 0x01, 0x0e, 0x00, 0x41, 0x07, 0xfb,
  0x00, 0x00, 0xfb, 0x16, 0x00, 0xfb, 0x02, 0x00, 0x00, 0x0b,
]);

async function wasm() {
  group("Wasm の実行 (必須)");
  // 要るのは SharedArrayBuffer (と Worker への受け渡し) で、isolated かどうかは情報として見る。
  // WebKitGTK の独自スキーム (`biwa://`) は isolated と見なされないが、プレイヤーが有効にした SAB は使える。
  row("crossOriginIsolated", null, String(globalThis.crossOriginIsolated));
  row(
    "SharedArrayBuffer",
    typeof SharedArrayBuffer !== "undefined",
    typeof SharedArrayBuffer,
  );

  await attempt("WasmGC + externref (最小モジュール)", async () => {
    if (!WebAssembly.validate(WASM_GC_MODULE))
      return [false, "validate() = false"];
    const { instance } = await WebAssembly.instantiate(WASM_GC_MODULE);
    const r = instance.exports.f(null);
    return [r === 7, `f() = ${r}`];
  });

  let gameModule = null;
  await attempt("ゲームの wasm をコンパイル", async () => {
    const res = await fetch("./game.wasm");
    if (!res.ok) return [null, `game.wasm が無い (${res.status})`];
    const bytes = await res.arrayBuffer();
    const started = performance.now();
    gameModule = await WebAssembly.compile(bytes);
    const ms = (performance.now() - started).toFixed(1);
    return [true, `${bytes.byteLength} bytes, ${ms} ms`];
  });

  if (gameModule !== null) {
    // エンジンはこれが投げても動く (worker.ts の declaredImports) ので情報として見る。
    await attempt("WebAssembly.Module.imports (ゲーム)", async () => {
      try {
        return [
          null,
          `${WebAssembly.Module.imports(gameModule).length} imports`,
        ];
      } catch (e) {
        return [null, `例外: ${e.message}`];
      }
    });

    // import の名前を知らなくても埋まるよう、どの名前にも空の関数を返す。
    await attempt("ゲームの wasm を instantiate (空の import)", async () => {
      const namespace = new Proxy({}, { get: () => () => {} });
      const imports = new Proxy({}, { get: () => namespace });
      const instance = await WebAssembly.instantiate(gameModule, imports);
      return [true, `${Object.keys(instance.exports).length} exports`];
    });
  }

  await attempt(
    "module Worker + Atomics.wait / notify",
    () =>
      new Promise((resolve) => {
        if (typeof SharedArrayBuffer === "undefined") {
          resolve([false, "SharedArrayBuffer が無い"]);
          return;
        }
        const buffer = new SharedArrayBuffer(4);
        const cell = new Int32Array(buffer);
        const worker = new Worker(new URL("./worker.js", import.meta.url), {
          type: "module",
          name: "probe",
        });
        const timer = setTimeout(() => {
          worker.terminate();
          resolve([false, "Worker が応答しない"]);
        }, 8000);
        worker.addEventListener("error", (e) => {
          clearTimeout(timer);
          worker.terminate();
          resolve([false, `worker error: ${e.message}`]);
        });
        worker.addEventListener("message", (event) => {
          const m = event.data;
          if (m.kind === "waiting") {
            setTimeout(() => {
              Atomics.store(cell, 0, 42);
              Atomics.notify(cell, 0);
            }, 100);
          } else if (m.kind === "woken") {
            clearTimeout(timer);
            worker.terminate();
            resolve([
              m.result === "ok" && m.value === 42,
              `wait=${m.result}, value=${m.value}, ${m.ms.toFixed(1)} ms`,
            ]);
          } else {
            clearTimeout(timer);
            worker.terminate();
            resolve([false, m.message]);
          }
        });
        worker.postMessage({ buffer });
      }),
  );
}

// --- 描画 ----------------------------------------------------------------

async function graphics() {
  group("描画");
  await attempt("WebGL2", async () => {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2");
    if (!gl) return [false, "getContext('webgl2') = null"];
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = dbg
      ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER);
    const vendor = dbg
      ? gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL)
      : gl.getParameter(gl.VENDOR);
    return [
      true,
      `${vendor} / ${renderer} / maxTexture=${gl.getParameter(gl.MAX_TEXTURE_SIZE)}`,
    ];
  });
  row("WebGPU (navigator.gpu)", null, String("gpu" in navigator));
  await attempt(
    "requestAnimationFrame (2 秒の平均 fps)",
    () =>
      new Promise((resolve) => {
        let frames = 0;
        let first = null;
        const tick = (t) => {
          if (first === null) first = t;
          frames++;
          if (t - first < 2000) requestAnimationFrame(tick);
          else
            resolve([
              null,
              `${(((frames - 1) * 1000) / (t - first)).toFixed(1)} fps`,
            ]);
        };
        requestAnimationFrame(tick);
      }),
  );
}

// --- メディア -------------------------------------------------------------

function canPlay(element, types) {
  const result = {};
  for (const [name, type] of Object.entries(types)) {
    result[name] = element.canPlayType(type) || "no";
  }
  return result;
}

async function media() {
  group("サウンド");
  await attempt("AudioContext", async () => {
    const Ctor = window.AudioContext || window.webkitAudioContext;
    if (!Ctor) return [false, "AudioContext が無い"];
    const ctx = new Ctor();
    const value = `state=${ctx.state} (操作前), sampleRate=${ctx.sampleRate}`;
    await ctx.close();
    return [true, value];
  });
  const audio = canPlay(document.createElement("audio"), {
    mp3: "audio/mpeg",
    "aac (m4a)": 'audio/mp4; codecs="mp4a.40.2"',
    "opus (ogg)": 'audio/ogg; codecs="opus"',
    "opus (webm)": 'audio/webm; codecs="opus"',
    "vorbis (ogg)": 'audio/ogg; codecs="vorbis"',
    wav: 'audio/wav; codecs="1"',
    flac: "audio/flac",
  });
  for (const [k, v] of Object.entries(audio)) row(`audio: ${k}`, null, v);

  group("動画");
  const video = canPlay(document.createElement("video"), {
    "H.264 (mp4)": 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"',
    "H.265 (mp4)": 'video/mp4; codecs="hvc1.1.6.L93.B0"',
    "VP9 (webm)": 'video/webm; codecs="vp9, opus"',
    "AV1 (mp4)": 'video/mp4; codecs="av01.0.05M.08"',
    "AV1 (webm)": 'video/webm; codecs="av01.0.05M.08, opus"',
  });
  for (const [k, v] of Object.entries(video)) row(`video: ${k}`, null, v);
  row("MediaSource", null, String(typeof MediaSource !== "undefined"));
  if (navigator.mediaCapabilities) {
    await attempt("mediaCapabilities: H.264 1080p", () =>
      decodingInfo('video/mp4; codecs="avc1.640028"'),
    );
    await attempt("mediaCapabilities: VP9 1080p", () =>
      decodingInfo('video/webm; codecs="vp09.00.40.08"'),
    );
  }

  group("フォント");
  row("FontFace", typeof FontFace !== "undefined", String(typeof FontFace));
  row("document.fonts", !!document.fonts, String(!!document.fonts));
}

async function decodingInfo(contentType) {
  const info = await navigator.mediaCapabilities.decodingInfo({
    type: "file",
    video: {
      contentType,
      width: 1920,
      height: 1080,
      bitrate: 5_000_000,
      framerate: 30,
    },
  });
  return [
    null,
    `supported=${info.supported}, smooth=${info.smooth}, powerEfficient=${info.powerEfficient}`,
  ];
}

// --- 実行 ----------------------------------------------------------------

environment();
await wasm();
await graphics();
await media();

const status = document.getElementById("status");
const failed = Object.entries(report)
  .filter(([, r]) => r.status === false)
  .map(([k]) => k);
status.textContent =
  failed.length === 0
    ? "完了: 必須項目はすべて OK"
    : `完了: NG ${failed.length} 件 (${failed.join(", ")})`;

const copy = document.getElementById("copy");
copy.disabled = false;
copy.addEventListener("click", async () => {
  const text = JSON.stringify(report, null, 2);
  try {
    await navigator.clipboard.writeText(text);
    copy.textContent = "コピーしました";
  } catch {
    // クリップボードが使えない WebView では、選択できる形で出す。
    const pre = document.createElement("pre");
    pre.textContent = text;
    document.body.appendChild(pre);
    copy.textContent = "下に表示しました";
  }
});
