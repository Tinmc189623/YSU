//! 布局：把文档树与计算样式变成带尺寸和位置的盒子树。

pub mod box_tree;
pub mod engine;
pub mod geometry;
pub mod text;

pub use box_tree::{
    BoxKind, FragmentContent, LayoutBox, LineBox, LineFragment, TextFragment, build_layout_tree,
    process_text,
};
pub use engine::{LayoutContext, LayoutEngine, LayoutTree};
pub use geometry::{Edges, Point, Rect, Size};
pub use text::{TextLayout, TextLine, TextMeasurer, TextStyle};
