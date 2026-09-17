//! 字符引用的解码。
//!
//! 覆盖具名引用与数字引用两种形式，按 HTML 规范的规则处理无效码位、
//! 历史遗留的无分号形式，以及属性值里那条「后面跟等号或字母数字就
//! 不解码」的特例。

use super::entities::NAMED_REFERENCES;

/// 具名字符引用的查找结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedReference {
    /// 解码出来的文本，可能是两个码位，比如 `&NotEqualTilde;`。
    pub characters: String,
    /// 从 `&` 之后开始算，这段引用占用的字节数，带分号的形式含分号。
    pub consumed: usize,
    /// 名字后面是否带了分号。
    pub has_semicolon: bool,
}

/// 表里最长的名字长度，用来限定查找时的尝试范围。
const MAX_NAME_LEN: usize = 33;

/// 在 `&` 之后的部分里查找最长的具名字符引用。
///
/// `input` 是 `&` 之后的全部剩余文本。返回 `None` 表示没有任何引用匹配，
/// 此时调用方应当把 `&` 当作普通字符处理。
pub fn lookup_named(input: &str) -> Option<NamedReference> {
    let limit = input.len().min(MAX_NAME_LEN);
    // 从最长的前缀开始试，第一个命中就是最长匹配。
    let mut len = limit;
    while len > 0 {
        if !input.is_char_boundary(len) {
            len -= 1;
            continue;
        }
        let candidate = &input[..len];
        if let Ok(index) = NAMED_REFERENCES.binary_search_by(|entry| entry.0.cmp(candidate)) {
            let (name, characters) = NAMED_REFERENCES[index];
            return Some(NamedReference {
                characters: characters.to_string(),
                // 表里的键就是要去掉的字节数，带分号的形式分号也在键里。
                consumed: name.len(),
                has_semicolon: name.ends_with(';'),
            });
        }
        len -= 1;
    }
    None
}

/// 数字字符引用的解码结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericReference {
    /// 解码出来的字符，无效码位会换成替换字符。
    pub character: char,
    /// 从 `&` 之后开始算，这段引用占用的字节数，带分号的形式含分号。
    pub consumed: usize,
    /// 引用后面是否带了分号。
    pub has_semicolon: bool,
}

/// 解码 `&#` 开头的数字字符引用。
///
/// `input` 是 `&#` 之后的剩余文本，进制前缀 `x` 或 `X` 由本函数自行识别。
/// 一个数字都没有时返回 `None`，调用方应当按解析错误处理并原样输出。
pub fn lookup_numeric(input: &str) -> Option<NumericReference> {
    let (digits, radix) = match input.chars().next() {
        Some('x') | Some('X') => (&input[1..], 16),
        _ => (input, 10),
    };

    let mut value: u32 = 0;
    let mut used = 0usize;
    for (index, character) in digits.char_indices() {
        let Some(digit) = character.to_digit(radix) else {
            break;
        };
        value = value.saturating_mul(radix);
        value = value.saturating_add(digit);
        used = index + character.len_utf8();
    }
    if used == 0 {
        return None;
    }

    let prefix_len = input.len() - digits.len();
    let has_semicolon = digits[used..].starts_with(';');
    let consumed = prefix_len + used + usize::from(has_semicolon);
    Some(NumericReference {
        character: sanitize_code_point(value),
        consumed,
        has_semicolon,
    })
}

/// 按规范修正数字引用解出来的码位。
///
/// 空字符、代理对范围与超出 Unicode 的码位一律换成替换字符；0x80 到 0x9F
/// 这一段是 Windows-1252 的历史遗留映射，规范要求照该表替换。
fn sanitize_code_point(value: u32) -> char {
    if value == 0 || value > 0x10_FFFF || (0xD800..=0xDFFF).contains(&value) {
        return '\u{FFFD}';
    }
    if (0x80..=0x9F).contains(&value) {
        return C1_REPLACEMENTS[(value - 0x80) as usize];
    }
    char::from_u32(value).unwrap_or('\u{FFFD}')
}

/// 0x80 到 0x9F 的 Windows-1252 映射表。
static C1_REPLACEMENTS: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_with_semicolon() {
        let reference = lookup_named("amp;rest").expect("应当匹配到 amp;");
        assert_eq!(reference.characters, "&");
        // 消耗掉 "amp;" 四个字节。
        assert_eq!(reference.consumed, 4);
        assert!(reference.has_semicolon);
    }

    #[test]
    fn named_longest_match_wins() {
        // `&notin;` 比 `&not` 长，应当优先匹配前者。
        let reference = lookup_named("notin;x").expect("应当匹配到 notin;");
        assert_eq!(reference.characters, "\u{2209}");
        assert!(reference.has_semicolon);
    }

    #[test]
    fn named_legacy_without_semicolon() {
        let reference = lookup_named("copy more").expect("应当匹配到 copy");
        assert_eq!(reference.characters, "©");
        assert!(!reference.has_semicolon);
        // 没有分号，只消耗名字本身。
        assert_eq!(reference.consumed, 4);
    }

    #[test]
    fn named_multi_codepoint() {
        let reference = lookup_named("NotEqualTilde;").expect("应当匹配到 NotEqualTilde;");
        assert_eq!(reference.characters.chars().count(), 2);
    }

    #[test]
    fn named_unknown_returns_none() {
        assert!(lookup_named("nope;").is_none());
        assert!(lookup_named("").is_none());
    }

    #[test]
    fn numeric_decimal() {
        let reference = lookup_numeric("65;").expect("应当解析出数字");
        assert_eq!(reference.character, 'A');
        assert!(reference.has_semicolon);
        assert_eq!(reference.consumed, 3);
    }

    #[test]
    fn numeric_hex() {
        let reference = lookup_numeric("x41;").expect("应当解析出数字");
        assert_eq!(reference.character, 'A');
        assert_eq!(reference.consumed, 4);
    }

    #[test]
    fn numeric_hex_without_semicolon() {
        let reference = lookup_numeric("x41").expect("应当解析出数字");
        assert_eq!(reference.character, 'A');
        assert!(!reference.has_semicolon);
        assert_eq!(reference.consumed, 3);
    }

    #[test]
    fn numeric_invalid_code_points_become_replacement() {
        assert_eq!(lookup_numeric("0;").expect("有数字").character, '\u{FFFD}');
        assert_eq!(
            lookup_numeric("xD800;").expect("有数字").character,
            '\u{FFFD}'
        );
        assert_eq!(
            lookup_numeric("x110000;").expect("有数字").character,
            '\u{FFFD}'
        );
    }

    #[test]
    fn numeric_c1_range_is_remapped() {
        // 0x80 按 Windows-1252 是欧元符号。
        assert_eq!(
            lookup_numeric("x80;").expect("有数字").character,
            '\u{20AC}'
        );
        assert_eq!(
            lookup_numeric("x99;").expect("有数字").character,
            '\u{2122}'
        );
    }

    #[test]
    fn numeric_without_digits_returns_none() {
        assert!(lookup_numeric(";").is_none());
        assert!(lookup_numeric("").is_none());
        assert!(lookup_numeric("x;").is_none());
    }

    #[test]
    fn numeric_uppercase_hex_prefix() {
        let reference = lookup_numeric("X41;").expect("应当解析出数字");
        assert_eq!(reference.character, 'A');
    }

    #[test]
    fn table_is_sorted_for_binary_search() {
        // 二分查找要求表严格有序，这里守一道。
        for window in NAMED_REFERENCES.windows(2) {
            assert!(
                window[0].0 < window[1].0,
                "表里 {} 与 {} 的顺序不对",
                window[0].0,
                window[1].0
            );
        }
    }

    #[test]
    fn table_has_no_ampersand_prefix() {
        for (name, _) in NAMED_REFERENCES {
            assert!(!name.starts_with('&'), "表里的名字不该带 & 前缀: {name}");
        }
    }
}
