//! 遍历盒子树产出显示列表。
//!
//! 绘制顺序按层叠规则来：先背景色，再边框，然后内容。建立了行内格式化
//! 上下文的盒子按行片段顺序画，遇到原子片段就递归画对应的子盒子，这样
//! 文字与行内块的前后关系不会乱。其余盒子按子元素顺序递归。
//!
//! 不透明度是累乘的，父元素的半透明会作用到整棵子树。

use super::display_list::{Corners, DisplayList, DrawCommand, LineStyle};
use crate::css::value::{Length, LengthUnit};
use crate::dom::NodeId;
use crate::layout::box_tree::{BoxKind, FragmentContent, LayoutBox};
use crate::layout::engine::LayoutTree;
use crate::layout::geometry::{Edges, Rect};
use crate::style::BorderStyle;

/// 列表标记到内容左边缘的距离，单位是像素。
const LIST_MARKER_GAP: f64 = 8.0;

/// 绘制器。
struct Painter {
    /// 正在产出的显示列表。
    list: DisplayList,
    /// 可见范围，完全落在它外面的盒子会被跳过。
    visible: Rect,
    /// 矩形是否与可见范围相交。
    culling: bool,
    /// 贡献了画布底色的元素。它的背景已经铺在画布上了，自己那层不再重复画。
    canvas_owner: Option<NodeId>,
}

impl Painter {
    /// 建一个绘制器。
    fn new(viewport_width: f64, viewport_height: f64, visible: Rect, culling: bool) -> Self {
        Self {
            list: DisplayList::new(viewport_width, viewport_height),
            visible,
            culling,
            canvas_owner: None,
        }
    }

    /// 画一个盒子及其子树。
    fn paint_box(&mut self, layout_box: &LayoutBox, opacity: f64, clip: Rect) {
        let rect = layout_box.rect;
        // 完全在可见范围外的盒子直接跳过。子元素不会超出屏幕，除非有溢出，
        // 所以这里只在盒子本身不可见时裁剪。
        if self.culling && !rect.intersects(&self.visible) && !layout_box.style.overflow.clips() {
            return;
        }
        // 完全透明的元素连同子树一起跳过。
        let opacity = opacity * layout_box.style.opacity;
        if opacity <= 0.0 {
            return;
        }

        let corners = corners_of(layout_box);
        // 有圆角又有实线边框时走一条捷径，见 `paint_rounded_border`。
        // 它一并把背景也画了，所以成功时不再走下面那条常规路径。
        if !self.paint_rounded_border(layout_box, opacity, corners, clip) {
            self.paint_background(layout_box, opacity, corners, clip);
            self.paint_border(layout_box, opacity, corners, clip);
        }

        // 溢出裁剪时把内容限制在内容盒里。
        let clips = layout_box.style.overflow.clips();
        let content_clip = if clips {
            let rect = layout_box.content;
            self.list.push(DrawCommand::PushClip { rect });
            rect
        } else {
            clip
        };

        if layout_box.kind == BoxKind::ListItem {
            self.paint_list_marker(layout_box, opacity);
        }

        if layout_box.lines.is_empty() {
            for child in &layout_box.children {
                self.paint_box(child, opacity, content_clip);
            }
        } else {
            // 行内上下文的绘制顺序按行片段走，原子片段在这里递归。
            for line in &layout_box.lines {
                for fragment in &line.fragments {
                    match &fragment.kind {
                        FragmentContent::Text(text) => {
                            if text.text.trim().is_empty() {
                                continue;
                            }
                            if self.culling && !fragment.rect.intersects(&self.visible) {
                                continue;
                            }
                            self.list.push(DrawCommand::Text {
                                rect: fragment.rect,
                                baseline: fragment.baseline,
                                text: text.text.clone(),
                                color: text.color.with_opacity(opacity),
                                font: text.font.clone(),
                                underline: text.underline,
                            });
                        }
                        FragmentContent::Atom { child_index } => {
                            if let Some(child) = layout_box.children.get(*child_index) {
                                self.paint_box(child, opacity, content_clip);
                            }
                        }
                    }
                }
            }
        }

        if clips {
            self.list.push(DrawCommand::PopClip);
        }
    }

    /// 画背景色。
    ///
    /// 背景默认铺满边框盒，与浏览器的 `background-clip: border-box` 一致。
    fn paint_background(
        &mut self,
        layout_box: &LayoutBox,
        opacity: f64,
        corners: Corners,
        clip: Rect,
    ) {
        let color = layout_box.style.background_color;
        if color.is_transparent() {
            return;
        }
        // 这一层已经作为画布底色铺满了可见范围，再画一遍自己那块会白费
        // 功夫，半透明色还会叠出两层深浅。
        if layout_box.node.is_some() && layout_box.node == self.canvas_owner {
            return;
        }
        let rect = layout_box.rect;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let visible = rect.intersection(&clip);
        if visible.width <= 0.0 || visible.height <= 0.0 {
            return;
        }
        self.list.push(DrawCommand::FillRect {
            rect: visible,
            color: color.with_opacity(opacity),
            corners,
        });
    }

    /// 画圆角边框，成功时把背景也一起画了。
    ///
    /// 圆角边框有个等价画法：先用边框色填满整个边框盒（圆角取外半径），
    /// 再用背景色填满内边距盒（半径按边框宽度收窄），两步叠出来正是带
    /// 圆角的一圈边。这比给边框分段那条路径逐段加圆角简单得多，代价是
    /// 只对「四边等宽、实线、背景不透明」成立——不满足时返回假，
    /// 调用方走原来的方角路径。
    fn paint_rounded_border(
        &mut self,
        layout_box: &LayoutBox,
        opacity: f64,
        corners: Corners,
        clip: Rect,
    ) -> bool {
        let widths = layout_box.border;
        let all_zero = corners.top_left == 0.0
            && corners.top_right == 0.0
            && corners.bottom_right == 0.0
            && corners.bottom_left == 0.0;
        if all_zero || widths.is_zero() {
            return false;
        }
        // 四边不等宽就没法用内外两圈表达。
        if widths.top != widths.left || widths.top != widths.right || widths.top != widths.bottom {
            return false;
        }
        // 虚线、点线这些不能靠实心填充表达。
        if dominant_border_style(&layout_box.style.border_style) != LineStyle::Solid {
            return false;
        }
        let background = layout_box.style.background_color;
        if background.is_transparent() {
            // 背景透明时内圈没法「抹掉」边框色，这条路不成立。
            return false;
        }

        let rect = layout_box.rect;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return false;
        }
        let outer = rect.intersection(&clip);
        if outer.width <= 0.0 || outer.height <= 0.0 {
            return false;
        }

        // 外圈：边框色铺满边框盒。
        self.list.push(DrawCommand::FillRect {
            rect: outer,
            color: layout_box.style.border_color.with_opacity(opacity),
            corners,
        });

        // 内圈：背景色铺满内边距盒，半径按边框宽度收窄。
        let inset = widths.left;
        let inner = Rect::new(
            rect.x + widths.left,
            rect.y + widths.top,
            (rect.width - widths.horizontal()).max(0.0),
            (rect.height - widths.vertical()).max(0.0),
        );
        let visible = inner.intersection(&clip);
        if visible.width <= 0.0 || visible.height <= 0.0 {
            return true;
        }
        let shrink = |radius: f64| (radius - inset).max(0.0);
        self.list.push(DrawCommand::FillRect {
            rect: visible,
            color: background.with_opacity(opacity),
            corners: Corners {
                top_left: shrink(corners.top_left),
                top_right: shrink(corners.top_right),
                bottom_right: shrink(corners.bottom_right),
                bottom_left: shrink(corners.bottom_left),
            },
        });
        true
    }

    /// 画边框。
    fn paint_border(&mut self, layout_box: &LayoutBox, opacity: f64, corners: Corners, clip: Rect) {
        let widths = layout_box.border;
        if widths.is_zero() {
            return;
        }
        let rect = layout_box.rect;
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        if !rect.intersects(&clip) {
            return;
        }
        let color = layout_box.style.border_color;
        if color.is_transparent() {
            return;
        }
        // 四边样式不同时取最靠上的那一个作为整体线型，简化处理。
        let style = dominant_border_style(&layout_box.style.border_style);
        self.list.push(DrawCommand::Border {
            rect,
            widths,
            color: color.with_opacity(opacity),
            style,
            corners,
        });
    }

    /// 画列表项的标记。
    ///
    /// 有序列表的编号需要计数器，这里统一画实心圆点。
    fn paint_list_marker(&mut self, layout_box: &LayoutBox, opacity: f64) {
        let Some(first_line) = layout_box.lines.first() else {
            return;
        };
        let font_size = layout_box.style.font_size_pixels();
        let marker_width = font_size * 0.5;
        let marker_rect = Rect::new(
            layout_box.content.x - LIST_MARKER_GAP - marker_width,
            first_line.top,
            marker_width,
            first_line.height,
        );
        if marker_rect.x < layout_box.rect.x - font_size {
            // 标记跑到盒子外面太多就放弃，避免盖住别的内容。
            return;
        }
        self.list.push(DrawCommand::Text {
            rect: marker_rect,
            baseline: first_line.top + first_line.height * 0.8,
            text: "•".to_string(),
            color: layout_box.style.color.with_opacity(opacity),
            font: crate::layout::text::TextStyle {
                families: layout_box.style.font_family.clone(),
                font_size: font_size as f32,
                weight: layout_box.style.font_weight,
                italic: layout_box.style.italic,
                line_height: layout_box.style.line_height,
                wrap: false,
            },
            underline: false,
        });
    }
}

/// 取盒子的圆角。
///
/// 计算样式里目前没有 `border-radius`，留出接口供后续接入。
fn corners_of(layout_box: &LayoutBox) -> Corners {
    let radii = layout_box.style.border_radius;
    if radii.is_zero() {
        return Corners::default();
    }
    let rect = layout_box.rect;
    // 百分比按盒子的短边算：正方形上的 50% 正好是个圆，这也是最常用的写法。
    let reference = rect.width.min(rect.height).max(0.0);
    // 半径超过短边的一半就没有意义了，圆角会互相吃掉。
    let limit = reference / 2.0;
    let font_size = layout_box.style.font_size_pixels();

    let resolve = |length: Length| -> f64 {
        let value = match length.unit {
            LengthUnit::None | LengthUnit::Px => length.value,
            LengthUnit::Percent => reference * length.value / 100.0,
            LengthUnit::Em => length.value * font_size,
            // rem 与视口单位在绘制阶段拿不到参照，按像素兜底。
            _ => length.value,
        };
        value.clamp(0.0, limit)
    };

    Corners {
        top_left: resolve(radii.top_left),
        top_right: resolve(radii.top_right),
        bottom_right: resolve(radii.bottom_right),
        bottom_left: resolve(radii.bottom_left),
    }
}

/// 四边样式不一致时挑一个代表，按可见性优先。
fn dominant_border_style(sides: &crate::style::BorderSides) -> LineStyle {
    for style in [sides.top, sides.right, sides.bottom, sides.left] {
        match style {
            BorderStyle::Solid => return LineStyle::Solid,
            BorderStyle::Dashed => return LineStyle::Dashed,
            BorderStyle::Dotted => return LineStyle::Dotted,
            BorderStyle::Double => return LineStyle::Double,
            BorderStyle::None => {}
        }
    }
    LineStyle::Solid
}

/// 画一棵布局树，产出显示列表。
pub fn paint_tree(tree: &LayoutTree) -> DisplayList {
    let visible = Rect::new(0.0, 0.0, tree.viewport_width, tree.viewport_height);
    paint_tree_region(tree, visible)
}

/// 只画指定范围内的内容。
///
/// 滚动时用这个避免重新绘制整页。
pub fn paint_tree_region(tree: &LayoutTree, visible: Rect) -> DisplayList {
    let mut painter = Painter::new(tree.viewport_width, tree.viewport_height, visible, true);
    // 先铺画布底色。根元素或 body 有背景时它已经传播上来，没有才是白色。
    // 铺的是可见范围而不是整个视口：滚动之后视口上沿在页面坐标里是负的，
    // 铺整屏会在下方留出一条没盖住的地方。
    painter.list.push(DrawCommand::FillRect {
        rect: visible,
        color: tree.canvas_background,
        corners: Corners::default(),
    });
    painter.canvas_owner = tree.canvas_owner;
    painter.paint_box(&tree.root, 1.0, visible);
    painter.list
}

/// 按给定的四边宽度构造边框盒的矩形，供渲染器分边绘制时使用。
pub fn border_rects(rect: Rect, widths: Edges) -> [Rect; 4] {
    [
        // 上边
        Rect::new(rect.x, rect.y, rect.width, widths.top),
        // 右边
        Rect::new(
            rect.right() - widths.right,
            rect.y + widths.top,
            widths.right,
            (rect.height - widths.top - widths.bottom).max(0.0),
        ),
        // 下边
        Rect::new(
            rect.x + widths.left,
            rect.bottom() - widths.bottom,
            (rect.width - widths.left - widths.right).max(0.0),
            widths.bottom,
        ),
        // 左边
        Rect::new(
            rect.x,
            rect.y + widths.top,
            widths.left,
            (rect.height - widths.top - widths.bottom).max(0.0),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;
    use crate::css::value::Color;
    use crate::html::parse_document;
    use crate::layout::engine::{LayoutContext, LayoutEngine};
    use crate::style::compute_styles;

    /// 解析、布局并绘制。
    fn render(html: &str, css: &str, width: f64, height: f64) -> DisplayList {
        let document = parse_document(html);
        let sheets = if css.trim().is_empty() {
            Vec::new()
        } else {
            vec![parse_stylesheet(css)]
        };
        let styles = compute_styles(&document, &sheets);
        let mut engine = LayoutEngine::new(LayoutContext {
            viewport_width: width,
            viewport_height: height,
            root_font_size: 16.0,
        });
        let tree = engine.layout(&document, &styles).expect("应当能布局");
        paint_tree(&tree)
    }

    /// 数出某一类命令的条数。
    fn count(list: &DisplayList, predicate: impl Fn(&DrawCommand) -> bool) -> usize {
        list.commands
            .iter()
            .filter(|command| predicate(command))
            .count()
    }

    #[test]
    fn empty_page_still_clears_canvas() {
        let list = render("", "", 800.0, 600.0);
        assert_eq!(list.len(), 1);
        assert!(matches!(list.commands[0], DrawCommand::FillRect { .. }));
    }

    #[test]
    fn text_produces_text_commands() {
        let list = render("<p>hello</p>", "", 800.0, 600.0);
        assert!(list.text_contents().contains(&"hello"));
    }

    #[test]
    fn background_colour_becomes_fill_rect() {
        let list = render(
            "<div id=d>x</div>",
            "#d { background: red; height: 10px }",
            800.0,
            600.0,
        );
        let reds = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. }
                    if *color == Color::rgba(255, 0, 0, 255)
            )
        });
        assert!(reds >= 1, "应当画出红色背景");
    }

    #[test]
    fn transparent_background_is_skipped() {
        let list = render("<div id=d>x</div>", "#d { height: 10px }", 800.0, 600.0);
        let transparent = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. } if color.is_transparent()
            )
        });
        assert_eq!(transparent, 0);
    }

    #[test]
    fn border_becomes_border_command() {
        let list = render(
            "<div id=d>x</div>",
            "#d { border: 2px solid black; height: 10px }",
            800.0,
            600.0,
        );
        let borders = count(&list, |command| {
            matches!(command, DrawCommand::Border { .. })
        });
        assert_eq!(borders, 1);
    }

    #[test]
    fn no_border_style_means_no_border_command() {
        let list = render(
            "<div id=d>x</div>",
            "#d { border-width: 2px; height: 10px }",
            800.0,
            600.0,
        );
        let borders = count(&list, |command| {
            matches!(command, DrawCommand::Border { .. })
        });
        assert_eq!(borders, 0, "没有样式就不该画边框");
    }

    #[test]
    fn nested_boxes_paint_in_order() {
        let list = render(
            "<div id=outer><div id=inner>x</div></div>",
            "#outer { background: blue; height: 50px } #inner { background: red; height: 20px }",
            800.0,
            600.0,
        );
        let blue = list
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    DrawCommand::FillRect { color, .. }
                        if *color == Color::rgba(0, 0, 255, 255)
                )
            })
            .expect("应当有蓝色背景");
        let red = list
            .commands
            .iter()
            .position(|command| {
                matches!(
                    command,
                    DrawCommand::FillRect { color, .. }
                        if *color == Color::rgba(255, 0, 0, 255)
                )
            })
            .expect("应当有红色背景");
        assert!(red > blue, "子元素应当画在父元素之后");
    }

    #[test]
    fn opacity_scales_alpha() {
        let list = render(
            "<div id=d>x</div>",
            "#d { background: rgba(0, 0, 0, 1); height: 10px; opacity: 0.5 }",
            800.0,
            600.0,
        );
        let faded = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. } if color.alpha == 128
            )
        });
        assert!(faded >= 1, "半透明元素的颜色应当被削半");
    }

    #[test]
    fn fully_transparent_element_is_skipped() {
        let list = render(
            "<div id=d>x</div>",
            "#d { background: red; height: 10px; opacity: 0 }",
            800.0,
            600.0,
        );
        let reds = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. }
                    if *color == Color::rgba(255, 0, 0, 255)
            )
        });
        assert_eq!(reds, 0);
    }

    #[test]
    fn overflow_hidden_emits_clip() {
        let list = render(
            "<div id=d><p>text</p></div>",
            "#d { overflow: hidden; height: 20px }",
            800.0,
            600.0,
        );
        assert!(
            count(&list, |command| matches!(
                command,
                DrawCommand::PushClip { .. }
            )) >= 1
        );
        assert!(count(&list, |command| matches!(command, DrawCommand::PopClip)) >= 1);
    }

    #[test]
    fn text_inside_clip_is_between_clip_commands() {
        let list = render(
            "<div id=d><p>inside</p></div>",
            "#d { overflow: hidden; height: 40px }",
            800.0,
            600.0,
        );
        let push = list
            .commands
            .iter()
            .position(|command| matches!(command, DrawCommand::PushClip { .. }))
            .expect("应当有裁剪");
        let pop = list
            .commands
            .iter()
            .position(|command| matches!(command, DrawCommand::PopClip))
            .expect("应当有裁剪结束");
        let text = list
            .commands
            .iter()
            .position(|command| matches!(command, DrawCommand::Text { .. }))
            .expect("应当有文字");
        assert!(push < text && text < pop);
    }

    #[test]
    fn list_item_gets_a_marker() {
        let list = render("<ul><li>item</li></ul>", "", 800.0, 600.0);
        assert!(list.text_contents().contains(&"•"), "列表项应当有标记");
    }

    #[test]
    fn styled_text_carries_colour_and_font() {
        let list = render(
            "<p>styled</p>",
            "p { color: red; font-size: 24px }",
            800.0,
            600.0,
        );
        let command = list
            .commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::Text {
                    text, color, font, ..
                } if text == "styled" => Some((color, font)),
                _ => None,
            })
            .expect("应当有文字命令");
        assert_eq!(*command.0, Color::rgba(255, 0, 0, 255));
        assert!((f64::from(command.1.font_size) - 24.0).abs() < 0.01);
    }

    #[test]
    fn underline_flag_is_carried() {
        let list = render("<a href='#'>link</a>", "", 800.0, 600.0);
        let underlined = count(&list, |command| {
            matches!(
                command,
                DrawCommand::Text {
                    underline: true,
                    ..
                }
            )
        });
        assert!(underlined >= 1, "链接应当带下划线");
    }

    #[test]
    fn offscreen_boxes_are_culled() {
        let html = "<div style='height: 5000px'>tall</div><p id=p>bottom</p>";
        let full = render(html, "", 800.0, 600.0);
        // 视口只有 600 高，底部的内容不该出现在命令里。
        assert!(
            !full.text_contents().contains(&"bottom"),
            "视口外的文字不该被画出来"
        );
    }

    #[test]
    fn region_painting_limits_commands() {
        let document = parse_document("<div style='height: 3000px'>tall</div>");
        let styles = compute_styles(&document, &[]);
        let mut engine = LayoutEngine::new(LayoutContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            root_font_size: 16.0,
        });
        let tree = engine.layout(&document, &styles).expect("应当能布局");

        let top = paint_tree_region(&tree, Rect::new(0.0, 0.0, 800.0, 600.0));
        let middle = paint_tree_region(&tree, Rect::new(0.0, 2000.0, 800.0, 600.0));
        assert!(!top.is_empty());
        // 中间区域里其实没有内容，但清屏命令总是有的。
        assert!(middle.len() <= top.len());
    }

    #[test]
    fn inline_block_is_painted_as_child() {
        let list = render(
            "<p>text <span id=s>box</span></p>",
            "#s { display: inline-block; background: red; width: 40px; height: 20px }",
            800.0,
            600.0,
        );
        let reds = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. }
                    if *color == Color::rgba(255, 0, 0, 255)
            )
        });
        assert_eq!(reds, 1, "行内块的背景应当被画出来");
    }

    #[test]
    fn border_rects_split_four_ways() {
        let rect = Rect::new(10.0, 10.0, 100.0, 50.0);
        let widths = Edges {
            top: 2.0,
            right: 3.0,
            bottom: 4.0,
            left: 5.0,
        };
        let parts = border_rects(rect, widths);
        assert_eq!(parts[0].height, 2.0);
        assert_eq!(parts[1].width, 3.0);
        assert_eq!(parts[2].height, 4.0);
        assert_eq!(parts[3].width, 5.0);
        assert_eq!(parts[0].y, rect.y);
        assert_eq!(parts[2].bottom(), rect.bottom());
    }

    #[test]
    fn paint_is_deterministic() {
        let first = render("<p>same</p>", "", 800.0, 600.0);
        let second = render("<p>same</p>", "", 800.0, 600.0);
        assert_eq!(first.commands.len(), second.commands.len());
        assert_eq!(first.all_text(), second.all_text());
    }

    #[test]
    fn nested_flex_children_are_painted() {
        let list = render(
            "<div id=c><span id=a>a</span><span id=b>b</span></div>",
            "#c { display: flex } #a, #b { background: red; width: 50px; height: 20px }",
            800.0,
            600.0,
        );
        let reds = count(&list, |command| {
            matches!(
                command,
                DrawCommand::FillRect { color, .. }
                    if *color == Color::rgba(255, 0, 0, 255)
            )
        });
        assert_eq!(reds, 2, "两个弹性项都应当被画出来");
    }
}
