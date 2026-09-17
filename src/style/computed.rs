//! 计算样式：每个元素最终用到的属性值。
//!
//! 这里只放布局与绘制真正要用到的属性。值都已经解析成具体类型，布局阶段
//! 不用再回头解析字符串。

use crate::css::value::{Color, Length, LengthUnit};

/// `display` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// 不生成盒子。
    None,
    /// 块级盒。
    #[default]
    Block,
    /// 行内盒。
    Inline,
    /// 行内块，对外像行内、对内像块。
    InlineBlock,
    /// 弹性容器。
    Flex,
    /// 行内弹性容器。
    InlineFlex,
    /// 列表项，会额外生成标记盒。
    ListItem,
    /// 表格容器，内部按表格规则排版。
    Table,
    /// 表格行组（`thead`、`tbody`、`tfoot`）。
    TableRowGroup,
    /// 表格行。
    TableRow,
    /// 表格单元格。
    TableCell,
    /// 表格标题。
    TableCaption,
}

impl Display {
    /// 是否参加行内布局。
    pub fn is_inline_level(self) -> bool {
        matches!(self, Self::Inline | Self::InlineBlock | Self::InlineFlex)
    }

    /// 是否生成块级盒。
    ///
    /// 表格内部的几种盒不在此列：它们的位置由所属表格统一分配，不参与
    /// 常规流的纵向堆叠，所以不能混进块级这一档。
    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            Self::Block | Self::Flex | Self::ListItem | Self::Table
        )
    }

    /// 是不是表格内部的东西。
    ///
    /// 这些盒只在表格容器的排版里出现，脱离表格就没有意义。
    pub fn is_table_internal(self) -> bool {
        matches!(
            self,
            Self::TableRowGroup | Self::TableRow | Self::TableCell | Self::TableCaption
        )
    }
}

/// `position` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    /// 常规流。
    #[default]
    Static,
    /// 相对定位，占位不变。
    Relative,
    /// 绝对定位，脱离常规流。
    Absolute,
    /// 固定定位，相对视口。
    Fixed,
    /// 粘性定位，按相对定位处理后再做吸附。
    Sticky,
}

impl Position {
    /// 是否脱离常规流。
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }
}

/// `overflow` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// 溢出内容照常显示。
    #[default]
    Visible,
    /// 裁剪溢出的内容。
    Hidden,
    /// 需要时出现滚动条。
    Scroll,
    /// 按需出现滚动条。
    Auto,
}

impl Overflow {
    /// 是否要把内容裁剪到盒子范围内。
    pub fn clips(self) -> bool {
        !matches!(self, Self::Visible)
    }
}

/// 尺寸属性的取值。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Size {
    /// 由内容或外部约束决定。
    #[default]
    Auto,
    /// 明确的长度。
    Length(Length),
}

impl Size {
    /// 是自动值。
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// 取长度，自动值返回 `None`。
    pub fn length(&self) -> Option<Length> {
        match self {
            Self::Auto => None,
            Self::Length(length) => Some(*length),
        }
    }
}

/// `flex-direction` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexDirection {
    /// 主轴水平，起点在左。
    #[default]
    Row,
    /// 主轴水平，起点在右。
    RowReverse,
    /// 主轴竖直，起点在上。
    Column,
    /// 主轴竖直，起点在下。
    ColumnReverse,
}

impl FlexDirection {
    /// 主轴是否是水平方向。
    pub fn is_row(self) -> bool {
        matches!(self, Self::Row | Self::RowReverse)
    }

    /// 是否需要反转主轴顺序。
    pub fn is_reversed(self) -> bool {
        matches!(self, Self::RowReverse | Self::ColumnReverse)
    }
}

/// `flex-wrap` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlexWrap {
    /// 不换行。
    #[default]
    NoWrap,
    /// 换行，侧轴正向排列。
    Wrap,
    /// 换行，侧轴反向排列。
    WrapReverse,
}

/// 主轴对齐方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JustifyContent {
    /// 靠起点。
    #[default]
    FlexStart,
    /// 靠终点。
    FlexEnd,
    /// 居中。
    Center,
    /// 两端对齐，中间等距。
    SpaceBetween,
    /// 每项两侧等距。
    SpaceAround,
    /// 各项之间与两端都等距。
    SpaceEvenly,
}

/// 侧轴对齐方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    /// 拉伸填满。
    #[default]
    Stretch,
    /// 靠起点。
    FlexStart,
    /// 靠终点。
    FlexEnd,
    /// 居中。
    Center,
    /// 按基线对齐。
    Baseline,
}

/// 多行之间的对齐方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignContent {
    /// 拉伸。
    #[default]
    Stretch,
    /// 靠起点。
    FlexStart,
    /// 靠终点。
    FlexEnd,
    /// 居中。
    Center,
    /// 两端对齐。
    SpaceBetween,
    /// 每行两侧等距。
    SpaceAround,
}

/// `text-align` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// 靠左。
    #[default]
    Left,
    /// 靠右。
    Right,
    /// 居中。
    Center,
    /// 两端对齐。
    Justify,
}

/// 四个角的圆角半径。
///
/// 字段顺序按 CSS 的书写顺序来（左上、右上、右下、左下），不套用
/// `Sides` 的上下左右——两者顺序不同，混用迟早出错。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radii {
    /// 左上角。
    pub top_left: Length,
    /// 右上角。
    pub top_right: Length,
    /// 右下角。
    pub bottom_right: Length,
    /// 左下角。
    pub bottom_left: Length,
}

impl Default for Radii {
    /// 默认四个角都是零，也就是直角。
    fn default() -> Self {
        Self::ZERO
    }
}

impl Radii {
    /// 四个角都是零。
    pub const ZERO: Self = Self {
        top_left: Length::ZERO,
        top_right: Length::ZERO,
        bottom_right: Length::ZERO,
        bottom_left: Length::ZERO,
    };

    /// 四个角是不是都是零。
    pub fn is_zero(&self) -> bool {
        self.top_left.is_zero()
            && self.top_right.is_zero()
            && self.bottom_right.is_zero()
            && self.bottom_left.is_zero()
    }
}

/// `white-space` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    /// 合并空白，允许换行。
    #[default]
    Normal,
    /// 保留换行，合并其余空白。
    PreLine,
    /// 原样保留，不自动换行。
    Pre,
    /// 原样保留，允许换行。
    PreWrap,
    /// 合并空白，不自动换行。
    NoWrap,
}

impl WhiteSpace {
    /// 是否保留源码里的换行。
    pub fn preserves_newlines(self) -> bool {
        matches!(self, Self::PreLine | Self::Pre | Self::PreWrap)
    }

    /// 是否保留连续空白。
    pub fn preserves_spaces(self) -> bool {
        matches!(self, Self::Pre | Self::PreWrap)
    }

    /// 是否允许自动换行。
    pub fn wraps(self) -> bool {
        !matches!(self, Self::NoWrap | Self::Pre)
    }
}

/// `border-style` 的取值，只保留会影响宽度的几种。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    /// 不画边框，宽度按零算。
    #[default]
    None,
    /// 实线。
    Solid,
    /// 虚线。
    Dashed,
    /// 点线。
    Dotted,
    /// 双线。
    Double,
}

impl BorderStyle {
    /// 是否真的会画出边框。
    pub fn is_visible(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// 四个方向的长度。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sides {
    /// 上。
    pub top: Length,
    /// 右。
    pub right: Length,
    /// 下。
    pub bottom: Length,
    /// 左。
    pub left: Length,
}

impl Sides {
    /// 四边取同一个值。
    pub const fn uniform(length: Length) -> Self {
        Self {
            top: length,
            right: length,
            bottom: length,
            left: length,
        }
    }

    /// 全部是零。
    pub const fn zero() -> Self {
        Self::uniform(Length::ZERO)
    }

    /// 四边是否都是零。
    pub fn is_zero(&self) -> bool {
        self.top.is_zero() && self.right.is_zero() && self.bottom.is_zero() && self.left.is_zero()
    }

    /// 横向总长度，左右相加。
    pub fn horizontal(&self) -> Length {
        add_lengths(self.left, self.right)
    }

    /// 纵向总长度，上下相加。
    pub fn vertical(&self) -> Length {
        add_lengths(self.top, self.bottom)
    }
}

/// 四边各自的一个值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderSides {
    /// 上。
    pub top: BorderStyle,
    /// 右。
    pub right: BorderStyle,
    /// 下。
    pub bottom: BorderStyle,
    /// 左。
    pub left: BorderStyle,
}

/// 把两个长度相加，单位不同时按像素近似处理。
///
/// 这里只在同单位或零的情况下精确，其余情况保留左侧单位，具体的换算留给
/// 布局阶段按上下文完成。
pub fn add_lengths(a: Length, b: Length) -> Length {
    if a.is_zero() {
        return b;
    }
    if b.is_zero() {
        return a;
    }
    if a.unit == b.unit {
        return Length {
            value: a.value + b.value,
            unit: a.unit,
        };
    }
    Length {
        value: a.value + b.value,
        unit: a.unit,
    }
}

/// 一个元素最终的计算样式。
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    /// 显示方式。
    pub display: Display,
    /// 定位方式。
    pub position: Position,
    /// `top`，自动值表示不偏移。
    pub top: Size,
    /// `right`。
    pub right: Size,
    /// `bottom`。
    pub bottom: Size,
    /// `left`。
    pub left: Size,
    /// 宽度。
    pub width: Size,
    /// 高度。
    pub height: Size,
    /// 最小宽度。
    pub min_width: Size,
    /// 最小高度。
    pub min_height: Size,
    /// 最大宽度。
    pub max_width: Size,
    /// 最大高度。
    pub max_height: Size,
    /// 外边距。
    pub margin: Sides,
    /// 四个角的圆角半径。
    pub border_radius: Radii,
    /// 内边距。
    pub padding: Sides,
    /// 边框宽度。
    pub border_width: Sides,
    /// 边框样式。
    pub border_style: BorderSides,
    /// 边框颜色。
    pub border_color: Color,
    /// 文字颜色。
    pub color: Color,
    /// 背景色。
    pub background_color: Color,
    /// 字体族，按优先级排列。
    pub font_family: Vec<String>,
    /// 字号。
    pub font_size: Length,
    /// 字重。
    pub font_weight: u16,
    /// 是否是斜体。
    pub italic: bool,
    /// 行高倍数。
    pub line_height: f64,
    /// 水平对齐。
    pub text_align: TextAlign,
    /// 是否有下划线。
    pub underline: bool,
    /// 空白处理方式。
    pub white_space: WhiteSpace,
    /// 主轴方向。
    pub flex_direction: FlexDirection,
    /// 是否换行。
    pub flex_wrap: FlexWrap,
    /// 主轴对齐。
    pub justify_content: JustifyContent,
    /// 侧轴对齐。
    pub align_items: AlignItems,
    /// 多行对齐。
    pub align_content: AlignContent,
    /// 自身的侧轴对齐，`None` 表示跟随容器。
    pub align_self: Option<AlignItems>,
    /// 伸展系数。
    pub flex_grow: f64,
    /// 收缩系数。
    pub flex_shrink: f64,
    /// 基准尺寸。
    pub flex_basis: Size,
    /// 行间距。
    pub row_gap: Length,
    /// 列间距。
    pub column_gap: Length,
    /// 溢出处理。
    pub overflow: Overflow,
    /// 不透明度。
    pub opacity: f64,
    /// 是否可见。
    pub visibility: bool,
    /// 层叠顺序，自动值当作零。
    pub z_index: i32,
}

impl Default for ComputedStyle {
    /// 规范的初始值。
    fn default() -> Self {
        Self::initial()
    }
}

impl ComputedStyle {
    /// 建一份全是初始值的样式。
    pub fn initial() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            top: Size::Auto,
            right: Size::Auto,
            bottom: Size::Auto,
            left: Size::Auto,
            width: Size::Auto,
            height: Size::Auto,
            min_width: Size::Auto,
            min_height: Size::Auto,
            max_width: Size::Auto,
            max_height: Size::Auto,
            margin: Sides::zero(),
            border_radius: Radii::ZERO,
            padding: Sides::zero(),
            border_width: Sides::zero(),
            border_style: BorderSides::default(),
            border_color: Color::BLACK,
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            font_family: vec!["sans-serif".to_string()],
            font_size: Length::px(16.0),
            font_weight: 400,
            italic: false,
            line_height: 1.2,
            text_align: TextAlign::Left,
            underline: false,
            white_space: WhiteSpace::Normal,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_content: AlignContent::Stretch,
            align_self: None,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Size::Auto,
            row_gap: Length::ZERO,
            column_gap: Length::ZERO,
            overflow: Overflow::Visible,
            opacity: 1.0,
            visibility: true,
            z_index: 0,
        }
    }

    /// 按继承规则从父样式派生出一份新样式。
    ///
    /// 字体、颜色这类可继承属性沿用父元素，其余回到初始值。
    pub fn inherit_from(parent: &ComputedStyle) -> Self {
        let mut style = Self::initial();
        style.color = parent.color;
        style.font_family = parent.font_family.clone();
        style.font_size = parent.font_size;
        style.font_weight = parent.font_weight;
        style.italic = parent.italic;
        style.line_height = parent.line_height;
        style.text_align = parent.text_align;
        style.white_space = parent.white_space;
        style.visibility = parent.visibility;
        style
    }

    /// 从根元素样式派生，根元素的字号按初始值算。
    pub fn for_root() -> Self {
        Self::initial()
    }

    /// 该元素是否要参与布局。
    pub fn generates_box(&self) -> bool {
        self.display != Display::None
    }

    /// 有效字号，单位换算成像素。
    ///
    /// `rem` 与百分比这类相对单位需要上下文，这里只处理绝对单位。
    pub fn font_size_pixels(&self) -> f64 {
        match self.font_size.unit {
            LengthUnit::Px | LengthUnit::None => self.font_size.value,
            LengthUnit::Pt => self.font_size.value * 4.0 / 3.0,
            LengthUnit::Pc => self.font_size.value * 16.0,
            LengthUnit::In => self.font_size.value * 96.0,
            LengthUnit::Cm => self.font_size.value * 96.0 / 2.54,
            LengthUnit::Mm => self.font_size.value * 96.0 / 25.4,
            // 相对单位在计算阶段已经由层叠换算过，这里兜底按 16 像素。
            _ => self.font_size.value * 16.0,
        }
    }

    /// 边框是否有可见的样式。
    pub fn has_visible_border(&self) -> bool {
        self.border_style.top.is_visible()
            || self.border_style.right.is_visible()
            || self.border_style.bottom.is_visible()
            || self.border_style.left.is_visible()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_match_spec() {
        let style = ComputedStyle::initial();
        assert_eq!(style.display, Display::Inline);
        assert_eq!(style.position, Position::Static);
        assert_eq!(style.color, Color::BLACK);
        assert_eq!(style.background_color, Color::TRANSPARENT);
        assert_eq!(style.font_size, Length::px(16.0));
        assert_eq!(style.font_weight, 400);
        assert_eq!(style.line_height, 1.2);
        assert_eq!(style.opacity, 1.0);
        assert!(style.visibility);
        assert_eq!(style.flex_shrink, 1.0);
        assert_eq!(style.flex_grow, 0.0);
    }

    #[test]
    fn inheritance_carries_text_properties() {
        let mut parent = ComputedStyle::initial();
        parent.color = Color::rgba(255, 0, 0, 255);
        parent.font_size = Length::px(20.0);
        parent.font_weight = 700;
        parent.text_align = TextAlign::Center;
        parent.white_space = WhiteSpace::Pre;

        let child = ComputedStyle::inherit_from(&parent);
        assert_eq!(child.color, Color::rgba(255, 0, 0, 255));
        assert_eq!(child.font_size, Length::px(20.0));
        assert_eq!(child.font_weight, 700);
        assert_eq!(child.text_align, TextAlign::Center);
        assert_eq!(child.white_space, WhiteSpace::Pre);
        // 不可继承的属性保持初始值。
        assert_eq!(child.display, Display::Inline);
        assert_eq!(child.margin, Sides::zero());
    }

    #[test]
    fn display_predicates() {
        assert!(Display::Block.is_block_level());
        assert!(!Display::Block.is_inline_level());
        assert!(Display::Inline.is_inline_level());
        assert!(Display::InlineBlock.is_inline_level());
        assert!(Display::Flex.is_block_level());
        assert!(Display::ListItem.is_block_level());
    }

    #[test]
    fn position_out_of_flow() {
        assert!(!Position::Static.is_out_of_flow());
        assert!(!Position::Relative.is_out_of_flow());
        assert!(Position::Absolute.is_out_of_flow());
        assert!(Position::Fixed.is_out_of_flow());
    }

    #[test]
    fn overflow_clipping() {
        assert!(!Overflow::Visible.clips());
        assert!(Overflow::Hidden.clips());
        assert!(Overflow::Auto.clips());
    }

    #[test]
    fn side_aggregation() {
        let sides = Sides {
            top: Length::px(1.0),
            right: Length::px(2.0),
            bottom: Length::px(3.0),
            left: Length::px(4.0),
        };
        assert_eq!(sides.horizontal(), Length::px(6.0));
        assert_eq!(sides.vertical(), Length::px(4.0));
        assert!(!sides.is_zero());
        assert!(Sides::zero().is_zero());
    }

    #[test]
    fn uniform_sides() {
        let sides = Sides::uniform(Length::px(5.0));
        assert_eq!(sides.top, Length::px(5.0));
        assert_eq!(sides.left, Length::px(5.0));
    }

    #[test]
    fn adding_lengths_with_zero() {
        assert_eq!(add_lengths(Length::ZERO, Length::px(3.0)), Length::px(3.0));
        assert_eq!(
            add_lengths(Length::percent(10.0), Length::ZERO),
            Length::percent(10.0)
        );
        assert_eq!(
            add_lengths(Length::px(1.0), Length::px(2.0)),
            Length::px(3.0)
        );
    }

    #[test]
    fn font_size_pixel_conversion() {
        let mut style = ComputedStyle::initial();
        style.font_size = Length::px(18.0);
        assert_eq!(style.font_size_pixels(), 18.0);

        style.font_size = Length {
            value: 12.0,
            unit: LengthUnit::Pt,
        };
        assert!((style.font_size_pixels() - 16.0).abs() < 1e-6);
    }

    #[test]
    fn border_visibility() {
        let mut style = ComputedStyle::initial();
        assert!(!style.has_visible_border());
        style.border_style.top = BorderStyle::Solid;
        assert!(style.has_visible_border());
    }

    #[test]
    fn white_space_behaviour() {
        assert!(WhiteSpace::Normal.wraps());
        assert!(!WhiteSpace::Normal.preserves_newlines());
        assert!(WhiteSpace::Pre.preserves_newlines());
        assert!(WhiteSpace::Pre.preserves_spaces());
        assert!(!WhiteSpace::Pre.wraps());
        assert!(!WhiteSpace::NoWrap.wraps());
        assert!(WhiteSpace::PreLine.preserves_newlines());
        assert!(WhiteSpace::PreLine.wraps());
    }

    #[test]
    fn flex_direction_helpers() {
        assert!(FlexDirection::Row.is_row());
        assert!(FlexDirection::ColumnReverse.is_reversed());
        assert!(!FlexDirection::Column.is_row());
    }

    #[test]
    fn generates_box_only_when_displayed() {
        let mut style = ComputedStyle::initial();
        assert!(style.generates_box());
        style.display = Display::None;
        assert!(!style.generates_box());
    }
}
