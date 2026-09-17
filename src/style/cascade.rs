//! 样式层叠：把匹配到的声明按优先级顺序作用到每个元素上。
//!
//! 优先级的比较顺序是「是否 important → 来源 → 选择器优先级 → 出现顺序」。
//! 来源分三层：浏览器默认样式、页面作者样式、元素的行内样式。作者写的
//! `!important` 能盖过行内样式，这与浏览器的行为一致。

use super::computed::{
    AlignContent, AlignItems, BorderSides, BorderStyle, ComputedStyle, Display, FlexDirection,
    FlexWrap, JustifyContent, Overflow, Position, Sides, Size, TextAlign, WhiteSpace,
};
use crate::css::value::{Length, LengthUnit};
use crate::css::{
    ComplexSelector, ElementState, MediaContext, PropertyValue, Specificity, StyleSheet, matches,
    parse_stylesheet,
};
use crate::dom::node::{Document, NodeId};
use std::collections::HashMap;
use std::sync::OnceLock;

/// 样式声明的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// 浏览器默认样式。
    UserAgent,
    /// 页面作者样式。
    Author,
    /// 元素上的 `style` 属性。
    Inline,
}

/// 一个候选声明，参与优先级比较。
struct Candidate<'a> {
    /// 是否带 `!important`。
    important: bool,
    /// 来源。
    origin: Origin,
    /// 选择器优先级，行内样式按零算，靠来源区分。
    specificity: Specificity,
    /// 出现顺序。
    order: usize,
    /// 属性名，已转小写。
    property: &'a str,
    /// 属性值。
    value: &'a PropertyValue,
}

/// 用来给元素算样式的解析器。
pub struct StyleResolver<'a> {
    /// 文档树。
    document: &'a Document,
    /// 作者样式表，按引入顺序排列。
    sheets: &'a [StyleSheet],
    /// 媒体查询的求值环境。
    media: MediaContext,
    /// 元素状态，影响动态伪类。
    state: ElementState,
}

impl<'a> StyleResolver<'a> {
    /// 构造解析器。
    pub fn new(document: &'a Document, sheets: &'a [StyleSheet]) -> Self {
        Self {
            document,
            sheets,
            media: MediaContext::default(),
            state: ElementState::default(),
        }
    }

    /// 设置媒体查询环境。
    pub fn with_media(mut self, media: MediaContext) -> Self {
        self.media = media;
        self
    }

    /// 设置元素状态。
    pub fn with_state(mut self, state: ElementState) -> Self {
        self.state = state;
        self
    }

    /// 给整棵树算样式，返回每个元素节点对应的计算样式。
    ///
    /// 按文档顺序遍历，子元素直接继承父元素已经算好的结果。
    pub fn compute_tree(&self) -> StyleMap {
        let mut map = StyleMap::new();
        let root = self.document.root();
        for node in self.document.descendants(root) {
            if self.document.node(node).as_element().is_none() {
                continue;
            }
            let parent_style = self
                .document
                .parent(node)
                .and_then(|parent| map.get(&parent))
                .cloned();
            let style = self.compute_element(node, parent_style.as_ref());
            map.insert(node, style);
        }
        map
    }

    /// 给单个元素算样式。
    ///
    /// `parent` 为 `None` 表示这是根元素，直接以初始值为起点。
    pub fn compute_element(
        &self,
        element: NodeId,
        parent: Option<&ComputedStyle>,
    ) -> ComputedStyle {
        let mut style = match parent {
            Some(parent) => ComputedStyle::inherit_from(parent),
            None => ComputedStyle::for_root(),
        };

        let mut candidates: Vec<Candidate> = Vec::new();
        let mut order = 0usize;

        // 浏览器默认样式。
        for rule in user_agent_sheet().rules.iter() {
            if !rule
                .selectors
                .iter()
                .any(|selector| matches(selector, self.document, element, &self.state))
            {
                continue;
            }
            for declaration in &rule.declarations {
                candidates.push(Candidate {
                    important: declaration.important,
                    origin: Origin::UserAgent,
                    specificity: best_specificity(
                        &rule.selectors,
                        self.document,
                        element,
                        &self.state,
                    ),
                    order,
                    property: &declaration.property,
                    value: &declaration.value,
                });
                order += 1;
            }
        }

        // 作者样式表。
        for sheet in self.sheets {
            for rule in &sheet.rules {
                if !rule
                    .selectors
                    .iter()
                    .any(|selector| matches(selector, self.document, element, &self.state))
                {
                    continue;
                }
                let specificity =
                    best_specificity(&rule.selectors, self.document, element, &self.state);
                for declaration in &rule.declarations {
                    candidates.push(Candidate {
                        important: declaration.important,
                        origin: Origin::Author,
                        specificity,
                        order,
                        property: &declaration.property,
                        value: &declaration.value,
                    });
                    order += 1;
                }
            }
        }

        // 行内样式。样式表要先绑定到外层变量上，不然候选声明借用的引用
        // 会随着 if let 的花括号一起失效。
        let inline_sheet = self.inline_style(element);
        if let Some(inline) = &inline_sheet {
            for declaration in inline.rules.iter().flat_map(|rule| &rule.declarations) {
                candidates.push(Candidate {
                    important: declaration.important,
                    origin: Origin::Inline,
                    specificity: Specificity::default(),
                    order,
                    property: &declaration.property,
                    value: &declaration.value,
                });
                order += 1;
            }
        }

        // 排序后按顺序应用，越靠后的优先级越高。
        candidates.sort_by(|a, b| {
            a.important
                .cmp(&b.important)
                .then(a.origin.cmp(&b.origin))
                .then(a.specificity.cmp(&b.specificity))
                .then(a.order.cmp(&b.order))
        });

        for candidate in candidates {
            apply_declaration(&mut style, candidate.property, candidate.value);
        }

        // 字号里的相对单位按父元素换算，其余相对单位留给布局阶段。
        resolve_font_size(&mut style, parent);

        style
    }

    /// 解析元素的行内样式。
    fn inline_style(&self, element: NodeId) -> Option<StyleSheet> {
        let attribute = self.document.element(element)?.get_attribute("style")?;
        if attribute.trim().is_empty() {
            return None;
        }
        // 行内样式没有选择器，套一层通配选择器再解析。
        let css = format!("* {{ {attribute} }}");
        Some(parse_stylesheet(&css))
    }
}

/// 在匹配到的选择器里取优先级最高的那个。
///
/// 一条规则可能有多个选择器命中同一个元素，按规范取最高的那个。
fn best_specificity(
    selectors: &[ComplexSelector],
    document: &Document,
    element: NodeId,
    state: &ElementState,
) -> Specificity {
    selectors
        .iter()
        .filter(|selector| matches(selector, document, element, state))
        .map(ComplexSelector::specificity)
        .max()
        .unwrap_or_default()
}

/// 按属性名把一条声明应用到样式上。
///
/// 认不出的属性直接忽略，与浏览器一致。
fn apply_declaration(style: &mut ComputedStyle, property: &str, value: &PropertyValue) {
    match property {
        "display" => {
            if let Some(keyword) = value.as_keyword() {
                style.display = match keyword {
                    "none" => Display::None,
                    "block" => Display::Block,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "list-item" => Display::ListItem,
                    // 表格相关的显示类型暂时按块处理，表格布局后续再补。
                    "table" => Display::Table,
                    // 行内表格先按块级表格排。差别只在与文字混排时的表现，
                    // 拿它当块处理不会把页面排坏，只是位置偏一点。
                    "inline-table" => Display::Table,
                    "table-row-group" | "table-header-group" | "table-footer-group" => {
                        Display::TableRowGroup
                    }
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "table-caption" => Display::TableCaption,
                    _ => return,
                };
            }
        }
        "position" => {
            if let Some(keyword) = value.as_keyword() {
                style.position = match keyword {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => return,
                };
            }
        }
        "top" => set_offset(&mut style.top, value),
        "right" => set_offset(&mut style.right, value),
        "bottom" => set_offset(&mut style.bottom, value),
        "left" => set_offset(&mut style.left, value),
        "width" => set_size(&mut style.width, value),
        "height" => set_size(&mut style.height, value),
        "min-width" => set_size(&mut style.min_width, value),
        "min-height" => set_size(&mut style.min_height, value),
        "max-width" => set_size(&mut style.max_width, value),
        "max-height" => set_size(&mut style.max_height, value),
        "margin" => set_sides(&mut style.margin, value),
        "border-radius" => set_radii(&mut style.border_radius, value),
        "border-top-left-radius" => set_radius(&mut style.border_radius.top_left, value),
        "border-top-right-radius" => set_radius(&mut style.border_radius.top_right, value),
        "border-bottom-right-radius" => set_radius(&mut style.border_radius.bottom_right, value),
        "border-bottom-left-radius" => set_radius(&mut style.border_radius.bottom_left, value),
        "margin-top" => set_side(&mut style.margin.top, value),
        "margin-right" => set_side(&mut style.margin.right, value),
        "margin-bottom" => set_side(&mut style.margin.bottom, value),
        "margin-left" => set_side(&mut style.margin.left, value),
        "padding" => set_sides(&mut style.padding, value),
        "padding-top" => set_side(&mut style.padding.top, value),
        "padding-right" => set_side(&mut style.padding.right, value),
        "padding-bottom" => set_side(&mut style.padding.bottom, value),
        "padding-left" => set_side(&mut style.padding.left, value),
        "border-width" => set_sides(&mut style.border_width, value),
        "border" => {
            // 简写里可能同时含宽度、样式与颜色，三样都要认。
            let Some(values) = mixed_values(value) else {
                // 只写宽度时按四边宽度处理。
                set_sides(&mut style.border_width, value);
                return;
            };
            for item in values {
                if let Some(length) = item.as_length() {
                    style.border_width = Sides::uniform(length);
                } else if let Some(keyword) = item.as_keyword() {
                    if let Some(border) = parse_border_style(keyword) {
                        style.border_style = BorderSides {
                            top: border,
                            right: border,
                            bottom: border,
                            left: border,
                        };
                    }
                } else if let Some(color) = item.as_color() {
                    style.border_color = color;
                }
            }
        }
        "border-top-width" => set_side(&mut style.border_width.top, value),
        "border-right-width" => set_side(&mut style.border_width.right, value),
        "border-bottom-width" => set_side(&mut style.border_width.bottom, value),
        "border-left-width" => set_side(&mut style.border_width.left, value),
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            // 简写里同时含宽度、样式与颜色。
            let Some(values) = mixed_values(value) else {
                return;
            };
            let side = &property[7..];
            for item in values {
                if let Some(length) = item.as_length() {
                    set_border_width_side(&mut style.border_width, side, length);
                } else if let Some(keyword) = item.as_keyword() {
                    if let Some(border) = parse_border_style(keyword) {
                        set_border_style_side(&mut style.border_style, side, border);
                    }
                } else if let Some(color) = item.as_color() {
                    style.border_color = color;
                }
            }
        }
        "border-style" => {
            if let Some(keyword) = value.as_keyword()
                && let Some(border) = parse_border_style(keyword)
            {
                style.border_style = BorderSides {
                    top: border,
                    right: border,
                    bottom: border,
                    left: border,
                };
            }
        }
        "border-color" => {
            if let Some(color) = value.as_color() {
                style.border_color = color;
            }
        }
        "color" => {
            if let Some(color) = value.as_color() {
                style.color = color;
            }
        }
        "background" | "background-color" => {
            if let Some(color) = value.as_color() {
                style.background_color = color;
            } else if let Some(values) = mixed_values(value) {
                // `background: red url(...)` 这种简写，取其中的颜色。
                for item in values {
                    if let Some(color) = item.as_color() {
                        style.background_color = color;
                        break;
                    }
                }
            }
        }
        "font-family" => {
            if let Some(list) = value.as_keyword_list() {
                style.font_family = list
                    .into_iter()
                    .map(|name| name.trim_matches('"').trim_matches('\'').to_string())
                    .collect();
            }
        }
        "font-size" => {
            if let Some(length) = value.as_length() {
                style.font_size = length;
            } else if let Some(keyword) = value.as_keyword() {
                // 字号关键字按规范给出的比例换算。
                let scale = match keyword {
                    "xx-small" => 0.6,
                    "x-small" => 0.75,
                    "small" => 0.889,
                    "medium" => 1.0,
                    "large" => 1.2,
                    "x-large" => 1.5,
                    "xx-large" => 2.0,
                    "smaller" => 0.833,
                    "larger" => 1.2,
                    _ => return,
                };
                style.font_size = Length::px(16.0 * scale);
            }
        }
        "font-weight" => {
            if let Some(keyword) = value.as_keyword() {
                style.font_weight = match keyword {
                    "normal" => 400,
                    "bold" => 700,
                    "bolder" => 700,
                    "lighter" => 300,
                    _ => style.font_weight,
                };
            } else if let Some(number) = value.as_number() {
                style.font_weight = number.clamp(1.0, 1000.0) as u16;
            }
        }
        "font-style" => {
            if let Some(keyword) = value.as_keyword() {
                style.italic = matches!(keyword, "italic" | "oblique");
            }
        }
        "font" => {
            if let Some(values) = mixed_values(value) {
                for item in values {
                    if let Some(number) = item.as_number()
                        && number >= 100.0
                    {
                        style.font_weight = number as u16;
                    }
                    if item.as_keyword() == Some("italic") || item.as_keyword() == Some("oblique") {
                        style.italic = true;
                    }
                    if item.as_keyword() == Some("bold") {
                        style.font_weight = 700;
                    }
                }
            }
        }
        "line-height" => {
            if let Some(number) = value.as_number() {
                // 无单位数字按倍数处理，带单位则换算成倍数。
                style.line_height = match value {
                    PropertyValue::Number(_) => number,
                    PropertyValue::Length(length) if length.unit == LengthUnit::None => {
                        length.value
                    }
                    _ => 1.2,
                };
            } else if let Some(keyword) = value.as_keyword()
                && keyword == "normal"
            {
                style.line_height = 1.2;
            }
        }
        "text-align" => {
            if let Some(keyword) = value.as_keyword() {
                style.text_align = match keyword {
                    "left" | "start" => TextAlign::Left,
                    "right" | "end" => TextAlign::Right,
                    "center" => TextAlign::Center,
                    "justify" => TextAlign::Justify,
                    _ => return,
                };
            }
        }
        "text-decoration" | "text-decoration-line" => {
            if let Some(keyword) = value.as_keyword() {
                if keyword == "none" {
                    style.underline = false;
                } else if keyword.contains("underline") {
                    style.underline = true;
                }
            } else if let Some(list) = value.as_keyword_list() {
                style.underline = list.iter().any(|item| item.contains("underline"));
            }
        }
        "white-space" => {
            if let Some(keyword) = value.as_keyword() {
                style.white_space = match keyword {
                    "normal" => WhiteSpace::Normal,
                    "pre" => WhiteSpace::Pre,
                    "pre-line" => WhiteSpace::PreLine,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    "nowrap" => WhiteSpace::NoWrap,
                    _ => return,
                };
            }
        }
        "flex-direction" => {
            if let Some(keyword) = value.as_keyword() {
                style.flex_direction = match keyword {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => return,
                };
            }
        }
        "flex-wrap" => {
            if let Some(keyword) = value.as_keyword() {
                style.flex_wrap = match keyword {
                    "nowrap" => FlexWrap::NoWrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => return,
                };
            }
        }
        "justify-content" => {
            if let Some(keyword) = value.as_keyword() {
                style.justify_content = match keyword {
                    "flex-start" | "start" => JustifyContent::FlexStart,
                    "flex-end" | "end" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => return,
                };
            }
        }
        "align-items" => {
            if let Some(keyword) = value.as_keyword()
                && let Some(align) = parse_align_items(keyword)
            {
                style.align_items = align;
            }
        }
        "align-self" => {
            if let Some(keyword) = value.as_keyword() {
                style.align_self = parse_align_items(keyword);
            }
        }
        "align-content" => {
            if let Some(keyword) = value.as_keyword() {
                style.align_content = match keyword {
                    "stretch" => AlignContent::Stretch,
                    "flex-start" | "start" => AlignContent::FlexStart,
                    "flex-end" | "end" => AlignContent::FlexEnd,
                    "center" => AlignContent::Center,
                    "space-between" => AlignContent::SpaceBetween,
                    "space-around" => AlignContent::SpaceAround,
                    _ => return,
                };
            }
        }
        "flex-grow" => {
            if let Some(number) = value.as_number() {
                style.flex_grow = number.max(0.0);
            }
        }
        "flex-shrink" => {
            if let Some(number) = value.as_number() {
                style.flex_shrink = number.max(0.0);
            }
        }
        "flex-basis" => set_size(&mut style.flex_basis, value),
        "flex" => {
            // `flex: none` 与 `flex: 1 1 auto` 这类简写。
            if let Some(keyword) = value.as_keyword() {
                if keyword == "none" {
                    style.flex_grow = 0.0;
                    style.flex_shrink = 0.0;
                    style.flex_basis = Size::Auto;
                }
                return;
            }
            let Some(values) = mixed_values(value) else {
                // 只有一个数字时按 `flex: <grow>` 处理。
                if let Some(number) = value.as_number() {
                    style.flex_grow = number;
                    style.flex_shrink = 1.0;
                    style.flex_basis = Size::Auto;
                }
                return;
            };
            let mut index = 0;
            for item in values {
                if let Some(number) = item.as_number() {
                    if index == 0 {
                        style.flex_grow = number;
                    } else if index == 1 {
                        style.flex_shrink = number;
                    }
                    index += 1;
                } else if let Some(length) = item.as_length() {
                    style.flex_basis = Size::Length(length);
                } else if item.as_keyword() == Some("auto") {
                    style.flex_basis = Size::Auto;
                }
            }
        }
        "gap" | "row-gap" | "column-gap" => {
            if let Some(length) = value.as_length() {
                if property != "column-gap" {
                    style.row_gap = length;
                }
                if property != "row-gap" {
                    style.column_gap = length;
                }
            }
        }
        "overflow" | "overflow-x" | "overflow-y" => {
            if let Some(keyword) = value.as_keyword() {
                style.overflow = match keyword {
                    "visible" => Overflow::Visible,
                    "hidden" | "clip" => Overflow::Hidden,
                    "scroll" => Overflow::Scroll,
                    "auto" => Overflow::Auto,
                    _ => return,
                };
            }
        }
        "opacity" => {
            if let Some(number) = value.as_number() {
                style.opacity = number.clamp(0.0, 1.0);
            }
        }
        "visibility" => {
            if let Some(keyword) = value.as_keyword() {
                style.visibility = match keyword {
                    "visible" => true,
                    "hidden" | "collapse" => false,
                    _ => return,
                };
            }
        }
        "z-index" => {
            if let Some(keyword) = value.as_keyword()
                && keyword == "auto"
            {
                style.z_index = 0;
            } else if let Some(number) = value.as_number() {
                style.z_index = number as i32;
            }
        }
        _ => {}
    }
}

/// 取声明值里的分量列表，单个值包装成长度为 1 的列表。
fn mixed_values(value: &PropertyValue) -> Option<Vec<PropertyValue>> {
    match value {
        PropertyValue::MixedList(list) => Some(list.clone()),
        PropertyValue::LengthList(list) => Some(
            list.iter()
                .map(|length| PropertyValue::Length(*length))
                .collect(),
        ),
        PropertyValue::KeywordList(list) => Some(
            list.iter()
                .map(|name| PropertyValue::Keyword(name.clone()))
                .collect(),
        ),
        _ => None,
    }
}

/// 设置偏移属性。
fn set_offset(target: &mut Size, value: &PropertyValue) {
    if let Some(keyword) = value.as_keyword() {
        if keyword == "auto" {
            *target = Size::Auto;
        }
        return;
    }
    if let Some(length) = value.as_length() {
        *target = Size::Length(length);
    }
}

/// 设置尺寸属性。
fn set_size(target: &mut Size, value: &PropertyValue) {
    if let Some(keyword) = value.as_keyword() {
        // `none` 与几个内容尺寸关键字都按自动值处理，布局阶段再按内容算。
        if matches!(
            keyword,
            "auto" | "none" | "fit-content" | "max-content" | "min-content"
        ) {
            *target = Size::Auto;
        }
        return;
    }
    if let Some(length) = value.as_length() {
        *target = Size::Length(length);
    }
}

/// 把一条属性值解释成一边的长度。
///
/// `auto` 与长度同处一个位置，写法上可以和长度混着出现（`margin: 0 auto`），
/// 所以外边距这一族不能只认纯长度列表。
fn side_from(value: &PropertyValue) -> Option<Length> {
    if let Some(keyword) = value.as_keyword()
        && keyword == "auto"
    {
        return Some(Length::AUTO);
    }
    value.as_length()
}

/// 设置一个角的圆角半径。
///
/// 只认一个值。CSS 允许写成 `水平 垂直` 两个值弄出椭圆角，那需要每角存两个
/// 半径，绘制那边也要跟着改；现在按圆角处理，取第一个值。
fn set_radius(target: &mut Length, value: &PropertyValue) {
    if let PropertyValue::LengthList(list) = value {
        if let Some(first) = list.first() {
            *target = *first;
        }
        return;
    }
    if let Some(length) = value.as_length() {
        *target = length;
    }
}

/// 设置四个角的圆角半径。
///
/// 写法与 `margin` 那套一样，一到四个值按左上、右上、右下、左下展开。
fn set_radii(target: &mut crate::style::Radii, value: &PropertyValue) {
    let list: Vec<Length> = if let Some(list) = value.as_length_list() {
        list
    } else if let PropertyValue::MixedList(items) = value {
        let mapped: Vec<Length> = items.iter().filter_map(side_from).collect();
        if mapped.len() != items.len() {
            return;
        }
        mapped
    } else {
        return;
    };
    match list.len() {
        1 => {
            target.top_left = list[0];
            target.top_right = list[0];
            target.bottom_right = list[0];
            target.bottom_left = list[0];
        }
        2 => {
            // 第一个值管左上与右下，第二个管右上与左下。
            target.top_left = list[0];
            target.bottom_right = list[0];
            target.top_right = list[1];
            target.bottom_left = list[1];
        }
        3 => {
            target.top_left = list[0];
            target.top_right = list[1];
            target.bottom_left = list[1];
            target.bottom_right = list[2];
        }
        _ => {
            target.top_left = list[0];
            target.top_right = list[1];
            target.bottom_right = list[2];
            target.bottom_left = list[3];
        }
    }
}

/// 设置单个方向的长度。
fn set_side(target: &mut Length, value: &PropertyValue) {
    if let Some(keyword) = value.as_keyword()
        && keyword == "auto"
    {
        // 外边距的 auto 在布局阶段单独处理，这里按零存。
        *target = Length::ZERO;
        return;
    }
    if let Some(length) = value.as_length() {
        *target = length;
    }
}

/// 设置四边长度，支持一到四个值的简写。
fn set_sides(target: &mut Sides, value: &PropertyValue) {
    // 先按纯长度列表认；认不下来再看是不是混着 `auto` 的写法。
    // `margin: 0 auto` 会在解析阶段变成混合列表，只认长度列表的话整条会被丢掉。
    let list: Vec<Length> = if let Some(list) = value.as_length_list() {
        list
    } else if let PropertyValue::MixedList(items) = value {
        let mapped: Vec<Length> = items.iter().filter_map(side_from).collect();
        // 有一项既不是长度也不是 auto，整条都不认。
        if mapped.len() != items.len() {
            return;
        }
        mapped
    } else {
        return;
    };
    match list.len() {
        1 => *target = Sides::uniform(list[0]),
        2 => {
            target.top = list[0];
            target.bottom = list[0];
            target.right = list[1];
            target.left = list[1];
        }
        3 => {
            target.top = list[0];
            target.right = list[1];
            target.left = list[1];
            target.bottom = list[2];
        }
        _ => {
            target.top = list[0];
            target.right = list[1];
            target.bottom = list[2];
            target.left = list[3];
        }
    }
}

/// 设置某个方向的边框宽度。
fn set_border_width_side(sides: &mut Sides, side: &str, length: Length) {
    match side {
        "top" => sides.top = length,
        "right" => sides.right = length,
        "bottom" => sides.bottom = length,
        "left" => sides.left = length,
        _ => {}
    }
}

/// 设置某个方向的边框样式。
fn set_border_style_side(sides: &mut BorderSides, side: &str, style: BorderStyle) {
    match side {
        "top" => sides.top = style,
        "right" => sides.right = style,
        "bottom" => sides.bottom = style,
        "left" => sides.left = style,
        _ => {}
    }
}

/// 解析边框样式关键字。
fn parse_border_style(keyword: &str) -> Option<BorderStyle> {
    Some(match keyword {
        "none" | "hidden" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        _ => return None,
    })
}

/// 解析侧轴对齐关键字。
fn parse_align_items(keyword: &str) -> Option<AlignItems> {
    Some(match keyword {
        "stretch" => AlignItems::Stretch,
        "flex-start" | "start" => AlignItems::FlexStart,
        "flex-end" | "end" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" => AlignItems::Baseline,
        _ => return None,
    })
}

/// 把字号里的相对单位按父元素换算掉。
///
/// `em` 与百分比在 `font-size` 上参照的是父元素的字号，其余属性上的
/// 相对单位参照元素自身，留给布局阶段处理。
fn resolve_font_size(style: &mut ComputedStyle, parent: Option<&ComputedStyle>) {
    let base = parent.map_or(16.0, ComputedStyle::font_size_pixels);
    let value = match style.font_size.unit {
        LengthUnit::Px | LengthUnit::None => return,
        LengthUnit::Em => base * style.font_size.value,
        LengthUnit::Percent => base * style.font_size.value / 100.0,
        LengthUnit::Rem => 16.0 * style.font_size.value,
        LengthUnit::Pt => style.font_size.value * 4.0 / 3.0,
        LengthUnit::Pc => style.font_size.value * 16.0,
        LengthUnit::In => style.font_size.value * 96.0,
        LengthUnit::Cm => style.font_size.value * 96.0 / 2.54,
        LengthUnit::Mm => style.font_size.value * 96.0 / 25.4,
        LengthUnit::Q => style.font_size.value * 96.0 / 101.6,
        // 视口单位在布局阶段才知道具体尺寸，这里按初始字号兜底。
        _ => base,
    };
    style.font_size = Length::px(value.max(0.0));
}

/// 每个元素的计算样式。
pub type StyleMap = HashMap<NodeId, ComputedStyle>;

/// 浏览器默认样式表，只在第一次用到时解析一次。
pub fn user_agent_sheet() -> &'static StyleSheet {
    static SHEET: OnceLock<StyleSheet> = OnceLock::new();
    SHEET.get_or_init(|| parse_stylesheet(USER_AGENT_CSS))
}

/// 浏览器默认样式。
///
/// 这是 HTML 规范建议的呈现规则里与静态排版相关的部分。
pub const USER_AGENT_CSS: &str = r#"
html, body, div, p, h1, h2, h3, h4, h5, h6, ul, ol, li, dl, dt, dd,
blockquote, pre, form, fieldset, hr, section, article, header,
footer, nav, aside, main, figure, figcaption, address, center {
    display: block;
}
li { display: list-item; }
table { display: table; }
caption { display: table-caption; text-align: center; }
thead, tbody, tfoot { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; }
head, title, meta, link, style, script, base, noscript, template { display: none; }
body { margin: 8px; }
p { margin: 1em 0; }
h1 { font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { font-size: 1.17em; font-weight: bold; margin: 1em 0; }
h4 { font-size: 1em; font-weight: bold; margin: 1.33em 0; }
h5 { font-size: 0.83em; font-weight: bold; margin: 1.67em 0; }
h6 { font-size: 0.67em; font-weight: bold; margin: 2.33em 0; }
strong, b { font-weight: bold; }
em, i, cite, var, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
a { text-decoration: underline; color: #0000ee; }
a:visited { color: #551a8b; }
ul, ol { margin: 1em 0; padding-left: 40px; }
ol { list-style-type: decimal; }
blockquote { margin: 1em 40px; }
pre { font-family: monospace; white-space: pre; margin: 1em 0; }
code, kbd, samp, tt { font-family: monospace; }
hr { border: 1px inset; margin: 0.5em auto; }
table { border-collapse: separate; border-spacing: 2px; }
td, th { padding: 1px; }
th { font-weight: bold; text-align: center; }
img { display: inline-block; }
br { display: inline; }
input, textarea, select, button { display: inline-block; }
button { text-align: center; }
fieldset { margin: 0 2px; padding: 0.35em 0.75em 0.625em; border: 2px groove; }
legend { padding: 0 2px; }
small { font-size: 0.83em; }
big { font-size: 1.17em; }
sup { font-size: 0.83em; vertical-align: super; }
sub { font-size: 0.83em; vertical-align: sub; }
mark { background-color: yellow; color: black; }
abbr, acronym { text-decoration: underline dotted; }
"#;

/// 给整棵树算样式，使用默认的媒体环境。
pub fn compute_styles(document: &Document, sheets: &[StyleSheet]) -> StyleMap {
    StyleResolver::new(document, sheets).compute_tree()
}

/// 给整棵树算样式，指定视口尺寸用于媒体查询与相对单位。
pub fn compute_styles_with_viewport(
    document: &Document,
    sheets: &[StyleSheet],
    media: MediaContext,
) -> StyleMap {
    StyleResolver::new(document, sheets)
        .with_media(media)
        .compute_tree()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::value::Color;
    use crate::html::parse_document;

    /// 解析文档与样式表并算样式。
    fn resolve(html: &str, css: &str) -> (Document, StyleMap) {
        let document = parse_document(html);
        let sheets = if css.trim().is_empty() {
            Vec::new()
        } else {
            vec![parse_stylesheet(css)]
        };
        let styles = compute_styles(&document, &sheets);
        (document, styles)
    }

    /// 取某个 id 元素的计算样式。
    fn style_of<'a>(document: &Document, styles: &'a StyleMap, id: &str) -> &'a ComputedStyle {
        let node = document
            .find_element(document.root(), |element| element.id() == Some(id))
            .unwrap_or_else(|| panic!("找不到 id 为 {id} 的元素"));
        styles
            .get(&node)
            .unwrap_or_else(|| panic!("{id} 没有计算样式"))
    }

    #[test]
    fn border_shorthand_and_radius_reach_the_computed_style() {
        // 真实网页上按钮多写成 `.button { border: 1px solid; border-radius: 4px }`，
        // 这两条都要能落到计算样式里，少一条圆角就画不出来。
        let (document, styles) = resolve(
            "<a id=b class=button>x</a>",
            ".button { border: 1px solid; border-radius: 4px }",
        );
        let style = style_of(&document, &styles, "b");
        assert_eq!(style.border_width.top, Length::px(1.0));
        assert_eq!(style.border_style.top, BorderStyle::Solid);
        assert_eq!(style.border_radius.top_left, Length::px(4.0));
        assert_eq!(style.border_radius.bottom_right, Length::px(4.0));
    }

    #[test]
    fn radius_shorthand_expands_like_the_spec_says() {
        // 两个值的写法：第一个管左上与右下，第二个管右上与左下。
        let (document, styles) = resolve("<div id=d></div>", "#d { border-radius: 8px 2px }");
        let style = style_of(&document, &styles, "d");
        assert_eq!(style.border_radius.top_left, Length::px(8.0));
        assert_eq!(style.border_radius.bottom_right, Length::px(8.0));
        assert_eq!(style.border_radius.top_right, Length::px(2.0));
        assert_eq!(style.border_radius.bottom_left, Length::px(2.0));
    }

    #[test]
    fn user_agent_defaults_make_div_block() {
        let (document, styles) = resolve("<div id=d>x</div>", "");
        assert_eq!(style_of(&document, &styles, "d").display, Display::Block);
    }

    #[test]
    fn user_agent_hides_head_content() {
        let (document, styles) = resolve(
            "<html><head><title>t</title></head><body>b</body></html>",
            "",
        );
        let title = document
            .find_element(document.root(), |element| element.name == "title")
            .expect("应当有 title");
        assert_eq!(styles[&title].display, Display::None);
    }

    #[test]
    fn user_agent_heading_sizes() {
        let (document, styles) = resolve("<h1 id=a>x</h1><h3 id=b>y</h3>", "");
        assert!((style_of(&document, &styles, "a").font_size_pixels() - 32.0).abs() < 0.01);
        assert!(style_of(&document, &styles, "b").font_size_pixels() < 32.0);
        assert_eq!(style_of(&document, &styles, "a").font_weight, 700);
    }

    #[test]
    fn author_rule_overrides_user_agent() {
        let (document, styles) = resolve("<div id=d>x</div>", "div { display: inline }");
        assert_eq!(style_of(&document, &styles, "d").display, Display::Inline);
    }

    #[test]
    fn specificity_decides_between_rules() {
        let (document, styles) = resolve(
            "<div id=d class=c>x</div>",
            ".c { color: red } div.c { color: green } #d { color: blue }",
        );
        assert_eq!(
            style_of(&document, &styles, "d").color,
            Color::rgba(0, 0, 255, 255)
        );
    }

    #[test]
    fn later_rule_wins_on_tie() {
        let (document, styles) = resolve(
            "<div id=d>x</div>",
            "div { color: red } div { color: blue }",
        );
        assert_eq!(
            style_of(&document, &styles, "d").color,
            Color::rgba(0, 0, 255, 255)
        );
    }

    #[test]
    fn important_beats_specificity() {
        let (document, styles) = resolve(
            "<div id=d>x</div>",
            "div { color: red !important } #d { color: blue }",
        );
        assert_eq!(
            style_of(&document, &styles, "d").color,
            Color::rgba(255, 0, 0, 255)
        );
    }

    #[test]
    fn inline_style_beats_author_rules() {
        let (document, styles) = resolve(
            r#"<div id=d style="color: red">x</div>"#,
            "#d { color: blue }",
        );
        assert_eq!(
            style_of(&document, &styles, "d").color,
            Color::rgba(255, 0, 0, 255)
        );
    }

    #[test]
    fn important_author_beats_inline() {
        let (document, styles) = resolve(
            r#"<div id=d style="color: red">x</div>"#,
            "div { color: blue !important }",
        );
        assert_eq!(
            style_of(&document, &styles, "d").color,
            Color::rgba(0, 0, 255, 255)
        );
    }

    #[test]
    fn inheritance_of_text_properties() {
        let (document, styles) = resolve(
            "<div id=parent><p id=child>x</p></div>",
            "div { color: red; font-size: 20px }",
        );
        assert_eq!(
            style_of(&document, &styles, "child").color,
            Color::rgba(255, 0, 0, 255)
        );
        assert!((style_of(&document, &styles, "child").font_size_pixels() - 20.0).abs() < 0.01);
    }

    #[test]
    fn em_font_size_resolves_against_parent() {
        let (document, styles) = resolve(
            "<div id=parent><span id=child>x</span></div>",
            "div { font-size: 20px } span { font-size: 2em }",
        );
        assert!(
            (style_of(&document, &styles, "child").font_size_pixels() - 40.0).abs() < 0.01,
            "2em 应当等于父元素的两倍"
        );
    }

    #[test]
    fn percent_font_size_resolves_against_parent() {
        let (document, styles) = resolve(
            "<div id=parent><span id=child>x</span></div>",
            "div { font-size: 20px } span { font-size: 150% }",
        );
        assert!((style_of(&document, &styles, "child").font_size_pixels() - 30.0).abs() < 0.01);
    }

    #[test]
    fn margin_shorthand_expansion() {
        let (document, styles) = resolve(
            "<div id=a>x</div><div id=b>y</div><div id=c>z</div>",
            "div { margin: 1px } #b { margin: 1px 2px } #c { margin: 1px 2px 3px 4px }",
        );
        let a = style_of(&document, &styles, "a");
        assert_eq!(a.margin.top, Length::px(1.0));
        assert_eq!(a.margin.left, Length::px(1.0));

        let b = style_of(&document, &styles, "b");
        assert_eq!(b.margin.top, Length::px(1.0));
        assert_eq!(b.margin.right, Length::px(2.0));
        assert_eq!(b.margin.bottom, Length::px(1.0));

        let c = style_of(&document, &styles, "c");
        assert_eq!(c.margin.top, Length::px(1.0));
        assert_eq!(c.margin.right, Length::px(2.0));
        assert_eq!(c.margin.bottom, Length::px(3.0));
        assert_eq!(c.margin.left, Length::px(4.0));
    }

    #[test]
    fn border_shorthand_sets_width_and_style() {
        let (document, styles) = resolve("<div id=d>x</div>", "div { border: 2px solid red }");
        let style = style_of(&document, &styles, "d");
        assert_eq!(style.border_width.top, Length::px(2.0));
        assert_eq!(style.border_style.top, BorderStyle::Solid);
        assert_eq!(style.border_color, Color::rgba(255, 0, 0, 255));
        assert!(style.has_visible_border());
    }

    #[test]
    fn display_none_is_recognised() {
        let (document, styles) = resolve("<div id=d>x</div>", "#d { display: none }");
        assert!(!style_of(&document, &styles, "d").generates_box());
    }

    #[test]
    fn flex_properties_are_parsed() {
        let (document, styles) = resolve(
            "<div id=d><span id=c>a</span></div>",
            "#d { display: flex; flex-direction: column; justify-content: center; align-items: flex-end; gap: 8px }
             #c { flex-grow: 2; flex-shrink: 0; flex-basis: 10px }",
        );
        let container = style_of(&document, &styles, "d");
        assert_eq!(container.display, Display::Flex);
        assert_eq!(container.flex_direction, FlexDirection::Column);
        assert_eq!(container.justify_content, JustifyContent::Center);
        assert_eq!(container.align_items, AlignItems::FlexEnd);
        assert_eq!(container.row_gap, Length::px(8.0));

        let child = style_of(&document, &styles, "c");
        assert_eq!(child.flex_grow, 2.0);
        assert_eq!(child.flex_shrink, 0.0);
        assert_eq!(child.flex_basis, Size::Length(Length::px(10.0)));
    }

    #[test]
    fn flex_shorthand() {
        let (document, styles) = resolve("<div id=d>x</div>", "#d { flex: 1 }");
        let style = style_of(&document, &styles, "d");
        assert_eq!(style.flex_grow, 1.0);
        assert_eq!(style.flex_shrink, 1.0);
    }

    #[test]
    fn flex_none_shorthand() {
        let (document, styles) = resolve("<div id=d>x</div>", "#d { flex: none }");
        let style = style_of(&document, &styles, "d");
        assert_eq!(style.flex_grow, 0.0);
        assert_eq!(style.flex_shrink, 0.0);
    }

    #[test]
    fn font_family_list_is_kept() {
        let (document, styles) = resolve(
            "<p id=p>x</p>",
            r#"p { font-family: "Helvetica Neue", Arial, sans-serif }"#,
        );
        let families = &style_of(&document, &styles, "p").font_family;
        assert_eq!(families[0], "Helvetica Neue");
        assert_eq!(families[1], "Arial");
    }

    #[test]
    fn unknown_properties_are_ignored() {
        let (document, styles) = resolve(
            "<div id=d>x</div>",
            "div { -unknown-thing: 5; width: 10px }",
        );
        let style = style_of(&document, &styles, "d");
        assert_eq!(style.width, Size::Length(Length::px(10.0)));
    }

    #[test]
    fn descendant_selector_applies() {
        let (document, styles) = resolve(
            "<div><p id=inner>x</p></div><p id=outer>y</p>",
            "div p { color: red }",
        );
        assert_eq!(
            style_of(&document, &styles, "inner").color,
            Color::rgba(255, 0, 0, 255)
        );
        assert_eq!(style_of(&document, &styles, "outer").color, Color::BLACK);
    }

    #[test]
    fn class_and_id_selectors() {
        let (document, styles) = resolve(
            r#"<p id=a class="note big">x</p><p id=b class="note">y</p>"#,
            ".note { color: red } .note.big { color: blue }",
        );
        assert_eq!(
            style_of(&document, &styles, "a").color,
            Color::rgba(0, 0, 255, 255)
        );
        assert_eq!(
            style_of(&document, &styles, "b").color,
            Color::rgba(255, 0, 0, 255)
        );
    }

    #[test]
    fn visibility_is_inherited_but_display_is_not() {
        let (document, styles) = resolve(
            "<div id=parent><p id=child>x</p></div>",
            "#parent { visibility: hidden; display: flex }",
        );
        assert!(!style_of(&document, &styles, "child").visibility);
        // display 不继承，子元素回到初始值。
        assert_eq!(
            style_of(&document, &styles, "child").display,
            Display::Block
        );
    }

    #[test]
    fn media_query_affects_cascade() {
        let document = parse_document("<div id=d>x</div>");
        let css = "@media (min-width: 600px) { div { color: red } }";

        let wide = compute_styles_with_viewport(
            &document,
            &[parse_stylesheet(css)],
            MediaContext {
                width: 1024.0,
                height: 800.0,
            },
        );
        let node = document
            .find_element(document.root(), |element| element.id() == Some("d"))
            .expect("应当有 div");
        assert_eq!(wide[&node].color, Color::rgba(255, 0, 0, 255));
    }

    #[test]
    fn user_agent_anchor_colour() {
        let (document, styles) = resolve(r#"<a id=a href="/x">l</a>"#, "");
        assert_eq!(
            style_of(&document, &styles, "a").color,
            Color::rgba(0x00, 0x00, 0xee, 255)
        );
        assert!(style_of(&document, &styles, "a").underline);
    }

    #[test]
    fn pre_keeps_whitespace() {
        let (document, styles) = resolve("<pre id=p>x</pre>", "");
        assert_eq!(
            style_of(&document, &styles, "p").white_space,
            WhiteSpace::Pre
        );
    }

    #[test]
    fn monospace_font_for_code() {
        let (document, styles) = resolve("<code id=c>x</code>", "");
        let families = &style_of(&document, &styles, "c").font_family;
        assert!(families.iter().any(|name| name == "monospace"));
    }
}
