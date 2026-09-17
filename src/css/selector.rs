//! CSS 选择器：类型定义、解析、匹配与优先级。

use super::tokenizer::Token;
use crate::dom::node::{Document, NodeData, NodeId};
use std::fmt;

/// 组合子，连接两个复合选择器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// 后代，空格。
    Descendant,
    /// 直接子元素，`>`。
    Child,
    /// 紧邻的兄弟，`+`。
    NextSibling,
    /// 之后的兄弟，`~`。
    SubsequentSibling,
}

/// 类型选择器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeSelector {
    /// `*`
    Universal,
    /// 标签名，已转小写。
    Name(String),
}

/// 属性选择器的比较方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeOperator {
    /// 只判断属性是否存在。
    Exists,
    /// `=`
    Equals,
    /// `~=`
    Includes,
    /// `|=`
    DashMatch,
    /// `^=`
    Prefix,
    /// `$=`
    Suffix,
    /// `*=`
    Substring,
}

/// 属性选择器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSelector {
    /// 属性名，已转小写。
    pub name: String,
    /// 比较方式。
    pub operator: AttributeOperator,
    /// 比较用的值。
    pub value: String,
    /// 是否忽略大小写，来自 `i` 标志。
    pub case_insensitive: bool,
}

/// `an+b` 形式的表达式，用在 `:nth-child()` 这类伪类里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NthExpression {
    /// 系数 a。
    pub a: i64,
    /// 常数 b。
    pub b: i64,
}

impl NthExpression {
    /// 判断序号 `index` 是否命中，`index` 从 1 开始。
    pub fn matches(&self, index: i64) -> bool {
        if self.a == 0 {
            return index == self.b;
        }
        let difference = index - self.b;
        difference % self.a == 0 && difference / self.a >= 0
    }
}

/// 伪类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PseudoClass {
    /// `:root`
    Root,
    /// `:empty`
    Empty,
    /// `:first-child`
    FirstChild,
    /// `:last-child`
    LastChild,
    /// `:only-child`
    OnlyChild,
    /// `:first-of-type`
    FirstOfType,
    /// `:last-of-type`
    LastOfType,
    /// `:only-of-type`
    OnlyOfType,
    /// `:nth-child()`
    NthChild(NthExpression),
    /// `:nth-last-child()`
    NthLastChild(NthExpression),
    /// `:nth-of-type()`
    NthOfType(NthExpression),
    /// `:nth-last-of-type()`
    NthLastOfType(NthExpression),
    /// `:not()`
    Not(Vec<CompoundSelector>),
    /// `:is()` 与 `:where()`，任一匹配即可。
    Is(Vec<CompoundSelector>),
    /// `:hover`
    Hover,
    /// `:active`
    Active,
    /// `:focus`
    Focus,
    /// `:visited`
    Visited,
    /// `:link`
    Link,
    /// `:checked`
    Checked,
    /// `:disabled`
    Disabled,
    /// `:enabled`
    Enabled,
    /// `:lang(语言标签)`
    ///
    /// 匹配元素所处的语言，按 CSS 2.1 §5.11.4 的规则：取元素自己的 `lang`
    /// 属性，没有就沿祖先往上找，因为语言是继承下来的。匹配按子标签算，
    /// `:lang(en)` 命中 `en-US`。
    Lang(String),
}

/// 一个不含组合子的选择器，如 `div.note[data-x="1"]:first-child`。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    /// 类型选择器，省略时等价于 `*`。
    pub type_selector: Option<TypeSelector>,
    /// id 选择器。
    pub id: Option<String>,
    /// 类选择器。
    pub classes: Vec<String>,
    /// 属性选择器。
    pub attributes: Vec<AttributeSelector>,
    /// 伪类。
    pub pseudo_classes: Vec<PseudoClass>,
    /// 伪元素。
    pub pseudo_element: Option<String>,
}

impl CompoundSelector {
    /// 该复合选择器是否只由伪元素构成，没有实际筛选条件。
    pub fn is_universal(&self) -> bool {
        matches!(self.type_selector, None | Some(TypeSelector::Universal))
            && self.id.is_none()
            && self.classes.is_empty()
            && self.attributes.is_empty()
            && self.pseudo_classes.is_empty()
    }
}

/// 一个完整的选择器，由若干复合选择器与组合子串起来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexSelector {
    /// 按书写顺序排列的复合选择器。
    pub compounds: Vec<CompoundSelector>,
    /// 相邻两个复合选择器之间的组合子，长度比 `compounds` 少一。
    pub combinators: Vec<Combinator>,
}

impl ComplexSelector {
    /// 预计算的优先级。
    pub fn specificity(&self) -> Specificity {
        let mut specificity = Specificity::default();
        for compound in &self.compounds {
            if compound.id.is_some() {
                specificity.ids += 1;
            }
            specificity.classes += compound.classes.len() as u32;
            specificity.classes += compound.attributes.len() as u32;
            for pseudo in &compound.pseudo_classes {
                match pseudo {
                    // `:not()` 与 `:is()` 取内部选择器里最高的那一个。
                    PseudoClass::Not(inner) | PseudoClass::Is(inner) => {
                        let best = inner
                            .iter()
                            .map(compound_specificity)
                            .max()
                            .unwrap_or_default();
                        specificity.ids += best.ids;
                        specificity.classes += best.classes;
                        specificity.types += best.types;
                    }
                    _ => specificity.classes += 1,
                }
            }
            if compound.pseudo_element.is_some() {
                specificity.types += 1;
            }
            if let Some(TypeSelector::Name(_)) = &compound.type_selector {
                specificity.types += 1
            }
        }
        specificity
    }

    /// 该选择器是否只针对伪元素。
    pub fn pseudo_element(&self) -> Option<&str> {
        self.compounds
            .last()
            .and_then(|compound| compound.pseudo_element.as_deref())
    }
}

/// 把复合选择器单独算一次优先级，供 `:not()` 这类嵌套使用。
fn compound_specificity(compound: &CompoundSelector) -> Specificity {
    ComplexSelector {
        compounds: vec![compound.clone()],
        combinators: Vec::new(),
    }
    .specificity()
}

/// 优先级三元组，依次是 id、类与属性、类型。
///
/// 比较时从左往右比，数值大的胜出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub struct Specificity {
    /// id 选择器的个数。
    pub ids: u32,
    /// 类、属性与伪类的个数。
    pub classes: u32,
    /// 类型与伪元素的个数。
    pub types: u32,
}

impl fmt::Display for Specificity {
    /// 输出成 `(a,b,c)` 的形式，方便调试与测试。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{},{})", self.ids, self.classes, self.types)
    }
}

/// 元素的状态，用于让动态伪类参与匹配。
///
/// 初排版时全部为假，交互引起状态变化后重新计算样式即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ElementState {
    /// 指针是否悬停在元素上。
    pub hovered: bool,
    /// 元素是否正被按下。
    pub active: bool,
    /// 元素是否持有焦点。
    pub focused: bool,
    /// 链接是否已访问。
    pub visited: bool,
}

/// 选择器解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError {
    /// 人类可读的说明。
    pub message: String,
}

impl SelectorError {
    /// 构造错误。
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SelectorError {
    /// 直接输出说明。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SelectorError {}

/// 从记号切片里解析选择器列表。
///
/// `tokens` 应当是一个规则前奏的全部记号，不含后面的花括号。
pub fn parse_selector_list(tokens: &[Token]) -> Result<Vec<ComplexSelector>, SelectorError> {
    let mut parser = SelectorParser { tokens, pos: 0 };
    let mut list = Vec::new();
    parser.skip_whitespace();
    if parser.at_end() {
        return Err(SelectorError::new("规则前面没有选择器"));
    }
    loop {
        list.push(parser.parse_complex()?);
        parser.skip_whitespace();
        if parser.peek_is_comma() {
            parser.pos += 1;
            parser.skip_whitespace();
            // 逗号两边都得是选择器。规范 G.1 的 selector_group 产生式不允许
            // 悬空的逗号，`div, { }` 是语法错误，整条规则作废——少了这一条，
            // 它会当成只有 div 一条，把写错的选择器静默吞掉。
            if parser.at_end() {
                return Err(SelectorError::new("逗号后面缺少选择器"));
            }
            continue;
        }
        if parser.at_end() {
            break;
        }
        return Err(SelectorError::new("选择器列表里出现了意外的记号"));
    }
    Ok(list)
}

/// 选择器解析器。
struct SelectorParser<'a> {
    /// 记号序列。
    tokens: &'a [Token],
    /// 当前位置。
    pos: usize,
}

impl SelectorParser<'_> {
    /// 当前位置是否已经读完。
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// 当前记号。
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// 当前记号是否是逗号。
    fn peek_is_comma(&self) -> bool {
        matches!(self.peek(), Some(Token::Comma))
    }

    /// 跳过空白。
    fn skip_whitespace(&mut self) {
        while self.tokens.get(self.pos).is_some_and(Token::is_whitespace) {
            self.pos += 1;
        }
    }

    /// 解析一个复杂选择器。
    fn parse_complex(&mut self) -> Result<ComplexSelector, SelectorError> {
        let mut compounds = vec![self.parse_compound()?];
        let mut combinators = Vec::new();

        loop {
            let had_whitespace = {
                let before = self.pos;
                self.skip_whitespace();
                self.pos > before
            };

            let combinator = match self.peek() {
                Some(Token::Delim('>')) => {
                    self.pos += 1;
                    Combinator::Child
                }
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    Combinator::NextSibling
                }
                Some(Token::Delim('~')) => {
                    self.pos += 1;
                    Combinator::SubsequentSibling
                }
                Some(Token::Comma) | None => break,
                _ if had_whitespace => Combinator::Descendant,
                _ => break,
            };

            self.skip_whitespace();
            // 组合子后面必须还有选择器。规范 G.1 的 selector 产生式里
            // 组合子两边都是必需的，`div > { }` 是语法错误，整条规则作废——
            // 少了这一条，悬空的组合子会被悄悄丢掉，规则照常生效。
            if self.at_end() || self.peek_is_comma() {
                return Err(SelectorError::new("组合子后面缺少选择器"));
            }
            compounds.push(self.parse_compound()?);
            combinators.push(combinator);
        }

        Ok(ComplexSelector {
            compounds,
            combinators,
        })
    }

    /// 解析一个复合选择器。
    fn parse_compound(&mut self) -> Result<CompoundSelector, SelectorError> {
        let mut compound = CompoundSelector::default();
        let mut consumed = false;

        // 开头的类型选择器。
        match self.peek() {
            Some(Token::Ident(name)) => {
                let name = name.clone();
                self.pos += 1;
                compound.type_selector = Some(TypeSelector::Name(name.to_ascii_lowercase()));
                consumed = true;
            }
            Some(Token::Delim('*')) => {
                self.pos += 1;
                compound.type_selector = Some(TypeSelector::Universal);
                consumed = true;
            }
            _ => {}
        }

        loop {
            match self.peek() {
                Some(Token::Hash(value, true)) => {
                    let value = value.clone();
                    self.pos += 1;
                    compound.id = Some(value);
                    consumed = true;
                }
                Some(Token::Delim('.')) => {
                    self.pos += 1;
                    match self.peek() {
                        Some(Token::Ident(name)) => {
                            compound.classes.push(name.clone());
                            self.pos += 1;
                            consumed = true;
                        }
                        _ => return Err(SelectorError::new("类选择器后面缺少名字")),
                    }
                }
                Some(Token::OpenSquare) => {
                    compound.attributes.push(self.parse_attribute()?);
                    consumed = true;
                }
                Some(Token::Colon) => {
                    self.pos += 1;
                    // `::` 之后一律是伪元素。
                    if matches!(self.peek(), Some(Token::Colon)) {
                        self.pos += 1;
                        match self.peek() {
                            Some(Token::Ident(name)) => {
                                compound.pseudo_element = Some(name.to_ascii_lowercase());
                                self.pos += 1;
                                consumed = true;
                            }
                            _ => return Err(SelectorError::new("伪元素后面缺少名字")),
                        }
                        continue;
                    }
                    // 单冒号后面跟的是 CSS 2.1 那四个伪元素之一时，它也是伪元素。
                    // 规范 §5.12 就是这么写的，双冒号是后来的写法，两种都收。
                    if compound.pseudo_element.is_none()
                        && let Some(Token::Ident(name)) = self.peek()
                        && is_legacy_pseudo_element(name)
                    {
                        compound.pseudo_element = Some(name.to_ascii_lowercase());
                        self.pos += 1;
                        consumed = true;
                        continue;
                    }
                    let pseudo = self.parse_pseudo_class()?;
                    compound.pseudo_classes.push(pseudo);
                    consumed = true;
                }
                _ => break,
            }
        }

        if !consumed {
            return Err(SelectorError::new("这里应当是一个选择器"));
        }
        Ok(compound)
    }

    /// 解析属性选择器，当前位置在 `[` 上。
    fn parse_attribute(&mut self) -> Result<AttributeSelector, SelectorError> {
        self.pos += 1;
        self.skip_whitespace();
        let name = match self.peek() {
            Some(Token::Ident(name)) => {
                let name = name.to_ascii_lowercase();
                self.pos += 1;
                name
            }
            _ => return Err(SelectorError::new("属性选择器缺少属性名")),
        };
        self.skip_whitespace();

        let operator = match self.peek() {
            Some(Token::CloseSquare) => {
                self.pos += 1;
                return Ok(AttributeSelector {
                    name,
                    operator: AttributeOperator::Exists,
                    value: String::new(),
                    case_insensitive: false,
                });
            }
            Some(Token::Delim('=')) => {
                self.pos += 1;
                AttributeOperator::Equals
            }
            Some(Token::Delim(character @ ('~' | '|' | '^' | '$' | '*'))) => {
                let character = *character;
                self.pos += 1;
                if !matches!(self.peek(), Some(Token::Delim('='))) {
                    return Err(SelectorError::new("属性选择器的运算符不完整"));
                }
                self.pos += 1;
                match character {
                    '~' => AttributeOperator::Includes,
                    '|' => AttributeOperator::DashMatch,
                    '^' => AttributeOperator::Prefix,
                    '$' => AttributeOperator::Suffix,
                    _ => AttributeOperator::Substring,
                }
            }
            _ => return Err(SelectorError::new("属性选择器里出现了意外的记号")),
        };

        self.skip_whitespace();
        let value = match self.peek() {
            Some(Token::Ident(value)) => {
                let value = value.clone();
                self.pos += 1;
                value
            }
            Some(Token::String(value)) => {
                let value = value.clone();
                self.pos += 1;
                value
            }
            Some(Token::Number { value, .. }) => {
                let value = number_to_css_string(*value);
                self.pos += 1;
                value
            }
            _ => return Err(SelectorError::new("属性选择器缺少比较值")),
        };

        self.skip_whitespace();
        let mut case_insensitive = false;
        if let Some(Token::Ident(flag)) = self.peek()
            && flag.eq_ignore_ascii_case("i")
        {
            case_insensitive = true;
            self.pos += 1;
            self.skip_whitespace();
        }

        if !matches!(self.peek(), Some(Token::CloseSquare)) {
            return Err(SelectorError::new("属性选择器没有闭合"));
        }
        self.pos += 1;
        Ok(AttributeSelector {
            name,
            operator,
            value,
            case_insensitive,
        })
    }

    /// 解析伪类，当前位置在 `:` 之后。
    fn parse_pseudo_class(&mut self) -> Result<PseudoClass, SelectorError> {
        let name = match self.peek() {
            Some(Token::Ident(name)) => {
                let name = name.to_ascii_lowercase();
                self.pos += 1;
                name
            }
            Some(Token::Function(name)) => {
                let name = name.to_ascii_lowercase();
                self.pos += 1;
                return self.parse_functional_pseudo(&name);
            }
            _ => return Err(SelectorError::new("伪类缺少名字")),
        };

        Ok(match name.as_str() {
            "root" => PseudoClass::Root,
            "empty" => PseudoClass::Empty,
            "first-child" => PseudoClass::FirstChild,
            "last-child" => PseudoClass::LastChild,
            "only-child" => PseudoClass::OnlyChild,
            "first-of-type" => PseudoClass::FirstOfType,
            "last-of-type" => PseudoClass::LastOfType,
            "only-of-type" => PseudoClass::OnlyOfType,
            "hover" => PseudoClass::Hover,
            "active" => PseudoClass::Active,
            "focus" => PseudoClass::Focus,
            "visited" => PseudoClass::Visited,
            "link" => PseudoClass::Link,
            "checked" => PseudoClass::Checked,
            "disabled" => PseudoClass::Disabled,
            "enabled" => PseudoClass::Enabled,
            other => {
                return Err(SelectorError::new(format!("不认识的伪类 :{other}")));
            }
        })
    }

    /// 解析带参数的伪类，函数名与左括号已经被消费。
    fn parse_functional_pseudo(&mut self, name: &str) -> Result<PseudoClass, SelectorError> {
        let pseudo = match name {
            "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                let expression = self.parse_nth_expression()?;
                match name {
                    "nth-child" => PseudoClass::NthChild(expression),
                    "nth-last-child" => PseudoClass::NthLastChild(expression),
                    "nth-of-type" => PseudoClass::NthOfType(expression),
                    _ => PseudoClass::NthLastOfType(expression),
                }
            }
            "lang" => PseudoClass::Lang(self.parse_lang_argument()?),
            "not" => {
                let inner = self.parse_selector_arguments()?;
                PseudoClass::Not(inner)
            }
            "is" | "where" => {
                let inner = self.parse_selector_arguments()?;
                PseudoClass::Is(inner)
            }
            other => {
                return Err(SelectorError::new(format!("不认识的函数式伪类 :{other}()")));
            }
        };
        if !matches!(self.peek(), Some(Token::CloseParen)) {
            return Err(SelectorError::new("伪类没有闭合"));
        }
        self.pos += 1;
        Ok(pseudo)
    }

    /// 解析 `:lang()` 里的语言标签，带引号与不带引号两种写法都收。
    ///
    /// 读完之后游标停在右括号上，由调用方统一收尾。
    fn parse_lang_argument(&mut self) -> Result<String, SelectorError> {
        self.skip_whitespace();
        let tag = match self.peek() {
            Some(Token::Ident(name)) => name.clone(),
            Some(Token::String(text)) => text.clone(),
            _ => return Err(SelectorError::new(":lang() 里应当是一个语言标签")),
        };
        self.pos += 1;
        self.skip_whitespace();
        // 标签统一成小写再比：`en-US` 与 `EN-us` 是同一个语言。
        Ok(tag.to_ascii_lowercase())
    }

    /// 解析括号里的选择器列表，只取复合选择器。
    fn parse_selector_arguments(&mut self) -> Result<Vec<CompoundSelector>, SelectorError> {
        let mut out = Vec::new();
        loop {
            self.skip_whitespace();
            if matches!(self.peek(), Some(Token::CloseParen)) {
                break;
            }
            out.push(self.parse_compound()?);
            self.skip_whitespace();
            if self.peek_is_comma() {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(out)
    }

    /// 解析 `an+b` 表达式。
    fn parse_nth_expression(&mut self) -> Result<NthExpression, SelectorError> {
        self.skip_whitespace();
        let mut a: i64 = 0;
        let mut b: i64 = 0;
        let mut has_a = false;

        // 奇数偶数关键字。
        if let Some(Token::Ident(keyword)) = self.peek() {
            let keyword = keyword.to_ascii_lowercase();
            if keyword == "odd" || keyword == "even" {
                self.pos += 1;
                self.skip_whitespace();
                return Ok(if keyword == "odd" {
                    NthExpression { a: 2, b: 1 }
                } else {
                    NthExpression { a: 2, b: 0 }
                });
            }
        }

        // a 部分，可能写成 `2n`、`-n`、`n`。
        match self.peek() {
            Some(Token::Dimension { value, unit, .. }) if unit.eq_ignore_ascii_case("n") => {
                a = *value as i64;
                has_a = true;
                self.pos += 1;
            }
            Some(Token::Ident(name)) if name.eq_ignore_ascii_case("n") => {
                a = 1;
                has_a = true;
                self.pos += 1;
            }
            Some(Token::Ident(name))
                if name.eq_ignore_ascii_case("-n") || name.eq_ignore_ascii_case("+n") =>
            {
                a = if name.starts_with('-') { -1 } else { 1 };
                has_a = true;
                self.pos += 1;
            }
            Some(Token::Delim('-')) => {
                if let Some(Token::Ident(name)) = self.tokens.get(self.pos + 1)
                    && name.eq_ignore_ascii_case("n")
                {
                    a = -1;
                    has_a = true;
                    self.pos += 2;
                }
            }
            _ => {}
        }

        if has_a {
            self.skip_whitespace();
            // 常数项有三种写法：`2n+1` 里的 `+1` 会被词法器合成一个正数记号，
            // `2n - 1` 则是单独的减号记号加数字，`2n-1` 又会合成负数记号。
            match self.peek() {
                Some(Token::Delim('+')) => {
                    self.pos += 1;
                    self.skip_whitespace();
                    match self.peek() {
                        Some(Token::Number { value, .. }) => {
                            b = *value as i64;
                            self.pos += 1;
                        }
                        _ => return Err(SelectorError::new("nth 表达式的常数项不完整")),
                    }
                }
                Some(Token::Delim('-')) => {
                    self.pos += 1;
                    self.skip_whitespace();
                    match self.peek() {
                        Some(Token::Number { value, .. }) => {
                            b = -(*value as i64);
                            self.pos += 1;
                        }
                        _ => return Err(SelectorError::new("nth 表达式的常数项不完整")),
                    }
                }
                Some(Token::Number { value, .. }) => {
                    b = *value as i64;
                    self.pos += 1;
                }
                _ => {}
            }
        } else {
            // 纯数字形式。
            match self.peek() {
                Some(Token::Number { value, .. }) => {
                    b = *value as i64;
                    self.pos += 1;
                }
                _ => return Err(SelectorError::new("nth 表达式无法识别")),
            }
        }

        self.skip_whitespace();
        Ok(NthExpression { a, b })
    }
}

/// 把数字转成 CSS 里的字面量写法。
///
/// 整数不写小数点，其余交给 Rust 的最短往返输出。
pub fn number_to_css_string(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// 判断选择器是否匹配某个元素。
pub fn matches(
    selector: &ComplexSelector,
    document: &Document,
    element: NodeId,
    state: &ElementState,
) -> bool {
    match_from(
        selector,
        selector.compounds.len() - 1,
        document,
        element,
        state,
    )
}

/// 从第 `index` 个复合选择器开始往前匹配。
fn match_from(
    selector: &ComplexSelector,
    index: usize,
    document: &Document,
    element: NodeId,
    state: &ElementState,
) -> bool {
    if !match_compound(&selector.compounds[index], document, element, state) {
        return false;
    }
    if index == 0 {
        return true;
    }

    match selector.combinators[index - 1] {
        Combinator::Child => match document.parent(element) {
            Some(parent) => match_from(selector, index - 1, document, parent, state),
            None => false,
        },
        Combinator::Descendant => {
            let mut current = document.parent(element);
            while let Some(ancestor) = current {
                if match_from(selector, index - 1, document, ancestor, state) {
                    return true;
                }
                current = document.parent(ancestor);
            }
            false
        }
        Combinator::NextSibling => match previous_element_sibling(document, element) {
            Some(sibling) => match_from(selector, index - 1, document, sibling, state),
            None => false,
        },
        Combinator::SubsequentSibling => {
            let mut current = previous_element_sibling(document, element);
            while let Some(sibling) = current {
                if match_from(selector, index - 1, document, sibling, state) {
                    return true;
                }
                current = previous_element_sibling(document, sibling);
            }
            false
        }
    }
}

/// 取上一个元素兄弟节点。
fn previous_element_sibling(document: &Document, element: NodeId) -> Option<NodeId> {
    let mut current = document.previous_sibling(element);
    while let Some(sibling) = current {
        if document.node(sibling).as_element().is_some() {
            return Some(sibling);
        }
        current = document.previous_sibling(sibling);
    }
    None
}

/// 判断一个复合选择器是否匹配某个元素。
fn match_compound(
    compound: &CompoundSelector,
    document: &Document,
    element: NodeId,
    state: &ElementState,
) -> bool {
    let Some(selector_element) = document.element(element) else {
        return false;
    };

    match &compound.type_selector {
        None | Some(TypeSelector::Universal) => {}
        Some(TypeSelector::Name(name)) => {
            if &selector_element.name != name {
                return false;
            }
        }
    }

    if let Some(id) = &compound.id
        && selector_element.id() != Some(id.as_str())
    {
        return false;
    }

    for class in &compound.classes {
        if !selector_element.has_token("class", class) {
            return false;
        }
    }

    for attribute in &compound.attributes {
        if !match_attribute(attribute, selector_element.get_attribute(&attribute.name)) {
            return false;
        }
    }

    for pseudo in &compound.pseudo_classes {
        if !match_pseudo_class(pseudo, document, element, state) {
            return false;
        }
    }

    true
}

/// 判断属性选择器是否匹配。
fn match_attribute(attribute: &AttributeSelector, actual: Option<&str>) -> bool {
    let Some(actual) = actual else {
        return false;
    };
    // 带 i 标志时忽略大小写。
    let (actual, expected) = if attribute.case_insensitive {
        (
            actual.to_ascii_lowercase(),
            attribute.value.to_ascii_lowercase(),
        )
    } else {
        (actual.to_string(), attribute.value.clone())
    };
    let actual = actual.as_str();
    let expected = expected.as_str();

    match attribute.operator {
        AttributeOperator::Exists => true,
        AttributeOperator::Equals => actual == expected,
        AttributeOperator::Includes => {
            !expected.is_empty() && actual.split_whitespace().any(|item| item == expected)
        }
        AttributeOperator::DashMatch => {
            actual == expected
                || actual
                    .strip_prefix(expected)
                    .is_some_and(|rest| rest.starts_with('-'))
        }
        AttributeOperator::Prefix => !expected.is_empty() && actual.starts_with(expected),
        AttributeOperator::Suffix => !expected.is_empty() && actual.ends_with(expected),
        AttributeOperator::Substring => !expected.is_empty() && actual.contains(expected),
    }
}

/// 判断伪类是否匹配。
fn match_pseudo_class(
    pseudo: &PseudoClass,
    document: &Document,
    element: NodeId,
    state: &ElementState,
) -> bool {
    match pseudo {
        PseudoClass::Root => document.parent(element) == Some(document.root()),
        PseudoClass::Empty => document.children(element).iter().all(|child| {
            match &document.node(*child).data {
                // 注释不算内容。
                NodeData::Comment(_) => true,
                NodeData::Text(text) => text.is_empty(),
                _ => false,
            }
        }),
        PseudoClass::FirstChild => element_index(document, element).is_some_and(|index| index == 0),
        PseudoClass::LastChild => element_index(document, element).is_some_and(|index| {
            document
                .parent(element)
                .is_some_and(|parent| index + 1 == element_count(document, parent))
        }),
        PseudoClass::OnlyChild => document
            .parent(element)
            .is_some_and(|parent| element_count(document, parent) == 1),
        PseudoClass::FirstOfType => type_index(document, element).is_some_and(|index| index == 0),
        PseudoClass::LastOfType => type_index(document, element).is_some_and(|index| {
            type_count(document, element).is_some_and(|count| index + 1 == count)
        }),
        PseudoClass::OnlyOfType => type_count(document, element).is_some_and(|count| count == 1),
        PseudoClass::NthChild(expression) => {
            element_index(document, element).is_some_and(|index| expression.matches(index + 1))
        }
        PseudoClass::NthLastChild(expression) => element_index(document, element)
            .and_then(|index| {
                document
                    .parent(element)
                    .map(|parent| element_count(document, parent) - 1 - index)
            })
            .is_some_and(|from_end| expression.matches(from_end + 1)),
        PseudoClass::NthOfType(expression) => {
            type_index(document, element).is_some_and(|index| expression.matches(index + 1))
        }
        PseudoClass::NthLastOfType(expression) => type_index(document, element)
            .and_then(|index| type_count(document, element).map(|count| count - 1 - index))
            .is_some_and(|from_end| expression.matches(from_end + 1)),
        PseudoClass::Not(inner) => !inner
            .iter()
            .any(|compound| match_compound(compound, document, element, state)),
        PseudoClass::Is(inner) => inner
            .iter()
            .any(|compound| match_compound(compound, document, element, state)),
        PseudoClass::Hover => state.hovered,
        PseudoClass::Active => state.active,
        PseudoClass::Focus => state.focused,
        PseudoClass::Visited => state.visited,
        PseudoClass::Link => document
            .element(element)
            .is_some_and(|el| el.is_html("a") && el.has_attribute("href")),
        PseudoClass::Checked => document
            .element(element)
            .is_some_and(|el| el.has_attribute("checked") || el.has_token("selected", "selected")),
        PseudoClass::Disabled => document
            .element(element)
            .is_some_and(|el| el.has_attribute("disabled")),
        PseudoClass::Enabled => document.element(element).is_some_and(|el| {
            matches!(
                el.name.as_str(),
                "input" | "button" | "select" | "textarea" | "option"
            ) && !el.has_attribute("disabled")
        }),
        PseudoClass::Lang(tag) => element_language(document, element).is_some_and(|language| {
            // 完全相等，或者待查标签是它的前缀且后面紧跟一个连字符——
            // `:lang(en)` 要命中 `en`、`en-US`、`en-GB`，但不命中 `enx`。
            language == *tag
                || language
                    .strip_prefix(tag.as_str())
                    .is_some_and(|rest| rest.starts_with('-'))
        }),
    }
}

/// 是不是 CSS 2.1 里那四个用单冒号写的伪元素。
///
/// 规范 §5.12 把它们写作 `:first-line`、`:first-letter`、`:before`、`:after`。
/// 后来的规范改成双冒号，同时要求实现继续接受这四种的单冒号写法。
fn is_legacy_pseudo_element(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "first-line" | "first-letter" | "before" | "after"
    )
}

/// 取元素所处的语言标签，小写，取不到是 `None`。
///
/// 按 CSS 2.1 §5.11.4：先看元素自己的 `lang` 属性，没有就沿祖先往上找，
/// 因为语言是从祖先继承下来的。`xml:lang` 与 `lang` 等价，两个都认。
fn element_language(document: &Document, element: NodeId) -> Option<String> {
    let mut current = Some(element);
    while let Some(id) = current {
        if let Some(node) = document.element(id)
            && let Some(value) = node
                .get_attribute("lang")
                .or_else(|| node.get_attribute("xml:lang"))
        {
            let tag = value.trim().to_ascii_lowercase();
            // 有 `lang` 属性就以它为准，哪怕值是空串。空串表示「语言未知」，
            // 这时候不该再往上继承——不然一段明确标了未知语言的文字会被
            // 祖先的语种认领走。
            return if tag.is_empty() { None } else { Some(tag) };
        }
        current = document.parent(id);
    }
    None
}

/// 元素在同级元素里的下标，从 0 开始。
fn element_index(document: &Document, element: NodeId) -> Option<i64> {
    let parent = document.parent(element)?;
    let mut index = 0i64;
    for child in document.children(parent) {
        if document.node(*child).as_element().is_some() {
            if *child == element {
                return Some(index);
            }
            index += 1;
        }
    }
    None
}

/// 父元素下元素子节点的个数。
fn element_count(document: &Document, parent: NodeId) -> i64 {
    document
        .children(parent)
        .iter()
        .filter(|child| document.node(**child).as_element().is_some())
        .count() as i64
}

/// 元素在同类型兄弟里的下标，从 0 开始。
fn type_index(document: &Document, element: NodeId) -> Option<i64> {
    let parent = document.parent(element)?;
    let name = document.element(element)?.name.clone();
    let mut index = 0i64;
    for child in document.children(parent) {
        if let Some(sibling) = document.element(*child)
            && sibling.name == name
        {
            if *child == element {
                return Some(index);
            }
            index += 1;
        }
    }
    None
}

/// 元素同类型兄弟的个数。
fn type_count(document: &Document, element: NodeId) -> Option<i64> {
    let parent = document.parent(element)?;
    let name = document.element(element)?.name.clone();
    Some(
        document
            .children(parent)
            .iter()
            .filter(|child| {
                document
                    .element(**child)
                    .is_some_and(|sibling| sibling.name == name)
            })
            .count() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse_document;

    /// 解析一段选择器文本。
    fn parse(text: &str) -> Vec<ComplexSelector> {
        let tokens = super::super::tokenizer::tokenize(text);
        // 去掉结尾的 Eof，其余交给选择器解析器。
        let tokens = &tokens[..tokens.len() - 1];
        parse_selector_list(tokens).unwrap_or_else(|error| panic!("{text} 解析失败: {error}"))
    }

    /// 一段选择器文本能不能解析成功。
    fn parses(text: &str) -> bool {
        let tokens = super::super::tokenizer::tokenize(text);
        let tokens = &tokens[..tokens.len() - 1];
        parse_selector_list(tokens).is_ok()
    }

    /// 解析一个选择器的优先级。
    fn specificity(text: &str) -> Specificity {
        let list = parse(text);
        assert_eq!(list.len(), 1, "只应当有一个选择器");
        list[0].specificity()
    }

    #[test]
    fn type_and_class_and_id() {
        let list = parse("div.note#main");
        assert_eq!(list.len(), 1);
        let compound = &list[0].compounds[0];
        assert_eq!(
            compound.type_selector,
            Some(TypeSelector::Name("div".into()))
        );
        assert_eq!(compound.id.as_deref(), Some("main"));
        assert_eq!(compound.classes, vec!["note"]);
    }

    #[test]
    fn universal_selector() {
        let list = parse("*");
        assert_eq!(
            list[0].compounds[0].type_selector,
            Some(TypeSelector::Universal)
        );
    }

    #[test]
    fn selector_list_with_commas() {
        assert_eq!(parse("h1, h2, h3").len(), 3);
    }

    #[test]
    fn descendant_and_child_combinators() {
        let list = parse("div p");
        assert_eq!(list[0].combinators, vec![Combinator::Descendant]);

        let list = parse("div > p");
        assert_eq!(list[0].combinators, vec![Combinator::Child]);

        let list = parse("h1 + p");
        assert_eq!(list[0].combinators, vec![Combinator::NextSibling]);

        let list = parse("h1 ~ p");
        assert_eq!(list[0].combinators, vec![Combinator::SubsequentSibling]);
    }

    #[test]
    fn whitespace_around_child_combinator() {
        let list = parse("div  >  p");
        assert_eq!(list[0].combinators, vec![Combinator::Child]);
        assert_eq!(list[0].compounds.len(), 2);
    }

    #[test]
    fn attribute_selectors() {
        // 用逗号分隔，空格会变成后代组合子。
        let list = parse(
            r#"[href], [type="text"], [class~=note], [lang|=en], [href^=http], [href$=png], [href*=ex]"#,
        );
        assert_eq!(list.len(), 7);
        assert_eq!(
            list[0].compounds[0].attributes[0].operator,
            AttributeOperator::Exists
        );
        assert_eq!(
            list[1].compounds[0].attributes[0].operator,
            AttributeOperator::Equals
        );
        assert_eq!(list[1].compounds[0].attributes[0].value, "text");
        assert_eq!(
            list[2].compounds[0].attributes[0].operator,
            AttributeOperator::Includes
        );
        assert_eq!(
            list[3].compounds[0].attributes[0].operator,
            AttributeOperator::DashMatch
        );
        assert_eq!(
            list[4].compounds[0].attributes[0].operator,
            AttributeOperator::Prefix
        );
        assert_eq!(
            list[5].compounds[0].attributes[0].operator,
            AttributeOperator::Suffix
        );
        assert_eq!(
            list[6].compounds[0].attributes[0].operator,
            AttributeOperator::Substring
        );
    }

    #[test]
    fn attribute_case_insensitive_flag() {
        let list = parse("[title=x i]");
        assert!(list[0].compounds[0].attributes[0].case_insensitive);
    }

    #[test]
    fn pseudo_classes_and_elements() {
        let list = parse("a:hover");
        assert_eq!(
            list[0].compounds[0].pseudo_classes,
            vec![PseudoClass::Hover]
        );

        let list = parse("p::before");
        assert_eq!(
            list[0].compounds[0].pseudo_element.as_deref(),
            Some("before")
        );
    }

    #[test]
    fn functional_pseudo_classes() {
        let list = parse("li:nth-child(2n+1)");
        assert_eq!(
            list[0].compounds[0].pseudo_classes,
            vec![PseudoClass::NthChild(NthExpression { a: 2, b: 1 })]
        );

        let list = parse("li:nth-child(odd)");
        assert_eq!(
            list[0].compounds[0].pseudo_classes,
            vec![PseudoClass::NthChild(NthExpression { a: 2, b: 1 })]
        );

        let list = parse("li:nth-child(3)");
        assert_eq!(
            list[0].compounds[0].pseudo_classes,
            vec![PseudoClass::NthChild(NthExpression { a: 0, b: 3 })]
        );

        let list = parse("li:nth-child(-n+3)");
        assert_eq!(
            list[0].compounds[0].pseudo_classes,
            vec![PseudoClass::NthChild(NthExpression { a: -1, b: 3 })]
        );
    }

    #[test]
    fn not_selector() {
        let list = parse("div:not(.hidden)");
        match &list[0].compounds[0].pseudo_classes[0] {
            PseudoClass::Not(inner) => assert_eq!(inner[0].classes, vec!["hidden"]),
            other => panic!("应当是 :not()，实际是 {other:?}"),
        }
    }

    #[test]
    fn specificity_computation() {
        assert_eq!(specificity("*").to_string(), "(0,0,0)");
        assert_eq!(specificity("li").to_string(), "(0,0,1)");
        assert_eq!(specificity("ul li").to_string(), "(0,0,2)");
        assert_eq!(specificity(".note").to_string(), "(0,1,0)");
        assert_eq!(specificity("#main").to_string(), "(1,0,0)");
        assert_eq!(specificity("div.note#main").to_string(), "(1,1,1)");
        assert_eq!(specificity("[href]").to_string(), "(0,1,0)");
        assert_eq!(specificity("a:hover").to_string(), "(0,1,1)");
        assert_eq!(specificity("li:nth-child(2)").to_string(), "(0,1,1)");
        assert_eq!(specificity("p::before").to_string(), "(0,0,2)");
        assert_eq!(specificity("div:not(#x)").to_string(), "(1,0,1)");
    }

    #[test]
    fn specificity_ordering() {
        assert!(specificity("#a") > specificity(".a"));
        assert!(specificity(".a") > specificity("div"));
        assert!(specificity("div.a") > specificity(".a"));
    }

    #[test]
    fn nth_expression_matching() {
        let odd = NthExpression { a: 2, b: 1 };
        assert!(odd.matches(1));
        assert!(!odd.matches(2));
        assert!(odd.matches(3));

        let first_three = NthExpression { a: -1, b: 3 };
        assert!(first_three.matches(1));
        assert!(first_three.matches(3));
        assert!(!first_three.matches(4));

        let third = NthExpression { a: 0, b: 3 };
        assert!(third.matches(3));
        assert!(!third.matches(4));
    }

    #[test]
    fn invalid_selector_is_rejected() {
        let tokens = super::super::tokenizer::tokenize("..bad");
        let tokens = &tokens[..tokens.len() - 1];
        assert!(parse_selector_list(tokens).is_err());
    }

    // ---- 匹配测试 ----

    /// 在解析出来的文档里找一个元素，用于匹配测试。
    fn find(document: &Document, id: &str) -> NodeId {
        document
            .find_element(document.root(), |element| element.id() == Some(id))
            .unwrap_or_else(|| panic!("找不到 id 为 {id} 的元素"))
    }

    /// 判断选择器是否匹配指定元素。
    fn matches_id(document: &Document, selector: &str, id: &str) -> bool {
        let list = parse(selector);
        let element = find(document, id);
        list.iter()
            .any(|selector| matches(selector, document, element, &ElementState::default()))
    }

    /// 一份用于匹配测试的文档。
    fn sample() -> Document {
        parse_document(
            r#"<html><body>
                <div id="outer" class="box wide">
                    <p id="first" class="note">a</p>
                    <p id="second">b</p>
                    <span id="third" data-kind="x">c</span>
                </div>
                <ul id="list"><li id="l1">1</li><li id="l2">2</li><li id="l3">3</li></ul>
                <a id="link" href="/x">link</a>
                <input id="field" type="text">
            </body></html>"#,
        )
    }

    #[test]
    fn match_by_type_and_class() {
        let document = sample();
        assert!(matches_id(&document, "div", "outer"));
        assert!(matches_id(&document, ".box", "outer"));
        assert!(matches_id(&document, ".box.wide", "outer"));
        assert!(!matches_id(&document, ".narrow", "outer"));
        assert!(!matches_id(&document, "span", "outer"));
    }

    #[test]
    fn match_by_id() {
        let document = sample();
        assert!(matches_id(&document, "#outer", "outer"));
        assert!(!matches_id(&document, "#nope", "outer"));
    }

    #[test]
    fn match_descendant_and_child() {
        let document = sample();
        assert!(matches_id(&document, "div p", "first"));
        assert!(matches_id(&document, "body p", "first"));
        assert!(matches_id(&document, "div > p", "first"));
        assert!(!matches_id(&document, "ul > p", "first"));
        assert!(matches_id(&document, "body > div", "outer"));
    }

    #[test]
    fn match_sibling_combinators() {
        let document = sample();
        assert!(matches_id(&document, "p + p", "second"));
        assert!(!matches_id(&document, "span + p", "first"));
        assert!(matches_id(&document, "p ~ span", "third"));
        assert!(!matches_id(&document, "span ~ p", "first"));
    }

    #[test]
    fn match_structural_pseudo_classes() {
        let document = sample();
        assert!(matches_id(&document, "p:first-child", "first"));
        assert!(!matches_id(&document, "p:first-child", "second"));
        assert!(matches_id(&document, "span:last-child", "third"));
        assert!(matches_id(&document, "li:nth-child(2)", "l2"));
        assert!(matches_id(&document, "li:nth-child(odd)", "l1"));
        assert!(!matches_id(&document, "li:nth-child(odd)", "l2"));
        assert!(matches_id(&document, "li:nth-last-child(1)", "l3"));
        assert!(matches_id(&document, "li:first-of-type", "l1"));
        assert!(matches_id(&document, "li:last-of-type", "l3"));
    }

    #[test]
    fn dangling_combinators_and_empty_selectors_are_rejected() {
        // 规范 G.1 的产生式里，组合子与逗号两边都必须有选择器。写错了要让
        // 整条规则作废，不能把多余的记号悄悄丢掉——那会把写错的选择器当成
        // 写对的处理，样式静默地加到了不该加的元素上。
        assert!(!parses("div >"));
        assert!(!parses("div +"));
        assert!(!parses("div ~"));
        assert!(!parses("div,"));
        assert!(!parses("div, > p"));
        assert!(!parses(""));
        // 正常的照样通过。
        assert!(parses("div"));
        assert!(parses("div > p"));
        assert!(parses("div, p"));
        assert!(parses("div , p"));
    }

    #[test]
    fn single_colon_pseudo_elements_are_accepted() {
        // 规范 §5.12 把伪元素写成单冒号，双冒号是后来的写法。
        // 实现要收单冒号，双冒号也不能丢。
        for text in [
            "p:before",
            "p:after",
            "p:first-line",
            "p:first-letter",
            "p::before",
            "p::after",
            "p::first-line",
            "p::first-letter",
        ] {
            let list = parse(text);
            assert_eq!(list.len(), 1, "{text} 应当解析出一个选择器");
        }
        // 单冒号那四种被当成伪元素，而不是不认识的伪类。
        let list = parse("p:before");
        assert_eq!(list[0].compounds[0].pseudo_element.as_deref(), Some("before"));
        assert!(list[0].compounds[0].pseudo_classes.is_empty());

        // 其余名字的单冒号写法仍是伪类，不认识就让整条规则作废。
        assert!(!parses("p:nosuchthing"));
    }

    #[test]
    fn match_lang_pseudo_class() {
        let document = parse_document(
            r#"<html lang="en-US"><body>
                 <p id="direct" lang="fr">texte</p>
                 <p id="inherited">text</p>
                 <div lang="en"><p id="sub">inside</p></div>
                 <p id="unknown" lang="">no language</p>
               </body></html>"#,
        );
        // 元素自己写了语言就用它。
        assert!(matches_id(&document, ":lang(fr)", "direct"));
        assert!(!matches_id(&document, ":lang(en)", "direct"));
        // 没写就沿祖先往上找。
        assert!(matches_id(&document, ":lang(en)", "inherited"));
        assert!(matches_id(&document, ":lang(en-US)", "inherited"));
        // 近的那个祖先先算。
        assert!(matches_id(&document, ":lang(en)", "sub"));
        // 子标签按前缀算：`en` 命中 `en-US`。
        assert!(matches_id(&document, ":lang(en-us)", "inherited"), "标签不分大小写");
        // 带引号的写法也认。
        assert!(matches_id(&document, r#":lang("en")"#, "sub"));
        // 前缀要停在连字符上，`en` 不该命中 `enx`。
        let other = parse_document(r#"<html lang="enx"><p id="x">a</p></html>"#);
        assert!(!matches_id(&other, ":lang(en)", "x"));
        // 空串表示语言未知，这时不往上继承。
        assert!(!matches_id(&document, ":lang(en)", "unknown"));
    }

    #[test]
    fn match_root_pseudo_class() {
        let document = sample();
        let html = document
            .find_element(document.root(), |element| element.name == "html")
            .expect("应当有 html");
        let list = parse(":root");
        assert!(matches(&list[0], &document, html, &ElementState::default()));
    }

    #[test]
    fn match_attribute_selectors() {
        let document = sample();
        assert!(matches_id(&document, "[data-kind]", "third"));
        assert!(matches_id(&document, "[data-kind=x]", "third"));
        assert!(matches_id(&document, "[data-kind~=x]", "third"));
        assert!(matches_id(&document, "[data-kind^=x]", "third"));
        assert!(matches_id(&document, "[data-kind$=x]", "third"));
        assert!(matches_id(&document, "[data-kind*=x]", "third"));
        assert!(!matches_id(&document, "[data-kind=y]", "third"));
    }

    #[test]
    fn match_not_and_is() {
        let document = sample();
        assert!(matches_id(&document, "p:not(.note)", "second"));
        assert!(!matches_id(&document, "p:not(.note)", "first"));
        assert!(matches_id(&document, "p:is(.note, .other)", "first"));
    }

    #[test]
    fn match_link_and_enabled() {
        let document = sample();
        assert!(matches_id(&document, "a:link", "link"));
        assert!(!matches_id(&document, "p:link", "first"));
        assert!(matches_id(&document, "input:enabled", "field"));
        assert!(!matches_id(&document, "input:disabled", "field"));
    }

    #[test]
    fn dynamic_pseudo_classes_follow_state() {
        let document = sample();
        let list = parse("p:hover");
        let element = find(&document, "first");

        let idle = ElementState::default();
        assert!(!matches(&list[0], &document, element, &idle));

        let hovered = ElementState {
            hovered: true,
            ..ElementState::default()
        };
        assert!(matches(&list[0], &document, element, &hovered));
    }

    #[test]
    fn empty_pseudo_class() {
        let document = parse_document("<div id=a></div><div id=b>x</div><div id=c><!--x--></div>");
        assert!(matches_id(&document, "div:empty", "a"));
        assert!(!matches_id(&document, "div:empty", "b"));
        assert!(matches_id(&document, "div:empty", "c"));
    }

    #[test]
    fn only_child_and_only_of_type() {
        let document = parse_document("<div id=p><span id=only></span></div>");
        assert!(matches_id(&document, "span:only-child", "only"));
        assert!(matches_id(&document, "span:only-of-type", "only"));
    }
}
