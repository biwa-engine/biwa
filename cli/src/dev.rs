//! `biwa dev`: 開発中のゲームを動かし、変更のたびに建て直す。
//!
//! 動かし方は 2 つ (`--platform`)。どちらもツールチェインのビルド済みの runtime を使い、Node.js は要らない。
//!   - `native` (既定): ネイティブの runtime (`engine/desktop-rust`) をウィンドウで動かす。
//!     `src/` の `.biwa` が変わったら建て直し、通ったら runtime に読み直させる
//!     (標準入力に `reload`。`engine/desktop-rust/crates/player/src/dev.rs`)。`assets/` が変わったときも読み直させる。
//!   - `web`: ビルド済みの Web runtime と開発中のゲームを、CLI に内蔵した HTTP サーバ (`serve.rs`) で配り、
//!     ブラウザで動かす。建て直したら (`assets/` が変わったときも) ページを読み直させる。
//!     エンジン (`engine/nodejs`) を開発するときは、`--web-runtime` で手元でビルドした Web runtime を使える
//!     (そのディレクトリも監視し、作り直されたらページを読み直させる)。
//!
//! どれも、最初のビルドが通らなければ始めない。コード生成のターゲットは wasm に固定している。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use biwac_base::Target;
use clap::{Args, ValueEnum};
use notify::{RecursiveMode, Watcher};

use crate::files;
use crate::project::Project;
use crate::serve::{DevServer, Site};
use crate::toolchain::{DesktopOs, DesktopTarget, Runtime, Toolchain};

/// 変更イベントが落ち着くのを待つ時間。
/// エディタは保存 1 回で複数のイベントを出すので、まとめてから 1 回だけ建て直す。
const DEBOUNCE: Duration = Duration::from_millis(120);

/// コード生成のターゲット。どの runtime も wasm を走らせる。
const TARGET: Target = Target::Wasm;

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

    /// runtime を取るツールチェイン (省略時は環境変数 BIWA_TOOLCHAIN、なければこの CLI と同じもの)
    #[arg(long, value_name = "NAME")]
    pub toolchain: Option<String>,

    /// `--platform web` で、ツールチェインの代わりにこのディレクトリの Web runtime を使う
    /// (エンジンの開発用。`engine/nodejs` の `npm run watch:runtime` の出力 `dist-runtime/` など)
    #[arg(long, value_name = "DIR")]
    pub web_runtime: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DevPlatform {
    /// この OS のウィンドウで、ネイティブの runtime で動かす
    Native,
    /// ブラウザで動かす (内蔵の HTTP サーバ)
    Web,
}

/// `--platform web` の開発サーバの既定のポート。塞がっていれば次を試す。
const DEFAULT_PORT: u16 = 5173;

pub fn run(args: DevArgs) -> Result<()> {
    let start_dir = args
        .package_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));
    let project = Project::discover(&start_dir)?;

    println!("Package: {} ({})", project.name, project.root.display());

    match args.platform {
        DevPlatform::Native => run_native(&project, &args),
        DevPlatform::Web => run_web(&project, &args),
    }
}

/// ネイティブの runtime で動かす。
fn run_native(project: &Project, args: &DevArgs) -> Result<()> {
    // runtime が揃っているかは、ビルドの前に確かめる。
    let toolchain = Toolchain::select(args.toolchain.as_deref())?;
    let runtime = toolchain.runtime(DesktopTarget::new(DesktopOs::host()?, Runtime::Native))?;
    println!("Toolchain: {}", toolchain.name);

    files::ensure_std(project)?;
    // 初回は通らないと動かす意味がないので、失敗したらそこで止める。
    compile(project, args.rebuild)?;

    // 終わったかを見る側と読み直させる側で共有する。
    let session = std::cell::RefCell::new(NativeSession::spawn(project, &runtime)?);
    watch_loop(
        project,
        &reload_dirs(project, None),
        || session.borrow_mut().try_wait(),
        |change| {
            if change == Change::Source {
                println!();
                if let Err(e) = compile(project, false) {
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
fn run_web(project: &Project, args: &DevArgs) -> Result<()> {
    // runtime が揃っているかは、ビルドの前に確かめる。
    let toolchain = Toolchain::select(args.toolchain.as_deref())?;
    let web_runtime = match &args.web_runtime {
        Some(dir) => {
            if !dir.join("index.html").is_file() {
                bail!("no web runtime at {} (expected index.html)", dir.display());
            }
            println!("Web runtime: {}", dir.display());
            dir.clone()
        }
        None => {
            println!("Toolchain: {}", toolchain.name);
            toolchain.web_runtime()?
        }
    };

    files::ensure_std(project)?;
    // 初回は通らないと動かす意味がないので、失敗したらそこで止める。
    compile(project, args.rebuild)?;

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
        wasm: compiled_wasm(project),
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

    // `--web-runtime` なら、それが作り直されたときも読み直させる (エンジンの開発)。
    watch_loop(
        project,
        &reload_dirs(project, args.web_runtime.as_deref()),
        || Ok(None),
        |change| {
            if change == Change::Source {
                println!();
                if let Err(e) = compile(project, false) {
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

/// コンパイラの出力。runtime はこれを `game.wasm` として読む。
fn compiled_wasm(project: &Project) -> PathBuf {
    project
        .generated_dir(TARGET)
        .join(format!("{}.{}", project.name, TARGET.bin_extension()))
}

/// コンパイラ本体の呼び出し。
///
/// biwac はライブラリとして呼ぶ (`biwac` コマンドは経由しない)。
/// まだ未実装のエラー表示経路で panic することがあるので、
/// 開発サーバごと落ちないようにここで受け止める。
pub(crate) fn compile(project: &Project, force_rebuild: bool) -> Result<()> {
    let root = project.root.clone();

    let result = std::panic::catch_unwind(move || {
        biwac_driver::compile(
            root,
            biwac_driver::BuildOptions {
                force_rebuild,
                // 中間表現は開発サーバの関心事ではない。
                emit_mir: false,
                target: TARGET,
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
    /// `src/` の `.biwa`。建て直す。
    Source,
    /// 読み直すだけの監視先 (`assets/` など) の中のどれか。
    Reload,
}

/// 読み直すだけの監視先。
struct ReloadDir {
    dir: PathBuf,
    /// 最後の変更からこれだけ静かになるまで待つ。
    quiet: Duration,
}

/// `assets/` と (あれば) `--web-runtime` のディレクトリ。
fn reload_dirs(project: &Project, web_runtime: Option<&Path>) -> Vec<ReloadDir> {
    let mut dirs = Vec::new();
    let assets = project.assets_dir();
    if assets.is_dir() {
        dirs.push(ReloadDir {
            dir: assets,
            quiet: DEBOUNCE,
        });
    }
    if let Some(dir) = web_runtime {
        // `vite build --watch` は出力をいったん空にしてから書き直すので、書き終わるまで待つ。
        dirs.push(ReloadDir {
            dir: dir.to_path_buf(),
            quiet: WEB_RUNTIME_QUIET,
        });
    }
    dirs
}

/// `--web-runtime` のディレクトリが静かになるまで待つ時間。
const WEB_RUNTIME_QUIET: Duration = Duration::from_millis(800);

/// `src/` と `reload_dirs` を監視し、変わるたびに `on_change` を呼ぶ。
///
/// `poll` が終了の状態を返したら (ウィンドウが閉じられた) こちらも終わる。
fn watch_loop(
    project: &Project,
    reload_dirs: &[ReloadDir],
    mut poll: impl FnMut() -> Result<Option<std::process::ExitStatus>>,
    mut on_change: impl FnMut(Change),
) -> Result<()> {
    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res| {
        // 受け手が居なくなっている (終了処理中) 場合は捨てる。
        let _ = tx.send(res);
    })
    .context("failed to start the file watcher")?;

    let dirs = files::watch_targets(project)
        .into_iter()
        .chain(reload_dirs.iter().map(|d| d.dir.clone()));
    for dir in dirs {
        watcher
            .watch(&dir, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch {}", dir.display()))?;
        println!("Watching {}", dir.display());
    }
    // 変更の種類と、静かになるまで待つ時間。
    let classify = |event: &notify::Event| -> Option<(Change, Duration)> {
        if touches_biwa_source(event) {
            return Some((Change::Source, DEBOUNCE));
        }
        reload_dirs
            .iter()
            .find(|d| touches_dir(event, &d.dir))
            .map(|d| (Change::Reload, d.quiet))
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

        let Some((mut change, mut quiet)) = classify(&event) else {
            continue;
        };

        // 続けて飛んでくるイベントを、静かになるまでまとめる。`.biwa` が 1 つでも変わっていれば建て直す。
        let mut deadline = Instant::now() + quiet;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    if let Some((c, q)) = classify(&event) {
                        if c == Change::Source {
                            change = Change::Source;
                        }
                        quiet = quiet.max(q);
                        deadline = Instant::now() + quiet;
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

/// 開発モードで動いているネイティブの runtime。CLI が先に終わっても道連れにする。
struct NativeSession {
    child: Child,
    /// 指示を送る口 (runtime の標準入力)。閉じると runtime は終わる。
    stdin: Option<ChildStdin>,
}

impl NativeSession {
    fn spawn(project: &Project, runtime: &Path) -> Result<Self> {
        let wasm = compiled_wasm(project);
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
