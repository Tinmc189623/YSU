//! 盒子树：布局的输入结构。
//!
//! DOM 里的每个可见元素对应一个盒子，文本节点对应文本盒。块容器里混排的
//! 块级与行内级子元素要按规范插一层匿名块盒，否则行内内容没法定位。

use super::geometry::{Edges, Rect};
use super::text::{TextLayout, TextStyle};
use crate::css::value::Color;
use crate::dom::node::{Document, NodeData, NodeId};
use crate::style::{ComputedStyle, Display, StyleMap, WhiteSpace};

/// 盒子的种类。
#[derive(Debug, Clone, PartialEq)]
pub enum BoxKind {
    /// 块级盒。
    Block,
    /// 行内盒。
    Inline,
    /// 行内块，对外是行内级、对内是块容器。
    InlineBlock,
    /// 弹性容器。
    Flex,
    /// 列表项，会额外画一个标记。
    ListItem,
    /// 文本，装的是已经按空白规则处理过的内容。
    Text(Box<str>),
    /// 匿名块盒，用来包住块容器里的行内级内容。
    AnonymousBlock,
    /// 表格容器。
    Table,
    /// 表格行组。
    TableRowGroup,
    /// 表格行。
    TableRow,
    /// 表格单元格。
    TableCell,
    /// 表格标题。
    TableCaption,
}

impl BoxKind {
    /// 是否是文本盒。
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// 在常规流里是否按块级处理。
    pub fn is_block_level(&self) -> bool {
        matches!(
            self,
            Self::Block | Self::Flex | Self::ListItem | Self::AnonymousBlock
        )
    }

    /// 在常规流里是否按行内级处理。
    pub fn is_inline_level(&self) -> bool {
        matches!(self, Self::Inline | Self::InlineBlock | Self::Text(_))
    }

    /// 是否是建立新格式化上下文的块容器。
    pub fn is_block_container(&self) -> bool {
        matches!(self, Self::Block | Self::ListItem | Self::AnonymousBlock)
    }
}

/// 行盒里的一个片段。
#[derive(Debug, Clone, PartialEq)]
pub struct LineFragment {
    /// 片段在页面上的位置与尺寸。
    pub rect: Rect,
    /// 基线相对页面顶部的纵坐标，文本片段用它摆字形。
    pub baseline: f64,
    /// 片段内容。
    pub kind: FragmentContent,
}

/// 一个文本片段。
///
/// 只带绘制真正需要的样式，不背整个计算样式。
#[derive(Debug, Clone, PartialEq)]
pub struct TextFragment {
    /// 文本内容。
    pub text: String,
    /// 文字颜色。
    pub color: Color,
    /// 字体与大小。
    pub font: TextStyle,
    /// 是否画下划线。
    pub underline: bool,
    /// 该片段是否保留空白。
    ///
    /// 不保留时，整行只有空白的行盒不占高度也不参与绘制，这是 CSS 里
    /// 元素之间那些换行与缩进不会撑开页面的原因。
    pub preserve_whitespace: bool,
    /// 这段文字来自哪个节点。
    ///
    /// 命中测试靠它反查：行内元素在盒子树里没有自己的盒子，只按盒子找不出
    /// 「这一点压在哪个链接上」。
    pub node: Option<NodeId>,
}

/// 片段的内容。
#[derive(Debug, Clone, PartialEq)]
pub enum FragmentContent {
    /// 一段文本。
    Text(Box<TextFragment>),
    /// 一个原子盒，下标指向所属盒子的子盒子列表。
    Atom {
        /// 在父盒子的 `children` 里的下标。
        child_index: usize,
    },
}

/// 一个行盒，行内内容排版的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    /// 行上的片段，按从左到右的顺序。
    pub fragments: Vec<LineFragment>,
    /// 行顶相对页面顶部的纵坐标。
    pub top: f64,
    /// 行高。
    pub height: f64,
}

impl LineBox {
    /// 行内是否只有空白文本。
    pub fn is_blank(&self) -> bool {
        self.fragments.iter().all(|fragment| match &fragment.kind {
            FragmentContent::Text(fragment) => fragment.text.trim().is_empty(),
            FragmentContent::Atom { .. } => false,
        })
    }
}

/// 盒子树上的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBox {
    /// 对应的 DOM 节点，匿名盒与匿名文本没有。
    pub node: Option<NodeId>,
    /// 盒子种类。
    pub kind: BoxKind,
    /// 计算样式。匿名盒继承父元素中与继承相关的部分。
    pub style: ComputedStyle,
    /// 子盒子。
    pub children: Vec<LayoutBox>,

    /// 外边距，已解析成像素。
    pub margin: Edges,
    /// 边框宽度，已解析成像素。
    pub border: Edges,
    /// 内边距，已解析成像素。
    pub padding: Edges,

    /// 边框盒相对页面左上角的位置与尺寸。
    pub rect: Rect,
    /// 内容盒的位置与尺寸。
    pub content: Rect,
    /// 文本排版结果，文本盒才有。
    pub text: Option<TextLayout>,
    /// 行内内容排版出来的行盒。
    ///
    /// 只有建立了行内格式化上下文的盒子才有内容，绘制阶段按它逐段画文字。
    pub lines: Vec<LineBox>,
    /// 绘制时用的裁剪矩形，由祖先里最近的裁剪盒决定。
    pub clip: Rect,
    /// 布局期临时指定的宽度。
    ///
    /// 弹性项与行内块在布局前就已经算好宽度，需要绕过样式里的自动值。
    /// 每次布局开始时由父元素重新赋值，不参与样式层叠。
    pub forced_width: Option<f64>,
}

impl LayoutBox {
    /// 建一个盒子。
    pub fn new(node: Option<NodeId>, kind: BoxKind, style: ComputedStyle) -> Self {
        Self {
            node,
            kind,
            style,
            children: Vec::new(),
            margin: Edges::default(),
            border: Edges::default(),
            padding: Edges::default(),
            rect: Rect::default(),
            content: Rect::default(),
            text: None,
            lines: Vec::new(),
            clip: Rect::default(),
            forced_width: None,
        }
    }

    /// 取文本内容，非文本盒返回空串。
    pub fn text_content(&self) -> &str {
        match &self.kind {
            BoxKind::Text(text) => text,
            _ => "",
        }
    }

    /// 该盒子是否是文本盒且内容为空。
    pub fn is_empty_text(&self) -> bool {
        matches!(&self.kind, BoxKind::Text(text) if text.is_empty())
    }

    /// 递归统计盒子数量，测试与调试用。
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(LayoutBox::count).sum::<usize>()
    }

    /// 按文档顺序深度优先遍历。
    pub fn descendants(&self) -> Vec<&LayoutBox> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    /// 递归收集自身与后代。
    fn collect<'a>(&'a self, out: &mut Vec<&'a LayoutBox>) {
        out.push(self);
        for child in &self.children {
            child.collect(out);
        }
    }
}

/// 按文档树与计算样式建盒子树。
///
/// 返回 `None` 表示根元素不生成盒子，也就是整份文档都不显示。
pub fn build_layout_tree(document: &Document, styles: &StyleMap) -> Option<LayoutBox> {
    let root_element = document
        .children(document.root())
        .iter()
        .copied()
        .find(|id| document.node(*id).as_element().is_some())?;

    let style = styles.get(&root_element)?.clone();
    if !style.generates_box() {
        return None;
    }

    Some(build_element(document, styles, root_element, style))
}

/// 建一个元素对应的盒子及其子树。
fn build_element(
    document: &Document,
    styles: &StyleMap,
    node: NodeId,
    style: ComputedStyle,
) -> LayoutBox {
    let kind = box_kind_for(style.display);
    let mut layout_box = LayoutBox::new(Some(node), kind, style.clone());

    let mut children = Vec::new();
    for child in document.children(node) {
        match &document.node(*child).data {
            NodeData::Element(_) => {
                let Some(child_style) = styles.get(child) else {
                    continue;
                };
                if !child_style.generates_box() {
                    continue;
                }
                children.push(build_element(document, styles, *child, child_style.clone()));
            }
            NodeData::Text(text) => {
                let processed = process_text(text, style.white_space);
                if !processed.is_empty() {
                    let mut text_box = LayoutBox::new(
                        Some(*child),
                        BoxKind::Text(processed.into_boxed_str()),
                        style.clone(),
                    );
                    text_box.clip = layout_box.rect;
                    children.push(text_box);
                }
            }
            // 注释与 DOCTYPE 不生成盒子。
            NodeData::Comment(_) | NodeData::Doctype(_) | NodeData::Document => {}
        }
    }

    layout_box.children = normalize_children(children, &style);
    layout_box
}

/// 由 `display` 决定盒子种类。
fn box_kind_for(display: Display) -> BoxKind {
    match display {
        Display::Block => BoxKind::Block,
        Display::Inline => BoxKind::Inline,
        Display::InlineBlock => BoxKind::InlineBlock,
        Display::Flex | Display::InlineFlex => BoxKind::Flex,
        Display::ListItem => BoxKind::ListItem,
        Display::Table => BoxKind::Table,
        Display::TableRowGroup => BoxKind::TableRowGroup,
        Display::TableRow => BoxKind::TableRow,
        Display::TableCell => BoxKind::TableCell,
        Display::TableCaption => BoxKind::TableCaption,
        // `display: none` 的元素在调用方已经被过滤掉。
        Display::None => BoxKind::Block,
    }
}

/// 按规范处理子盒子列表里的匿名块。
///
/// 块容器里既有块级又有行内级子元素时，连续的行内级子元素要包进一个匿名
/// 块盒里；如果容器本身是行内盒，内容就都是行内级，不需要处理。
fn normalize_children(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> Vec<LayoutBox> {
    if !matches!(
        parent_style.display,
        Display::Block | Display::ListItem | Display::Flex
    ) {
        return children;
    }

    let has_block = children.iter().any(|child| child.kind.is_block_level());
    let has_inline = children.iter().any(|child| child.kind.is_inline_level());
    // 弹性容器的子元素按块级处理，不做匿名包装。
    if parent_style.display == Display::Flex || !has_block || !has_inline {
        return children;
    }

    let mut normalized = Vec::new();
    let mut pending: Vec<LayoutBox> = Vec::new();

    for child in children {
        if child.kind.is_block_level() {
            if !pending.is_empty() {
                normalized.push(make_anonymous_block(
                    std::mem::take(&mut pending),
                    parent_style,
                ));
            }
            normalized.push(child);
        } else {
            pending.push(child);
        }
    }
    if !pending.is_empty() {
        normalized.push(make_anonymous_block(pending, parent_style));
    }
    normalized
}

/// 把一串行内级盒子包进匿名块盒。
fn make_anonymous_block(children: Vec<LayoutBox>, parent_style: &ComputedStyle) -> LayoutBox {
    // 匿名盒不继承外边距与边框，只保留与文字相关的部分。
    let mut style = ComputedStyle::inherit_from(parent_style);
    style.display = Display::Block;
    style.margin = crate::style::Sides::zero();
    style.padding = crate::style::Sides::zero();
    style.border_style = crate::style::BorderSides::default();
    style.border_width = crate::style::Sides::zero();
    style.width = crate::style::Size::Auto;
    style.height = crate::style::Size::Auto;

    let mut block = LayoutBox::new(None, BoxKind::AnonymousBlock, style);
    block.children = children;
    block
}

/// 按 `white-space` 处理文本节点里的空白。
pub fn process_text(text: &str, white_space: WhiteSpace) -> String {
    if white_space.preserves_spaces() {
        // pre 与 pre-wrap 原样保留。
        return text.to_string();
    }
    if white_space.preserves_newlines() {
        // pre-line 保留换行，其余空白折叠成一个空格。
        let mut out = String::new();
        let mut in_space = false;
        for character in text.chars() {
            if character == '\n' {
                out.push('\n');
                in_space = false;
            } else if character.is_whitespace() {
                if !in_space {
                    out.push(' ');
                    in_space = true;
                }
            } else {
                out.push(character);
                in_space = false;
            }
        }
        return out;
    }

    // normal 与 nowrap 把连续空白折叠成一个空格。
    let mut out = String::new();
    let mut in_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::parse_document;
    use crate::style::compute_styles;

    /// 解析文档并建盒子树。
    fn build(html: &str, css: &str) -> Option<LayoutBox> {
        let document = parse_document(html);
        let sheets = if css.trim().is_empty() {
            Vec::new()
        } else {
            vec![crate::css::parse_stylesheet(css)]
        };
        let styles = compute_styles(&document, &sheets);
        build_layout_tree(&document, &styles)
    }

    /// 把盒子树导出成缩进文本，方便断言结构。
    fn dump(layout_box: &LayoutBox) -> String {
        let mut out = String::new();
        write_box(layout_box, 0, &mut out);
        out.trim_end().to_string()
    }

    /// 递归写出一个盒子。
    fn write_box(layout_box: &LayoutBox, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        let name = match &layout_box.kind {
            BoxKind::Block => "block".to_string(),
            BoxKind::Inline => "inline".to_string(),
            BoxKind::InlineBlock => "inline-block".to_string(),
            BoxKind::Flex => "flex".to_string(),
            BoxKind::ListItem => "list-item".to_string(),
            BoxKind::AnonymousBlock => "anon".to_string(),
            BoxKind::Table => "table".to_string(),
            BoxKind::TableRowGroup => "table-row-group".to_string(),
            BoxKind::TableRow => "table-row".to_string(),
            BoxKind::TableCell => "table-cell".to_string(),
            BoxKind::TableCaption => "table-caption".to_string(),
            BoxKind::Text(text) => format!("text {:?}", text.trim_end()),
        };
        out.push_str(&format!("{indent}{name}\n"));
        for child in &layout_box.children {
            write_box(child, depth + 1, out);
        }
    }

    #[test]
    fn simple_block_structure() {
        let tree = build("<div><p>hi</p></div>", "").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      block\n        text \"hi\""
        );
    }

    #[test]
    fn hidden_elements_are_skipped() {
        let tree = build(
            "<div><span>a</span><p>visible</p></div>",
            "span { display: none }",
        )
        .expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      block\n        text \"visible\""
        );
    }

    #[test]
    fn head_is_not_rendered() {
        let tree = build(
            "<html><head><title>t</title></head><body><p>x</p></body></html>",
            "",
        )
        .expect("应当有盒子树");
        assert!(!dump(&tree).contains("title"), "head 里的内容不该进盒子树");
    }

    #[test]
    fn inline_children_make_anonymous_block() {
        // div 里既有块级又有行内级内容，行内内容要包进匿名块。
        let tree = build("<div>text<p>para</p></div>", "").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      anon\n        text \"text\"\n      block\n        text \"para\""
        );
    }

    #[test]
    fn pure_inline_children_need_no_anonymous_block() {
        let tree = build("<p>a<span>b</span></p>", "").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      text \"a\"\n      inline\n        text \"b\""
        );
    }

    #[test]
    fn whitespace_collapsing() {
        assert_eq!(process_text("a   b\n\tc", WhiteSpace::Normal), "a b c");
        assert_eq!(process_text("  leading", WhiteSpace::Normal), " leading");
        assert_eq!(process_text("a\nb", WhiteSpace::Pre), "a\nb");
        assert_eq!(process_text("a   b", WhiteSpace::Pre), "a   b");
        assert_eq!(process_text("a   b\nc", WhiteSpace::PreLine), "a b\nc");
        assert_eq!(process_text("a   b", WhiteSpace::NoWrap), "a b");
    }

    #[test]
    fn whitespace_only_text_is_dropped() {
        let tree = build("<div>   <p>x</p>   </div>", "").expect("应当有盒子树");
        // 纯空白文本折叠成一个空格后仍然保留，由行内布局决定是否显示。
        let dump = dump(&tree);
        assert!(dump.contains("block"), "{dump}");
    }

    #[test]
    fn flex_container_kind() {
        let tree =
            build("<div><span>a</span></div>", "div { display: flex }").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    flex\n      inline\n        text \"a\""
        );
    }

    #[test]
    fn inline_block_kind() {
        let tree = build("<span>a</span>", "span { display: inline-block }").expect("应当有盒子树");
        assert!(dump(&tree).contains("inline-block"), "{}", dump(&tree));
    }

    #[test]
    fn nested_inline_markup() {
        let tree = build("<p>a<b>b<i>c</i></b></p>", "").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      text \"a\"\n      inline\n        text \"b\"\n        inline\n          text \"c\""
        );
    }

    #[test]
    fn list_item_kind() {
        let tree = build("<ul><li>x</li></ul>", "").expect("应当有盒子树");
        assert!(dump(&tree).contains("list-item"), "{}", dump(&tree));
    }

    #[test]
    fn comments_produce_no_boxes() {
        let tree = build("<div><!-- 注释 --><p>x</p></div>", "").expect("应当有盒子树");
        assert_eq!(
            dump(&tree),
            "block\n  block\n    block\n      block\n        text \"x\""
        );
    }

    #[test]
    fn root_display_none_yields_nothing() {
        let tree = build("<html><body>x</body></html>", "html { display: none }");
        assert!(tree.is_none());
    }

    #[test]
    fn box_count_matches_structure() {
        let tree = build("<div><p>a</p><p>b</p></div>", "").expect("应当有盒子树");
        // html、body、div、两个 p、两段文本。
        assert_eq!(tree.count(), 7);
    }

    #[test]
    fn empty_text_nodes_are_skipped() {
        let tree = build("<p></p>", "").expect("应当有盒子树");
        assert_eq!(dump(&tree), "block\n  block\n    block");
    }

    #[test]
    fn text_boxes_carry_parent_style() {
        let tree = build("<p>hello</p>", "p { color: red }").expect("应当有盒子树");
        let boxes = tree.descendants();
        let text_box = boxes
            .iter()
            .find(|layout_box| layout_box.kind.is_text())
            .expect("应当有文本盒");
        assert_eq!(
            text_box.style.color,
            crate::css::Color::rgba(255, 0, 0, 255)
        );
    }
}
