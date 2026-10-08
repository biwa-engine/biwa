//! エンジン (Node.js 版) の配置と、コンパイル結果の受け渡し。
//!
//! biwac の出力は `<project>/.biwa_build/typescript/` に生えるが、
//! Vite が見るのは `<project>/.biwa_runtime/` である。
//! ここが両者をつなぐ。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use biwac_base::Target;

use crate::assets::{self, Engine, Std};
use crate::project::Project;

/// エンジンを展開したときの同一性を記録するファイル。
const ENGINE_STAMP_FILE: &str = ".biwa-engine-stamp";
/// std を展開したときの同一性を記録するファイル。
///
/// このファイルが無い `deps/std` は「利用者が自分で用意したもの」とみなし、
/// CLI は一切触らない (リポジトリのシンボリックリンクなど)。
const STD_STAMP_FILE: &str = ".biwa-std-stamp";

/// 生成物とスタブを置くディレクトリ (`.biwa_runtime/` からの相対)。
const GAME_DIR: &str = "src/game";
/// Vite がそのまま URL のルートに出すディレクトリ (`.biwa_runtime/` からの相対)。
const PUBLIC_DIR: &str = "public";
/// エンジンが import するスタブ。ファイル名も default export もエンジンと合意済みの規約。
const ENTRY_FILE: &str = "entry.ts";
/// wasm の生成物を置く名前。パッケージ名に依らない固定名にして、
/// エンジン側の import (`./game.wasm?url`) を安定させる。
const WASM_FILE: &str = "game.wasm";
/// std が `GameWindow` の組み立て口として host export している名前。
///
/// `[[host_export="__biwa_std_game_window_new"]]` (`library/std/src/game/ui.biwa`)。
/// 依存 (std) の host export は playable package のモジュールから再 export されるので、
/// エントリポイントと同じくパッケージのモジュールから import できる。
const GAME_WINDOW_NEW_NAME: &str = "__biwa_std_game_window_new";

/// UI を出す関数の固定名。中身はゲーム側の `fn app()` (中で `Window` を `show()` する)。
const APP_NAME: &str = "__biwa_app";

/// 依存パッケージとして `std` を用意する。
///
/// biwac は依存が `<root>/.biwa_build/deps/<name>/` に取得済みであることを前提とする。
/// 取得の実装はまだ無いので、同梱している std をここに置くのが CLI の仕事になる。
pub fn ensure_std(project: &Project) -> Result<()> {
    let deps_dir = project.dependencies_dir();
    let std_dir = deps_dir.join("std");
    let stamp_path = std_dir.join(STD_STAMP_FILE);
    let stamp = assets::stamp::<Std>();

    if std_dir.exists() {
        if !stamp_path.is_file() {
            // 利用者が自分で置いたもの。バージョンの管理も利用者の責任。
            return Ok(());
        }
        if std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp.as_str()) {
            return Ok(());
        }
        println!("Updating bundled `std`");
    } else {
        println!("Placing bundled `std` into {}", deps_dir.display());
    }

    std::fs::create_dir_all(&std_dir)
        .with_context(|| format!("failed to create {}", std_dir.display()))?;
    assets::extract::<Std>(&std_dir)?;
    std::fs::write(&stamp_path, &stamp)
        .with_context(|| format!("failed to write {}", stamp_path.display()))?;

    Ok(())
}

/// エンジンを `.biwa_runtime/` に展開し、必要なら依存をインストールする。
///
/// 展開済みで内容も一致していれば何もしない。
/// `node_modules` は展開の対象外なので、更新しても再インストールは走らない。
pub fn ensure_engine(project: &Project) -> Result<()> {
    let runtime_dir = project.runtime_dir();
    let stamp_path = runtime_dir.join(ENGINE_STAMP_FILE);
    let stamp = assets::stamp::<Engine>();

    let up_to_date = std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(stamp.as_str());

    if !up_to_date {
        if runtime_dir.exists() {
            println!("Updating engine in {}", runtime_dir.display());
        } else {
            println!("Placing engine into {}", runtime_dir.display());
        }
        std::fs::create_dir_all(&runtime_dir)
            .with_context(|| format!("failed to create {}", runtime_dir.display()))?;
        assets::extract::<Engine>(&runtime_dir)?;
        std::fs::write(&stamp_path, &stamp)
            .with_context(|| format!("failed to write {}", stamp_path.display()))?;
    }

    if !runtime_dir.join("node_modules").is_dir() {
        npm_install(&runtime_dir)?;
    }

    Ok(())
}

/// パッケージの `assets/` をエンジンから引ける場所に繋ぐ。
///
/// エンジンは Vite プロジェクトなので、`public/` に置いたものが
/// そのまま URL のルートに出る。そこへ `assets` という名前で
/// パッケージの `assets/` を向けたシンボリックリンクを張る。
/// これでエンジンから見たアセットの位置は常に `<base>assets/<path>` になり、
/// `.biwa` が書いた「`assets/` を基準とする相対パス」がそのまま使える。
///
/// リンクにしているのは、ファイルを足しても `biwa dev` を建て直さずに済むからである。
/// 配布用の `biwa build` ではコピーすることになる。
pub fn link_assets(project: &Project) -> Result<()> {
    let assets_dir = project.assets_dir();

    if !assets_dir.exists() {
        // 規約で決まっている置き場所なので、無ければこちらで用意する。
        println!("Creating {}", assets_dir.display());
        std::fs::create_dir_all(&assets_dir)
            .with_context(|| format!("failed to create {}", assets_dir.display()))?;
    }

    let link = project
        .runtime_dir()
        .join(PUBLIC_DIR)
        .join(crate::ASSETS_DIRECTORY_NAME);
    // `.biwa_runtime` は常にプロジェクト直下なので、相対のままで安定する。
    let target = Path::new("..")
        .join("..")
        .join(crate::ASSETS_DIRECTORY_NAME);

    if std::fs::read_link(&link).is_ok_and(|current| current == target) {
        return Ok(());
    }

    // 別物 (古いリンク、コピーされたディレクトリ) が居たら退ける。
    // `.biwa_runtime/` は CLI が所有しているので消してよい。
    match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&link),
        Ok(_) => std::fs::remove_file(&link),
        Err(_) => Ok(()),
    }
    .with_context(|| format!("failed to replace {}", link.display()))?;

    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    symlink_dir(&target, &link).with_context(|| {
        format!(
            "failed to link {} to {}",
            link.display(),
            assets_dir.display()
        )
    })
}

/// ディレクトリへのシンボリックリンクを張る。
///
/// Windows ではディレクトリとファイルで API が分かれていて、
/// しかも作成に開発者モードか管理者権限が要る。
#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(target, link).context(
        "creating a symbolic link requires Developer Mode (or an elevated prompt) on Windows",
    )?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn symlink_dir(_target: &Path, _link: &Path) -> Result<()> {
    bail!("this platform does not support symbolic links, which `biwa dev` needs for `assets/`")
}

fn npm_install(runtime_dir: &Path) -> Result<()> {
    println!("Installing engine dependencies (npm install)");

    let status = std::process::Command::new("npm")
        .arg("install")
        .current_dir(runtime_dir)
        .status()
        .context("failed to run `npm`; Biwa requires Node.js (with npm) to be installed")?;

    if !status.success() {
        bail!("`npm install` failed in {}", runtime_dir.display());
    }

    Ok(())
}

/// コンパイル結果をエンジンから見える場所へ移し、エントリポイントのスタブを書く。
///
/// biwac の出力は `<project>/.biwa_build/<target>/` に生えるが、
/// Vite が見るのは `<project>/.biwa_runtime/` である。
/// 何をどう移すかはターゲットで違うが、
/// 「エンジンは `src/game/entry.ts` の default export だけを見る」点は共通である。
pub fn sync_generated(project: &Project, target: Target) -> Result<()> {
    let src_dir = project.generated_dir(target);
    let dst_dir = project.runtime_dir().join(GAME_DIR);

    if !src_dir.is_dir() {
        bail!(
            "no compiler output found at {} (did the build succeed?)",
            src_dir.display()
        );
    }

    std::fs::create_dir_all(&dst_dir)
        .with_context(|| format!("failed to create {}", dst_dir.display()))?;

    let mut placed = match target {
        Target::TypeScript => place_typescript(project, &src_dir, &dst_dir)?,
        Target::Wasm => place_wasm(project, &src_dir, &dst_dir)?,
    };
    placed.insert(std::ffi::OsString::from(ENTRY_FILE));

    prune(&dst_dir, &placed)
}

/// TypeScript の生成物を配る。
///
/// biwac は自パッケージと推移的依存の `.ts` を 1 つのディレクトリに並べて出力する
/// (相互 import が `./<package>.ts` のため)。その関係を保ったまま丸ごと移す。
fn place_typescript(
    project: &Project,
    src_dir: &Path,
    dst_dir: &Path,
) -> Result<std::collections::HashSet<std::ffi::OsString>> {
    let mut placed = std::collections::HashSet::new();

    for entry in std::fs::read_dir(src_dir)
        .with_context(|| format!("failed to read {}", src_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };

        let contents =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        write_if_changed(&dst_dir.join(file_name), &contents)?;
        placed.insert(file_name.to_os_string());
    }

    if placed.is_empty() {
        bail!("no generated TypeScript found in {}", src_dir.display());
    }

    write_typescript_entry_stub(project, dst_dir)?;

    Ok(placed)
}

/// wasm の生成物を配る。
///
/// 単相化で std ごと 1 つにまとまるので、運ぶのはこの 1 ファイルだけである。
fn place_wasm(
    project: &Project,
    src_dir: &Path,
    dst_dir: &Path,
) -> Result<std::collections::HashSet<std::ffi::OsString>> {
    let src = src_dir.join(format!("{}.{}", project.name, Target::Wasm.bin_extension()));

    let contents = std::fs::read(&src).with_context(|| {
        format!(
            "failed to read {} (did the build produce a wasm module?)",
            src.display()
        )
    })?;

    write_if_changed(&dst_dir.join(WASM_FILE), &contents)?;
    write_wasm_entry_stub(project, dst_dir, &content_id(&contents))?;

    Ok([std::ffi::OsString::from(WASM_FILE)].into_iter().collect())
}

/// 前のターゲットや、依存から外れたパッケージの生成物を消す。
///
/// 残しておくと古いコードが型検査に混ざり続けるし、
/// ターゲットを切り替えたときにどちらが動いているのか分からなくなる。
fn prune(dst_dir: &Path, placed: &std::collections::HashSet<std::ffi::OsString>) -> Result<()> {
    for entry in std::fs::read_dir(dst_dir)
        .with_context(|| format!("failed to read {}", dst_dir.display()))?
    {
        let path = entry?.path();
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ts") | Some("wasm")
        ) {
            continue;
        }
        if path.file_name().is_none_or(|name| !placed.contains(name)) {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    Ok(())
}

/// 中身が変わっていなければ書かない。
///
/// 毎回上書きすると、内容が同じでも Vite がリロードを撒いてしまう。
fn write_if_changed(path: &Path, contents: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

/// 生成物の中身から決まる短い値。FNV-1a。
///
/// wasm の URL は再ビルドしても変わらないので、これを付けて
/// ブラウザが古いモジュールを掴んだままになるのを防ぐ。
fn content_id(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// TypeScript 生成物のためのスタブ。
///
/// 生成物のシンボルはマングルされていてパッケージごとに名前が変わるが、
/// エンジンが呼ぶもの (`fn app()` と std の `GameWindow` の組み立て口) だけは
/// `__biwa_app` / `__biwa_std_game_window_new` という固定名で export されている。
/// (scene は SceneStartButton から預けた関数を通して始まるので、`__biwa_entrypoint` /
/// `__biwa_on_new_game` はもう使わない)
/// それをさらに固定のファイル名・固定の default export に均し、
/// エンジンがパッケージ名を知らなくても済むようにする。
fn write_typescript_entry_stub(project: &Project, dst_dir: &Path) -> Result<()> {
    let contents = format!(
        r#"// AUTO-GENERATED by `biwa dev`. Do not edit.
//
// コンパイル結果のエントリポイントを、エンジンが知っている形に均すスタブ。
import {{
  {game_window_new},
  {app},
}} from "./{pkg}.ts";
import type {{
  BiwaApp,
  BiwaBackend,
  BiwaGameWindowNew,
}} from "../engine/game";

const backend: BiwaBackend = {{
  kind: "typescript",
  packageName: "{pkg}",
  // 生成コードの型はマングル名なので、エンジン側の構造的な型に読み替える。
  gameWindowNew: {game_window_new} as unknown as BiwaGameWindowNew,
  app: {app} as unknown as BiwaApp,
}};

export default backend;
"#,
        game_window_new = GAME_WINDOW_NEW_NAME,
        app = APP_NAME,
        pkg = project.name,
    );

    write_if_changed(&dst_dir.join(ENTRY_FILE), contents.as_bytes())
}

/// wasm 生成物のためのスタブ。
///
/// wasm は import できないので、エンジンには置き場所だけを教える。
/// 読み込みと実行は Worker がやる。
///
/// `buildId` が毎回変わることには、ブラウザのキャッシュ避けのほかに
/// 「このファイルが変わる = Vite がページを作り直す」という役目もある。
/// `.wasm` は Vite のモジュールグラフでは葉なので、
/// これが無いと再ビルドしても画面が古いままになる。
fn write_wasm_entry_stub(project: &Project, dst_dir: &Path, build_id: &str) -> Result<()> {
    let contents = format!(
        r#"// AUTO-GENERATED by `biwa dev`. Do not edit.
//
// コンパイル結果 (wasm) の在り処を、エンジンが知っている形に均すスタブ。
import type {{ BiwaBackend }} from "../engine/game";
import wasmUrl from "./{wasm}?url";

const backend: BiwaBackend = {{
  kind: "wasm",
  packageName: "{pkg}",
  url: wasmUrl,
  buildId: "{build_id}",
}};

export default backend;
"#,
        wasm = WASM_FILE,
        pkg = project.name,
        build_id = build_id,
    );

    write_if_changed(&dst_dir.join(ENTRY_FILE), contents.as_bytes())
}

/// Vite の開発サーバを起動する。
pub fn spawn_vite(project: &Project, port: Option<u16>) -> Result<std::process::Child> {
    let runtime_dir = project.runtime_dir();
    let vite_bin = runtime_dir.join("node_modules").join(".bin").join("vite");

    if !vite_bin.exists() {
        bail!(
            "vite is not installed at {} (try removing {} and running again)",
            vite_bin.display(),
            runtime_dir.display()
        );
    }

    let mut command = std::process::Command::new(vite_bin);
    command.current_dir(&runtime_dir);
    if let Some(port) = port {
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--strictPort");
    }

    command
        .spawn()
        .context("failed to start the vite dev server")
}

/// 開発サーバが監視するディレクトリ。
///
/// 依存パッケージは監視しない。ビルド成果物がその中 (`deps/<name>/.biwa_build`)
/// に書かれるため、監視すると再ビルドが自分自身を呼び続ける。
pub fn watch_targets(project: &Project) -> Vec<PathBuf> {
    [project.src_dir()]
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}
