//! ツールチェインの名前と、`biwa-package.json` の `toolchain` (要件)。
//!
//! 名前は 2 通りある。
//!   - 版: `0.1.0` (タグ `v0.1.0` の release)。新旧を比べられる。
//!   - それ以外: `nightly-<commit>` (コミットは先頭 12 文字)、手で置いた `local` など。新旧は比べられない。
//!
//! 版はまだ出ていない (2026-11 の時点では nightly だけ)。

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::Deserialize;

/// `<major>.<minor>.<patch>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl FromStr for Version {
    type Err = ();

    /// `0.1.0` と `v0.1.0` を受け付ける。
    fn from_str(s: &str) -> Result<Self, ()> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.');
        let mut next = || -> Result<u64, ()> {
            let p = parts.next().ok_or(())?;
            // 先頭の 0 (`01`) や符号は版として読まない。
            if p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
            {
                return Err(());
            }
            p.parse().map_err(|_| ())
        };
        let v = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
        };
        if parts.next().is_some() {
            return Err(());
        }
        Ok(v)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// ツールチェインの名前の種類。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainName {
    Version(Version),
    /// 版でない名前 (コミットのハッシュなど)。
    Other(String),
}

impl ToolchainName {
    pub fn parse(name: &str) -> Self {
        match name.parse() {
            Ok(v) => Self::Version(v),
            Err(()) => Self::Other(name.to_string()),
        }
    }
}

/// ディレクトリ名として使える名前か。
pub fn validate(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('-')
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        bail!("invalid toolchain name `{name}`");
    }
    Ok(())
}

/// `biwa-package.json` の `toolchain`。パッケージが要るツールチェイン。
///
/// ```json
/// "toolchain": "0.1.0"              // 短い書き方 (= { "min": "0.1.0" })
/// "toolchain": { "min": "0.1.0" }
/// "toolchain": "nightly-3f2a9c1b7d4e"       // ハッシュ: そのものが要る
/// ```
///
/// 将来 `max` (互換の上限) を足す予定 (`~/memo/biwa-toolchain-install-update.md`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub min: ToolchainName,
}

impl Requirement {
    /// 名前 `name` のツールチェインがこの要件を満たすか。
    ///
    /// 版なら `min` 以上、ハッシュなどは名前がそのまま一致するときだけ (新旧を比べられないため)。
    pub fn satisfied_by(&self, name: &str) -> bool {
        match (&self.min, ToolchainName::parse(name)) {
            (ToolchainName::Version(min), ToolchainName::Version(v)) => v >= *min,
            (ToolchainName::Other(want), _) => want == name,
            (ToolchainName::Version(_), ToolchainName::Other(_)) => false,
        }
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.min {
            ToolchainName::Version(v) => write!(f, ">= {v}"),
            ToolchainName::Other(s) => write!(f, "`{s}`"),
        }
    }
}

impl<'de> Deserialize<'de> for Requirement {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Short(String),
            Full { min: String },
        }
        let min = match Raw::deserialize(d)? {
            Raw::Short(s) | Raw::Full { min: s } => s,
        };
        validate(&min).map_err(serde::de::Error::custom)?;
        Ok(Self {
            min: ToolchainName::parse(&min),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(json: &str) -> Requirement {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            "v0.1.0".parse(),
            Ok(Version {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
        assert_eq!(
            "10.2.33".parse::<Version>().map(|v| v.to_string()),
            Ok("10.2.33".into())
        );
        for bad in [
            "0.1",
            "0.1.0.0",
            "01.1.0",
            "0.1.x",
            "",
            "nightly-3f2a9c1b7d4e",
            "v",
            "0.1.-1",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
        assert!(Version::from_str("0.10.0").unwrap() > Version::from_str("0.9.9").unwrap());
    }

    #[test]
    fn version_requirements_are_minimums() {
        let r = req(r#"{ "min": "v0.2.0" }"#);
        assert_eq!(r, req(r#""0.2.0""#));
        assert!(r.satisfied_by("0.2.0"));
        assert!(r.satisfied_by("1.0.0"));
        assert!(!r.satisfied_by("0.1.9"));
        // ハッシュの名前は版と比べられない。
        assert!(!r.satisfied_by("nightly-3f2a9c1b7d4e"));
    }

    #[test]
    fn hash_requirements_need_the_same_name() {
        let r = req(r#""nightly-3f2a9c1b7d4e""#);
        assert!(r.satisfied_by("nightly-3f2a9c1b7d4e"));
        assert!(!r.satisfied_by("nightly-aaaaaaaaaaaa"));
        assert!(!r.satisfied_by("0.1.0"));
    }

    #[test]
    fn rejects_bad_names() {
        assert!(serde_json::from_str::<Requirement>(r#""../x""#).is_err());
        assert!(serde_json::from_str::<Requirement>(r#"{ "max": "1.0.0" }"#).is_err());
    }
}
