/// ソース文字列を「通常コード」と「ノベルモード」の区間に分割する前処理。
///
/// 仕様:
/// - `//` 以降(文字列の外)はコメント → ノベルモード検出対象外
/// - `"..."` の中はスキャン対象外
/// - コード部分で `{{` を発見したら、次に行頭(インデント後)が `}}` で始まる行まで
///   ノベルモード区間とする
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 通常のコード区間。`src[start..end]`
    Code { start: usize, end: usize },
    /// ノベルモード区間 ({{ から }} の直後まで、}} 行を含む)。
    Novel { start: usize, end: usize },
}

pub fn pre_scan(src: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut seg_start = 0;

    while pos < len {
        // 文字列リテラルをスキップ
        if bytes[pos] == b'"' {
            pos += 1;
            while pos < len && bytes[pos] != b'"' {
                pos += 1;
            }
            if pos < len {
                pos += 1; // closing "
            }
            continue;
        }

        // 行コメントをスキップ (行末まで)
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        // `{{` を発見したらノベルモード開始
        if pos + 1 < len && bytes[pos] == b'{' && bytes[pos + 1] == b'{' {
            // {{ の直前までをコードセグメントとして記録
            if seg_start < pos {
                segments.push(Segment::Code {
                    start: seg_start,
                    end: pos,
                });
            }
            let novel_start = pos;
            pos += 2; // skip {{

            // 行頭(空白トリム後)が `}}` で始まる行を探す
            loop {
                // 行頭に移動
                // まず現在の行末まで進む
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                if pos >= len {
                    // EOF: ノベルモードが閉じられていない。末尾までをnovelとする
                    segments.push(Segment::Novel {
                        start: novel_start,
                        end: len,
                    });
                    return segments;
                }
                pos += 1; // skip \n → 次行の先頭

                // 行頭の空白をスキップ
                let line_start = pos;
                while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                    pos += 1;
                }

                // `}}` で始まる行か確認
                if pos + 1 < len && bytes[pos] == b'}' && bytes[pos + 1] == b'}' {
                    // `}}` の後ろ(行末まで)も含めてノベルモード区間に入れる
                    pos += 2; // skip }}
                    while pos < len && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                    if pos < len {
                        pos += 1; // skip \n
                    }
                    segments.push(Segment::Novel {
                        start: novel_start,
                        end: pos,
                    });
                    seg_start = pos;
                    break;
                }
                // `}}` でなければ次の行へ
                let _ = line_start;
            }
            continue;
        }

        pos += 1;
    }

    // 残りのコード区間
    if seg_start < len {
        segments.push(Segment::Code {
            start: seg_start,
            end: len,
        });
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_code_only() {
        let src = "fn foo() -> Int { 1 }";
        let segs = pre_scan(src);
        assert_eq!(
            segs,
            vec![Segment::Code {
                start: 0,
                end: src.len()
            }]
        );
    }

    #[test]
    fn scene_with_novel_mode() {
        let src = "scene foo(g: MyGame) -> MyGame {{\n  Hello!\n}}\n";
        let segs = pre_scan(src);
        // code part before {{ + novel part
        assert_eq!(segs.len(), 2);
        assert!(matches!(segs[0], Segment::Code { .. }));
        assert!(matches!(segs[1], Segment::Novel { .. }));
    }

    #[test]
    fn code_then_scene() {
        let src = "fn a() {}\nscene s() -> G {{\n  text\n}}\nfn b() {}";
        let segs = pre_scan(src);
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[0], Segment::Code { .. }));
        assert!(matches!(segs[1], Segment::Novel { .. }));
        assert!(matches!(segs[2], Segment::Code { .. }));
    }

    #[test]
    fn double_brace_in_string_not_novel() {
        let src = r#"let x = "{{not novel}}";"#;
        let segs = pre_scan(src);
        assert_eq!(
            segs,
            vec![Segment::Code {
                start: 0,
                end: src.len()
            }]
        );
    }

    #[test]
    fn double_brace_after_comment_not_novel() {
        let src = "// {{\nfn foo() {}";
        let segs = pre_scan(src);
        assert_eq!(
            segs,
            vec![Segment::Code {
                start: 0,
                end: src.len()
            }]
        );
    }
}
