//! 样式计算：从匹配到的声明算出每个元素最终用到的属性值。

pub mod cascade;
pub mod computed;

pub use cascade::{
    Origin, StyleMap, StyleResolver, USER_AGENT_CSS, compute_styles, compute_styles_with_viewport,
    user_agent_sheet,
};
pub use computed::{
    AlignContent, AlignItems, BorderSides, BorderStyle, ComputedStyle, Display, FlexDirection,
    FlexWrap, JustifyContent, Overflow, Position, Radii, Sides, Size, TextAlign, WhiteSpace,
};
