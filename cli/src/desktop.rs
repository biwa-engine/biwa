//! デスクトップ版の配布物を組み立てる。
//!
//! ビルド済みのプレイヤー (`player.rs`) にゲームのデータ (`game/`) を添えるだけで、
//! コンパイルは走らない。そのため、どの OS の上でも全 OS 向けの配布物を作れる。
//!
//! | OS      | 配布物                                                              |
//! | ------- | ------------------------------------------------------------------- |
//! | Windows | `<name>/<name>.exe` + `<name>/game/` を zip に                      |
//! | macOS   | `<name>.app` (`Contents/Resources/game/`) を ad-hoc 署名して zip に |
//! | Linux   | `<name>/<name>` + `<name>/game/` を tar.gz に                       |
//!
//! zip / tar.gz にするのは、実行ビットや .app の構造を、どの OS で固めても確実に残すためである
//! (CI の成果物やコピーの途中で実行ビットは落ちやすい)。

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::player::DesktopOs;
use crate::project::Project;
use crate::runtime;

/// プレイヤーが探すゲームのデータのディレクトリ名 (`engine/desktop-tauri/src-tauri/src/main.rs`)。
const GAME_DIR: &str = "game";
/// macOS の .app に入れるアイコン。
const MACOS_ICON: &[u8] = include_bytes!("../../engine/desktop-tauri/src-tauri/icons/icon.icns");
/// macOS の下限。WasmGC が要るので、Safari 18.2 相当の WebKit が入る版 (要確認)。
const MACOS_MINIMUM_VERSION: &str = "13.0";

/// `os` 向けの配布物を `dist/<os>/` に組み立て、固めたファイルのパスを返す。
pub fn assemble(
    project: &Project,
    os: DesktopOs,
    player: &Path,
    game_data: &Path,
) -> Result<PathBuf> {
    let out_dir = project.dist_dir().join(os.name());
    remove_if_exists(&out_dir)?;
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let name = &project.name;
    let archive_base = format!("{name}-{}", os.player_id());

    match os {
        DesktopOs::Windows | DesktopOs::Linux => {
            let app_dir = out_dir.join(name);
            let exe_name = format!(
                "{name}{}",
                if os == DesktopOs::Windows { ".exe" } else { "" }
            );
            copy_executable(player, &app_dir.join(&exe_name))?;
            runtime::copy_dir(game_data, &app_dir.join(GAME_DIR))?;

            let executables = [format!("{name}/{exe_name}")];
            if os == DesktopOs::Windows {
                let archive = out_dir.join(format!("{archive_base}.zip"));
                zip_dir(&out_dir, name, &archive, &executables)?;
                Ok(archive)
            } else {
                let archive = out_dir.join(format!("{archive_base}.tar.gz"));
                tar_gz_dir(&out_dir, name, &archive, &executables)?;
                Ok(archive)
            }
        }
        DesktopOs::Macos => {
            let bundle = format!("{name}.app");
            let app = out_dir.join(&bundle);
            let contents = app.join("Contents");
            copy_executable(player, &contents.join("MacOS").join(name))?;
            runtime::copy_dir(game_data, &contents.join("Resources").join(GAME_DIR))?;
            write(&contents.join("Resources").join("icon.icns"), MACOS_ICON)?;
            write(&contents.join("Info.plist"), info_plist(project).as_bytes())?;

            // データを足すと署名が合わなくなるので、組み立てたあとで .app ごと署名し直す。
            // 鍵を与えないので ad-hoc 署名になる (Apple Silicon で起動するのに最低限要る)。
            println!("Signing {bundle} (ad-hoc)");
            apple_codesign::BundleSigner::new_from_path(&app)
                .and_then(|signer| {
                    signer.write_signed_bundle(&app, &apple_codesign::SigningSettings::default())
                })
                .with_context(|| format!("failed to sign {}", app.display()))?;

            let archive = out_dir.join(format!("{archive_base}.zip"));
            let executables = [format!("{bundle}/Contents/MacOS/{name}")];
            zip_dir(&out_dir, &bundle, &archive, &executables)?;
            Ok(archive)
        }
    }
}

fn info_plist(project: &Project) -> String {
    let name = xml_escape(&project.name);
    let version = xml_escape(project.version.as_deref().unwrap_or("0.1.0"));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{name}</string>
  <key>CFBundleExecutable</key>
  <string>{name}</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleIdentifier</key>
  <string>{identifier}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>LSMinimumSystemVersion</key>
  <string>{minimum}</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#,
        identifier = identifier(&project.name),
        minimum = MACOS_MINIMUM_VERSION,
    )
}

/// アプリの識別子 (macOS の bundle identifier)。英数字・`-`・`.` 以外は `-` に寄せる。
fn identifier(package_name: &str) -> String {
    let name: String = package_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("dev.biwa.game.{name}")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn remove_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

/// プレイヤーをコピーし、(Unix の上なら) 実行ビットを立てる。
fn copy_executable(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to make {} executable", dst.display()))?;
    }
    Ok(())
}

/// `base_dir/<top>` 以下のファイルを、`<top>/...` という名前で並べる (ディレクトリは末尾 `/`)。
fn entries(base_dir: &Path, top: &str) -> Result<Vec<(String, PathBuf, bool)>> {
    let mut out = Vec::new();
    let mut stack = vec![(top.to_string(), base_dir.join(top))];
    while let Some((name, path)) = stack.pop() {
        if path.is_dir() {
            out.push((format!("{name}/"), path.clone(), true));
            let mut children: Vec<_> = std::fs::read_dir(&path)
                .with_context(|| format!("failed to read {}", path.display()))?
                .collect::<std::io::Result<_>>()?;
            children.sort_by_key(|e| e.file_name());
            for child in children.into_iter().rev() {
                let child_name = child.file_name().to_string_lossy().into_owned();
                stack.push((format!("{name}/{child_name}"), child.path()));
            }
        } else {
            out.push((name, path, false));
        }
    }
    Ok(out)
}

/// `base_dir/<top>` を zip に固める。`executables` に挙げたものには実行ビットを付ける。
fn zip_dir(base_dir: &Path, top: &str, archive: &Path, executables: &[String]) -> Result<()> {
    use zip::write::SimpleFileOptions;

    let file =
        File::create(archive).with_context(|| format!("failed to create {}", archive.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let mut base =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(now) = zip_now() {
        base = base.last_modified_time(now);
    }

    for (name, path, is_dir) in entries(base_dir, top)? {
        if is_dir {
            zip.add_directory(name.trim_end_matches('/'), base.unix_permissions(0o755))?;
            continue;
        }
        let mode = if executables.contains(&name) {
            0o755
        } else {
            0o644
        };
        zip.start_file(name, base.unix_permissions(mode))?;
        let data =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        zip.write_all(&data)?;
    }

    zip.finish()?;
    Ok(())
}

/// zip に書く現在時刻 (UTC)。zip の時刻は暦の形で持つので、UNIX 時刻から暦に直す。
///
/// 書かないと 1980-01-01 になり、展開したファイルの日付がそれになる。
fn zip_now() -> Option<zip::DateTime> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));

    // 日数から暦 (Howard Hinnant の civil_from_days)。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    zip::DateTime::from_date_and_time(
        u16::try_from(year).ok()?,
        month as u8,
        day as u8,
        (rem / 3600) as u8,
        (rem % 3600 / 60) as u8,
        (rem % 60) as u8,
    )
    .ok()
}

/// `base_dir/<top>` を tar.gz に固める。`executables` に挙げたものには実行ビットを付ける。
fn tar_gz_dir(base_dir: &Path, top: &str, archive: &Path, executables: &[String]) -> Result<()> {
    let file =
        File::create(archive).with_context(|| format!("failed to create {}", archive.display()))?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for (name, path, is_dir) in entries(base_dir, top)? {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(now);
        if is_dir {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            header.set_cksum();
            tar.append_data(&mut header, &name, std::io::empty())?;
            continue;
        }
        let data =
            std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(if executables.contains(&name) {
            0o755
        } else {
            0o644
        });
        header.set_size(data.len() as u64);
        header.set_cksum();
        tar.append_data(&mut header, &name, data.as_slice())?;
    }

    tar.into_inner()?.finish()?;
    Ok(())
}
