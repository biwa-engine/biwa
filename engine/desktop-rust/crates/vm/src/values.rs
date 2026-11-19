//! wasm の値のうち、実体をホストが持つもの (externref の中身)。
//!
//! 生成物は `String` / `Vec` / `Map` を externref として持ち、その操作を syscall で呼ぶ
//! (`sys_string_concat` / `sys_vec_*` / `sys_map_*`)。JS 版では実体が JS の値だったので
//! Worker の中でしか触れなかった。ここでは Rust の値を externref に入れる。
//!
//! Vec と Map の要素は anyref (struct・i31・externref を anyref にしたもの) で、
//! GC の対象なので [`OwnedRooted`] で持つ (持っている間は回収されない)。

use wasmtime::{AnyRef, AsContext, AsContextMut, ExternRef, OwnedRooted, Result, Rooted};

/// `Vec[T]` の実体。
pub(crate) struct HostVec(pub Vec<Option<OwnedRooted<AnyRef>>>);

/// `Map[K, V]` の実体。キーの比べ方は [`same_key`]。要素が少ない前提で線形に探す。
pub(crate) struct HostMap(pub Vec<(Option<OwnedRooted<AnyRef>>, Option<OwnedRooted<AnyRef>>)>);

/// 文字列を externref にする。
pub(crate) fn new_string(store: impl AsContextMut, text: String) -> Result<Rooted<ExternRef>> {
    ExternRef::new(store, text)
}

/// externref の文字列を取り出す。文字列でなければエラー。
pub(crate) fn string_of(store: impl AsContext, value: Option<Rooted<ExternRef>>) -> Result<String> {
    let value = value.ok_or_else(|| wasmtime::format_err!("expected a String, got null"))?;
    let data = value.data(&store)?;
    data.and_then(|d| d.downcast_ref::<String>())
        .cloned()
        .ok_or_else(|| wasmtime::format_err!("expected a String"))
}

/// anyref を、生成物の外 (Vec・Map の中) に持てる形にする。
pub(crate) fn own(
    store: impl AsContextMut,
    value: Option<Rooted<AnyRef>>,
) -> Result<Option<OwnedRooted<AnyRef>>> {
    value.map(|v| v.to_owned_rooted(store)).transpose()
}

/// Map のキーとして同じか。
///
/// JS の `Map` (SameValueZero) に合わせる: 文字列と数値 (i31) は値で、それ以外 (struct など) は参照で比べる。
pub(crate) fn same_key(
    mut store: impl AsContextMut,
    a: Option<Rooted<AnyRef>>,
    b: Option<Rooted<AnyRef>>,
) -> Result<bool> {
    let (a, b) = match (a, b) {
        (None, None) => return Ok(true),
        (Some(a), Some(b)) => (a, b),
        _ => return Ok(false),
    };
    if a.is_i31(&store)? || b.is_i31(&store)? {
        return Ok(a.as_i31(&store)?.map(|i| i.get_i32()) == b.as_i31(&store)?.map(|i| i.get_i32()));
    }
    if Rooted::ref_eq(&store, &a, &b)? {
        return Ok(true);
    }
    // 文字列は externref を anyref にしたものとして入っている。中身で比べる。
    let a = ExternRef::convert_any(&mut store, a)?;
    let b = ExternRef::convert_any(&mut store, b)?;
    let text = |r: &Rooted<ExternRef>| -> Result<Option<String>> {
        Ok(r.data(&store)?.and_then(|d| d.downcast_ref::<String>()).cloned())
    };
    Ok(matches!((text(&a)?, text(&b)?), (Some(x), Some(y)) if x == y))
}

/// `Float` を文字列にする。
///
/// TypeScript 版 (`String(Number(value.toPrecision(9)))`) と同じ結果にする。
/// f32 として来るので、そのまま 10 進にすると `0.30000001192092896` のような桁が出る。
/// f32 が表せる精度 (有効数字 9 桁) で丸めてから、最短の表記にする。
pub(crate) fn float_to_string(value: f32) -> String {
    let value = f64::from(value);
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let rounded: f64 = format!("{value:.8e}").parse().unwrap_or(value);
    if rounded == 0.0 {
        return "0".into();
    }
    let abs = rounded.abs();
    // JS の数の文字列化は、この範囲の外で指数表記になる。
    if !(1e-6..1e21).contains(&abs) {
        let s = format!("{rounded:e}");
        // Rust は `1e21`、JS は `1e+21`。
        return match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        };
    }
    format!("{rounded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期待値は node で `String(Number(Math.fround(v).toPrecision(9)))` を実行して得たもの。
    #[test]
    fn floats_like_javascript() {
        assert_eq!(float_to_string(0.3), "0.300000012");
        assert_eq!(float_to_string(5.0), "5");
        assert_eq!(float_to_string(-1.5), "-1.5");
        assert_eq!(float_to_string(0.25), "0.25");
        assert_eq!(float_to_string(1.0e21), "1.00000002e+21");
        assert_eq!(float_to_string(1.0e-7), "1.00000001e-7");
        assert_eq!(float_to_string(f32::NAN), "NaN");
    }
}
