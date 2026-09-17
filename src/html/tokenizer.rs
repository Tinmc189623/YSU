//! HTML 词法分析。
//!
//! 按 HTML 规范的状态机切分输入，产出标签、注释、DOCTYPE 与文本记号。
//! `<script>` 与 `<style>` 这类元素的内容不是普通文本，解析器需要在读到
//! 对应的起始标签后通知词法器切换模式，所以这里对外暴露
//! [`Tokenizer::set_raw_text_mode`]。

use super::references::{lookup_named, lookup_numeric};

/// 标签里的一个属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// 属性名，已转成小写。
    pub name: String,
    /// 属性值，字符引用已经解码。
    pub value: String,
}

/// 一个起始标签或结束标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// 标签名，已转成小写。
    pub name: String,
    /// 属性列表，重名的只保留第一个。
    pub attributes: Vec<Attribute>,
    /// 是否是自闭合写法，如 `<br/>`。
    pub self_closing: bool,
}

/// DOCTYPE 声明。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doctype {
    /// 文档类型名，通常是 `html`。
    pub name: String,
    /// 公开标识符。
    pub public_id: Option<String>,
    /// 系统标识符。
    pub system_id: Option<String>,
    /// 解析过程中出现了语法错误，需要触发怪异模式。
    pub force_quirks: bool,
}

/// 词法记号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// DOCTYPE 声明。
    Doctype(Doctype),
    /// 起始标签。
    StartTag(Tag),
    /// 结束标签。
    EndTag(Tag),
    /// 注释，内容是 `<!--` 与 `-->` 之间的部分。
    Comment(String),
    /// CDATA 段，内容是 `<![CDATA[` 与 `]]>` 之间的部分。
    ///
    /// 同一个记号在两种上下文里处理完全不同：在外来内容（SVG 与 MathML）里
    /// 它是一段文本，在 HTML 内容里它是一次解析错误、按注释收场。词法器不知道
    /// 自己在哪种上下文里，所以原样交出去让树构建判断。
    Cdata(String),
    /// 一段文本。
    Character(String),
    /// 输入结束。
    Eof,
}

/// 原始文本模式，决定标签内容按什么规则扫描。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawTextMode {
    /// 可解析字符数据，字符引用仍然生效，如 `<title>` 与 `<textarea>`。
    Rcdata,
    /// 原始文本，字符引用不生效，如 `<style>` 与 `<xmp>`。
    Rawtext,
    /// 脚本数据，额外处理 `<!--` 引起的转义状态。
    ScriptData,
    /// 纯文本，其后所有内容都是文本，如 `<plaintext>`。
    Plaintext,
}

/// 当前扫描模式。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// 普通数据。
    Data,
    /// 可解析字符数据，记着该由哪个结束标签终止。
    Rcdata { end_tag: String },
    /// 原始文本。
    Rawtext { end_tag: String },
    /// 脚本数据。
    ScriptData { end_tag: String },
    /// 纯文本。
    Plaintext,
}

/// HTML 词法分析器。
pub struct Tokenizer<'a> {
    /// 完整输入。
    input: &'a str,
    /// 当前字节位置。
    pos: usize,
    /// 当前扫描模式。
    mode: Mode,
    /// 已经产出过 EOF，再调用只会继续返回 EOF。
    finished: bool,
}

impl<'a> Tokenizer<'a> {
    /// 基于一段 HTML 文本构造词法器。
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            mode: Mode::Data,
            finished: false,
        }
    }

    /// 当前字节位置。
    pub fn position(&self) -> usize {
        self.pos
    }

    /// 切换到原始文本模式，`end_tag` 是终止该模式的结束标签名。
    pub fn set_raw_text_mode(&mut self, mode: RawTextMode, end_tag: &str) {
        let end_tag = end_tag.to_ascii_lowercase();
        self.mode = match mode {
            RawTextMode::Rcdata => Mode::Rcdata { end_tag },
            RawTextMode::Rawtext => Mode::Rawtext { end_tag },
            RawTextMode::ScriptData => Mode::ScriptData { end_tag },
            RawTextMode::Plaintext => Mode::Plaintext,
        };
    }

    /// 产出下一个记号。
    pub fn next_token(&mut self) -> Token {
        if self.finished {
            return Token::Eof;
        }
        match self.mode.clone() {
            Mode::Data => self.tokenize_data(),
            Mode::Rcdata { end_tag } => self.tokenize_raw(&end_tag, true, false),
            Mode::Rawtext { end_tag } => self.tokenize_raw(&end_tag, false, false),
            Mode::ScriptData { end_tag } => self.tokenize_raw(&end_tag, false, true),
            Mode::Plaintext => self.tokenize_plaintext(),
        }
    }

    // ---- 位置与字符访问 ----

    /// 剩余输入。
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    /// 剩余输入是否为空。
    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// 当前位置的字符。
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// 当前位置往后第 n 个字符。
    fn peek_nth(&self, index: usize) -> Option<char> {
        self.rest().chars().nth(index)
    }

    /// 当前位置的字节值，超出范围或非 ASCII 时返回 `None`。
    fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    /// 前进一个字符。
    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.pos += character.len_utf8();
        Some(character)
    }

    /// 前进指定字节数，调用方负责保证落在字符边界上。
    fn bump_bytes(&mut self, count: usize) {
        self.pos = (self.pos + count).min(self.input.len());
    }

    /// 剩余输入是否以给定字符串开头，忽略 ASCII 大小写。
    fn starts_with_ci(&self, needle: &str) -> bool {
        let rest = self.rest();
        rest.len() >= needle.len()
            && rest.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
    }

    /// 消费掉指定长度的字节。
    fn consume(&mut self, count: usize) {
        self.bump_bytes(count);
    }

    // ---- 普通数据状态 ----

    /// 普通数据状态：文本与标记混在一起。
    fn tokenize_data(&mut self) -> Token {
        if self.at_end() {
            self.finished = true;
            return Token::Eof;
        }
        if self.peek_byte() == Some(b'<') {
            return self.handle_less_than();
        }
        Token::Character(self.consume_text(false))
    }

    /// 在数据状态里读到 `<` 之后的分派。
    fn handle_less_than(&mut self) -> Token {
        // 注释。
        if self.rest().starts_with("<!--") {
            self.bump_bytes(4);
            return Token::Comment(self.consume_comment());
        }
        // CDATA 段。词法器不去判断当前在不在外来内容里——那是树构建才知道的
        // 事，而且判断结果不同，同一个记号的处理完全不同。这里只把内容原样
        // 交出去，由树构建按上下文决定它是文本还是注释。
        if self.rest().starts_with("<![CDATA[") {
            self.bump_bytes(9);
            return Token::Cdata(self.consume_until("]]>"));
        }
        // DOCTYPE。
        if self.starts_with_ci("<!doctype") {
            self.bump_bytes(9);
            return Token::Doctype(self.consume_doctype());
        }
        // 其他 `<!` 开头的内容按不合法注释处理。
        if self.rest().starts_with("<!") {
            self.bump_bytes(2);
            return Token::Comment(self.consume_until(">"));
        }
        // 处理指令这类历史遗留写法同样按不合法注释处理。这里只吃掉 `<`：
        // 问号属于注释内容，`<?xml version="1.0">` 得到的是 `?xml version="1.0"`。
        if self.rest().starts_with("<?") {
            self.bump_bytes(1);
            return Token::Comment(self.consume_until(">"));
        }
        // 结束标签。
        if self.rest().starts_with("</") {
            if let Some(character) = self.peek_nth(2)
                && character.is_ascii_alphabetic()
            {
                self.bump_bytes(2);
                return self.consume_tag(true);
            }
            // `</>` 与 `</ ` 都是解析错误，按注释处理掉。
            self.bump_bytes(2);
            return Token::Comment(self.consume_until(">"));
        }
        // 起始标签。
        if let Some(character) = self.peek_nth(1)
            && character.is_ascii_alphabetic()
        {
            self.bump_bytes(1);
            return self.consume_tag(false);
        }
        // 剩下的 `<` 只是普通文本。
        self.bump_bytes(1);
        Token::Character("<".to_string())
    }

    /// 纯文本状态，读到输入结束为止。
    fn tokenize_plaintext(&mut self) -> Token {
        if self.at_end() {
            self.finished = true;
            return Token::Eof;
        }
        let text = self.rest().to_string();
        self.pos = self.input.len();
        Token::Character(text)
    }

    /// 原始文本类状态：一直读到对应的结束标签为止。
    ///
    /// `decode_references` 为真时解码字符引用，对应 RCDATA；
    /// `script` 为真时额外处理 `<!--` 引起的转义状态。
    fn tokenize_raw(&mut self, end_tag: &str, decode_references: bool, script: bool) -> Token {
        if self.at_end() {
            self.finished = true;
            return Token::Eof;
        }

        // 找到合适的结束标签位置。
        match self.find_raw_end_tag(end_tag, script) {
            Some(start) => {
                if start == self.pos {
                    // 当前位置就是结束标签，切换回数据模式并按结束标签解析。
                    self.mode = Mode::Data;
                    self.bump_bytes(2);
                    return self.consume_tag(true);
                }
                let text = &self.input[self.pos..start];
                let text = replace_nulls(&if decode_references {
                    decode_character_references(text)
                } else {
                    text.to_string()
                });
                self.pos = start;
                Token::Character(text)
            }
            None => {
                let text = &self.input[self.pos..];
                let text = replace_nulls(&if decode_references {
                    decode_character_references(text)
                } else {
                    text.to_string()
                });
                self.pos = self.input.len();
                self.finished = true;
                if text.is_empty() {
                    Token::Eof
                } else {
                    Token::Character(text)
                }
            }
        }
    }

    /// 从当前位置开始找合适的结束标签，返回它的起始字节位置。
    ///
    /// 脚本数据有两个转义层级，语义容易记反，这里按规范写清楚：
    ///
    /// - `<!--` 进入单转义状态。此时 `-->` 退出转义，而 `</script>` 照样
    ///   结束元素，所以 `<!-- </script> -->` 里的脚本会在第一个 `</script>`
    ///   处就断掉。
    /// - 单转义状态下遇到 `<script>` 会进入双重转义。此时 `</script>` 只是
    ///   退回单转义状态，不结束元素，这才是 `<!--<script>...</script>-->`
    ///   那种写法能保护脚本内容的原因。
    /// - `-->` 可以出现在任何位置，不必跟在 `<` 后面。
    fn find_raw_end_tag(&self, end_tag: &str, script: bool) -> Option<usize> {
        let bytes = self.input.as_bytes();
        let mut index = self.pos;
        let mut escaped = false;
        let mut double_escaped = false;

        while index < self.input.len() {
            let rest = &self.input[index..];

            if script && escaped && rest.starts_with("-->") {
                escaped = false;
                double_escaped = false;
                index += 3;
                continue;
            }

            if bytes[index] == b'<' {
                if script {
                    if !escaped && rest.starts_with("<!--") {
                        escaped = true;
                        index += 4;
                        continue;
                    }
                    if escaped
                        && !double_escaped
                        && rest.starts_with("<script")
                        && self.is_tag_boundary(index + 7)
                    {
                        double_escaped = true;
                        index += 7;
                        continue;
                    }
                    if escaped && rest.starts_with("</script") && self.is_tag_boundary(index + 8) {
                        if double_escaped {
                            // 只退回单转义状态，元素继续。
                            double_escaped = false;
                            index += 8;
                            continue;
                        }
                        return Some(index);
                    }
                }

                if self.match_end_tag_at(index, end_tag).is_some() {
                    return Some(index);
                }
            }

            let character = self.input[index..].chars().next()?;
            index += character.len_utf8();
        }
        None
    }

    /// 判断某个位置上的 `</tagname` 是否是合适的结束标签。
    ///
    /// 返回结束标签结束后的位置，不匹配时返回 `None`。
    fn match_end_tag_at(&self, index: usize, end_tag: &str) -> Option<usize> {
        let rest = &self.input[index..];
        if !rest.starts_with("</") {
            return None;
        }
        let after = &rest[2..];
        if after.len() < end_tag.len()
            || !after.as_bytes()[..end_tag.len()].eq_ignore_ascii_case(end_tag.as_bytes())
        {
            return None;
        }
        let boundary = index + 2 + end_tag.len();
        if self.is_tag_boundary(boundary) {
            Some(boundary)
        } else {
            None
        }
    }

    /// 判断某个位置是否是标签名的合法结尾。
    fn is_tag_boundary(&self, index: usize) -> bool {
        match self.input.as_bytes().get(index) {
            None => true,
            Some(byte) => matches!(byte, b'>' | b'/' | b'\t' | b'\n' | b'\x0c' | b' '),
        }
    }

    /// 消费一段普通文本，停在 `<` 或 `&` 之前。
    ///
    /// `in_attribute` 只影响字符引用的解码规则。
    fn consume_text(&mut self, in_attribute: bool) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => break,
                Some('<') => break,
                Some('&') => {
                    self.bump_bytes(1);
                    match self.decode_reference_at_position(in_attribute) {
                        Some(decoded) => out.push_str(&decoded),
                        None => out.push('&'),
                    }
                }
                Some(character) => {
                    out.push(character);
                    self.bump_bytes(character.len_utf8());
                }
            }
        }
        out
    }

    /// 当前位置是 `&` 之后的部分，尝试解码一个字符引用。
    ///
    /// 返回 `None` 表示没有匹配到任何引用，调用方应当原样输出 `&`。
    fn decode_reference_at_position(&mut self, in_attribute: bool) -> Option<String> {
        let rest = self.rest();
        if let Some(digits) = rest.strip_prefix('#') {
            let reference = lookup_numeric(digits)?;
            self.consume(1 + reference.consumed);
            return Some(reference.character.to_string());
        }

        let reference = lookup_named(rest)?;
        // 属性值里，不带分号的历史写法如果后面紧跟等号或字母数字，
        // 就不能当作引用解码，否则 `?a=b&copy=1` 会被解错。
        if in_attribute
            && !reference.has_semicolon
            && let Some(next) = rest[reference.consumed..].chars().next()
            && (next == '=' || next.is_ascii_alphanumeric())
        {
            return None;
        }
        self.consume(reference.consumed);
        Some(reference.characters)
    }

    // ---- 标签 ----

    /// 消费一个标签，当前位置在标签名开头。
    fn consume_tag(&mut self, is_end_tag: bool) -> Token {
        let name = self.consume_tag_name();
        let mut attributes: Vec<Attribute> = Vec::new();
        let mut self_closing = false;

        loop {
            self.skip_whitespace();
            match self.peek() {
                None => break,
                Some('>') => {
                    self.bump_bytes(1);
                    break;
                }
                Some('/') => {
                    self.bump_bytes(1);
                    if self.peek() == Some('>') {
                        self.bump_bytes(1);
                        self_closing = true;
                        break;
                    }
                    // `<div/ foo>` 里的斜杠只是被忽略的字符。
                    continue;
                }
                Some(_) => {
                    if let Some(attribute) = self.consume_attribute_name() {
                        let attribute = self.finish_attribute(attribute);
                        // 重名属性只保留第一个。
                        if !attributes
                            .iter()
                            .any(|existing| existing.name == attribute.name)
                        {
                            attributes.push(attribute);
                        }
                    }
                }
            }
        }

        let tag = Tag {
            name,
            attributes,
            self_closing,
        };
        if is_end_tag {
            Token::EndTag(tag)
        } else {
            Token::StartTag(tag)
        }
    }

    /// 消费标签名，转成小写。
    fn consume_tag_name(&mut self) -> String {
        let start = self.pos;
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() || matches!(character, '/' | '>') {
                break;
            }
            self.bump_bytes(character.len_utf8());
        }
        self.input[start..self.pos].to_ascii_lowercase()
    }

    /// 跳过标签内部的空白。
    fn skip_whitespace(&mut self) {
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() {
                self.bump_bytes(character.len_utf8());
            } else {
                break;
            }
        }
    }

    /// 消费一个属性名，返回属性名。遇到无法作为属性名开头的内容时返回 `None`。
    fn consume_attribute_name(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() || matches!(character, '/' | '>' | '=') {
                break;
            }
            // 引号出现在属性名里是解析错误，按规范忽略掉。
            if matches!(character, '"' | '\'' | '<') {
                self.bump_bytes(character.len_utf8());
                continue;
            }
            self.bump_bytes(character.len_utf8());
        }
        if self.pos == start {
            // 一个字符都没消费，说明卡在了 `=` 这类字符上，跳过它避免死循环。
            if self.peek().is_some() {
                self.bump_bytes(1);
            }
            return None;
        }
        Some(self.input[start..self.pos].to_ascii_lowercase())
    }

    /// 读完属性名之后，继续读可能的属性值。
    fn finish_attribute(&mut self, name: String) -> Attribute {
        self.skip_whitespace();
        if self.peek() != Some('=') {
            return Attribute {
                name,
                value: String::new(),
            };
        }
        self.bump_bytes(1);
        self.skip_whitespace();
        let value = match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.bump_bytes(1);
                let value = self.consume_quoted_value(quote);
                self.bump_bytes(1);
                value
            }
            _ => self.consume_unquoted_value(),
        };
        Attribute { name, value }
    }

    /// 消费引号包裹的属性值，字符引用按属性规则解码。
    fn consume_quoted_value(&mut self, quote: char) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => break,
                Some(character) if character == quote => break,
                Some('&') => {
                    self.bump_bytes(1);
                    match self.decode_reference_at_position(true) {
                        Some(decoded) => out.push_str(&decoded),
                        None => out.push('&'),
                    }
                }
                Some(character) => {
                    out.push(character);
                    self.bump_bytes(character.len_utf8());
                }
            }
        }
        out
    }

    /// 消费不带引号的属性值。
    fn consume_unquoted_value(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => break,
                Some(character) if character.is_ascii_whitespace() || character == '>' => {
                    break;
                }
                Some('&') => {
                    self.bump_bytes(1);
                    match self.decode_reference_at_position(true) {
                        Some(decoded) => out.push_str(&decoded),
                        None => out.push('&'),
                    }
                }
                Some(character) => {
                    out.push(character);
                    self.bump_bytes(character.len_utf8());
                }
            }
        }
        out
    }

    // ---- 注释与 DOCTYPE ----

    /// 消费注释内容，当前位置在 `<!--` 之后。
    fn consume_comment(&mut self) -> String {
        /// 注释状态机的那几个状态。
        #[derive(PartialEq)]
        enum State {
            /// 紧跟 `<!--` 之后。
            Start,
            /// `<!---` 之后。
            StartDash,
            /// 正文。
            Body,
            /// 正文里遇到了一个短横。
            EndDash,
            /// 连续两个短横之后。
            End,
            /// `--!` 之后。
            EndBang,
        }

        let mut out = String::new();
        let mut state = State::Start;

        loop {
            let character = self.peek();
            match state {
                // 紧跟 `<!--` 的 `>` 是「注释被意外关闭」，注释内容为空。
                State::Start => match character {
                    Some('>') => {
                        self.bump();
                        return out;
                    }
                    Some('-') => {
                        self.bump();
                        state = State::StartDash;
                    }
                    Some(_) => state = State::Body,
                    None => return out,
                },
                // `<!--->` 同样是空注释。
                State::StartDash => match character {
                    Some('>') => {
                        self.bump();
                        return out;
                    }
                    Some('-') => {
                        self.bump();
                        state = State::End;
                    }
                    Some(_) => {
                        out.push('-');
                        state = State::Body;
                    }
                    None => return out,
                },
                State::Body => match character {
                    Some('-') => {
                        self.bump();
                        state = State::EndDash;
                    }
                    Some('\0') => {
                        self.bump();
                        out.push('\u{fffd}');
                    }
                    Some(character) => {
                        self.bump();
                        out.push(character);
                    }
                    None => return out,
                },
                // 只遇到一个短横时它还是正文。
                State::EndDash => match character {
                    Some('-') => {
                        self.bump();
                        state = State::End;
                    }
                    Some(character) => {
                        self.bump();
                        out.push('-');
                        out.push(character);
                        state = State::Body;
                    }
                    None => return out,
                },
                State::End => match character {
                    Some('>') => {
                        self.bump();
                        return out;
                    }
                    Some('!') => {
                        self.bump();
                        state = State::EndBang;
                    }
                    Some('-') => {
                        self.bump();
                        out.push('-');
                    }
                    Some(character) => {
                        self.bump();
                        out.push_str("--");
                        out.push(character);
                        state = State::Body;
                    }
                    None => return out,
                },
                // `--!>` 也是合法的收尾，属历史遗留写法。
                State::EndBang => match character {
                    Some('-') => {
                        self.bump();
                        out.push_str("--!");
                        state = State::EndDash;
                    }
                    Some('>') => {
                        self.bump();
                        return out;
                    }
                    Some(character) => {
                        self.bump();
                        out.push_str("--!");
                        out.push(character);
                        state = State::Body;
                    }
                    None => return out,
                },
            }
        }
    }

    /// 消费一段直到指定终止串为止的内容，终止串本身也被吃掉。
    fn consume_until(&mut self, terminator: &str) -> String {
        let start = self.pos;
        match self.rest().find(terminator) {
            Some(offset) => {
                let text = self.input[start..start + offset].to_string();
                self.bump_bytes(offset + terminator.len());
                text
            }
            None => {
                let text = self.input[start..].to_string();
                self.pos = self.input.len();
                text
            }
        }
    }

    /// 消费 DOCTYPE 声明，当前位置在 `<!doctype` 之后。
    fn consume_doctype(&mut self) -> Doctype {
        let mut doctype = Doctype::default();
        self.skip_whitespace();

        // 文档类型名，直到空白或 `>` 为止。
        let start = self.pos;
        while let Some(character) = self.peek() {
            if character.is_ascii_whitespace() || character == '>' {
                break;
            }
            self.bump_bytes(character.len_utf8());
        }
        doctype.name = self.input[start..self.pos].to_ascii_lowercase();

        self.skip_whitespace();
        if self.peek() == Some('>') {
            self.bump_bytes(1);
            if doctype.name.is_empty() {
                doctype.force_quirks = true;
            }
            return doctype;
        }

        // PUBLIC 与 SYSTEM 标识符。
        if self.starts_with_ci("public") {
            self.bump_bytes(6);
            self.skip_whitespace();
            doctype.public_id = Some(self.consume_quoted_identifier());
            self.skip_whitespace();
            if let Some(character) = self.peek()
                && (character == '"' || character == '\'')
            {
                doctype.system_id = Some(self.consume_quoted_identifier());
                self.skip_whitespace();
            }
        } else if self.starts_with_ci("system") {
            self.bump_bytes(6);
            self.skip_whitespace();
            doctype.system_id = Some(self.consume_quoted_identifier());
            self.skip_whitespace();
        }

        // 丢弃到 `>` 为止的剩余内容。
        while let Some(character) = self.peek() {
            self.bump_bytes(character.len_utf8());
            if character == '>' {
                break;
            }
        }
        if doctype.name.is_empty() {
            doctype.force_quirks = true;
        }
        doctype
    }

    /// 读取一个引号包裹的标识符，未加引号时读到空白为止。
    fn consume_quoted_identifier(&mut self) -> String {
        match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.bump_bytes(1);
                let value = self.consume_quoted_value(quote);
                self.bump_bytes(1);
                value
            }
            _ => {
                let start = self.pos;
                while let Some(character) = self.peek() {
                    if character.is_ascii_whitespace() || character == '>' {
                        break;
                    }
                    self.bump_bytes(character.len_utf8());
                }
                self.input[start..self.pos].to_string()
            }
        }
    }
}

/// 解码一段文本里的全部字符引用，非引用部分原样保留。
pub fn decode_character_references(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut tokenizer = Tokenizer::new(text);
    let mut out = String::new();
    // 用一个临时词法器复用解码逻辑，扫描全程不会有 `<` 干扰。
    loop {
        match tokenizer.peek() {
            None => break,
            Some('&') => {
                tokenizer.bump_bytes(1);
                match tokenizer.decode_reference_at_position(false) {
                    Some(decoded) => out.push_str(&decoded),
                    None => out.push('&'),
                }
            }
            Some(character) => {
                out.push(character);
                tokenizer.bump_bytes(character.len_utf8());
            }
        }
    }
    out
}

/// 把文本里的空字符换成替换字符。
///
/// 规范里 RCDATA、RAWTEXT、脚本数据与纯文本这几个状态遇到 U+0000 都产出
/// U+FFFD。正文那边是直接丢掉，两条路的处理不一样，不能共用同一个函数。
fn replace_nulls(text: &str) -> String {
    if !text.contains('\0') {
        return text.to_string();
    }
    text.chars()
        .map(|character| {
            if character == '\0' {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

/// 把整段 HTML 切成记号序列，末尾带一个 `Eof`。
///
/// 这个入口不做原始文本模式切换，`<script>` 里的内容会按普通标记解析。
/// 需要正确处理脚本与样式的内容时，请自行驱动 [`Tokenizer`] 并在读到
/// 对应起始标签后调用 [`Tokenizer::set_raw_text_mode`]。
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

    /// 取出一段 HTML 的全部记号，去掉结尾的 Eof。
    fn tokens(input: &str) -> Vec<Token> {
        let mut tokens = tokenize(input);
        tokens.pop();
        tokens
    }

    /// 构造一个起始标签记号的便捷函数。
    fn start(name: &str, attributes: &[(&str, &str)], self_closing: bool) -> Token {
        Token::StartTag(Tag {
            name: name.to_string(),
            attributes: attributes
                .iter()
                .map(|(name, value)| Attribute {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            self_closing,
        })
    }

    /// 构造一个结束标签记号。
    fn end(name: &str) -> Token {
        Token::EndTag(Tag {
            name: name.to_string(),
            attributes: Vec::new(),
            self_closing: false,
        })
    }

    #[test]
    fn plain_text() {
        assert_eq!(tokens("hello"), vec![Token::Character("hello".into())]);
    }

    #[test]
    fn empty_input() {
        assert!(tokens("").is_empty());
    }

    #[test]
    fn simple_element() {
        assert_eq!(
            tokens("<p>hi</p>"),
            vec![
                start("p", &[], false),
                Token::Character("hi".into()),
                end("p"),
            ]
        );
    }

    #[test]
    fn tag_name_is_lowercased() {
        assert_eq!(tokens("<DIV>"), vec![start("div", &[], false)]);
    }

    #[test]
    fn self_closing_tag() {
        assert_eq!(tokens("<br/>"), vec![start("br", &[], true)]);
        assert_eq!(tokens("<br />"), vec![start("br", &[], true)]);
    }

    #[test]
    fn attributes_in_various_forms() {
        assert_eq!(
            tokens(r#"<a href="x" title='y' data-z=1 hidden>"#),
            vec![start(
                "a",
                &[
                    ("href", "x"),
                    ("title", "y"),
                    ("data-z", "1"),
                    ("hidden", "")
                ],
                false
            )]
        );
    }

    #[test]
    fn attribute_names_are_lowercased_values_are_not() {
        assert_eq!(
            tokens("<a HREF='X'>"),
            vec![start("a", &[("href", "X")], false)]
        );
    }

    #[test]
    fn duplicate_attributes_keep_first() {
        assert_eq!(
            tokens("<a x=1 x=2>"),
            vec![start("a", &[("x", "1")], false)]
        );
    }

    #[test]
    fn character_references_in_text() {
        assert_eq!(
            tokens("&amp;&lt;&#65;&#x42;"),
            vec![Token::Character("&<AB".into())]
        );
    }

    #[test]
    fn unknown_reference_is_literal() {
        assert_eq!(
            tokens("a &nosuch; b"),
            vec![Token::Character("a &nosuch; b".into())]
        );
    }

    #[test]
    fn character_references_in_attributes() {
        assert_eq!(
            tokens(r#"<a href="?a=1&amp;b=2">"#),
            vec![start("a", &[("href", "?a=1&b=2")], false)]
        );
    }

    #[test]
    fn legacy_reference_followed_by_equals_is_not_decoded() {
        // 属性值里 `&copy=1` 不该被解码成 `©=1`。
        assert_eq!(
            tokens("<a href='?x&copy=1'>"),
            vec![start("a", &[("href", "?x&copy=1")], false)]
        );
    }

    #[test]
    fn legacy_reference_at_end_is_decoded() {
        assert_eq!(
            tokens("<a href='?x&copy'>"),
            vec![start("a", &[("href", "?x©")], false)]
        );
    }

    #[test]
    fn comment() {
        assert_eq!(tokens("<!-- hi -->"), vec![Token::Comment(" hi ".into())]);
    }

    #[test]
    fn comment_ends_with_bang_form() {
        assert_eq!(tokens("<!-- hi --!>"), vec![Token::Comment(" hi ".into())]);
    }

    #[test]
    fn bogus_comment_from_question_mark() {
        // 问号属于注释内容：不合法注释是从 `<!` 或 `<?` 之后那一个字符开始
        // 记的，`<?` 只吃掉尖括号。
        assert_eq!(tokens("<?php ?>"), vec![Token::Comment("?php ?".into())]);
    }

    #[test]
    fn comment_start_states_produce_empty_comments() {
        // `<!-->` 与 `<!--->` 都是「注释被意外关闭」，得到空注释，后面的
        // 内容照常解析，不能一并吞掉。
        assert_eq!(
            tokens("<!-->rest"),
            vec![Token::Comment(String::new()), Token::Character("rest".into())]
        );
        assert_eq!(
            tokens("<!--->rest"),
            vec![Token::Comment(String::new()), Token::Character("rest".into())]
        );
    }

    #[test]
    fn comment_end_states_keep_the_dashes() {
        // 只有 `-->` 与 `--!>` 收尾。中间那些短横是内容的一部分。
        assert_eq!(tokens("<!-- a -- b -->"), vec![Token::Comment(" a -- b ".into())]);
        assert_eq!(tokens("<!-- a --->"), vec![Token::Comment(" a -".into())]);
        assert_eq!(tokens("<!-- a --!>"), vec![Token::Comment(" a ".into())]);
    }

    #[test]
    fn unterminated_comment_runs_to_end() {
        assert_eq!(
            tokens("<!-- no end"),
            vec![Token::Comment(" no end".into())]
        );
    }

    #[test]
    fn doctype_simple() {
        assert_eq!(
            tokens("<!DOCTYPE html>"),
            vec![Token::Doctype(Doctype {
                name: "html".into(),
                public_id: None,
                system_id: None,
                force_quirks: false,
            })]
        );
    }

    #[test]
    fn doctype_with_identifiers() {
        assert_eq!(
            tokens(
                r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd">"#
            ),
            vec![Token::Doctype(Doctype {
                name: "html".into(),
                public_id: Some("-//W3C//DTD HTML 4.01//EN".into()),
                system_id: Some("http://www.w3.org/TR/html4/strict.dtd".into()),
                force_quirks: false,
            })]
        );
    }

    #[test]
    fn bare_less_than_is_text() {
        assert_eq!(
            tokens("a < b"),
            vec![
                Token::Character("a ".into()),
                Token::Character("<".into()),
                Token::Character(" b".into()),
            ]
        );
    }

    #[test]
    fn stray_end_tag_is_a_comment() {
        assert_eq!(tokens("</>"), vec![Token::Comment(String::new())]);
    }

    #[test]
    fn rawtext_stops_at_end_tag() {
        let mut tokenizer = Tokenizer::new("<style>a > b</style>after");
        assert_eq!(tokenizer.next_token(), start("style", &[], false));
        tokenizer.set_raw_text_mode(RawTextMode::Rawtext, "style");
        assert_eq!(tokenizer.next_token(), Token::Character("a > b".into()));
        assert_eq!(tokenizer.next_token(), end("style"));
        assert_eq!(tokenizer.next_token(), Token::Character("after".into()));
    }

    #[test]
    fn rawtext_does_not_decode_references() {
        let mut tokenizer = Tokenizer::new("<style>&amp;</style>");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::Rawtext, "style");
        assert_eq!(tokenizer.next_token(), Token::Character("&amp;".into()));
    }

    #[test]
    fn rcdata_decodes_references() {
        let mut tokenizer = Tokenizer::new("<title>a&amp;b</title>");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::Rcdata, "title");
        assert_eq!(tokenizer.next_token(), Token::Character("a&b".into()));
    }

    #[test]
    fn rawtext_ignores_partial_tag_name_match() {
        // `</scriptx>` 不是结束标签，应当留在文本里。
        let mut tokenizer = Tokenizer::new("<script>a</scriptx>b</script>");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::ScriptData, "script");
        assert_eq!(
            tokenizer.next_token(),
            Token::Character("a</scriptx>b".into())
        );
        assert_eq!(tokenizer.next_token(), end("script"));
    }

    #[test]
    fn single_escape_does_not_protect_end_tag() {
        // 单转义状态下的 `</script>` 依然结束元素，这是规范的行为，
        // 也是 `<!-- </script> -->` 这种写法会提前断开脚本的原因。
        let mut tokenizer = Tokenizer::new("<script><!-- </script> --></script>tail");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::ScriptData, "script");
        assert_eq!(tokenizer.next_token(), Token::Character("<!-- ".into()));
        assert_eq!(tokenizer.next_token(), end("script"));
        assert_eq!(tokenizer.next_token(), Token::Character(" -->".into()));
        assert_eq!(tokenizer.next_token(), end("script"));
        assert_eq!(tokenizer.next_token(), Token::Character("tail".into()));
    }

    #[test]
    fn double_escape_protects_end_tag() {
        // 单转义里再遇到 `<script>` 进入双重转义，其中的 `</script>`
        // 只退回单转义状态，不结束元素。
        let mut tokenizer = Tokenizer::new("<script><!--<script></script>--></script>tail");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::ScriptData, "script");
        assert_eq!(
            tokenizer.next_token(),
            Token::Character("<!--<script></script>-->".into())
        );
        assert_eq!(tokenizer.next_token(), end("script"));
        assert_eq!(tokenizer.next_token(), Token::Character("tail".into()));
    }

    #[test]
    fn escape_closes_on_dash_dash_anywhere() {
        // `-->` 不必跟在 `<` 后面，脚本里任何位置出现都能退出转义。
        let mut tokenizer = Tokenizer::new("<script><!-- a--> </script>");
        tokenizer.next_token();
        tokenizer.set_raw_text_mode(RawTextMode::ScriptData, "script");
        assert_eq!(
            tokenizer.next_token(),
            Token::Character("<!-- a--> ".into())
        );
        assert_eq!(tokenizer.next_token(), end("script"));
    }

    #[test]
    fn plaintext_swallows_everything() {
        let mut tokenizer = Tokenizer::new("<plaintext><b>bold</b>");
        assert_eq!(tokenizer.next_token(), start("plaintext", &[], false));
        tokenizer.set_raw_text_mode(RawTextMode::Plaintext, "");
        assert_eq!(
            tokenizer.next_token(),
            Token::Character("<b>bold</b>".into())
        );
    }

    #[test]
    fn cdata_section_is_its_own_token() {
        // 同一个 CDATA 段在外来内容里是文本、在 HTML 内容里是注释，怎么处理
        // 要看上下文，词法器只把内容原样交出去。
        assert_eq!(tokens("<![CDATA[x]]>"), vec![Token::Cdata("x".into())]);
        assert_eq!(
            tokens("<![CDATA[a>b]]>"),
            vec![Token::Cdata("a>b".into())],
            "CDATA 段里的一对中括号之间连 `>` 也算内容"
        );
    }

    #[test]
    fn attribute_without_value() {
        assert_eq!(
            tokens("<input disabled>"),
            vec![start("input", &[("disabled", "")], false)]
        );
    }

    #[test]
    fn stray_equals_in_tag_is_skipped() {
        // `<div = x>` 里的 `=` 是错误写法，跳过它继续解析属性。
        let result = tokens("<div =x>");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn unquoted_attribute_value_ends_at_whitespace() {
        assert_eq!(
            tokens("<a class=foo bar>"),
            vec![start("a", &[("class", "foo"), ("bar", "")], false)]
        );
    }

    #[test]
    fn eof_inside_tag_terminates_cleanly() {
        assert_eq!(
            tokens("<div class=\"a"),
            vec![start("div", &[("class", "a")], false)]
        );
    }

    #[test]
    fn decode_reference_helper() {
        assert_eq!(decode_character_references("a&amp;b"), "a&b");
        assert_eq!(decode_character_references("no refs"), "no refs");
        // 那条「后面跟等号就不解码」的规则只管属性值，纯文本里照解不误。
        assert_eq!(decode_character_references("&copy=1"), "©=1");
    }

    #[test]
    fn utf8_text_is_preserved() {
        assert_eq!(
            tokens("<p>中文与 emoji 🎉</p>"),
            vec![
                start("p", &[], false),
                Token::Character("中文与 emoji 🎉".into()),
                end("p"),
            ]
        );
    }

    #[test]
    fn multiple_attributes_with_references() {
        assert_eq!(
            tokens(r#"<a x="1" y='&#38;z'>"#),
            vec![start("a", &[("x", "1"), ("y", "&z")], false)]
        );
    }
}
