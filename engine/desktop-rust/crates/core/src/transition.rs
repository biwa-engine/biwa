//! 遷移 (transition) の番号と曲線。
//!
//! `engine/nodejs/src/engine/api/transition.ts` と同じ取り決めで、std の wasm 版はこの番号を `.wat` に直書きしている。
//! 設計は `docs/media-object-model.md`。

/// 遷移できるパラメータ。値はオブジェクトの状態の添字でもある。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    /// 画像の中心の x。canvas の中央が原点で右が正。canvas の範囲が -50..50 の単位。
    X = 0,
    /// 画像の中心の y。canvas の中央が原点で**上が正**。
    Y = 1,
    /// 幅。100 が canvas の幅。負なら「指定しない」(画像の元の大きさから決める)。
    W = 2,
    /// 高さ。100 が canvas の高さ。負なら「指定しない」。
    H = 3,
    /// 不透明度。0 (透明) 〜 255 (不透明)。
    Alpha = 4,
    /// 回転 [度]。回転の中心は画像の中心で、正は反時計回り。
    Theta = 5,
}

pub const PARAM_COUNT: usize = 6;

impl Param {
    pub fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            0 => Self::X,
            1 => Self::Y,
            2 => Self::W,
            3 => Self::H,
            4 => Self::Alpha,
            5 => Self::Theta,
            _ => return None,
        })
    }

    /// 遷移の値を `val_i` (整数) で受け取るか。alpha だけが整数で、他は `val_f`。
    pub fn is_integer(self) -> bool {
        self == Self::Alpha
    }
}

/// 遷移のかかり方。0〜15 が一度限り、16 以降が周期的 (終わらない)。
///
/// `EaseIn` は「ゆっくり始まって速く終わる」(CSS と同じ意味)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    /// 遷移しない。その時点の値で固定して終わる (周期系を止める手段でもある)。
    None = 0,
    Linear = 1,
    EaseIn = 2,
    EaseOut = 3,
    EaseInOut = 4,
    EaseOutIn = 5,
    Sin = 16,
    Triangle = 17,
    Square = 18,
    /// 0 から振幅まで線形に増えて戻る。
    Saw = 19,
}

impl Curve {
    pub fn from_raw(raw: i32) -> Option<Self> {
        Some(match raw {
            0 => Self::None,
            1 => Self::Linear,
            2 => Self::EaseIn,
            3 => Self::EaseOut,
            4 => Self::EaseInOut,
            5 => Self::EaseOutIn,
            16 => Self::Sin,
            17 => Self::Triangle,
            18 => Self::Square,
            19 => Self::Saw,
            _ => return None,
        })
    }

    /// 終端を持たない (= 完了しない) 遷移か。
    pub fn is_periodic(self) -> bool {
        self as i32 >= 16
    }

    /// 一度限りの遷移の進み具合。`p` は時間の比 (0〜1)、返すのは値の比 (0〜1)。
    pub fn at(self, p: f64) -> f64 {
        match self {
            Self::EaseIn => p * p * p,
            Self::EaseOut => 1.0 - (1.0 - p).powi(3),
            Self::EaseInOut => {
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    1.0 - (-2.0 * p + 2.0).powi(3) / 2.0
                }
            }
            Self::EaseOutIn => {
                if p < 0.5 {
                    (1.0 - (1.0 - 2.0 * p).powi(3)) / 2.0
                } else {
                    ((2.0 * p - 1.0).powi(3) + 1.0) / 2.0
                }
            }
            _ => p,
        }
    }

    /// 周期系の波形。`phase` は周期内の位置 (0 以上 1 未満)。どれも `phase = 0` で 0。
    pub fn wave(self, phase: f64) -> f64 {
        match self {
            Self::Sin => (std::f64::consts::TAU * phase).sin(),
            Self::Triangle => {
                if phase < 0.25 {
                    4.0 * phase
                } else if phase < 0.75 {
                    2.0 - 4.0 * phase
                } else {
                    4.0 * phase - 4.0
                }
            }
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::Saw => phase,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_zero_and_end_at_one() {
        for c in [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut, Curve::EaseOutIn] {
            assert!(c.at(0.0).abs() < 1e-9, "{c:?}");
            assert!((c.at(1.0) - 1.0).abs() < 1e-9, "{c:?}");
        }
        assert!(Curve::EaseIn.at(0.5) < 0.5);
        assert!(Curve::EaseOut.at(0.5) > 0.5);
    }

    #[test]
    fn waves_start_at_zero() {
        for c in [Curve::Sin, Curve::Triangle, Curve::Saw] {
            assert!(c.wave(0.0).abs() < 1e-9, "{c:?}");
        }
        assert!((Curve::Triangle.wave(0.25) - 1.0).abs() < 1e-9);
        assert!((Curve::Triangle.wave(0.75) + 1.0).abs() < 1e-9);
    }
}
