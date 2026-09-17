//! CSS 词法分析。
//!
//! 按 CSS 语法规范切分输入。这里产出的是记号流，规则与声明怎么组织由
//! 解析器决定。注释在词法阶段就丢掉了。

use std::fmt;

/// CSS 记号。
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// 标识符。
    Ident(String),
    /// 函数名，后面紧跟 `(`。
    Function(String),
    /// `@` 开头的关键字。
    AtKeyword(String),
    /// `@charset `，只允许出现在样式表最开头，用来声明这份样式表的字符集。
    ///
    /// 它单独成一个记号而不是 AtKeyword：规范 G.2 里它是带尾空格的字面量
    /// `"@charset "`，而且它有别于普通 at 规则的语义——不是条件组，是编码声明。
    Charset,
    /// `#` 开头的值，布尔量表示它是否可能是个 id 选择器。
    Hash(String, bool),
    /// 字符串，引号已剥掉。
    String(String),
    /// 遇到未转义的换行而收尾的字符串，规范里叫 BAD_STRING。
    ///
    /// 它不是普通的字符串：§4.2 要求把含它的那条声明整条丢掉，所以得让它
    /// 一路传到声明级解析那里，中途不能被当成值用掉。
    BadString,
    /// `url(...)` 里的地址。
    Url(String),
    /// 含非法字符或坏字符串的 `url(...)`，规范里叫 BAD_URI，同样是整条声明作废。
    BadUrl,
    /// 数字，布尔量表示源码里是否写出了小数点或指数。
    Number {
        /// 数值。
        value: f64,
        /// 是否是整数写法。
        integer: bool,
    },
    /// 百分比，已经除以一百。
    Percentage(f64),
    /// 带单位的数字。
    Dimension {
        /// 数值。
        value: f64,
        /// 是否是整数写法。
        integer: bool,
        /// 单位，已转成小写。
        unit: String,
    },
    /// 一段空白。
    Whitespace,
    /// `<!--`
    Cdo,
    /// `-->`
    Cdc,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `[`
    OpenSquare,
    /// `]`
    CloseSquare,
    /// `(`
    OpenParen,
    /// `)`
    CloseParen,
    /// `{`
    OpenCurly,
    /// `}`
    CloseCurly,
    /// 单个字符，不属于上面任何一类。
    Delim(char),
    /// 输入结束。
    Eof,
}

impl Token {
    /// 该记号是否是一段空白。
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Self::Whitespace)
    }

    /// 该记号是否可以直接丢掉，不参与解析。
    pub fn is_ignorable(&self) -> bool {
        matches!(self, Self::Whitespace | Self::Cdo | Self::Cdc)
    }
}

impl fmt::Display for Token {
    /// 输出适合放进错误信息的简短描述。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ident(name) => write!(f, "标识符 {name}"),
            Self::Function(name) => write!(f, "函数 {name}("),
            Self::AtKeyword(name) => write!(f, "@{name}"),
            Self::Charset => f.write_str("@charset"),
            Self::Hash(value, _) => write!(f, "#{value}"),
            Self::String(value) => write!(f, "字符串 {value:?}"),
            Self::BadString => f.write_str("未收尾的字符串"),
            Self::Url(value) => write!(f, "地址 {value}"),
            Self::BadUrl => f.write_str("非法的地址"),
            Self::Number { value, .. } => write!(f, "数字 {value}"),
            Self::Percentage(value) => write!(f, "百分比 {value}"),
            Self::Dimension { value, unit, .. } => write!(f, "尺寸 {value}{unit}"),
            Self::Whitespace => f.write_str("空白"),
            Self::Cdo => f.write_str("<!--"),
            Self::Cdc => f.write_str("-->"),
            Self::Colon => f.write_str(":"),
            Self::Semicolon => f.write_str(";"),
            Self::Comma => f.write_str(","),
            Self::OpenSquare => f.write_str("["),
            Self::CloseSquare => f.write_str("]"),
            Self::OpenParen => f.write_str("("),
            Self::CloseParen => f.write_str(")"),
            Self::OpenCurly => f.write_str("{"),
            Self::CloseCurly => f.write_str("}"),
            Self::Delim(character) => write!(f, "{character}"),
            Self::Eof => f.write_str("输入结束"),
        }
    }
}

/// CSS 词法分析器。
pub struct Tokenizer<'a> {
    /// 完整输入。
    input: &'a str,
    /// 当前字节位置。
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    /// 基于一段 CSS 文本构造词法器。
    pub fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    /// 当前字节位置。
    pub fn position(&self) -> usize {
        self.pos
    }

    /// 产出下一个记号。
    pub fn next_token(&mut self) -> Token {
        self.skip_comments();
        let Some(character) = self.peek() else {
            return Token::Eof;
        };

        if character.is_ascii_whitespace() {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.bump();
            }
            return Token::Whitespace;
        }

        match character {
            '"' | '\'' => {
                let (value, bad) = self.consume_string(character);
                if bad {
                    Token::BadString
                } else {
                    Token::String(value)
                }
            }
            '#' => self.consume_hash(),
            '(' => {
                self.bump();
                Token::OpenParen
            }
            ')' => {
                self.bump();
                Token::CloseParen
            }
            '[' => {
                self.bump();
                Token::OpenSquare
            }
            ']' => {
                self.bump();
                Token::CloseSquare
            }
            '{' => {
                self.bump();
                Token::OpenCurly
            }
            '}' => {
                self.bump();
                Token::CloseCurly
            }
            ',' => {
                self.bump();
                Token::Comma
            }
            ':' => {
                self.bump();
                Token::Colon
            }
            ';' => {
                self.bump();
                Token::Semicolon
            }
            '@' => {
                self.bump();
                let name = self.consume_name();
                if name.is_empty() {
                    Token::Delim('@')
                } else if name.eq_ignore_ascii_case("charset") {
                    // 规范 G.2 把它写成带尾空格的字面量 `"@charset "`，这里不强制
                    // 那个空格：少写一个空格在多数字样表里也无害，认出来比不认好。
                    Token::Charset
                } else {
                    Token::AtKeyword(name.to_ascii_lowercase())
                }
            }
            '<' if self.rest().starts_with("<!--") => {
                self.pos += 4;
                Token::Cdo
            }
            '-' if self.rest().starts_with("-->") => {
                self.pos += 3;
                Token::Cdc
            }
            _ => {
                if self.starts_number() {
                    return self.consume_numeric();
                }
                if self.starts_identifier() {
                    let name = self.consume_name();
                    // 标识符后面紧跟 `(` 时是函数记号。
                    if self.peek() == Some('(') {
                        self.bump();
                        if name.eq_ignore_ascii_case("url") {
                            return self.consume_url();
                        }
                        return Token::Function(name.to_ascii_lowercase());
                    }
                    return Token::Ident(name);
                }
                self.bump();
                Token::Delim(character)
            }
        }
    }

    // ---- 位置访问 ----

    /// 剩余输入。
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    /// 当前位置的字符。
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// 往后第 n 个字符。
    fn peek_nth(&self, index: usize) -> Option<char> {
        self.rest().chars().nth(index)
    }

    /// 前进一个字符。
    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.pos += character.len_utf8();
        Some(character)
    }

    /// 跳过注释。
    fn skip_comments(&mut self) {
        while self.rest().starts_with("/*") {
            match self.rest().find("*/") {
                Some(end) => self.pos += end + 2,
                None => {
                    self.pos = self.input.len();
                    return;
                }
            }
        }
    }

    /// 当前位置是否是一个标识符的开头。
    ///
    /// 规范的 nmstart 是 `[_a-z]|{nonascii}|{escape}`，所以转义也能起头——
    /// `.\35 5ft` 这种以数字开头的类名全靠这一支。
    fn starts_identifier(&self) -> bool {
        match self.peek() {
            Some('-') => match self.peek_nth(1) {
                Some(next) => {
                    next.is_ascii_alphabetic()
                        || next == '-'
                        || next == '_'
                        || !next.is_ascii()
                        || next == '\\'
                }
                None => false,
            },
            Some('\\') => self.starts_escape(),
            Some(character) => {
                character.is_ascii_alphabetic() || character == '_' || !character.is_ascii()
            }
            None => false,
        }
    }

    /// 当前位置的反斜杠是否构成一个转义的开头。
    ///
    /// 规范的 escape 是 `{unicode}|\\[^\n\r\f0-9a-f]`，两条都不允许后面跟换行。
    fn starts_escape(&self) -> bool {
        matches!(self.peek_nth(1), Some(next) if !matches!(next, '\n' | '\r' | '\x0c'))
    }

    /// 读一个转义序列并返回它代表的字符，当前位置必须是反斜杠。
    ///
    /// 转义有两种形状：一是 `\` 加一至六个十六进制数字，后面可以跟一个空白
    /// 作分隔（那个空白是给下一个字符让路的，要吃掉）；二是 `\` 加任意一个
    /// 不是换行也不是十六进制数字的字符，那个字符就是它的值。
    ///
    /// 十六进制那一支里，码点为 0、超出 Unicode 上限、或落在代理区间的，按
    /// U+FFFD 处理。规范要求这样，而且这几个值本来就装不进 Rust 的 `char`。
    /// 返回 `None` 表示这个反斜杠后面没有可用的内容。
    fn read_escape(&mut self) -> Option<char> {
        self.bump();
        let mut digits = String::new();
        while digits.len() < 6 {
            match self.peek() {
                Some(character) if character.is_ascii_hexdigit() => {
                    digits.push(character);
                    self.bump();
                }
                _ => break,
            }
        }
        if digits.is_empty() {
            return match self.peek() {
                Some(character) if !matches!(character, '\n' | '\r' | '\x0c') => {
                    self.bump();
                    Some(character)
                }
                _ => None,
            };
        }
        // 十六进制写法后面允许跟一个空白作分隔，吃掉它。
        if self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.bump();
        }
        let value = u32::from_str_radix(&digits, 16).unwrap_or(0);
        Some(match value {
            0 => '\u{fffd}',
            0xd800..=0xdfff => '\u{fffd}',
            value => char::from_u32(value).unwrap_or('\u{fffd}'),
        })
    }

    /// 当前位置是否是一个数字的开头。
    fn starts_number(&self) -> bool {
        match self.peek() {
            Some(character) if character.is_ascii_digit() => true,
            Some('+') | Some('-') => match self.peek_nth(1) {
                Some(next) if next.is_ascii_digit() => true,
                Some('.') => self.peek_nth(2).is_some_and(|third| third.is_ascii_digit()),
                _ => false,
            },
            Some('.') => self.peek_nth(1).is_some_and(|next| next.is_ascii_digit()),
            _ => false,
        }
    }

    /// 消费一个名字，也就是标识符的内容。
    ///
    /// 转义在这里就解开了：返回的是解码后的名字，不是原文切片。所以
    /// `.a\41 b` 与 `.aAb` 拿到的是同一个类名。规范 §4.1.3 要求转义对标识符
    /// 透明，留着一串反斜杠会让选择器匹配不上。
    fn consume_name(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(character) if is_name_continue(character) => {
                    out.push(character);
                    self.bump();
                }
                Some('\\') if self.starts_escape() => match self.read_escape() {
                    Some(character) => out.push(character),
                    None => break,
                },
                _ => break,
            }
        }
        out
    }

    /// 消费字符串字面量，引号已剥掉，转义已解码。
    ///
    /// 返回值里的布尔量表示这是不是个「坏字符串」。规范 §4.2 对两种结束方式
    /// 的处理不一样，这里是它们的分界：
    ///
    /// - 遇到没转义的换行：就地收尾，**并且丢掉含它的那条声明**，所以标成坏。
    /// - 输入结束：§4.2 的「样式表意外结束」要求把所有没闭合的构造补上收尾，
    ///   这个字符串照样有效，只是没写完。
    ///
    /// `\` 后跟换行是续行，接着往下读，不产生字符。
    fn consume_string(&mut self, quote: char) -> (String, bool) {
        self.bump();
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return (out, false),
                Some(character) if character == quote => {
                    self.bump();
                    return (out, false);
                }
                Some('\n' | '\r' | '\x0c') => return (out, true),
                Some('\\') => match self.peek_nth(1) {
                    Some('\n') => {
                        self.bump();
                        self.bump();
                    }
                    Some('\r') => {
                        self.bump();
                        self.bump();
                        if self.peek() == Some('\n') {
                            self.bump();
                        }
                    }
                    _ => match self.read_escape() {
                        Some(character) => out.push(character),
                        // 反斜杠后面就是输入末尾，按未写完收场。
                        None => return (out, false),
                    },
                },
                Some(character) => {
                    out.push(character);
                    self.bump();
                }
            }
        }
    }

    /// 消费 `#` 开头的记号。
    ///
    /// 规范里就一个 HASH 记号，形状是 `#` 加 `{name}`，而 `name` 是 `{nmchar}+`——
    /// 数字在 `nmchar` 里，所以 `#123abc` 是个合法的 HASH，也就是个合法的 id
    /// 选择器。首字符可以是数字，也可以是一个转义（`#\31 23`）。
    ///
    /// 第二个返回值是「像不像 id」这个标记，选择器那边用它区分 id 与颜色值。
    /// 按规范这里没有可分的：能构成名字的就是 HASH。
    fn consume_hash(&mut self) -> Token {
        self.bump();
        let starts_name = self
            .peek()
            .is_some_and(|c| is_name_continue(c) || (c == '\\' && self.starts_escape()));
        if !starts_name {
            return Token::Delim('#');
        }
        let name = self.consume_name();
        Token::Hash(name, true)
    }

    /// 消费数字、百分比或尺寸。
    fn consume_numeric(&mut self) -> Token {
        let (value, integer) = self.consume_number();
        if self.peek() == Some('%') {
            self.bump();
            return Token::Percentage(value / 100.0);
        }
        if self.starts_identifier() {
            let unit = self.consume_name();
            return Token::Dimension {
                value,
                integer,
                unit: unit.to_ascii_lowercase(),
            };
        }
        Token::Number { value, integer }
    }

    /// 消费一个数字，返回数值与是否是整数写法。
    fn consume_number(&mut self) -> (f64, bool) {
        let start = self.pos;
        let mut integer = true;

        if matches!(self.peek(), Some('+') | Some('-')) {
            self.bump();
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            integer = false;
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        // 这里没有指数那一支，是照规范来的：G.2 的 `num` 只有 `[0-9]+` 与
        // `[0-9]*"."[0-9]+` 两种形状，没有科学计数法。所以 `1e3` 不是一个数字，
        // 而是 `{num}{ident}`——数字 1 加单位 e3，也就是一个 DIMENSION。
        // 按数字读会把它算成 1000，长度单位就此变成一个离谱的值。

        let text = &self.input[start..self.pos];
        (text.parse::<f64>().unwrap_or(0.0), integer)
    }

    /// 消费 `url(` 之后的内容。
    ///
    /// 两种写法：带引号的按字符串规则来，不带的读到 `)` 或空白为止。两种都会
    /// 解转义——`url(a\41 b.png)` 与 `url(aAb.png)` 是同一个地址。
    ///
    /// 出现非法字符（裸引号、括号、不可打印字符）或反斜杠后跟换行时，整个地址
    /// 作废，产出 BAD_URI，让上层把含它的那条声明丢掉。
    fn consume_url(&mut self) -> Token {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.bump();
        }
        // 带引号的形式按普通字符串处理。
        if let Some(quote @ ('"' | '\'')) = self.peek() {
            let (value, bad) = self.consume_string(quote);
            self.skip_url_tail();
            return if bad { Token::BadUrl } else { Token::Url(value) };
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                // 输入结束。§4.2 的「样式表意外结束」要求把没闭合的括号补上，
                // 所以这个地址仍然算数，只是没写完。
                None => return Token::Url(out),
                Some(')') => {
                    self.bump();
                    return Token::Url(out);
                }
                Some(character) if character.is_ascii_whitespace() => {
                    self.skip_url_tail();
                    return Token::Url(out);
                }
                Some('\\') if self.starts_escape() => match self.read_escape() {
                    Some(character) => out.push(character),
                    None => {
                        self.consume_bad_url_remnants();
                        return Token::BadUrl;
                    }
                },
                // 反斜杠后跟换行。
                Some('\\') => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                // 地址里不允许出现的字符。
                Some(character) if url_forbidden(character) => {
                    self.consume_bad_url_remnants();
                    return Token::BadUrl;
                }
                Some(character) => {
                    out.push(character);
                    self.bump();
                }
            }
        }
    }

    /// 吃掉一个坏地址剩下的部分，直到右括号或输入结束。
    ///
    /// 发现非法字符时不能就地停手。停手的话，地址里剩下的文字会以普通记号的
    /// 身份流出去，把后面那条声明的解析带偏——`url(a"b) color: red` 会让
    /// `b)` 变成一个字符串和半个括号。规范要求读到该构造的末尾再恢复。
    fn consume_bad_url_remnants(&mut self) {
        loop {
            match self.peek() {
                None => return,
                Some(')') => {
                    self.bump();
                    return;
                }
                Some('\\') if self.starts_escape() => {
                    self.read_escape();
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    /// 跳过 `url(` 结尾的空白与右括号。
    fn skip_url_tail(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.bump();
        }
        if self.peek() == Some(')') {
            self.bump();
        }
    }
}

/// 是否可以作为名字的后续字符。
fn is_name_continue(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || character == '-'
        || character == '_'
        || !character.is_ascii()
}

/// 不带引号的 `url(...)` 里不允许出现的字符。
///
/// 规范的字符类是 `[!#$%&*-\[\]-~]`，也就是可打印 ASCII 里去掉双引号、单引号
/// 和圆括号剩下的那些。反斜杠另算：它构成转义时合法，不构成时作废，那一条在
/// 消费地址的循环里单独判。非 ASCII 一律放行。
fn url_forbidden(character: char) -> bool {
    match character {
        '"' | '\'' | '(' | ')' | '\\' => true,
        character if character.is_ascii() => character < '!' || character == '\x7f',
        _ => false,
    }
}

/// 把整段 CSS 切成记号序列，末尾带一个 `Eof`。
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokenizer = Tokenizer::new(input);
    let mut tokens = Vec::new();
    loop {
        let token = tokenizer.next_token();
        let is_eof = matches!(token, Token::Eof);
        tokens.push(token);
        if is_eof {
            return tokens;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取出一段 CSS 的全部记号，去掉首尾空白与 Eof。
    fn tokens(input: &str) -> Vec<Token> {
        let mut tokens = tokenize(input);
        tokens.pop();
        tokens.retain(|token| !token.is_whitespace());
        tokens
    }

    #[test]
    fn identifiers_and_keywords() {
        assert_eq!(
            tokens("color RED -webkit-box _x"),
            vec![
                Token::Ident("color".into()),
                Token::Ident("RED".into()),
                Token::Ident("-webkit-box".into()),
                Token::Ident("_x".into()),
            ]
        );
    }

    #[test]
    fn function_token() {
        assert_eq!(
            tokens("calc(1px)"),
            vec![
                Token::Function("calc".into()),
                Token::Dimension {
                    value: 1.0,
                    integer: true,
                    unit: "px".into()
                },
                Token::CloseParen,
            ]
        );
    }

    #[test]
    fn at_keyword() {
        assert_eq!(tokens("@media"), vec![Token::AtKeyword("media".into())]);
    }

    #[test]
    fn numbers_and_percentages() {
        assert_eq!(
            tokens("0 1.5 -2 +3 .5 10%"),
            vec![
                Token::Number {
                    value: 0.0,
                    integer: true
                },
                Token::Number {
                    value: 1.5,
                    integer: false
                },
                Token::Number {
                    value: -2.0,
                    integer: true
                },
                Token::Number {
                    value: 3.0,
                    integer: true
                },
                Token::Number {
                    value: 0.5,
                    integer: false
                },
                Token::Percentage(0.1),
            ]
        );
    }

    #[test]
    fn dimensions() {
        assert_eq!(
            tokens("12px 1.5em 100vh"),
            vec![
                Token::Dimension {
                    value: 12.0,
                    integer: true,
                    unit: "px".into()
                },
                Token::Dimension {
                    value: 1.5,
                    integer: false,
                    unit: "em".into()
                },
                Token::Dimension {
                    value: 100.0,
                    integer: true,
                    unit: "vh".into()
                },
            ]
        );
    }

    #[test]
    fn exponents_are_not_part_of_a_number() {
        // 规范 G.2 的 `num` 只有 `[0-9]+` 与 `[0-9]*"."[0-9]+` 两种形状，没有
        // 科学计数法。所以 `1e3` 不是一千，而是 `{num}{ident}` 那一支：数字 1
        // 加单位 e3。`2e-2` 的单位是 `e-2`，因为 `-` 属于 nmchar。
        assert_eq!(
            tokens("1e3 2e-2 1e"),
            vec![
                Token::Dimension {
                    value: 1.0,
                    integer: true,
                    unit: "e3".into()
                },
                Token::Dimension {
                    value: 2.0,
                    integer: true,
                    unit: "e-2".into()
                },
                Token::Dimension {
                    value: 1.0,
                    integer: true,
                    unit: "e".into()
                },
            ]
        );
    }

    #[test]
    fn strings_with_escapes() {
        assert_eq!(
            tokens(r#""a\"b" 'x\ny'"#),
            vec![Token::String("a\"b".into()), Token::String("xny".into())]
        );
    }

    #[test]
    fn url_forms() {
        assert_eq!(
            tokens("url(a.png) url('b.png') url( c.png )"),
            vec![
                Token::Url("a.png".into()),
                Token::Url("b.png".into()),
                Token::Url("c.png".into()),
            ]
        );
    }

    #[test]
    fn hash_tokens() {
        // 规范里 HASH 就是 `#` 加 `{name}`，`name` 是 `{nmchar}+`，数字在其中。
        // 所以 `#123` 也是个合法的 HASH，`#123abc` 能当 id 选择器用。
        // 一个孤零零的 `#` 不是 HASH，那是 DELIM。
        // `\#` 不是 HASH：最长匹配下 `\#` 属于 `{ident}` 的转义起头，所以它
        // 是个标识符。而以转义起头的名字反过来能构成 HASH，`#\31 23` 就是
        // id 为 123 的选择器。
        assert_eq!(
            tokens(r"#main #123 #123abc # #\31 23 \#"),
            vec![
                Token::Hash("main".into(), true),
                Token::Hash("123".into(), true),
                Token::Hash("123abc".into(), true),
                Token::Delim('#'),
                Token::Hash("123".into(), true),
                Token::Ident("#".into()),
            ]
        );
    }

    #[test]
    fn escapes_are_decoded_in_names() {
        // 规范 §4.1.3：转义对标识符透明，`\41 ` 就是字母 A。十六进制写法最多
        // 六位，后面若跟一个空白，那个空白是分隔符、要吃掉——所以 `a\41 b`
        // 是 aAb，而 `a\41b` 会被读成码点 0x41b 那一个字符。
        //
        // 因此想选 class="5ft" 要写 `.\35 ft`：空格是给 `\35` 收尾的，`ft`
        // 才是剩下的名字。写成 `.\35 5ft` 得到的是 55ft。
        assert_eq!(
            tokens(r".a\41 b .\35 ft a\41b"),
            vec![
                Token::Delim('.'),
                Token::Ident("aAb".into()),
                Token::Delim('.'),
                Token::Ident("5ft".into()),
                Token::Ident("a\u{41b}".into()),
            ]
        );
    }

    #[test]
    fn non_hex_escapes_take_the_character_literally() {
        // 反斜杠后面不是十六进制数字时，那个字符就是它的值。
        assert_eq!(
            tokens(r"\@x a\:b"),
            vec![Token::Ident("@x".into()), Token::Ident("a:b".into())]
        );
    }

    #[test]
    fn escapes_are_decoded_in_strings_and_urls() {
        assert_eq!(
            tokens(r#""\41 b" url(a\41 b.png)"#),
            vec![
                Token::String("Ab".into()),
                Token::Url("aAb.png".into()),
            ]
        );
    }

    #[test]
    fn a_string_ended_by_a_newline_is_a_bad_string() {
        // §4.2 的「字符串意外结束」：遇到没转义的换行就地收尾，并丢掉含它的
        // 那条声明。所以这里给的必须是 BAD_STRING，不能当普通字符串用掉。
        assert_eq!(tokens("\"abc\nx"), vec![Token::BadString, Token::Ident("x".into())]);
    }

    #[test]
    fn a_string_cut_off_by_the_end_of_input_is_still_a_string() {
        // §4.2 的「样式表意外结束」要求把没闭合的构造补上收尾，这条是有效的。
        assert_eq!(tokens("\"abc"), vec![Token::String("abc".into())]);
    }

    #[test]
    fn a_backslash_before_a_newline_continues_the_string() {
        // `\` 后跟换行是续行，换行本身不进结果。
        assert_eq!(tokens("\"ab\\\ncd\""), vec![Token::String("abcd".into())]);
    }

    #[test]
    fn urls_reject_forbidden_characters() {
        // 裸引号、括号在地址里非法，整条作废；输入断在地址中间则照常收尾。
        assert_eq!(
            tokens("url(a\"b) url(a(b) url(ok"),
            vec![
                Token::BadUrl,
                Token::BadUrl,
                Token::Url("ok".into()),
            ]
        );
    }

    #[test]
    fn charset_is_its_own_token() {
        // 规范 G.2 里它是字面量 `"@charset "`。它不是一个普通的 at 规则，
        // 单独成一个记号，好让解码那一步认得出。
        assert_eq!(
            tokens("@charset \"utf-8\"; @import \"a.css\";"),
            vec![
                Token::Charset,
                Token::String("utf-8".into()),
                Token::Semicolon,
                Token::AtKeyword("import".into()),
                Token::String("a.css".into()),
                Token::Semicolon,
            ]
        );
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            tokens("a /* comment */ b"),
            vec![Token::Ident("a".into()), Token::Ident("b".into())]
        );
    }

    #[test]
    fn unterminated_comment_swallows_rest() {
        assert_eq!(tokens("a /* no end"), vec![Token::Ident("a".into())]);
    }

    #[test]
    fn punctuation() {
        assert_eq!(
            tokens("{}()[]:;,"),
            vec![
                Token::OpenCurly,
                Token::CloseCurly,
                Token::OpenParen,
                Token::CloseParen,
                Token::OpenSquare,
                Token::CloseSquare,
                Token::Colon,
                Token::Semicolon,
                Token::Comma,
            ]
        );
    }

    #[test]
    fn cdo_and_cdc() {
        assert_eq!(tokens("<!-- -->"), vec![Token::Cdo, Token::Cdc]);
    }

    #[test]
    fn descendant_combinator_is_whitespace() {
        assert_eq!(
            tokenize("div p"),
            vec![
                Token::Ident("div".into()),
                Token::Whitespace,
                Token::Ident("p".into()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn negation_of_numbers() {
        // `-` 后面不是数字也不是标识符时是单个字符记号。
        assert_eq!(tokens("- "), vec![Token::Delim('-')]);
    }

    #[test]
    fn utf8_identifiers() {
        assert_eq!(
            tokens(".标题"),
            vec![Token::Delim('.'), Token::Ident("标题".into())]
        );
    }

    #[test]
    fn uppercase_unit_is_lowercased() {
        assert_eq!(
            tokens("10PX"),
            vec![Token::Dimension {
                value: 10.0,
                integer: true,
                unit: "px".into()
            }]
        );
    }

    #[test]
    fn empty_input() {
        assert_eq!(tokenize(""), vec![Token::Eof]);
    }
}
