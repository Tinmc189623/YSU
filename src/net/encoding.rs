//! 响应正文的字符集解码。
//!
//! 按 HTML 规范的嗅探顺序来：先看字节序标记，再看 `Content-Type` 里的
//! `charset`，最后在前一千字节里找 `<meta charset>`。三处都没有就按
//! UTF-8 处理，解码失败时用替换字符兜底。

use encoding_rs::{Encoding, UTF_8};

/// 嗅探时最多看多少字节的正文。
const SNIFF_LIMIT: usize = 1024;

/// 把一个响应体解码成文本。
///
/// `content_type` 是响应头里的 `Content-Type`，可以是 `None`。
pub fn decode(bytes: &[u8], content_type: Option<&str>) -> String {
    let encoding = sniff(bytes, content_type);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

/// 判断该用哪种编码。
pub fn sniff(bytes: &[u8], content_type: Option<&str>) -> &'static Encoding {
    if let Some(encoding) = from_bom(bytes) {
        return encoding;
    }
    if let Some(encoding) = content_type.and_then(charset_from_content_type) {
        return encoding;
    }
    if let Some(encoding) = charset_from_meta(bytes) {
        return encoding;
    }
    UTF_8
}

/// 按字节序标记判断编码。
fn from_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some(UTF_8);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Some(encoding_rs::UTF_16LE);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Some(encoding_rs::UTF_16BE);
    }
    None
}

/// 从 `Content-Type` 里取 `charset`。
fn charset_from_content_type(content_type: &str) -> Option<&'static Encoding> {
    let lower = content_type.to_ascii_lowercase();
    let index = lower.find("charset")?;
    encoding_after_charset(&content_type[index + "charset".len()..])
}

/// 从 `charset` 之后的文本里取出编码名。
///
/// `Content-Type` 与 `<meta>` 两种写法在 `charset` 之后的形式是一样的，
/// 都是等号加编码名，编码名可能带引号也可能不带，所以合并处理。
fn encoding_after_charset(rest: &str) -> Option<&'static Encoding> {
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let rest = rest.trim_start_matches(['"', '\'']);
    let label = read_label(rest);
    Encoding::for_label(label.as_bytes())
}

/// 在正文开头找 `<meta charset>` 或 `http-equiv` 里的 charset。
fn charset_from_meta(bytes: &[u8]) -> Option<&'static Encoding> {
    let limit = bytes.len().min(SNIFF_LIMIT);
    // 只按 ASCII 扫，非 ASCII 字节直接跳过。
    let head: String = bytes[..limit]
        .iter()
        .map(|byte| {
            if byte.is_ascii() {
                char::from(*byte)
            } else {
                '\u{fffd}'
            }
        })
        .collect();
    let lower = head.to_ascii_lowercase();
    let index = lower.find("charset")?;
    // `<meta charset="x">` 与 `content="text/html; charset=x"` 两种写法。
    encoding_after_charset(&head[index + "charset".len()..])
}

/// 从一段文本开头读出一个编码名。
///
/// 编码名只由字母、数字、减号和下划线组成。
fn read_label(text: &str) -> String {
    text.chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_is_decoded() {
        let text = "纯中文内容";
        assert_eq!(decode(text.as_bytes(), None), text);
    }

    #[test]
    fn bom_wins_over_content_type() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("中文".as_bytes());
        assert_eq!(decode(&bytes, Some("text/html; charset=gbk")), "中文");
    }

    #[test]
    fn content_type_charset_is_used() {
        // “中文”的 GBK 编码是 D6 D0 CE C4。
        let bytes = [0xD6, 0xD0, 0xCE, 0xC4];
        assert_eq!(decode(&bytes, Some("text/html; charset=gbk")), "中文");
        assert_eq!(decode(&bytes, Some("text/html; charset=GB2312")), "中文");
    }

    #[test]
    fn meta_charset_is_used_when_header_missing() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"<html><head><meta charset=\"gbk\"></head><body>");
        bytes.extend_from_slice(&[0xD6, 0xD0, 0xCE, 0xC4]);
        bytes.extend_from_slice(b"</body></html>");
        let text = decode(&bytes, None);
        assert!(text.contains("中文"), "实际内容：{text}");
    }

    #[test]
    fn meta_http_equiv_form_is_recognised() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=gbk\">",
        );
        bytes.extend_from_slice(&[0xD6, 0xD0, 0xCE, 0xC4]);
        let text = decode(&bytes, None);
        assert!(text.contains("中文"), "实际内容：{text}");
    }

    #[test]
    fn meta_beyond_the_sniff_window_is_ignored() {
        let mut bytes = vec![b' '; SNIFF_LIMIT + 50];
        bytes.extend_from_slice(b"<meta charset=\"gbk\">");
        bytes.extend_from_slice(&[0xD6, 0xD0, 0xCE, 0xC4]);
        // 编码声明在窗口之外，按 UTF-8 处理，GBK 字节会变成替换字符。
        let text = decode(&bytes, None);
        assert!(text.contains('\u{fffd}'));
    }

    #[test]
    fn unknown_charset_falls_back_to_utf8() {
        let text = "内容";
        assert_eq!(
            decode(text.as_bytes(), Some("text/html; charset=不存在的编码")),
            text
        );
    }

    #[test]
    fn charset_with_quotes_is_read() {
        let bytes = [0xD6, 0xD0, 0xCE, 0xC4];
        assert_eq!(decode(&bytes, Some("text/html; charset=\"gbk\"")), "中文");
    }

    #[test]
    fn utf16_with_bom_is_decoded() {
        let text = "中文";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes, None), text);
    }

    #[test]
    fn malformed_utf8_does_not_panic() {
        let bytes = [0xFF, 0xFE, 0xFD, 0x00, 0x80];
        let text = decode(&bytes, Some("text/html; charset=utf-8"));
        assert!(!text.is_empty());
    }

    #[test]
    fn empty_body_is_empty_text() {
        assert_eq!(decode(&[], None), "");
    }

    #[test]
    fn charset_label_without_equals_is_ignored() {
        // `charset` 后面没有等号时不当作编码声明。
        let text = "charset 不是声明";
        assert_eq!(decode(text.as_bytes(), None), text);
    }
}
