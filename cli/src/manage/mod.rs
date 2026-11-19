//! ツールチェインの管理 (issue #24): ランチャー、`biwa toolchain …`、`biwa update`、`biwa self update`。
//!
//! 実行ファイルは 1 つで、置き場所で役割が変わる。
//!   - `<home>/bin/biwa`: **ランチャー**。管理のコマンドは自分でこなし、それ以外 (`dev` / `build` …) は
//!     選んだツールチェインの `toolchains/<name>/bin/biwa` に引数を丸ごと渡す。
//!   - それ以外 (`toolchains/<name>/bin/biwa`、cargo でビルドしたもの): **実体**。
//!     管理のコマンドも使える (どちらも `<home>` を相手にするだけなので)。
//!
//! ランチャーが振り分けるコマンドの引数は解釈しない (`--toolchain` と `--package-path` を探すだけ)。
//! ランチャーとツールチェインの版が違っても、ツールチェインの側の引数がそのまま通るようにするためである。

pub mod home;
pub mod install;
pub mod name;
pub mod releases;

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use home::{Home, running_as_launcher};
use install::Installed;
use releases::Channel;

use crate::toolchain::NAME as THIS_TOOLCHAIN;

/// ランチャーが自分でこなすコマンド (それ以外は振り分ける)。
const MANAGEMENT_COMMANDS: &[&str] = &["toolchain", "update", "self", "help"];

#[derive(Debug, Subcommand)]
pub enum ToolchainCommand {
    /// インストール済みのツールチェインを並べる
    List,

    /// ツールチェインを入れる
    ///
    /// SPEC: `stable` / `nightly` (それぞれの最新)、`0.1.0` (版)、コミットのハッシュ (nightly)。
    /// 省略すると最新の版 (版がまだ無ければ最新の nightly)。
    Install {
        #[arg(value_name = "SPEC", conflicts_with = "archive")]
        spec: Option<String>,
        /// 配布物ではなく手元の圧縮ファイル (`biwa-<name>-<host>.{tar.gz,zip}`) から入れる
        #[arg(long, value_name = "FILE")]
        archive: Option<PathBuf>,
        /// 入れたものを既定にする (既定がまだ無ければ言わなくてもそうなる)
        #[arg(long)]
        default: bool,
        /// 同じ名前のものが既にあっても入れ直す
        #[arg(long)]
        force: bool,
    },

    /// 既定のツールチェインを見る・決める
    Default {
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },

    /// ツールチェインを消す
    Remove {
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SelfCommand {
    /// ランチャー (`~/.biwa/bin/biwa`) を最新にする (ツールチェインは変えない)
    Update,
}

/// ランチャーとして起動されて、振り分けるコマンドなら振り分ける。終了コードを返す。
/// 自分でこなすべきとき (ランチャーでない・管理のコマンド・引数なし) は `None`。
pub fn forward_if_launcher() -> Option<Result<i32>> {
    let home = Home::locate().ok()?;
    install::cleanup_old_launcher(&home);
    if !running_as_launcher(&home) {
        return None;
    }
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let command = args.first().and_then(|a| a.to_str())?;
    if command.starts_with('-') || MANAGEMENT_COMMANDS.contains(&command) {
        return None;
    }
    Some(forward(&home, &args))
}

fn forward(home: &Home, args: &[OsString]) -> Result<i32> {
    let explicit = option_value(args, "--toolchain", None).or_else(|| {
        std::env::var("BIWA_TOOLCHAIN")
            .ok()
            .filter(|v| !v.is_empty())
    });
    let start = option_value(args, "--package-path", Some("-p"))
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    // パッケージの外 (`biwa publish --help` など) では要件は無い。
    let requirement = crate::project::find_requirement(&start)?;
    let name = home.select(explicit.as_deref(), requirement.as_ref())?;
    let exe = home.toolchain_exe(&name);
    if !exe.is_file() {
        bail!(
            "the Biwa toolchain `{name}` has no CLI ({} is missing); reinstall it with `biwa toolchain install {name} --force`",
            exe.display()
        );
    }

    let mut command = std::process::Command::new(&exe);
    command.args(args).env("BIWA_TOOLCHAIN", &name);
    #[cfg(unix)]
    {
        // シグナルや終了コードをそのまま伝えるため、プロセスを置き換える。
        use std::os::unix::process::CommandExt;
        let e = command.exec();
        Err(e).with_context(|| format!("failed to run {}", exe.display()))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("failed to run {}", exe.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// 引数の列から `--name VALUE` / `--name=VALUE` (と短い名前) の値を探す。
fn option_value(args: &[OsString], long: &str, short: Option<&str>) -> Option<String> {
    let mut iter = args.iter().filter_map(|a| a.to_str());
    while let Some(arg) = iter.next() {
        if arg == "--" {
            break;
        }
        if arg == long || Some(arg) == short {
            return iter.next().map(str::to_string);
        }
        if let Some(v) = arg.strip_prefix(long).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

pub fn run_toolchain(command: ToolchainCommand) -> Result<()> {
    let home = Home::locate()?;
    match command {
        ToolchainCommand::List => {
            let default = home.default_toolchain()?;
            let names = home.installed();
            if names.is_empty() {
                println!("no toolchains are installed (install one with `biwa toolchain install`)");
            }
            for name in names {
                let mut notes = Vec::new();
                if default.as_deref() == Some(name.as_str()) {
                    notes.push("default");
                }
                if name == THIS_TOOLCHAIN {
                    notes.push("this CLI");
                }
                if !home.toolchain_exe(&name).is_file() {
                    notes.push("runtime only");
                }
                if notes.is_empty() {
                    println!("{name}");
                } else {
                    println!("{name} ({})", notes.join(", "));
                }
            }
            Ok(())
        }
        ToolchainCommand::Install {
            spec,
            archive,
            default,
            force,
        } => {
            let installed = match archive {
                Some(path) => install::from_archive(&home, &path, None, force)?,
                None => {
                    let client = releases::client()?;
                    let list = releases::list(&client)?;
                    let release = releases::find(&list, spec.as_deref())?;
                    install::from_release(&home, &client, release, force)?
                }
            };
            report_installed(&installed);
            after_install(&home, installed.name(), default)
        }
        ToolchainCommand::Default { name: None } => {
            match home.default_toolchain()? {
                Some(d) => println!("{d}"),
                None => println!("no default toolchain is set"),
            }
            Ok(())
        }
        ToolchainCommand::Default { name: Some(name) } => {
            name::validate(&name)?;
            if !home.is_installed(&name) {
                bail!("{}", home.not_installed(&name));
            }
            home.set_default(&name)?;
            println!("the default toolchain is now `{name}`");
            Ok(())
        }
        ToolchainCommand::Remove { name } => {
            name::validate(&name)?;
            if !home.is_installed(&name) {
                bail!("{}", home.not_installed(&name));
            }
            if home.default_toolchain()?.as_deref() == Some(name.as_str()) {
                bail!(
                    "`{name}` is the default toolchain; pick another one with `biwa toolchain default <NAME>` first"
                );
            }
            let dir = home.toolchain_dir(&name);
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("failed to remove {} (is it in use?)", dir.display()))?;
            println!("removed `{name}`");
            Ok(())
        }
    }
}

fn report_installed(installed: &Installed) {
    match installed {
        Installed::New(n) => println!("installed the toolchain `{n}`"),
        Installed::AlreadyThere(n) => {
            println!("the toolchain `{n}` is already installed (reinstall it with --force)")
        }
    }
}

/// 入れたあと: 既定を決め、ランチャーが無ければ置く。
fn after_install(home: &Home, name: &str, make_default: bool) -> Result<()> {
    if make_default || home.default_toolchain()?.is_none() {
        home.set_default(name)?;
        println!("the default toolchain is now `{name}`");
    }
    if !home.launcher().is_file() {
        install::replace_launcher(home, &home.toolchain_exe(name))?;
        println!("installed the launcher at {}", home.launcher().display());
    }
    Ok(())
}

/// `biwa update`: 既定のツールチェインのチャンネルの最新を入れて既定にし、ランチャーもそれにする。
pub fn run_update() -> Result<()> {
    let home = Home::locate()?;
    let client = releases::client()?;
    let list = releases::list(&client)?;
    let current = home.default_toolchain()?;
    let release = match current.as_deref() {
        Some(c) => releases::latest(&list, Channel::of(c))
            .with_context(|| format!("no release is available on the channel of `{c}`"))?,
        None => releases::find(&list, None)?,
    };
    let installed = install::from_release(&home, &client, release, false)?;
    let name = installed.name().to_string();
    if current.as_deref() == Some(name.as_str()) && matches!(installed, Installed::AlreadyThere(_))
    {
        println!("the toolchain `{name}` is up to date");
    } else {
        report_installed(&installed);
        home.set_default(&name)?;
        println!("the default toolchain is now `{name}`");
    }
    install::replace_launcher(&home, &home.toolchain_exe(&name))?;
    Ok(())
}

/// `biwa self update`: ランチャーだけを、既定のツールチェインのチャンネルの最新の CLI にする。
pub fn run_self(command: SelfCommand) -> Result<()> {
    match command {
        SelfCommand::Update => {
            let home = Home::locate()?;
            let client = releases::client()?;
            let list = releases::list(&client)?;
            let release = match home.default_toolchain()? {
                Some(c) => releases::latest(&list, Channel::of(&c))
                    .with_context(|| format!("no release is available on the channel of `{c}`"))?,
                None => releases::find(&list, None)?,
            };
            install::launcher_from_release(&home, &client, release)?;
            println!(
                "updated the launcher at {} (from `{}`)",
                home.launcher().display(),
                release.toolchain_name().unwrap_or_default()
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_option_values() {
        let args: Vec<OsString> = ["build", "--platform", "web", "--toolchain=abc", "-p", "x"]
            .map(Into::into)
            .to_vec();
        assert_eq!(
            option_value(&args, "--toolchain", None).as_deref(),
            Some("abc")
        );
        assert_eq!(
            option_value(&args, "--package-path", Some("-p")).as_deref(),
            Some("x")
        );
        let args: Vec<OsString> = ["dev", "--toolchain", "0.1.0"].map(Into::into).to_vec();
        assert_eq!(
            option_value(&args, "--toolchain", None).as_deref(),
            Some("0.1.0")
        );
        assert_eq!(option_value(&args, "--package-path", Some("-p")), None);
    }
}
