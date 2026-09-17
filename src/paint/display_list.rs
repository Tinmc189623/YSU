//! 显示列表：绘制阶段产出的平台无关命令序列。
//!
//! 渲染器只认这些命令，不知道盒子树长什么样。换渲染后端时只要重写渲染器，
//! 布局与绘制阶段都不受影响。

use crate::css::value::Color;
use crate::layout::geometry::{Edges, Rect};
use crate::layout::text::TextStyle;

/// 圆角半径，四个角分开记。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners {
    /// 左上角。
    pub top_left: f64,
    /// 右上角。
    pub top_right: f64,
    /// 右下角。
    pub bottom_right: f64,
    /// 左下角。
    pub bottom_left: f64,
}

impl Corners {
    /// 四角取同一个半径。
    pub const fn uniform(radius: f64) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    /// 是否有任何一个角是圆的。
    pub fn is_rounded(&self) -> bool {
        self.top_left > 0.0
            || self.top_right > 0.0
            || self.bottom_right > 0.0
            || self.bottom_left > 0.0
    }
}

/// 边框的线型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    /// 实线。
    Solid,
    /// 虚线。
    Dashed,
    /// 点线。
    Dotted,
    /// 双线。
    Double,
}

/// 一条绘制命令。
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCommand {
    /// 填充一个矩形。
    FillRect {
        /// 矩形范围。
        rect: Rect,
        /// 填充色。
        color: Color,
        /// 圆角。
        corners: Corners,
    },
    /// 画一个边框，四边宽度可以不同。
    Border {
        /// 边框盒的范围。
        rect: Rect,
        /// 四边宽度。
        widths: Edges,
        /// 颜色。
        color: Color,
        /// 线型。
        style: LineStyle,
        /// 圆角。
        corners: Corners,
    },
    /// 画一段文字。
    Text {
        /// 文字占据的范围，用于裁剪与对齐参考。
        rect: Rect,
        /// 基线纵坐标。
        baseline: f64,
        /// 文本内容。
        text: String,
        /// 颜色。
        color: Color,
        /// 字体与大小。
        font: TextStyle,
        /// 是否画下划线。
        underline: bool,
    },
    /// 画一张图片。
    Image {
        /// 图片占据的范围。
        rect: Rect,
        /// 图片地址，相对路径已经解析过。
        source: String,
    },
    /// 把后续绘制裁剪到指定范围。
    PushClip {
        /// 裁剪矩形。
        rect: Rect,
    },
    /// 结束最近一次裁剪。
    PopClip,
}

impl DrawCommand {
    /// 该命令涉及的矩形，用于裁剪判断。
    pub fn bounds(&self) -> Rect {
        match self {
            Self::FillRect { rect, .. }
            | Self::Border { rect, .. }
            | Self::Text { rect, .. }
            | Self::Image { rect, .. }
            | Self::PushClip { rect } => *rect,
            // 裁剪结束没有自己的范围，用一个空矩形表示。
            Self::PopClip => Rect::default(),
        }
    }
}

/// 一份完整的显示列表。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    /// 按绘制顺序排列的命令。
    pub commands: Vec<DrawCommand>,
    /// 目标画布尺寸。
    pub viewport_width: f64,
    /// 目标画布高度。
    pub viewport_height: f64,
}

impl DisplayList {
    /// 建一份空列表。
    pub fn new(viewport_width: f64, viewport_height: f64) -> Self {
        Self {
            commands: Vec::new(),
            viewport_width,
            viewport_height,
        }
    }

    /// 追加一条命令。
    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    /// 命令条数。
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// 列表是否为空。
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// 统计各类型命令的条数，测试与调试用。
    pub fn summary(&self) -> (usize, usize, usize, usize) {
        let mut rects = 0;
        let mut borders = 0;
        let mut texts = 0;
        let mut images = 0;
        for command in &self.commands {
            match command {
                DrawCommand::FillRect { .. } => rects += 1,
                DrawCommand::Border { .. } => borders += 1,
                DrawCommand::Text { .. } => texts += 1,
                DrawCommand::Image { .. } => images += 1,
                DrawCommand::PushClip { .. } | DrawCommand::PopClip => {}
            }
        }
        (rects, borders, texts, images)
    }

    /// 取出全部文本命令的文本内容，测试用。
    pub fn text_contents(&self) -> Vec<&str> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// 拼接全部文本内容。
    pub fn all_text(&self) -> String {
        self.text_contents().join("")
    }

    /// 把整份列表按增量平移。
    ///
    /// 外壳把页面从页面坐标搬到屏幕坐标时用这个。
    pub fn translate(&mut self, dx: f64, dy: f64) {
        for command in &mut self.commands {
            match command {
                DrawCommand::FillRect { rect, .. }
                | DrawCommand::Border { rect, .. }
                | DrawCommand::Text { rect, .. }
                | DrawCommand::Image { rect, .. }
                | DrawCommand::PushClip { rect } => {
                    *rect = rect.translate(dx, dy);
                }
                // 裁剪结束没有范围，跟着上下文走即可。
                DrawCommand::PopClip => {}
            }
        }
    }

    /// 给整份列表套一层裁剪。
    ///
    /// 外壳用它把页面限制在工具栏以下的区域里，滚上去的内容才不会盖住
    /// 浏览器界面。
    pub fn clip_to(&mut self, rect: Rect) {
        if self.commands.is_empty() {
            return;
        }
        self.commands.insert(0, DrawCommand::PushClip { rect });
        self.commands.push(DrawCommand::PopClip);
    }

    /// 把另一份列表追加到末尾。
    pub fn append(&mut self, other: DisplayList) {
        self.commands.extend(other.commands);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::Point;

    #[test]
    fn corners_uniform_and_rounded_check() {
        assert!(!Corners::default().is_rounded());
        assert!(Corners::uniform(4.0).is_rounded());
    }

    #[test]
    fn command_bounds() {
        let command = DrawCommand::FillRect {
            rect: Rect::new(1.0, 2.0, 3.0, 4.0),
            color: Color::BLACK,
            corners: Corners::default(),
        };
        assert_eq!(command.bounds(), Rect::new(1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn display_list_counts_commands() {
        let mut list = DisplayList::new(800.0, 600.0);
        list.push(DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::WHITE,
            corners: Corners::default(),
        });
        list.push(DrawCommand::Text {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            baseline: 8.0,
            text: "hi".into(),
            color: Color::BLACK,
            font: TextStyle::default(),
            underline: false,
        });
        assert_eq!(list.len(), 2);
        assert_eq!(list.summary(), (1, 0, 1, 0));
        assert_eq!(list.all_text(), "hi");
    }

    #[test]
    fn empty_list_is_empty() {
        let list = DisplayList::new(100.0, 100.0);
        assert!(list.is_empty());
        assert_eq!(list.summary(), (0, 0, 0, 0));
    }

    #[test]
    fn translate_moves_every_command() {
        let mut list = DisplayList::new(100.0, 100.0);
        list.push(DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::WHITE,
            corners: Corners::default(),
        });
        list.push(DrawCommand::PushClip {
            rect: Rect::new(0.0, 0.0, 5.0, 5.0),
        });
        list.translate(3.0, 7.0);
        assert_eq!(list.commands[0].bounds(), Rect::new(3.0, 7.0, 10.0, 10.0));
        assert_eq!(list.commands[1].bounds(), Rect::new(3.0, 7.0, 5.0, 5.0));
    }

    #[test]
    fn clip_to_wraps_the_whole_list() {
        let mut list = DisplayList::new(100.0, 100.0);
        list.push(DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::WHITE,
            corners: Corners::default(),
        });
        list.clip_to(Rect::new(0.0, 50.0, 100.0, 50.0));
        assert_eq!(list.len(), 3);
        assert!(matches!(list.commands[0], DrawCommand::PushClip { .. }));
        assert!(matches!(list.commands[2], DrawCommand::PopClip));
    }

    #[test]
    fn clip_to_empty_list_is_a_noop() {
        let mut list = DisplayList::new(100.0, 100.0);
        list.clip_to(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(list.is_empty());
    }

    #[test]
    fn append_keeps_order() {
        let mut first = DisplayList::new(100.0, 100.0);
        first.push(DrawCommand::FillRect {
            rect: Rect::default(),
            color: Color::WHITE,
            corners: Corners::default(),
        });
        let mut second = DisplayList::new(100.0, 100.0);
        second.push(DrawCommand::Text {
            rect: Rect::default(),
            baseline: 0.0,
            text: "x".into(),
            color: Color::BLACK,
            font: TextStyle::default(),
            underline: false,
        });
        first.append(second);
        assert_eq!(first.len(), 2);
        assert!(matches!(first.commands[1], DrawCommand::Text { .. }));
    }

    #[test]
    fn clip_commands_have_no_meaningful_bounds() {
        assert_eq!(DrawCommand::PopClip.bounds(), Rect::default());
        assert!(
            DrawCommand::PushClip {
                rect: Rect::new(0.0, 0.0, 5.0, 5.0)
            }
            .bounds()
            .contains(Point::new(1.0, 1.0))
        );
    }
}
