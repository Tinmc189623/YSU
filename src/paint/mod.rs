//! 绘制：把布局树翻译成平台无关的显示列表。

pub mod display_list;
pub mod painter;

pub use display_list::{Corners, DisplayList, DrawCommand, LineStyle};
pub use painter::{border_rects, paint_tree, paint_tree_region};
