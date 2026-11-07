//! `biwa dev`: 開発中のゲームを動かし、変更のたびに建て直す。
//!
//! 動かし方は 2 つ (`--platform`)。
//!   - `native` (既定): ツールチェインのネイティブの runtime (`engine/desktop-rust`) をウィンドウで動かす。
//!     Node.js は要らない。`src/` の `.biwa` が変わったら建て直し、通ったら runtime に読み直させる
//!     (標準入力に `reload`。`engine/desktop-rust/crates/player/src/dev.rs`)。`assets/` が変わったときも読み直させる。
//!   - `web`: ツールチェインのビルド済みの Web runtime と開発中のゲームを、CLI に内蔵した HTTP サーバ (`serve.rs`)
//!     で配り、ブラウザで動かす。Node.js は要らない。建て直したら (`assets/` が変わったときも) ページを読み直させる。
//!     `--vite` を付けると、エンジン (Vite のプロジェクト) を展開して Vite の開発サーバを立てる従来の動かし方になる
//!     (エンジンの開発や、`--target typescript` のため。Node.js が要る)。
//!
//! どれも、最初のビルドが通らなければ始めない。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use biwac_base::Target;
use clap::{Args, ValueEnum};
use notify::{RecursiveMode, Watcher};

use crate::project::Project;
use crate::runtime;
use crate::serve::{DevServer, Site};
use crate::toolchain::{DesktopOs, DesktopTarget, Runtime, Toolchain};

/// 変更イベントが落ち着くのを待つ時間。
/// エディタは保存 1 回で複数のイベントを出すので、まとめてから 1 回だけ建て直す。
const DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Args)]
pub struct DevArgs {
    /// 対象の biwa パッケージ (省略時はカレントディレクトリから探す)
    #[arg(short, long, value_name = "DIR")]
    pub package_path: Option<PathBuf>,

    /// キャッシュを無視して最初から建て直す
    #[arg(short, long)]
    pub rebuild: bool,

    /// 動かし方 (native: この OS のウィンドウ、web: ブラウザ)
    #[arg(long, value_enum, default_value = "native")]
    pub platform: DevPlatform,

    /// 開発サーバのポート (`--platform web` のみ。省略時は 5173 から空いているもの)
    #[arg(long)]
    pub port: Option<u16>,

    /// `--platform web` で、内蔵の HTTP サーバの代わりに Vite の開発サーバを使う
    /// (エンジンの開発や `--target typescript` のため。Node.js が要る)
    #[arg(long)]
    pub vite: bool,

    /// コード生成のターゲット
    ///
    /// 既定は `wasm`。生成物は Worker で走り、エンジンの API は
    /// ホスト関数の呼び出し (= syscall) になる。
    /// `typescript` を選ぶと、scene が generator として出力され、
    /// エンジンのメインスレッドで動く従来の経路になる (`--platform web` のみ)。
    #[arg(long, default_value = DEFAULT_TARGET)]
    pub target: String,

    /// runtime を取るツールチェイン (`--platform native` のみ。省略時は環境変数 BIWA_TOOLCHAIN、
    /// なければこの CLI と同じもの)
    #[arg(long, value_name = "NAME")]
    pub toolchain: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DevPlatform {
    /// この OS のウィンドウで、ネイティブの runtime で動かす
    Native,
    /// ブラウザで動かす (内蔵の HTTP サーバ)
    Web,
}

/// `--platform web` の開発サーバの既定のポート (Vite と同じ)。塞がっていれば次を試す。
const DEFAULT_PORT: u16 = 5173;

/// `--target` の既定値。
const DEFAULT_TARGET: &str = "wasm";

/// `--target` を解決する。
///
/// 名前が存在するかと、このビルドの biwac が実際に生成できるかは別である。
/// 前者は `Target::from_name`、後者は `available_targets` が答える。
fn resolve_target(name: &str) -> Result<Target> {
    let available = biwac_generator::available_targets();

    match Target::from_name(name) {
        Some(t) if available.contains(&t) => Ok(t),
        Some(t) => bail!(
            "target `{t}` is not available in this build of biwa (available: {})",
            biwac_base::describe_targets(&available)
        ),
        None => bail!(
            "unknown target `{name}` (available: {})",
            biwac_base::describe_targets(&available)
        ),
    }
}

pub fn run(args: DevArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;
    let target = resolve_target(&args.target)?;

    println!("Package: {} ({})", project.name, project.root.display());
    println!("Target: {target}");

    match args.platform {
        DevPlatform::Native => run_native(&project, target, &args),
        DevPlatform::Web if args.vite => run_vite(&project, target, &args),
        DevPlatform::Web => run_web(&project, target, &args),
    }
}

/// ネイティブの runtime で動かす。
fn run_native(project: &Project, target: Target, args: &DevArgs) -> Result<()> {
    if target != Target::Wasm {
        bail!("the native runtime runs wasm only; use `--target wasm` or `--platform web`");
    }
    // runtime が揃っているかは、ビルドの前に確かめる。
    let toolchain = Toolchain::select(args.toolchain.as_deref())?;
    let runtime = toolchain.runtime(DesktopTarget::new(DesktopOs::host()?, Runtime::Native))?;
    println!("Toolchain: {}", toolchain.name);

    runtime::ensure_std(project)?;
    // 初回は通らないと動かす意味がないので、失敗したらそこで止める。
    compile(project, target, args.rebuild)?;

    // 終わったかを見る側と読み直させる側で共有する。
    let session = std::cell::RefCell::new(NativeSession::spawn(project, &runtime)?);
    watch_loop(
        project,
        true,
        || session.borrow_mut().try_wait(),
        |change| {
            if change == Change::Source {
                println!();
                if let Err(e) = compile(project, target, false) {
                    // 書きかけのコードは当然コンパイルが通らない。前のゲームを動かしたまま、次の保存を待つ。
                    eprintln!("error: {e:#}");
                    return;
                }
            }
            session.borrow_mut().reload();
        },
    )
}

/// 内蔵の HTTP サーバで、ビルド済みの Web runtime にゲームを添えてブラウザで動かす。
fn run_web(project: &Project, target: Target, args: &DevArgs) -> Result<()> {
    if target != Target::Wasm {
        bail!(
            "the web runtime runs wasm only; use `--target wasm`, or `--vite` for the TypeScript target"
        );
    }
    // runtime が揃っているかは、ビルドの前に確かめる。
    let toolchain = Toolchain::select(args.toolchain.as_deref())?;
    let web_runtime = toolchain.web_runtime()?;
    println!("Toolchain: {}", toolchain.name);

    runtime::ensure_std(project)?;
    // 初回は通らないと動かす意味がないので、失敗したらそこで止める。
    compile(project, target, args.rebuild)?;

    let mut info = serde_json::Map::new();
    info.insert("name".into(), project.name.clone().into());
    info.insert("title".into(), format!("{} (dev)", project.name).into());
    info.insert(
        "version".into(),
        project.version.as_deref().unwrap_or("0.1.0").into(),
    );
    info.insert("toolchain".into(), toolchain.name.clone().into());
    let site = Site {
        web_runtime,
        wasm: project.generated_dir(target).join(format!(
            "{}.{}",
            project.name,
            target.bin_extension()
        )),
        assets: project.assets_dir(),
        info,
    };
    let server = DevServer::start(site, args.port.unwrap_or(DEFAULT_PORT), args.port.is_some())?;
    println!();
    println!(
        "  Open {} in a browser (Chrome, Edge, Firefox or Safari 18.2+).",
        server.url()
    );
    println!();

    watch_loop(
        project,
        true,
        || Ok(None),
        |change| {
            if change == Change::Source {
                println!();
                if let Err(e) = compile(project, target, false) {
                    // 書きかけのコードは当然コンパイルが通らない。前のゲームを動かしたまま、次の保存を待つ。
                    eprintln!("error: {e:#}");
                    return;
                }
            }
            println!("Reloading the page");
            server.reload();
        },
    )
}

/// `--platform web --vite`: Vite の開発サーバで動かす (従来の動かし方。Node.js が要る)。
fn run_vite(project: &Project, target: Target, args: &DevArgs) -> Result<()> {
    runtime::ensure_std(project)?;
    runtime::ensure_engine(project)?;
    // エンジンを展開したあとに張る。展開は `.biwa_runtime/` を掃除しないので、
    // 次回以降エンジンが更新されてもこのリンクは残る。
    runtime::link_assets(project)?;

    // 初回は通らないと開発サーバを立てる意味がないので、失敗したらそこで止める。
    build(project, target, args.rebuild)?;

    let mut vite = ViteProcess::spawn(project, args.port)?;

    // アセットは Vite がリンク越しに配るので、建て直すのは `.biwa` が変わったときだけ。
    watch_loop(
        project,
        false,
        || vite.try_wait(),
        |_| {
            println!();
            if let Err(e) = build(project, target, false) {
                // 書きかけのコードは当然コンパイルが通らない。
                // 直せば次の保存でまた走るので、開発サーバは動かし続ける。
                eprintln!("error: {e:#}");
            }
        },
    )
}

/// biwac を呼んで、生成物をエンジン側へ配る。
pub(crate) fn build(project: &Project, target: Target, force_rebuild: bool) -> Result<()> {
    compile(project, target, force_rebuild)?;
    runtime::sync_generated(project, target)
}

/// コンパイラ本体の呼び出し。
///
/// biwac はライブラリとして呼ぶ (`biwac` コマンドは経由しない)。
/// まだ未実装のエラー表示経路で panic することがあるので、
/// 開発サーバごと落ちないようにここで受け止める。
pub(crate) fn compile(project: &Project, target: Target, force_rebuild: bool) -> Result<()> {
    let root = project.root.clone();

    let result = std::panic::catch_unwind(move || {
        biwac_driver::compile(
            root,
            biwac_driver::BuildOptions {
                force_rebuild,
                // 中間表現は開発サーバの関心事ではない。
                emit_mir: false,
                target,
            },
        )
    });

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => bail!("build failed"),
        Err(_) => bail!("the compiler panicked while building this package"),
    }
}

/// 何が変わったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// `src/` の `.biwa`。
    Source,
    /// `assets/` の中のどれか。
    Assets,
}

/// `src/` (と `watch_assets` なら `assets/`) を監視し、変わるたびに `on_change` を呼ぶ。
///
/// `poll` が終了の状態を返したら (Vite が落ちた・ウィンドウが閉じられた) こちらも終わる。
fn watch_loop(
    project: &Project,
    watch_assets: bool,
    mut poll: impl FnMut() -> Result<Option<std::process::ExitStatus>>,
    mut on_change: impl FnMut(Change),
) -> Result<()> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res| {
        // 受け手が居なくなっている (終了処理中) 場合は捨てる。
        let _ = tx.send(res);
    })
    .context("failed to start the file watcher")?;

    let assets_dir = project.assets_dir();
    let mut dirs = runtime::watch_targets(project);
    if watch_assets && assets_dir.is_dir() {
        dirs.push(assets_dir.clone());
    }
    for dir in &dirs {
        watcher
            .watch(dir, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", dir.display()))?;
        println!("Watching {}", dir.display());
    }
    let classify = |event: &notify::Event| -> Option<Change> {
        if touches_biwa_source(event) {
            Some(Change::Source)
        } else if watch_assets && touches_dir(event, &assets_dir) {
            Some(Change::Assets)
        } else {
            None
        }
    };

    loop {
        if let Some(status) = poll()? {
            if status.success() {
                return Ok(());
            }
            bail!("exited with {status}");
        }

        let event = match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(Ok(event)) => event,
            Ok(Err(e)) => {
                eprintln!("warning: file watcher error: {e}");
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };

        let Some(mut change) = classify(&event) else {
            continue;
        };

        // 続けて飛んでくるイベントをまとめる。`.biwa` が 1 つでも変わっていれば建て直す。
        let deadline = Instant::now() + DEBOUNCE;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    if classify(&event) == Some(Change::Source) {
                        change = Change::Source;
                    }
                }
                Ok(Err(_)) => continue,
                Err(_) => break,
            }
        }

        on_change(change);
    }
}

fn touches_dir(event: &notify::Event, dir: &Path) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    ) && event.paths.iter().any(|p| p.starts_with(dir))
}

fn touches_biwa_source(event: &notify::Event) -> bool {
    if !matches!(
        event.kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    ) {
        return false;
    }

    event
        .paths
        .iter()
        .any(|p| p.extension().and_then(|e| e.to_str()) == Some(biwac_base::BIWA_EXTENSION))
}

/// Vite の子プロセス。CLI が先に終わっても道連れにする。
struct ViteProcess(std::process::Child);

impl ViteProcess {
    fn spawn(project: &Project, port: Option<u16>) -> Result<Self> {
        runtime::spawn_vite(project, port).map(Self)
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.0
            .try_wait()
            .context("failed to check the vite dev server")
    }
}

impl Drop for ViteProcess {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// 開発モードで動いているネイティブの runtime。CLI が先に終わっても道連れにする。
struct NativeSession {
    child: Child,
    /// 指示を送る口 (runtime の標準入力)。閉じると runtime は終わる。
    stdin: Option<ChildStdin>,
}

impl NativeSession {
    fn spawn(project: &Project, runtime: &Path) -> Result<Self> {
        let wasm = project.generated_dir(Target::Wasm).join(format!(
            "{}.{}",
            project.name,
            Target::Wasm.bin_extension()
        ));
        let mut command = Command::new(runtime);
        command
            .arg("--dev")
            .arg("--wasm")
            .arg(&wasm)
            .arg("--title")
            .arg(format!("{} (dev)", project.name))
            .stdin(Stdio::piped());
        let assets = project.assets_dir();
        if assets.is_dir() {
            command.arg("--assets").arg(&assets);
        }
        println!("Starting {}", runtime.display());
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start {}", runtime.display()))?;
        let stdin = child.stdin.take();
        Ok(Self { child, stdin })
    }

    /// wasm とアセットを読み直させる。
    fn reload(&mut self) {
        let Some(stdin) = &mut self.stdin else { return };
        if writeln!(stdin, "reload")
            .and_then(|()| stdin.flush())
            .is_err()
        {
            // runtime が終わっている。次の `try_wait` で気づく。
            self.stdin = None;
        }
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().context("failed to check the runtime")
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        // 標準入力を閉じれば runtime は自分で終わる。終わらなければ止める。
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
