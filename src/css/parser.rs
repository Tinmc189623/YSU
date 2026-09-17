//! CSS 规则与声明的解析。
//!
//! 输入是一段 CSS 文本，输出是若干条规则，每条规则带自己的选择器与声明表。
//! `@media` 会就地按当前视口求值，满足条件的规则直接提升到外层；`@font-face`
//! 收进字体表；其余不影响静态排版的 at 规则整块跳过。

use super::selector::{ComplexSelector, parse_selector_list};
use super::tokenizer::Token;
use super::value::{Color, Length, LengthUnit, parse_color};
use std::fmt;

/// 声明里的值。
///
/// 能归类的都归了类，归不了的保留原始记号，交给使用方按需解释。
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// 关键字，已转小写，如 `block`、`relative`。
    Keyword(String),
    /// 长度。
    Length(Length),
    /// 纯数字。
    Number(f64),
    /// 角度，单位是度。
    Angle(f64),
    /// 颜色。
    Color(Color),
    /// 字符串。
    String(String),
    /// 地址。
    Url(String),
    /// 长度列表，如 `margin: 1px 2px`。
    LengthList(Vec<Length>),
    /// 数字列表，如 `line-height: 1.2` 之外的复合写法。
    NumberList(Vec<f64>),
    /// 关键字列表，如 `font-family: A, B`。
    KeywordList(Vec<String>),
    /// 混合列表，如 `border: 1px solid red`。
    MixedList(Vec<PropertyValue>),
}

impl PropertyValue {
    /// 取关键字，非关键字返回 `None`。
    pub fn as_keyword(&self) -> Option<&str> {
        match self {
            Self::Keyword(name) => Some(name),
            _ => None,
        }
    }

    /// 取长度，纯数字零也按零长度处理。
    pub fn as_length(&self) -> Option<Length> {
        match self {
            Self::Length(length) => Some(*length),
            Self::Number(value) if *value == 0.0 => Some(Length::ZERO),
            _ => None,
        }
    }

    /// 取数字，无单位长度里的数值也算。
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Length(length) if length.unit == LengthUnit::None => Some(length.value),
            _ => None,
        }
    }

    /// 取颜色。
    pub fn as_color(&self) -> Option<Color> {
        match self {
            Self::Color(color) => Some(*color),
            _ => None,
        }
    }

    /// 取长度列表，单个长度会包装成长度为 1 的列表。
    pub fn as_length_list(&self) -> Option<Vec<Length>> {
        match self {
            Self::LengthList(list) => Some(list.clone()),
            Self::Length(length) => Some(vec![*length]),
            Self::Number(value) if *value == 0.0 => Some(vec![Length::ZERO]),
            _ => None,
        }
    }

    /// 取关键字列表。
    pub fn as_keyword_list(&self) -> Option<Vec<String>> {
        match self {
            Self::KeywordList(list) => Some(list.clone()),
            Self::Keyword(name) => Some(vec![name.clone()]),
            Self::String(text) => Some(vec![text.clone()]),
            _ => None,
        }
    }
}

impl fmt::Display for PropertyValue {
    /// 输出成 CSS 写法，主要给调试用。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keyword(name) => f.write_str(name),
            Self::Length(length) => write!(f, "{length}"),
            Self::Number(value) => f.write_str(&super::selector::number_to_css_string(*value)),
            Self::Angle(value) => write!(f, "{}deg", super::selector::number_to_css_string(*value)),
            Self::Color(color) => write!(f, "{color}"),
            Self::String(text) => write!(f, "{text:?}"),
            Self::Url(url) => write!(f, "url({url})"),
            Self::LengthList(list) => {
                let parts: Vec<String> = list.iter().map(Length::to_string).collect();
                f.write_str(&parts.join(" "))
            }
            Self::NumberList(list) => {
                let parts: Vec<String> = list
                    .iter()
                    .map(|value| super::selector::number_to_css_string(*value))
                    .collect();
                f.write_str(&parts.join(" "))
            }
            Self::KeywordList(list) => f.write_str(&list.join(", ")),
            Self::MixedList(list) => {
                let parts: Vec<String> = list.iter().map(PropertyValue::to_string).collect();
                f.write_str(&parts.join(" "))
            }
        }
    }
}

/// 一条声明。
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// 属性名，已转小写。
    pub property: String,
    /// 属性值。
    pub value: PropertyValue,
    /// 是否带 `!important`。
    pub important: bool,
}

/// 一条限定规则：选择器加声明块。
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// 选择器列表。
    pub selectors: Vec<ComplexSelector>,
    /// 声明列表。
    pub declarations: Vec<Declaration>,
}

/// `@font-face` 里声明的一种字体。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontFace {
    /// 字体族名。
    pub family: String,
    /// 字重。
    pub weight: String,
    /// 字形样式。
    pub style: String,
    /// 字体文件地址。
    pub source: String,
}

/// 一份样式表。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleSheet {
    /// 规则列表，按源码顺序。
    pub rules: Vec<Rule>,
    /// 字体声明。
    pub font_faces: Vec<FontFace>,
}

impl StyleSheet {
    /// 解析一段 CSS 文本。
    pub fn parse(css: &str) -> Self {
        Parser::new(css).parse_stylesheet()
    }

    /// 把另一份样式表的规则并进来。
    pub fn extend(&mut self, other: StyleSheet) {
        self.rules.extend(other.rules);
        self.font_faces.extend(other.font_faces);
    }
}

/// 媒体查询的求值环境。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaContext {
    /// 视口宽度，单位像素。
    pub width: f64,
    /// 视口高度，单位像素。
    pub height: f64,
}

impl Default for MediaContext {
    /// 默认按一个常见的桌面视口处理。
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
        }
    }
}

/// CSS 解析器。
struct Parser {
    /// 记号序列，末尾没有 EOF。
    tokens: Vec<Token>,
    /// 当前位置。
    pos: usize,
    /// 媒体查询求值环境。
    media: MediaContext,
    /// 正在解析的样式表。
    sheet: StyleSheet,
}

impl Parser {
    /// 基于一段 CSS 文本构造解析器。
    fn new(css: &str) -> Self {
        let mut tokens = super::tokenizer::tokenize(css);
        tokens.retain(|token| !matches!(token, Token::Eof));
        Self {
            tokens,
            pos: 0,
            media: MediaContext::default(),
            sheet: StyleSheet::default(),
        }
    }

    /// 设置媒体查询求值环境。
    fn with_media(mut self, media: MediaContext) -> Self {
        self.media = media;
        self
    }

    /// 解析整份样式表。
    fn parse_stylesheet(&mut self) -> StyleSheet {
        self.parse_rule_list(false);
        std::mem::take(&mut self.sheet)
    }

    /// 解析规则列表，`nested` 表示当前在某个块里面。
    fn parse_rule_list(&mut self, nested: bool) {
        loop {
            self.skip_ignorable();
            if self.at_end() {
                return;
            }
            if nested && matches!(self.peek(), Some(Token::CloseCurly)) {
                return;
            }

            match self.peek() {
                Some(Token::AtKeyword(_)) => self.parse_at_rule(),
                // `@charset "utf-8";` 是编码声明，不是规则。值那部分要等解码
                // 那一步才用得上，这里只管把整条吃掉，不让它掉进选择器解析。
                Some(Token::Charset) => self.skip_to_semicolon(),
                Some(Token::CloseCurly) => {
                    // 多余的右花括号直接丢掉。
                    self.pos += 1;
                }
                Some(Token::Semicolon) => {
                    self.pos += 1;
                }
                _ => self.parse_qualified_rule(),
            }
        }
    }

    /// 解析一条 at 规则。
    fn parse_at_rule(&mut self) {
        let name = match self.peek() {
            Some(Token::AtKeyword(name)) => name.clone(),
            _ => return,
        };
        self.pos += 1;

        // 前奏一直取到 `{` 或 `;`。
        let prelude_start = self.pos;
        let mut depth = 0i32;
        let mut block_starts_at = None;
        while !self.at_end() {
            match self.peek() {
                Some(Token::OpenCurly) if depth == 0 => {
                    block_starts_at = Some(self.pos);
                    break;
                }
                Some(Token::Semicolon) if depth == 0 => break,
                Some(Token::OpenParen | Token::OpenSquare | Token::Function(_)) => depth += 1,
                Some(Token::CloseParen | Token::CloseSquare) => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        let prelude: Vec<Token> = self.tokens[prelude_start..self.pos].to_vec();

        // 没有块，规则在这里就结束了。
        let Some(block_start) = block_starts_at else {
            if matches!(self.peek(), Some(Token::Semicolon)) {
                self.pos += 1;
            }
            return;
        };

        // 跳过 `{`。
        self.pos = block_start + 1;
        match name.as_str() {
            "media" => {
                let matched = evaluate_media_query(&prelude, self.media);
                if matched {
                    self.parse_rule_list(true);
                } else {
                    self.skip_block();
                }
                self.consume_close_curly();
            }
            "font-face" => {
                let declarations = self.parse_declaration_list();
                self.sheet.font_faces.push(font_face_from(&declarations));
                self.consume_close_curly();
            }
            _ => {
                // 其余 at 规则不影响静态排版，整块跳过。
                self.skip_block();
                self.consume_close_curly();
            }
        }
    }

    /// 解析一条限定规则。
    fn parse_qualified_rule(&mut self) {
        let prelude_start = self.pos;
        let mut depth = 0i32;
        let mut block_starts_at = None;
        while !self.at_end() {
            match self.peek() {
                Some(Token::OpenCurly) if depth == 0 => {
                    block_starts_at = Some(self.pos);
                    break;
                }
                Some(Token::OpenParen | Token::OpenSquare | Token::Function(_)) => depth += 1,
                Some(Token::CloseParen | Token::CloseSquare) => depth -= 1,
                Some(Token::Semicolon) if depth == 0 => {
                    // 没有块的规则是错误写法，丢掉算数。
                    self.pos += 1;
                    return;
                }
                _ => {}
            }
            self.pos += 1;
        }

        let Some(block_start) = block_starts_at else {
            self.pos = self.tokens.len();
            return;
        };
        let prelude: Vec<Token> = self.tokens[prelude_start..block_start].to_vec();
        self.pos = block_start + 1;

        let declarations = self.parse_declaration_list();
        self.consume_close_curly();

        // 选择器解析失败时整条规则作废，这与浏览器的行为一致。
        let Ok(selectors) = parse_selector_list(&prelude) else {
            return;
        };
        if selectors.is_empty() || declarations.is_empty() {
            return;
        }
        self.sheet.rules.push(Rule {
            selectors,
            declarations,
        });
    }

    /// 解析声明列表，直到遇到右花括号。
    fn parse_declaration_list(&mut self) -> Vec<Declaration> {
        let mut declarations = Vec::new();
        loop {
            self.skip_whitespace();
            if self.at_end() || matches!(self.peek(), Some(Token::CloseCurly)) {
                return declarations;
            }
            if matches!(self.peek(), Some(Token::Semicolon)) {
                self.pos += 1;
                continue;
            }
            // 声明块里嵌 at 规则的情况很少见，跳过整块。
            if matches!(self.peek(), Some(Token::AtKeyword(_))) {
                self.skip_at_rule_in_block();
                continue;
            }
            if let Some(declaration) = self.parse_declaration() {
                declarations.push(declaration);
            }
        }
    }

    /// 解析一条声明，当前位置在属性名上。
    fn parse_declaration(&mut self) -> Option<Declaration> {
        let name = match self.peek() {
            Some(Token::Ident(name)) => name.to_ascii_lowercase(),
            _ => {
                // 认不出的内容，跳到下一个分号。
                self.skip_to_semicolon();
                return None;
            }
        };
        self.pos += 1;
        self.skip_whitespace();
        if !matches!(self.peek(), Some(Token::Colon)) {
            self.skip_to_semicolon();
            return None;
        }
        self.pos += 1;

        let value_start = self.pos;
        let mut depth = 0i32;
        let mut important = false;
        while !self.at_end() {
            match self.peek() {
                Some(Token::Semicolon) if depth == 0 => break,
                Some(Token::CloseCurly) if depth == 0 => break,
                Some(Token::OpenParen | Token::OpenSquare | Token::Function(_)) => depth += 1,
                Some(Token::CloseParen | Token::CloseSquare) => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }

        let mut value_tokens: Vec<Token> = self.tokens[value_start..self.pos].to_vec();
        while value_tokens.first().is_some_and(Token::is_whitespace) {
            value_tokens.remove(0);
        }
        while value_tokens.last().is_some_and(Token::is_whitespace) {
            value_tokens.pop();
        }

        // 剥掉结尾的 `!important`。
        if let Some(index) = find_important(&value_tokens) {
            important = true;
            value_tokens.truncate(index);
            while value_tokens.last().is_some_and(Token::is_whitespace) {
                value_tokens.pop();
            }
        }

        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.pos += 1;
        }

        if value_tokens.is_empty() {
            return None;
        }
        let value = interpret_value(&value_tokens);
        Some(Declaration {
            property: name,
            value,
            important,
        })
    }

    /// 跳过声明块里的 at 规则。
    fn skip_at_rule_in_block(&mut self) {
        while !self.at_end() {
            match self.peek() {
                Some(Token::OpenCurly) => {
                    self.skip_block();
                    self.consume_close_curly();
                    return;
                }
                Some(Token::Semicolon) => {
                    self.pos += 1;
                    return;
                }
                _ => self.pos += 1,
            }
        }
    }

    /// 跳过一整个块，当前位置在 `{` 之后。
    fn skip_block(&mut self) {
        let mut depth = 1i32;
        while !self.at_end() {
            match self.peek() {
                Some(Token::OpenCurly) => depth += 1,
                Some(Token::CloseCurly) => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// 如果当前是右花括号就消费掉。
    fn consume_close_curly(&mut self) {
        if matches!(self.peek(), Some(Token::CloseCurly)) {
            self.pos += 1;
        }
    }

    /// 跳到下一个分号。
    fn skip_to_semicolon(&mut self) {
        while !self.at_end() {
            match self.peek() {
                Some(Token::Semicolon) => {
                    self.pos += 1;
                    return;
                }
                Some(Token::CloseCurly) => return,
                _ => self.pos += 1,
            }
        }
    }

    /// 跳过空白与注释。
    fn skip_whitespace(&mut self) {
        while self.tokens.get(self.pos).is_some_and(Token::is_whitespace) {
            self.pos += 1;
        }
    }

    /// 跳过空白、注释，以及 `<!--` 与 `-->`。
    ///
    /// 规范 §4.2 的 `stylesheet` 产生式把 CDO 与 CDC 摆在和空白同等的位置上，
    /// 顶层与每条规则之后都允许出现。老网页给样式表套 HTML 注释是很常见的写法，
    /// 不跳它们的话，`<style><!-- div { color: red } --></style>` 里的第一条
    /// 规则会被当成无效选择器整条丢掉。
    ///
    /// 声明内部不适用这一条——那里的 CDO 是货真价实的语法错误。
    fn skip_ignorable(&mut self) {
        while self.tokens.get(self.pos).is_some_and(Token::is_ignorable) {
            self.pos += 1;
        }
    }

    /// 当前位置是否读完。
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// 当前记号。
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
}

/// 找 `!important` 里感叹号的位置。
///
/// 从结尾往回看，允许中间夹空白，顺序不对就说明没有 `!important`。
fn find_important(tokens: &[Token]) -> Option<usize> {
    let mut index = tokens.len();
    loop {
        index = index.checked_sub(1)?;
        match &tokens[index] {
            Token::Whitespace => continue,
            Token::Ident(name) if name.eq_ignore_ascii_case("important") => break,
            _ => return None,
        }
    }
    loop {
        index = index.checked_sub(1)?;
        match &tokens[index] {
            Token::Whitespace => continue,
            Token::Delim('!') => return Some(index),
            _ => return None,
        }
    }
}

/// 把记号序列解释成声明值。
fn interpret_value(tokens: &[Token]) -> PropertyValue {
    // 单个记号的情况先处理掉。
    if tokens.len() == 1 {
        match &tokens[0] {
            Token::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                if let Some(color) = parse_color(&lower) {
                    // 具名颜色按颜色处理，其余按关键字。
                    return PropertyValue::Color(color);
                }
                return PropertyValue::Keyword(lower);
            }
            Token::Number { value, .. } => {
                if *value == 0.0 {
                    return PropertyValue::Length(Length::ZERO);
                }
                return PropertyValue::Number(*value);
            }
            Token::Percentage(value) => {
                return PropertyValue::Length(Length::percent(*value * 100.0));
            }
            Token::Dimension { value, unit, .. } => {
                if let Some(unit) = LengthUnit::from_name(unit) {
                    return PropertyValue::Length(Length {
                        value: *value,
                        unit,
                    });
                }
                if unit == "deg" {
                    return PropertyValue::Angle(*value);
                }
                return PropertyValue::Keyword(format!(
                    "{}{}",
                    super::selector::number_to_css_string(*value),
                    unit
                ));
            }
            Token::String(text) => return PropertyValue::String(text.clone()),
            Token::Hash(_, _) => {
                if let Some(color) = parse_hash_token(&tokens[0]) {
                    return PropertyValue::Color(color);
                }
            }
            _ => {}
        }
    }

    // 函数调用如 `rgb(...)`、`url(...)` 统一按原文解析。
    let text = render_tokens(tokens);
    if let Some(color) = parse_color(&text) {
        return PropertyValue::Color(color);
    }

    // 逗号分隔的列表，如 `font-family: A, B`。
    let groups = split_top_level(tokens, Token::Comma);
    if groups.len() > 1 {
        let mut keywords = Vec::new();
        let mut all_keywords = true;
        for group in &groups {
            let group_text = render_tokens(group);
            let trimmed = group_text.trim().to_string();
            if trimmed.is_empty() {
                all_keywords = false;
                break;
            }
            keywords.push(trimmed);
        }
        if all_keywords {
            return PropertyValue::KeywordList(keywords);
        }
    }

    // 空格分隔的列表，按每个记号分别解释。
    let parts = split_top_level(tokens, Token::Whitespace);
    if parts.len() > 1 {
        let values: Vec<PropertyValue> = parts
            .iter()
            .filter(|part| !part.is_empty())
            .map(|part| interpret_value(part))
            .collect();
        let all_lengths = values.iter().all(|value| value.as_length().is_some());
        if all_lengths {
            return PropertyValue::LengthList(
                values.iter().filter_map(PropertyValue::as_length).collect(),
            );
        }
        let all_numbers = values
            .iter()
            .all(|value| matches!(value, PropertyValue::Number(_)));
        if all_numbers {
            return PropertyValue::NumberList(
                values.iter().filter_map(PropertyValue::as_number).collect(),
            );
        }
        if values.len() == 1 {
            return values.into_iter().next().expect("长度已经确认");
        }
        return PropertyValue::MixedList(values);
    }

    PropertyValue::Keyword(text)
}

/// 把 `#` 记号还原成颜色文本再解析。
fn parse_hash_token(token: &Token) -> Option<Color> {
    match token {
        Token::Hash(value, _) => parse_color(&format!("#{value}")),
        _ => None,
    }
}

/// 把记号序列还原成近似源码的文本。
fn render_tokens(tokens: &[Token]) -> String {
    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Whitespace => out.push(' '),
            Token::Ident(name) => out.push_str(name),
            Token::Function(name) => {
                out.push_str(name);
                out.push('(');
            }
            Token::Hash(value, _) => {
                out.push('#');
                out.push_str(value);
            }
            Token::String(text) => {
                out.push('"');
                out.push_str(text);
                out.push('"');
            }
            // 没写完的字符串按源码的样子还原个开头即可，这是给错误信息看的。
            Token::BadString => out.push('"'),
            Token::Url(url) => {
                out.push_str("url(");
                out.push_str(url);
                out.push(')');
            }
            Token::BadUrl => out.push_str("url("),
            Token::Number { value, .. } => {
                out.push_str(&super::selector::number_to_css_string(*value));
            }
            Token::Percentage(value) => {
                out.push_str(&super::selector::number_to_css_string(value * 100.0));
                out.push('%');
            }
            Token::Dimension { value, unit, .. } => {
                out.push_str(&super::selector::number_to_css_string(*value));
                out.push_str(unit);
            }
            Token::AtKeyword(name) => {
                out.push('@');
                out.push_str(name);
            }
            Token::Charset => out.push_str("@charset"),
            Token::Comma => out.push(','),
            Token::Colon => out.push(':'),
            Token::Semicolon => out.push(';'),
            Token::OpenParen => out.push('('),
            Token::CloseParen => out.push(')'),
            Token::OpenSquare => out.push('['),
            Token::CloseSquare => out.push(']'),
            Token::OpenCurly => out.push('{'),
            Token::CloseCurly => out.push('}'),
            Token::Delim(character) => out.push(*character),
            Token::Cdo => out.push_str("<!--"),
            Token::Cdc => out.push_str("-->"),
            Token::Eof => {}
        }
    }
    out.trim().to_string()
}

/// 按最外层的某个记号切分。
fn split_top_level(tokens: &[Token], separator: Token) -> Vec<Vec<Token>> {
    let mut groups = vec![Vec::new()];
    let mut depth = 0i32;
    for token in tokens {
        match token {
            Token::OpenParen | Token::OpenSquare | Token::Function(_) => depth += 1,
            Token::CloseParen | Token::CloseSquare => depth -= 1,
            _ => {}
        }
        if depth == 0 && *token == separator {
            groups.push(Vec::new());
            continue;
        }
        groups.last_mut().expect("至少有一组").push(token.clone());
    }
    groups
}

/// 从声明表里取出一条字体声明。
fn font_face_from(declarations: &[Declaration]) -> FontFace {
    let mut face = FontFace::default();
    for declaration in declarations {
        match declaration.property.as_str() {
            "font-family" => {
                face.family = match &declaration.value {
                    PropertyValue::String(text) => text.clone(),
                    other => other.to_string(),
                };
            }
            "font-weight" => face.weight = declaration.value.to_string(),
            "font-style" => face.style = declaration.value.to_string(),
            "src" => face.source = declaration.value.to_string(),
            _ => {}
        }
    }
    face
}

/// 求值一条媒体查询。
///
/// 支持媒体类型 `all` 与 `screen`，以及 `min-width`、`max-width`、
/// `min-height`、`max-height` 几个特性，用 `and` 串起来。
fn evaluate_media_query(tokens: &[Token], context: MediaContext) -> bool {
    let text = render_tokens(tokens).to_ascii_lowercase();
    if text.trim().is_empty() {
        return true;
    }

    // 多个查询用逗号分隔，命中任意一个即可。
    for query in text.split(',') {
        if evaluate_single_query(query.trim(), context) {
            return true;
        }
    }
    false
}

/// 求值单个媒体查询。
fn evaluate_single_query(query: &str, context: MediaContext) -> bool {
    if query.is_empty() {
        return true;
    }
    // `not` 开头的查询整体取反。
    let (negated, body) = match query.strip_prefix("not ") {
        Some(rest) => (true, rest.trim()),
        None => (false, query),
    };

    let mut result = true;
    let mut saw_condition = false;

    for part in body.split(" and ") {
        // 特性条件写成 `(min-width: 600px)`，先把外层括号剥掉。
        let part = part.trim();
        let part = part
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .unwrap_or(part)
            .trim();
        if part.is_empty() {
            continue;
        }
        if part == "all" || part == "screen" {
            saw_condition = true;
            continue;
        }
        if part == "print" || part == "speech" {
            // 打印样式在屏幕上看不生效。
            result = false;
            saw_condition = true;
            continue;
        }
        if let Some(value) = part.strip_prefix("min-width:") {
            saw_condition = true;
            if let Some(length) = parse_media_length(value.trim()) {
                result &= context.width >= length;
                continue;
            }
        }
        if let Some(value) = part.strip_prefix("max-width:") {
            saw_condition = true;
            if let Some(length) = parse_media_length(value.trim()) {
                result &= context.width <= length;
                continue;
            }
        }
        if let Some(value) = part.strip_prefix("min-height:") {
            saw_condition = true;
            if let Some(length) = parse_media_length(value.trim()) {
                result &= context.height >= length;
                continue;
            }
        }
        if let Some(value) = part.strip_prefix("max-height:") {
            saw_condition = true;
            if let Some(length) = parse_media_length(value.trim()) {
                result &= context.height <= length;
                continue;
            }
        }
        // 认不出的条件按不匹配处理，宁可少应用也不要错应用。
        result = false;
        saw_condition = true;
    }

    if !saw_condition {
        return !negated;
    }
    result != negated
}

/// 解析媒体查询里的长度，只认绝对单位与像素。
fn parse_media_length(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if let Some(number) = trimmed.strip_suffix("px") {
        return number.trim().parse::<f64>().ok();
    }
    if let Some(number) = trimmed.strip_suffix("em") {
        // 媒体查询里的 em 按初始字号 16 像素算。
        return number.trim().parse::<f64>().ok().map(|value| value * 16.0);
    }
    if let Some(number) = trimmed.strip_suffix("rem") {
        return number.trim().parse::<f64>().ok().map(|value| value * 16.0);
    }
    trimmed.parse::<f64>().ok()
}

/// 解析一段 CSS，使用默认的媒体环境。
pub fn parse_stylesheet(css: &str) -> StyleSheet {
    StyleSheet::parse(css)
}

/// 解析一段 CSS，按给定的视口求值媒体查询。
pub fn parse_stylesheet_with_media(css: &str, media: MediaContext) -> StyleSheet {
    Parser::new(css).with_media(media).parse_stylesheet()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析样式表，解析失败时直接 panic。
    fn parse(css: &str) -> StyleSheet {
        StyleSheet::parse(css)
    }

    /// 取某条规则某个属性的值。
    fn value_of(sheet: &StyleSheet, index: usize, property: &str) -> Option<PropertyValue> {
        sheet.rules[index]
            .declarations
            .iter()
            .find(|declaration| declaration.property == property)
            .map(|declaration| declaration.value.clone())
    }

    #[test]
    fn simple_rule() {
        let sheet = parse("div { color: red; margin: 0 }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert_eq!(
            value_of(&sheet, 0, "color"),
            Some(PropertyValue::Color(Color::rgba(255, 0, 0, 255)))
        );
        assert_eq!(
            value_of(&sheet, 0, "margin"),
            Some(PropertyValue::Length(Length::ZERO))
        );
    }

    #[test]
    fn multiple_rules_and_selectors() {
        let sheet = parse("h1, h2 { color: blue } p { color: green }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
    }

    #[test]
    fn lengths_are_parsed() {
        let sheet = parse("div { width: 50%; height: 12px; margin: 1em 2em }");
        assert_eq!(
            value_of(&sheet, 0, "width"),
            Some(PropertyValue::Length(Length::percent(50.0)))
        );
        assert_eq!(
            value_of(&sheet, 0, "height"),
            Some(PropertyValue::Length(Length::px(12.0)))
        );
        assert_eq!(
            value_of(&sheet, 0, "margin"),
            Some(PropertyValue::LengthList(vec![
                Length {
                    value: 1.0,
                    unit: LengthUnit::Em
                },
                Length {
                    value: 2.0,
                    unit: LengthUnit::Em
                },
            ]))
        );
    }

    #[test]
    fn colors_in_various_forms() {
        let sheet = parse(
            "a { color: #ff0000 } b { color: rgb(0, 255, 0) } c { color: rgba(0,0,0,0.5) } d { color: transparent }",
        );
        assert_eq!(
            value_of(&sheet, 0, "color"),
            Some(PropertyValue::Color(Color::rgba(255, 0, 0, 255)))
        );
        assert_eq!(
            value_of(&sheet, 1, "color"),
            Some(PropertyValue::Color(Color::rgba(0, 255, 0, 255)))
        );
        assert_eq!(
            value_of(&sheet, 2, "color"),
            Some(PropertyValue::Color(Color::rgba(0, 0, 0, 128)))
        );
        assert_eq!(
            value_of(&sheet, 3, "color"),
            Some(PropertyValue::Color(Color::TRANSPARENT))
        );
    }

    #[test]
    fn important_flag() {
        let sheet = parse("div { color: red !important; width: 1px }");
        let declarations = &sheet.rules[0].declarations;
        assert!(declarations[0].important);
        assert!(!declarations[1].important);
    }

    #[test]
    fn important_with_spaces() {
        let sheet = parse("div { color: red ! important }");
        assert!(sheet.rules[0].declarations[0].important);
    }

    #[test]
    fn mixed_value_list() {
        let sheet = parse("div { border: 1px solid red }");
        match value_of(&sheet, 0, "border") {
            Some(PropertyValue::MixedList(values)) => assert_eq!(values.len(), 3),
            other => panic!("应当是混合列表，实际是 {other:?}"),
        }
    }

    #[test]
    fn keyword_list_for_font_family() {
        let sheet = parse("div { font-family: Arial, sans-serif }");
        assert_eq!(
            value_of(&sheet, 0, "font-family"),
            Some(PropertyValue::KeywordList(vec![
                "Arial".into(),
                "sans-serif".into()
            ]))
        );
    }

    #[test]
    fn url_values() {
        let sheet = parse("div { background-image: url(a.png) }");
        match value_of(&sheet, 0, "background-image") {
            Some(value) => assert!(value.to_string().contains("a.png"), "实际是 {value}"),
            None => panic!("应当有 background-image"),
        }
    }

    #[test]
    fn comments_are_ignored() {
        let sheet = parse("/* 头部注释 */ div { /* 内部 */ color: red } /* 尾部 */");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }

    #[test]
    fn html_comments_around_rules_are_ignored() {
        // 老网页把样式表整段套进 HTML 注释里，规范允许 CDO 与 CDC 出现在顶层
        // 和每条规则之后。不跳它们的话第一条规则会被当成无效选择器丢掉。
        let sheet = parse("<!-- div { color: red } --> p { color: blue } -->");
        assert_eq!(sheet.rules.len(), 2, "两条规则都该留下");
        assert_eq!(
            value_of(&sheet, 0, "color"),
            Some(PropertyValue::Color(Color::rgba(255, 0, 0, 255)))
        );
        assert_eq!(
            value_of(&sheet, 1, "color"),
            Some(PropertyValue::Color(Color::rgba(0, 0, 255, 255)))
        );
    }

    #[test]
    fn a_charset_declaration_does_not_disturb_the_rules_after_it() {
        // `@charset` 只声明这份样式表的编码，不是一条规则。它不能把后面的
        // 规则带下水，也不能自己变成一条规则。
        let sheet = parse("@charset \"utf-8\"; div { color: red }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert_eq!(
            value_of(&sheet, 0, "color"),
            Some(PropertyValue::Color(Color::rgba(255, 0, 0, 255)))
        );
    }

    #[test]
    fn malformed_declarations_are_skipped() {
        let sheet = parse("div { color red; width: 1px; : ; height: 2px }");
        assert_eq!(sheet.rules.len(), 1);
        let properties: Vec<&str> = sheet.rules[0]
            .declarations
            .iter()
            .map(|declaration| declaration.property.as_str())
            .collect();
        assert_eq!(properties, vec!["width", "height"]);
    }

    #[test]
    fn rule_without_block_is_dropped() {
        let sheet = parse("div; p { color: red }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn media_query_matching_viewport() {
        let css = "@media (min-width: 600px) { div { color: red } }";
        let wide = parse_stylesheet_with_media(
            css,
            MediaContext {
                width: 1024.0,
                height: 800.0,
            },
        );
        assert_eq!(wide.rules.len(), 1, "宽视口下规则应当生效");

        let narrow = parse_stylesheet_with_media(
            css,
            MediaContext {
                width: 400.0,
                height: 800.0,
            },
        );
        assert!(narrow.rules.is_empty(), "窄视口下规则不应当生效");
    }

    #[test]
    fn media_query_max_width() {
        let css = "@media (max-width: 600px) { div { color: red } }";
        let narrow = parse_stylesheet_with_media(
            css,
            MediaContext {
                width: 400.0,
                height: 800.0,
            },
        );
        assert_eq!(narrow.rules.len(), 1);
    }

    #[test]
    fn media_query_screen_type() {
        let sheet = parse("@media screen { div { color: red } }");
        assert_eq!(sheet.rules.len(), 1);

        let print = parse("@media print { div { color: red } }");
        assert!(print.rules.is_empty());
    }

    #[test]
    fn media_query_with_and() {
        let css = "@media screen and (min-width: 400px) { div { color: red } }";
        let sheet = parse(css);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn media_query_negation() {
        let css = "@media not print { div { color: red } }";
        let sheet = parse(css);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn media_query_list() {
        let css = "@media print, screen { div { color: red } }";
        let sheet = parse(css);
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn font_face_is_collected() {
        let sheet =
            parse("@font-face { font-family: \"MyFont\"; src: url(my.woff2); font-weight: 400 }");
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "MyFont");
        assert!(sheet.font_faces[0].source.contains("my.woff2"));
    }

    #[test]
    fn unknown_at_rules_are_skipped() {
        let sheet = parse(
            "@charset \"utf-8\"; @keyframes spin { from { color: red } } div { color: blue }",
        );
        assert_eq!(sheet.rules.len(), 1, "只有普通的 div 规则应当保留");
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }

    #[test]
    fn invalid_selector_drops_rule_only() {
        let sheet = parse("..bad { color: red } p { color: blue }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn nested_parentheses_do_not_break_rules() {
        let sheet = parse("div { width: calc((100% - 10px) / 2); color: red }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
    }

    #[test]
    fn empty_stylesheet() {
        let sheet = parse("");
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn unclosed_block_still_yields_declarations() {
        let sheet = parse("div { color: red");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }

    #[test]
    fn multiple_declarations_keep_order() {
        let sheet = parse("div { color: red; color: blue }");
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(
            sheet.rules[0]
                .declarations
                .last()
                .map(|declaration| declaration.value.to_string()),
            Some("#0000ff".to_string())
        );
    }
}
