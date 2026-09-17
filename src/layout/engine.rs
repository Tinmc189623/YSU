//! 布局算法。
//!
//! 三种格式化上下文都实现了：块级盒纵向堆叠（含外边距合并）、行内内容
//! 排版（文本断行与行内块）、弹性容器（主轴分配与侧轴对齐）。
//!
//! 坐标约定：每个盒子接收的 `origin` 是它的外边距盒左上角，函数自己把
//! 外边距、边框、内边距减进去算出内容盒位置，返回外边距盒的总高度，供
//! 父元素继续往下排。

use super::box_tree::{BoxKind, FragmentContent, LayoutBox, LineBox, LineFragment, TextFragment};
use super::geometry::{Edges, Rect};
use super::text::{TextMeasurer, TextStyle};
use crate::css::value::{Color, Length, LengthUnit};
use crate::dom::NodeId;
use crate::dom::node::Document;
use crate::style::{
    AlignItems, ComputedStyle, JustifyContent, Size as CssSize, StyleMap, TextAlign,
};

/// 布局上下文里与视口相关的量。
#[derive(Debug, Clone, Copy)]
pub struct LayoutContext {
    /// 视口宽度。
    pub viewport_width: f64,
    /// 视口高度。
    pub viewport_height: f64,
    /// 根元素字号，`rem` 参照它。
    pub root_font_size: f64,
}

impl Default for LayoutContext {
    /// 一个常见的桌面视口。
    fn default() -> Self {
        Self {
            viewport_width: 1280.0,
            viewport_height: 800.0,
            root_font_size: 16.0,
        }
    }
}

/// 布局结果。
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree {
    /// 根盒子。
    pub root: LayoutBox,
    /// 视口宽度。
    pub viewport_width: f64,
    /// 视口高度。
    pub viewport_height: f64,
    /// 画布底色，由根元素或 body 传播上来。
    pub canvas_background: Color,
    /// 贡献了画布底色的元素，绘制时它自己那层背景不再重复画。
    pub canvas_owner: Option<NodeId>,
}

impl LayoutTree {
    /// 整份文档的高度，取根盒子下边界。
    pub fn document_height(&self) -> f64 {
        self.root.rect.bottom().max(self.viewport_height)
    }

    /// 按文档顺序遍历全部盒子。
    pub fn descendants(&self) -> Vec<&LayoutBox> {
        self.root.descendants()
    }

    /// 找出包含某个点的文字片段来自哪个节点。
    ///
    /// 行内元素在盒子树里没有自己的盒子，文字被折进所在块的行里，所以
    /// 只按盒子找不出「这一点压在哪个链接上」。这里反过来：先找到包含该点
    /// 的盒子，再从它的行里挑出压在该点上的那个片段，片段的来源节点就是
    /// 答案。调用方顺着它往上找祖先即可。
    pub fn fragment_node_at(&self, x: f64, y: f64) -> Option<NodeId> {
        let point = super::geometry::Point::new(x, y);
        let mut best: Option<NodeId> = None;
        for layout_box in self.descendants() {
            if !layout_box.rect.contains(point) {
                continue;
            }
            for line in &layout_box.lines {
                for fragment in &line.fragments {
                    if !fragment.rect.contains(point) {
                        continue;
                    }
                    let node = match &fragment.kind {
                        FragmentContent::Text(text) => text.node,
                        FragmentContent::Atom { child_index } => layout_box
                            .children
                            .get(*child_index)
                            .and_then(|child| child.node),
                    };
                    if let Some(node) = node {
                        best = Some(node);
                    }
                }
            }
        }
        best
    }

    /// 找出包含某个点的最深的盒子，用于命中测试。
    pub fn hit_test(&self, x: f64, y: f64) -> Option<&LayoutBox> {
        let mut best: Option<&LayoutBox> = None;
        for layout_box in self.descendants() {
            if layout_box.rect.contains(super::geometry::Point::new(x, y)) {
                best = Some(layout_box);
            }
        }
        best
    }
}

/// 布局引擎。
pub struct LayoutEngine {
    /// 文本度量器。
    measurer: TextMeasurer,
    /// 与视口相关的量。
    context: LayoutContext,
}

impl Default for LayoutEngine {
    /// 建一个使用默认视口的引擎。
    fn default() -> Self {
        Self::new(LayoutContext::default())
    }
}

impl LayoutEngine {
    /// 建一个引擎。
    pub fn new(context: LayoutContext) -> Self {
        Self {
            measurer: TextMeasurer::new(),
            context,
        }
    }

    /// 更新视口尺寸，窗口大小变化后调用。
    pub fn set_context(&mut self, context: LayoutContext) {
        self.context = context;
    }

    /// 取当前上下文。
    pub fn context(&self) -> LayoutContext {
        self.context
    }

    /// 取文本度量器的可变引用，绘制阶段取字形时要用。
    pub fn measurer_mut(&mut self) -> &mut TextMeasurer {
        &mut self.measurer
    }

    /// 对整份文档做布局。
    pub fn layout(&mut self, document: &Document, styles: &StyleMap) -> Option<LayoutTree> {
        let root_font_size = self.root_font_size(document, styles);
        let mut context = self.context;
        context.root_font_size = root_font_size;
        self.context = context;

        let mut root = super::box_tree::build_layout_tree(document, styles)?;
        let viewport_width = self.context.viewport_width;
        self.layout_box(&mut root, viewport_width, None, 0.0, 0.0);
        // 根元素至少要铺满视口高度，这样背景色能盖住整屏。
        let viewport_height = self.context.viewport_height;
        if root.rect.height < viewport_height {
            let extra = viewport_height - root.rect.height;
            root.rect.height += extra;
            root.content.height += extra;
        }

        let (canvas_background, canvas_owner) = canvas_background(document, styles);

        Some(LayoutTree {
            root,
            viewport_width,
            viewport_height,
            canvas_background,
            canvas_owner,
        })
    }

    /// 取根元素的字号，供 `rem` 使用。
    fn root_font_size(&self, document: &Document, styles: &StyleMap) -> f64 {
        let Some(root_element) = document
            .children(document.root())
            .iter()
            .copied()
            .find(|id| document.node(*id).as_element().is_some())
        else {
            return 16.0;
        };
        styles
            .get(&root_element)
            .map_or(16.0, ComputedStyle::font_size_pixels)
    }

    // ---- 长度解析 ----

    /// 把一个 CSS 长度解析成像素。
    pub fn resolve_length(&self, length: Length, containing: f64, font_size: f64) -> f64 {
        match length.unit {
            LengthUnit::None | LengthUnit::Px => length.value,
            LengthUnit::Percent => length.value / 100.0 * containing,
            LengthUnit::Em => length.value * font_size,
            LengthUnit::Rem => length.value * self.context.root_font_size,
            LengthUnit::Vw => length.value / 100.0 * self.context.viewport_width,
            LengthUnit::Vh => length.value / 100.0 * self.context.viewport_height,
            LengthUnit::Vmin => {
                length.value / 100.0
                    * self
                        .context
                        .viewport_width
                        .min(self.context.viewport_height)
            }
            LengthUnit::Vmax => {
                length.value / 100.0
                    * self
                        .context
                        .viewport_width
                        .max(self.context.viewport_height)
            }
            // `ch` 与 `ex` 需要真实字体度量，这里按字号的比例近似。
            LengthUnit::Ch => length.value * font_size * 0.5,
            LengthUnit::Ex => length.value * font_size * 0.5,
            unit => unit
                .absolute_pixels()
                .map_or(length.value, |scale| length.value * scale),
        }
    }

    /// 解析四边长度。
    fn resolve_sides(&self, sides: crate::style::Sides, containing: f64, font_size: f64) -> Edges {
        Edges {
            top: self.resolve_length(sides.top, containing, font_size),
            right: self.resolve_length(sides.right, containing, font_size),
            bottom: self.resolve_length(sides.bottom, containing, font_size),
            left: self.resolve_length(sides.left, containing, font_size),
        }
    }

    /// 解析盒子的外边距、边框与内边距。
    fn resolve_edges(
        &self,
        layout_box: &LayoutBox,
        containing_width: f64,
    ) -> (Edges, Edges, Edges) {
        let font_size = layout_box.style.font_size_pixels();
        let margin = self.resolve_sides(layout_box.style.margin, containing_width, font_size);
        let padding = self.resolve_sides(layout_box.style.padding, containing_width, font_size);
        let mut border =
            self.resolve_sides(layout_box.style.border_width, containing_width, font_size);
        // 没有可见样式的边框宽度按零算。
        let style = &layout_box.style.border_style;
        if !style.top.is_visible() {
            border.top = 0.0;
        }
        if !style.right.is_visible() {
            border.right = 0.0;
        }
        if !style.bottom.is_visible() {
            border.bottom = 0.0;
        }
        if !style.left.is_visible() {
            border.left = 0.0;
        }
        (margin, border, padding)
    }

    /// 解析一个尺寸属性，自动值返回 `None`。
    fn resolve_size(&self, size: CssSize, containing: f64, font_size: f64) -> Option<f64> {
        size.length()
            .map(|length| self.resolve_length(length, containing, font_size))
    }

    /// 把尺寸限制在最小与最大值之间。
    fn apply_size_limits(
        &self,
        value: f64,
        style: &ComputedStyle,
        containing: f64,
        font_size: f64,
    ) -> f64 {
        let mut result = value;
        if let Some(max) = self.resolve_size(style.max_width, containing, font_size)
            && max >= 0.0
        {
            result = result.min(max);
        }
        if let Some(min) = self.resolve_size(style.min_width, containing, font_size) {
            result = result.max(min);
        }
        result
    }

    // ---- 主分派 ----

    /// 布局一个盒子，`origin` 是外边距盒左上角，返回外边距盒高度。
    fn layout_box(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        containing_height: Option<f64>,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        match layout_box.kind.clone() {
            BoxKind::Text(_) => 0.0,
            BoxKind::Inline => {
                self.layout_inline_box(layout_box, containing_width, origin_x, origin_y)
            }
            BoxKind::InlineBlock => self.layout_inline_block(
                layout_box,
                containing_width,
                containing_height,
                origin_x,
                origin_y,
            ),
            BoxKind::Table => self.layout_table(layout_box, containing_width, origin_x, origin_y),
            // 表格内部的行组、行、单元格由 `layout_table` 统一摆放，
            // 走到这里说明它们脱离了表格（比如一行直接放在 body 里），
            // 那就按块级处理，至少不会重叠成一团。
            BoxKind::TableRowGroup
            | BoxKind::TableRow
            | BoxKind::TableCell
            | BoxKind::TableCaption => self.layout_block(
                layout_box,
                containing_width,
                containing_height,
                origin_x,
                origin_y,
            ),
            BoxKind::Flex => self.layout_flex(
                layout_box,
                containing_width,
                containing_height,
                origin_x,
                origin_y,
            ),
            BoxKind::Block | BoxKind::ListItem | BoxKind::AnonymousBlock => self.layout_block(
                layout_box,
                containing_width,
                containing_height,
                origin_x,
                origin_y,
            ),
        }
    }

    /// 块级盒的布局。
    fn layout_block(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        containing_height: Option<f64>,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        let (mut margin, border, padding) = self.resolve_edges(layout_box, containing_width);
        let font_size = layout_box.style.font_size_pixels();

        // 内容宽度：布局期指定了就用它，其次看样式里的明确宽度，最后撑满。
        let available =
            containing_width - margin.horizontal() - border.horizontal() - padding.horizontal();
        let content_width = match layout_box.forced_width {
            Some(width) => width,
            None => match self.resolve_size(layout_box.style.width, containing_width, font_size) {
                Some(width) => width,
                None => available.max(0.0),
            },
        };
        let content_width = self.apply_size_limits(
            content_width,
            &layout_box.style,
            containing_width,
            font_size,
        );

        // 宽度确定时，左右自动外边距把盒子居中。
        let specified_width =
            self.resolve_size(layout_box.style.width, containing_width, font_size);
        if specified_width.is_some() {
            let free = containing_width
                - content_width
                - border.horizontal()
                - padding.horizontal()
                - margin.left
                - margin.right;
            // 只有明确写成 `auto` 的那一边才吃剩余空间。早先这里判的是
            // 「是不是零」，于是 `margin: 0` 与 `margin: auto` 分不开，
            // 任何带明确宽度的块都被居中了。
            if free > 0.0 {
                match (
                    layout_box.style.margin.left.is_auto(),
                    layout_box.style.margin.right.is_auto(),
                ) {
                    (true, true) => {
                        margin.left = free / 2.0;
                        margin.right = free / 2.0;
                    }
                    (true, false) => margin.left = free,
                    (false, true) => margin.right = free,
                    (false, false) => {}
                }
            }
        }

        // 上边框或内边距为零时，首个子元素的上外边距会与父元素合并。
        let collapses_top = border.top == 0.0 && padding.top == 0.0;
        let collapses_bottom = border.bottom == 0.0 && padding.bottom == 0.0;
        if collapses_top
            && let Some(first) = layout_box.children.first()
            && first.kind.is_block_level()
        {
            let first_margin = self.resolve_edges(first, content_width).0;
            margin.top = margin.top.max(first_margin.top);
        }

        let rect_x = origin_x + margin.left;
        let rect_y = origin_y + margin.top;
        let border_width = border.horizontal() + padding.horizontal() + content_width;
        let content_left = rect_x + border.left + padding.left;
        let content_top = rect_y + border.top + padding.top;

        layout_box.margin = margin;
        layout_box.border = border;
        layout_box.padding = padding;

        // 内容高度：由子元素决定，或按设定值。
        let has_inline_children = layout_box
            .children
            .iter()
            .any(|child| child.kind.is_inline_level());
        let content_height =
            if layout_box.children.is_empty() && layout_box.kind == BoxKind::AnonymousBlock {
                0.0
            } else if has_inline_children {
                self.layout_inline_content(
                    layout_box,
                    content_width,
                    content_left,
                    content_top,
                    collapses_top,
                )
            } else {
                self.layout_block_children(
                    layout_box,
                    content_width,
                    content_left,
                    content_top,
                    collapses_top,
                    collapses_bottom,
                )
            };

        let content_height = match self.resolve_size(
            layout_box.style.height,
            containing_height.unwrap_or(0.0),
            font_size,
        ) {
            Some(height) => height,
            None => content_height,
        };
        let content_height = self.apply_height_limits(content_height, &layout_box.style, font_size);

        layout_box.content = Rect::new(content_left, content_top, content_width, content_height);
        layout_box.rect = Rect::new(
            rect_x,
            rect_y,
            border_width,
            border.vertical() + padding.vertical() + content_height,
        );

        layout_box.margin.top + layout_box.rect.height + layout_box.margin.bottom
    }

    /// 对高度应用最小与最大值限制。
    ///
    /// 纵向的百分比按包含块高度算，包含块高度不确定时忽略百分比。
    fn apply_height_limits(&self, value: f64, style: &ComputedStyle, font_size: f64) -> f64 {
        let mut result = value;
        if let Some(max) = self.resolve_size(style.max_height, 0.0, font_size)
            && max > 0.0
        {
            result = result.min(max);
        }
        if let Some(min) = self.resolve_size(style.min_height, 0.0, font_size) {
            result = result.max(min);
        }
        result
    }

    /// 纵向堆叠块级子元素。
    ///
    /// 相邻兄弟元素的下外边距与上外边距取较大者，不做累加。
    fn layout_block_children(
        &mut self,
        layout_box: &mut LayoutBox,
        content_width: f64,
        content_left: f64,
        content_top: f64,
        collapses_top: bool,
        collapses_bottom: bool,
    ) -> f64 {
        let mut y = content_top;
        let mut previous_bottom_margin = 0.0f64;
        let mut first = true;
        let mut last_child_bottom_margin = 0.0f64;

        let font_size = layout_box.style.font_size_pixels();
        let children = std::mem::take(&mut layout_box.children);
        let mut new_children = Vec::with_capacity(children.len());

        for mut child in children {
            let child_margins = self.resolve_edges(&child, content_width).0;
            let top_margin = if first && collapses_top {
                // 与父元素合并了，父元素那边已经算过。
                0.0
            } else {
                previous_bottom_margin.max(child_margins.top)
            };
            // 子元素自己还会再加一次外边距，这里先把合并后的差额减掉，
            // 免得同一个外边距被算两遍。
            let child_y = y + top_margin - child_margins.top;

            // 百分比高度需要父元素的确定高度，这里拿不到就按内容算。
            self.layout_box(&mut child, content_width, None, content_left, child_y);
            let bottom_margin = child.margin.bottom;
            y = child.rect.bottom();
            previous_bottom_margin = bottom_margin;
            last_child_bottom_margin = bottom_margin;
            let _ = font_size;
            first = false;
            new_children.push(child);
        }
        layout_box.children = new_children;

        let mut height = y - content_top;
        // 最后一个子元素的下外边距在没有下边框与内边距时合并出去。
        if !collapses_bottom && !first {
            height += last_child_bottom_margin;
        }
        height
    }

    /// 行内内容容器的布局：把行内子内容排成若干行。
    fn layout_inline_content(
        &mut self,
        layout_box: &mut LayoutBox,
        content_width: f64,
        content_left: f64,
        content_top: f64,
        _collapses_top: bool,
    ) -> f64 {
        let children = std::mem::take(&mut layout_box.children);
        let mut items = Vec::new();
        self.collect_inline_items(children, &mut items);

        let (height, atoms, lines) = self.flow_inline_items(
            items,
            content_width,
            content_left,
            content_top,
            layout_box.style.text_align,
        );
        layout_box.children = atoms;
        layout_box.lines = lines;
        height
    }

    /// 把行内子树拆成文本片段与原子盒。
    ///
    /// 没有背景也没有边框的行内盒会被摊平，其余当作原子盒整体处理。
    fn collect_inline_items(&mut self, children: Vec<LayoutBox>, items: &mut Vec<InlineItem>) {
        for child in children {
            match &child.kind {
                BoxKind::Text(text) => {
                    if !text.is_empty() {
                        items.push(InlineItem::Text(Box::new(TextRun {
                            text: text.to_string(),
                            style: child.style.clone(),
                            node: child.node,
                        })));
                    }
                }
                BoxKind::InlineBlock => items.push(InlineItem::Atom(Box::new(child))),
                BoxKind::Inline => {
                    let has_decoration = !child.style.background_color.is_transparent()
                        || child.style.has_visible_border();
                    if has_decoration {
                        items.push(InlineItem::Atom(Box::new(child)));
                    } else {
                        self.collect_inline_items(child.children, items);
                    }
                }
                // 块级盒不会出现在行内上下文里，出现了也按原子处理。
                _ => items.push(InlineItem::Atom(Box::new(child))),
            }
        }
    }

    /// 把行内片段排进行盒，返回总高度、原子盒列表与行盒列表。
    fn flow_inline_items(
        &mut self,
        items: Vec<InlineItem>,
        available_width: f64,
        left: f64,
        top: f64,
        text_align: TextAlign,
    ) -> (f64, Vec<LayoutBox>, Vec<LineBox>) {
        let mut builders: Vec<LineBuilder> = Vec::new();
        let mut current = LineBuilder::new(left);
        let mut atoms: Vec<LayoutBox> = Vec::new();

        for item in items {
            match item {
                InlineItem::Text(run) => {
                    self.flow_text(&run, available_width, &mut current, &mut builders, left);
                }
                InlineItem::Atom(boxx) => {
                    let mut atom = *boxx;
                    let (margin, border, padding) = self.resolve_edges(&atom, available_width);
                    let content_width = self.measure_atomic_content_width(&atom, available_width);
                    atom.forced_width = Some(content_width);
                    let outer = content_width
                        + border.horizontal()
                        + padding.horizontal()
                        + margin.horizontal();
                    // 放不下就换行，但空行上再宽也要放。
                    if !current.is_empty() && current.width + outer > available_width {
                        builders.push(std::mem::replace(&mut current, LineBuilder::new(left)));
                    }
                    let atom_x = current.left + current.width + margin.left;
                    self.layout_box(&mut atom, available_width, None, atom_x, 0.0);
                    let atom_height = atom.rect.height + margin.vertical();
                    let child_index = atoms.len();
                    current.push_atom(atom_x, outer, atom_height, child_index);
                    atoms.push(atom);
                }
            }
        }
        if !current.is_empty() {
            builders.push(current);
        }
        if builders.is_empty() {
            builders.push(LineBuilder::new(left));
        }

        // 逐行确定高度与片段位置。
        let mut lines = Vec::with_capacity(builders.len());
        let mut y = top;
        for builder in builders {
            let line = builder.finish(y, available_width, text_align);
            y += line.height;
            lines.push(line);
        }

        // 原子盒在排版时用的是行内临时坐标，这里按最终的行位置平移过去。
        for line in &lines {
            for fragment in &line.fragments {
                if let FragmentContent::Atom { child_index } = fragment.kind
                    && let Some(atom) = atoms.get_mut(child_index)
                {
                    let atom_height = atom.rect.height;
                    let offset = ((line.height - atom_height) / 2.0).max(0.0);
                    atom.translate_to(fragment.rect.x, line.top + offset);
                }
            }
        }

        (y - top, atoms, lines)
    }

    /// 把一段文本排进当前行，放不下就换行。
    ///
    /// 一次度量就能拿到全部行，所以只有当前行已经有内容时才需要重新度量，
    /// 长段落不会退化成反复整形。
    fn flow_text(
        &mut self,
        run: &TextRun,
        available_width: f64,
        current: &mut LineBuilder,
        lines: &mut Vec<LineBuilder>,
        left: f64,
    ) {
        if run.text.is_empty() {
            return;
        }
        let text_style = text_style_of(&run.style);
        let style = &run.style;
        let node = run.node;
        let mut remaining = run.text.clone();

        loop {
            let space_left = (available_width - current.width).max(1.0);
            let layout = self
                .measurer
                .layout(&remaining, &text_style, Some(space_left as f32));
            if layout.lines.is_empty() {
                return;
            }

            let line_count = layout.lines.len();
            if line_count == 1 {
                // 剩下的都放得下，接到当前行上就结束了。
                let line = &layout.lines[0];
                if !line.text.is_empty() {
                    current.push_text(
                        line.text.clone(),
                        style,
                        line.width as f64,
                        line.height as f64,
                        node,
                    );
                }
                return;
            }

            // 多行的情况：先看当前行是不是空行。
            if current.is_empty() {
                // 当前行是空的，说明这次是按整行宽度度量的，前 n-1 行各自
                // 独占一行，最后一行留在当前行上等后续内容接着排。
                for line in &layout.lines[..line_count - 1] {
                    let mut builder = LineBuilder::new(left);
                    if !line.text.is_empty() {
                        builder.push_text(
                            line.text.clone(),
                            style,
                            line.width as f64,
                            line.height as f64,
                            node,
                        );
                    }
                    lines.push(builder);
                }
                let last = &layout.lines[line_count - 1];
                if !last.text.is_empty() {
                    current.push_text(
                        last.text.clone(),
                        style,
                        last.width as f64,
                        last.height as f64,
                        node,
                    );
                }
                return;
            }

            // 当前行已经有内容，只有第一行能接上去，剩下的从新行继续。
            let first = &layout.lines[0];
            if !first.text.is_empty() {
                current.push_text(
                    first.text.clone(),
                    style,
                    first.width as f64,
                    first.height as f64,
                    node,
                );
            }
            let rest = remaining
                .get(first.raw_len..)
                .unwrap_or("")
                .trim_start_matches([' ', '\t', '\n', '\r'])
                .to_string();
            lines.push(std::mem::replace(current, LineBuilder::new(left)));
            if rest.is_empty() {
                return;
            }
            remaining = rest;
        }
    }

    /// 估算原子行内盒的内容宽度，不含边框、内边距与外边距。
    fn measure_atomic_content_width(&mut self, atom: &LayoutBox, available: f64) -> f64 {
        let font_size = atom.style.font_size_pixels();
        if let Some(width) = self.resolve_size(atom.style.width, available, font_size) {
            return width;
        }
        // 没有明确宽度时按内容的固有宽度收缩到适配。
        let (margin, border, padding) = self.resolve_edges(atom, available);
        let content_budget =
            available - margin.horizontal() - border.horizontal() - padding.horizontal();
        let intrinsic = self.intrinsic_width(atom, available);
        intrinsic.min(content_budget.max(0.0)).max(0.0)
    }

    /// 估算盒子的固有内容宽度。
    ///
    /// 行内内容按不换行时的宽度累加，块级子元素取其中最宽的一个。只算内容，
    /// 不含外边距、边框与内边距，调用方按需要自己加减。
    fn intrinsic_width(&mut self, layout_box: &LayoutBox, containing: f64) -> f64 {
        let mut content = 0.0f64;
        for child in &layout_box.children {
            match &child.kind {
                BoxKind::Text(text) => {
                    let style = text_style_of(&child.style);
                    content += f64::from(self.measurer.measure_width(text, &style));
                }
                BoxKind::Inline | BoxKind::InlineBlock => {
                    let (margin, border, padding) = self.resolve_edges(child, containing);
                    let inner = self.intrinsic_width(child, containing)
                        + border.horizontal()
                        + padding.horizontal()
                        + margin.horizontal();
                    content += inner;
                }
                _ => {
                    content = content.max(self.intrinsic_width(child, containing));
                }
            }
        }
        content
    }

    /// 行内盒的布局：按收缩到适配的宽度当块处理。
    fn layout_inline_box(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        self.layout_inline_block(layout_box, containing_width, None, origin_x, origin_y)
    }

    /// 行内块与行内盒的公共布局：宽度取收缩到适配。
    fn layout_inline_block(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        containing_height: Option<f64>,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        let font_size = layout_box.style.font_size_pixels();
        let (margin, border, padding) = self.resolve_edges(layout_box, containing_width);
        let specified = match layout_box.forced_width {
            Some(width) => Some(width),
            None => self.resolve_size(layout_box.style.width, containing_width, font_size),
        };
        let content_width = match specified {
            Some(width) => width,
            None => {
                let intrinsic = self.intrinsic_width(layout_box, containing_width);
                let outer = intrinsic.min(containing_width)
                    - margin.horizontal()
                    - border.horizontal()
                    - padding.horizontal();
                outer.max(0.0)
            }
        };

        let rect_x = origin_x + margin.left;
        let rect_y = origin_y + margin.top;
        let content_left = rect_x + border.left + padding.left;
        let content_top = rect_y + border.top + padding.top;

        layout_box.margin = margin;
        layout_box.border = border;
        layout_box.padding = padding;

        let has_inline_children = layout_box
            .children
            .iter()
            .any(|child| child.kind.is_inline_level());
        let content_height = if has_inline_children {
            self.layout_inline_content(layout_box, content_width, content_left, content_top, true)
        } else {
            self.layout_block_children(
                layout_box,
                content_width,
                content_left,
                content_top,
                true,
                true,
            )
        };
        let content_height = match self.resolve_size(
            layout_box.style.height,
            containing_height.unwrap_or(0.0),
            font_size,
        ) {
            Some(height) => height,
            None => content_height,
        };

        layout_box.content = Rect::new(content_left, content_top, content_width, content_height);
        layout_box.rect = Rect::new(
            rect_x,
            rect_y,
            border.horizontal() + padding.horizontal() + content_width,
            border.vertical() + padding.vertical() + content_height,
        );
        layout_box.margin.top + layout_box.rect.height + layout_box.margin.bottom
    }

    // ---- 表格布局 ----

    /// 表格的布局。
    ///
    /// 最小可用版本：列宽按各列内容取，行高取该行最高的单元格，
    /// 单元格按列对齐。不做的是表格布局里最麻烦的那几样——列宽自动分配
    /// 算法（`table-layout: auto` 的完整约束求解）、边框合并、跨行跨列。
    /// 这几样缺了之后表格会偏窄或偏宽，但行列是齐的，能看。
    fn layout_table(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        let (margin, border, padding) = self.resolve_edges(layout_box, containing_width);
        let font_size = layout_box.style.font_size_pixels();
        let available =
            (containing_width - margin.horizontal() - border.horizontal() - padding.horizontal())
                .max(0.0);
        // 先只记下每行的位置，不拿出可变引用——后面还要用 `&mut self` 去
        // 布局单元格，同时持有子盒的可变借用会和它打架。
        let rows = row_paths(&layout_box.children);
        let columns = rows
            .iter()
            .map(|path| row_cells(row_at(&layout_box.children, path)).len())
            .max()
            .unwrap_or(0);
        let mut widths = self.column_widths(&layout_box.children, &rows, columns, available);

        // 表格宽度：指定了就听指定的，没指定就收到内容宽度——表格不该像块
        // 那样默认撑满容器。
        let content_width =
            match self.resolve_size(layout_box.style.width, containing_width, font_size) {
                Some(width) => {
                    self.apply_size_limits(width, &layout_box.style, containing_width, font_size)
                }
                None => widths.iter().sum::<f64>().min(available),
            };
        // 列宽按最终的表格宽度再分一次：定宽表格要铺满，这样右边才不会空一块。
        let total: f64 = widths.iter().sum();
        if total > 0.0 && (total - content_width).abs() > 0.5 {
            let factor = content_width / total;
            for width in &mut widths {
                *width *= factor;
            }
        }

        let rect_x = origin_x + margin.left;
        let rect_y = origin_y + margin.top;
        let content_left = rect_x + border.left + padding.left;
        let content_top = rect_y + border.top + padding.top;

        layout_box.margin = margin;
        layout_box.border = border;
        layout_box.padding = padding;

        let mut y = content_top;
        for path in &rows {
            let row = row_at_mut(&mut layout_box.children, path);
            let cells = row_cells(row);
            let mut x = content_left;
            let mut row_height = 0.0f64;
            for (index, cell_index) in cells.iter().enumerate() {
                let Some(width) = widths.get(index).copied() else {
                    continue;
                };
                let cell = &mut row.children[*cell_index];
                // 单元格自己的外边距、边框与内边距要从列宽里扣掉，
                // 剩下的才是它能用的内容宽度。
                let (cell_margin, cell_border, cell_padding) = self.resolve_edges(cell, width);
                let inner = (width
                    - cell_margin.horizontal()
                    - cell_border.horizontal()
                    - cell_padding.horizontal())
                .max(0.0);
                cell.forced_width = Some(inner);
                self.layout_box(cell, width, None, x, y);
                row_height = row_height.max(cell.rect.height + cell_margin.vertical());
                x += width;
            }

            // 行盒自己不画东西，它的矩形只是把这一行的位置与范围记下来，
            // 行组的高度靠它累加。
            let row_width: f64 = widths.iter().take(cells.len()).sum();
            row.rect = Rect::new(content_left, y, row_width, row_height);
            row.content = row.rect;
            y += row_height;
        }

        // 行组的高度就是它里面所有行的高度之和。行组自己没有内容盒，
        // 位置随第一行，高度累加到最后一行的下边界。
        for child in &mut layout_box.children {
            if child.kind != BoxKind::TableRowGroup {
                continue;
            }
            let mut top = None;
            let mut bottom = 0.0f64;
            for row in &child.children {
                if row.kind != BoxKind::TableRow {
                    continue;
                }
                top.get_or_insert(row.rect.y);
                bottom = bottom.max(row.rect.bottom());
            }
            if let Some(top) = top {
                child.rect = Rect::new(content_left, top, content_width, bottom - top);
                child.content = child.rect;
            }
        }

        let table_height = (y - content_top).max(0.0);
        layout_box.rect = Rect::new(
            rect_x,
            rect_y,
            border.horizontal() + padding.horizontal() + content_width,
            border.vertical() + padding.vertical() + table_height,
        );
        layout_box.content = Rect::new(content_left, content_top, content_width, table_height);

        layout_box.margin.top + layout_box.rect.height + layout_box.margin.bottom
    }

    /// 按各列内容的偏好宽度分配列宽。
    ///
    /// 先取每列里最宽的那个单元格的固有宽度，总和放得下就用它——表格因此
    /// 收得比容器窄，符合表格的实际习惯；放不下就按比例压缩到可用宽度。
    fn column_widths(
        &mut self,
        children: &[LayoutBox],
        rows: &[Vec<usize>],
        columns: usize,
        available: f64,
    ) -> Vec<f64> {
        if columns == 0 {
            return Vec::new();
        }
        let mut widths = vec![0.0f64; columns];
        for path in rows {
            let row = row_at(children, path);
            for (index, cell_index) in row_cells(row).iter().enumerate() {
                if index >= columns {
                    break;
                }
                let cell = &row.children[*cell_index];
                let (cell_margin, cell_border, cell_padding) = self.resolve_edges(cell, available);
                let intrinsic = self.intrinsic_width(cell, available)
                    + cell_border.horizontal()
                    + cell_padding.horizontal()
                    + cell_margin.horizontal();
                widths[index] = widths[index].max(intrinsic);
            }
        }

        let total: f64 = widths.iter().sum();
        if total <= 0.0 {
            // 内容完全量不出宽度（空单元格），那就平分。
            return vec![available / columns as f64; columns];
        }
        if total <= available {
            return widths;
        }
        let factor = available / total;
        widths.iter().map(|width| width * factor).collect()
    }

    // ---- 弹性布局 ----

    /// 弹性容器的布局。
    fn layout_flex(
        &mut self,
        layout_box: &mut LayoutBox,
        containing_width: f64,
        containing_height: Option<f64>,
        origin_x: f64,
        origin_y: f64,
    ) -> f64 {
        let (margin, border, padding) = self.resolve_edges(layout_box, containing_width);
        let font_size = layout_box.style.font_size_pixels();
        let is_row = layout_box.style.flex_direction.is_row();

        let available =
            containing_width - margin.horizontal() - border.horizontal() - padding.horizontal();
        let content_width =
            match self.resolve_size(layout_box.style.width, containing_width, font_size) {
                Some(width) => width,
                None => available.max(0.0),
            };

        let rect_x = origin_x + margin.left;
        let rect_y = origin_y + margin.top;
        let content_left = rect_x + border.left + padding.left;
        let content_top = rect_y + border.top + padding.top;

        layout_box.margin = margin;
        layout_box.border = border;
        layout_box.padding = padding;

        let children = std::mem::take(&mut layout_box.children);
        let content_height = if children.is_empty() {
            0.0
        } else if is_row {
            self.layout_flex_row(
                children,
                layout_box,
                content_width,
                containing_height,
                content_left,
                content_top,
            )
        } else {
            self.layout_flex_column(
                children,
                layout_box,
                content_width,
                containing_height,
                content_left,
                content_top,
            )
        };

        let content_height = match self.resolve_size(
            layout_box.style.height,
            containing_height.unwrap_or(0.0),
            font_size,
        ) {
            Some(height) => height,
            None => content_height,
        };

        layout_box.content = Rect::new(content_left, content_top, content_width, content_height);
        layout_box.rect = Rect::new(
            rect_x,
            rect_y,
            border.horizontal() + padding.horizontal() + content_width,
            border.vertical() + padding.vertical() + content_height,
        );
        layout_box.margin.top + layout_box.rect.height + layout_box.margin.bottom
    }

    /// 主轴为水平方向的弹性布局。
    fn layout_flex_row(
        &mut self,
        children: Vec<LayoutBox>,
        container: &mut LayoutBox,
        content_width: f64,
        containing_height: Option<f64>,
        content_left: f64,
        content_top: f64,
    ) -> f64 {
        let style = container.style.clone();
        let gap = self.resolve_length(style.column_gap, content_width, style.font_size_pixels());
        let mut main_sizes: Vec<f64> = Vec::with_capacity(children.len());
        let mut total_main = 0.0f64;

        // 先量出每项的基准主轴尺寸。
        for child in &children {
            let font_size = child.style.font_size_pixels();
            let (margin, border, padding) = self.resolve_edges(child, content_width);
            let basis = match child.style.flex_basis.length() {
                Some(length) => self.resolve_length(length, content_width, font_size),
                None => match self.resolve_size(child.style.width, content_width, font_size) {
                    Some(width) => width,
                    None => {
                        let intrinsic = self.intrinsic_width(child, content_width);
                        intrinsic - margin.horizontal() - border.horizontal() - padding.horizontal()
                    }
                },
            };
            let outer =
                basis.max(0.0) + margin.horizontal() + border.horizontal() + padding.horizontal();
            main_sizes.push(outer);
            total_main += outer;
        }

        let gaps = gap * (children.len().saturating_sub(1)) as f64;
        let free = content_width - total_main - gaps;

        // 按伸展与收缩系数重新分配剩余空间。
        let mut final_main = main_sizes.clone();
        if free > 0.0 {
            let total_grow: f64 = children.iter().map(|child| child.style.flex_grow).sum();
            if total_grow > 0.0 {
                for (index, child) in children.iter().enumerate() {
                    final_main[index] += free * child.style.flex_grow / total_grow;
                }
            }
        } else if free < 0.0 {
            let total_shrink: f64 = children
                .iter()
                .enumerate()
                .map(|(index, child)| child.style.flex_shrink * main_sizes[index])
                .sum();
            if total_shrink > 0.0 {
                for (index, child) in children.iter().enumerate() {
                    let share = child.style.flex_shrink * main_sizes[index] / total_shrink;
                    final_main[index] = (final_main[index] + free * share).max(0.0);
                }
            }
        }

        // 主轴起点位置。
        let used: f64 = final_main.iter().sum::<f64>() + gaps;
        let leftover = (content_width - used).max(0.0);
        let (mut cursor, spacing) = match style.justify_content {
            JustifyContent::FlexStart => (content_left, 0.0),
            JustifyContent::FlexEnd => (content_left + leftover, 0.0),
            JustifyContent::Center => (content_left + leftover / 2.0, 0.0),
            JustifyContent::SpaceBetween => {
                let extra = if children.len() > 1 {
                    leftover / (children.len() - 1) as f64
                } else {
                    0.0
                };
                (content_left, extra)
            }
            JustifyContent::SpaceAround => {
                let extra = leftover / children.len() as f64;
                (content_left + extra / 2.0, extra)
            }
            JustifyContent::SpaceEvenly => {
                let extra = leftover / (children.len() + 1) as f64;
                (content_left + extra, extra)
            }
        };

        // 先按主轴尺寸布局，记下每项的外框高度以便算容器高度。
        let mut laid_out = Vec::with_capacity(children.len());
        for (index, mut child) in children.into_iter().enumerate() {
            let (margin, border, padding) = self.resolve_edges(&child, content_width);
            let outer = final_main[index];
            let inner =
                (outer - margin.horizontal() - border.horizontal() - padding.horizontal()).max(0.0);
            child.forced_width = Some(inner);
            self.layout_box(
                &mut child,
                content_width,
                containing_height,
                cursor,
                content_top,
            );
            cursor += outer + spacing + gap;
            laid_out.push(child);
        }

        // 容器高度取各行的最大高度。
        let mut content_height = 0.0f64;
        for child in &laid_out {
            let outer_height = child.margin.vertical() + child.rect.height;
            content_height = content_height.max(outer_height);
        }

        // 侧轴对齐。
        for child in &mut laid_out {
            let align = child.style.align_self.unwrap_or(style.align_items);
            let outer_height = child.margin.vertical() + child.rect.height;
            let offset = match align {
                AlignItems::Stretch => {
                    // 拉伸：把内容高度补到容器高度。
                    let target = content_height - child.margin.vertical();
                    if target > child.rect.height {
                        let extra = target - child.rect.height;
                        child.rect.height += extra;
                        child.content.height += extra;
                    }
                    0.0
                }
                AlignItems::FlexStart | AlignItems::Baseline => 0.0,
                AlignItems::FlexEnd => content_height - outer_height,
                AlignItems::Center => (content_height - outer_height) / 2.0,
            };
            if offset != 0.0 {
                child.translate_to(child.rect.x, content_top + offset + child.margin.top);
            }
        }

        container.children = laid_out;
        content_height
    }

    /// 主轴为竖直方向的弹性布局。
    fn layout_flex_column(
        &mut self,
        children: Vec<LayoutBox>,
        container: &mut LayoutBox,
        content_width: f64,
        containing_height: Option<f64>,
        content_left: f64,
        content_top: f64,
    ) -> f64 {
        let style = container.style.clone();
        let gap = self.resolve_length(style.row_gap, content_width, style.font_size_pixels());

        // 列方向下宽度先确定，高度按内容算，再按剩余空间分配。
        let mut laid_out = Vec::with_capacity(children.len());
        let mut total_height = 0.0f64;
        for mut child in children {
            let align = child.style.align_self.unwrap_or(style.align_items);
            let font_size = child.style.font_size_pixels();
            let child_width = match self.resolve_size(child.style.width, content_width, font_size) {
                Some(width) => width,
                None => content_width,
            };
            let (_, _, _) = self.resolve_edges(&child, content_width);
            let content_x = match align {
                AlignItems::Center => content_left + (content_width - child_width) / 2.0,
                AlignItems::FlexEnd => content_left + content_width - child_width,
                _ => content_left,
            };
            child.forced_width = Some(child_width);
            self.layout_box(
                &mut child,
                content_width,
                containing_height,
                content_x,
                content_top + total_height,
            );
            total_height += child.margin.vertical() + child.rect.height + gap;
            laid_out.push(child);
        }
        let content_height = (total_height - gap).max(0.0);

        // 主轴对齐：在容器有确定高度时才有剩余空间可分。
        let definite_height = self.resolve_size(
            style.height,
            containing_height.unwrap_or(0.0),
            style.font_size_pixels(),
        );
        if let Some(container_height) = definite_height {
            let leftover = (container_height - content_height).max(0.0);
            let offset = match style.justify_content {
                JustifyContent::FlexStart | JustifyContent::SpaceBetween => 0.0,
                JustifyContent::FlexEnd => leftover,
                JustifyContent::Center => leftover / 2.0,
                JustifyContent::SpaceAround | JustifyContent::SpaceEvenly => leftover / 2.0,
            };
            if offset > 0.0 {
                for child in &mut laid_out {
                    child.translate_to(child.rect.x, child.rect.y + offset);
                }
            }
        }

        container.children = laid_out;
        content_height
    }
}

/// 行内流里的一项。
enum InlineItem {
    /// 一段文本。
    Text(Box<TextRun>),
    /// 一个原子盒，整体参与行内排版。
    Atom(Box<LayoutBox>),
}

/// 一段连续的同体式文本。
struct TextRun {
    /// 文本内容。
    text: String,
    /// 所属元素的样式，决定字体与颜色。
    style: ComputedStyle,
    /// 这段文字来自哪个节点。
    ///
    /// 命中测试要用：行内元素在盒子树里没有自己的盒子，文字被折进行里，
    /// 只靠盒子找不出「这一点压在哪个链接上」，得顺着文字反查它属于谁。
    node: Option<NodeId>,
}

/// 排版中尚未确定纵坐标的一个片段。
struct PendingFragment {
    /// 左边界。
    x: f64,
    /// 宽度。
    width: f64,
    /// 高度。
    height: f64,
    /// 基线相对片段顶部的偏移。
    baseline_offset: f64,
    /// 内容。
    kind: FragmentContent,
}

/// 行内排版过程中累积的一行。
struct LineBuilder {
    /// 起始横坐标。
    left: f64,
    /// 已用宽度。
    width: f64,
    /// 行内最高的片段。
    max_height: f64,
    /// 待定位的片段。
    fragments: Vec<PendingFragment>,
}

impl LineBuilder {
    /// 建一个空行。
    fn new(left: f64) -> Self {
        Self {
            left,
            width: 0.0,
            max_height: 0.0,
            fragments: Vec::new(),
        }
    }

    /// 行里是否还没有内容。
    fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    /// 追加一个文本片段。
    fn push_text(
        &mut self,
        text: String,
        style: &ComputedStyle,
        width: f64,
        height: f64,
        node: Option<NodeId>,
    ) {
        // 文本基线按字体行高的八成位置估算，与常见字体的 ascent 比例接近。
        let baseline_offset = height * 0.8;
        self.fragments.push(PendingFragment {
            x: self.left + self.width,
            width,
            height,
            baseline_offset,
            kind: FragmentContent::Text(Box::new(TextFragment {
                text,
                color: style.color,
                font: text_style_of(style),
                underline: style.underline,
                preserve_whitespace: style.white_space.preserves_spaces(),
                node,
            })),
        });
        self.width += width;
        self.max_height = self.max_height.max(height);
    }

    /// 追加一个原子盒。
    fn push_atom(&mut self, x: f64, width: f64, height: f64, child_index: usize) {
        self.fragments.push(PendingFragment {
            x,
            // 原子的宽度要计进去，行游标才会往右走。这里曾经写死零宽，
            // 于是同一行上的多个行内块全叠在同一个位置上。
            width,
            height,
            baseline_offset: height,
            kind: FragmentContent::Atom { child_index },
        });
        self.width += width;
        self.max_height = self.max_height.max(height);
    }

    /// 结算这一行：定高度，按对齐方式横向摆放，产出最终的行盒。
    fn finish(self, top: f64, available: f64, align: TextAlign) -> LineBox {
        // 整行只有可折叠的空白时不占高度，也不产出片段。元素之间的换行与
        // 缩进就是这种情况，它们不该把页面撑开。
        let blank = self.fragments.iter().all(|fragment| match &fragment.kind {
            FragmentContent::Text(text) => !text.preserve_whitespace && text.text.trim().is_empty(),
            FragmentContent::Atom { .. } => false,
        });
        if blank {
            return LineBox {
                fragments: Vec::new(),
                top,
                height: 0.0,
            };
        }

        let height = self.max_height.max(0.0);
        let free = (available - self.width).max(0.0);
        let offset = match align {
            TextAlign::Left | TextAlign::Justify => 0.0,
            TextAlign::Right => free,
            TextAlign::Center => free / 2.0,
        };

        let fragments = self
            .fragments
            .into_iter()
            .map(|fragment| LineFragment {
                rect: Rect::new(fragment.x + offset, top, fragment.width, fragment.height),
                baseline: top + fragment.baseline_offset,
                kind: fragment.kind,
            })
            .collect();

        LineBox {
            fragments,
            top,
            height,
        }
    }
}

/// 定出画布底色，以及它是谁贡献的。
///
/// CSS 规定根元素的背景要传播到画布、铺满整个视口；根元素没有背景时轮到
/// body 顶上，而贡献出去的那一层自己就不再画背景盒。少了这一步，页面内容
/// 比视口短的时候，body 的底色只铺到内容结束，下面露出一截白色。
fn canvas_background(document: &Document, styles: &StyleMap) -> (Color, Option<NodeId>) {
    let mut html = None;
    let mut body = None;
    for node in document.descendants(document.root()) {
        let Some(element) = document.element(node) else {
            continue;
        };
        if html.is_none() && element.is_html("html") {
            html = Some(node);
        } else if body.is_none() && element.is_html("body") {
            body = Some(node);
        }
    }

    for candidate in [html, body].into_iter().flatten() {
        if let Some(style) = styles.get(&candidate)
            && !style.background_color.is_transparent()
        {
            return (style.background_color, Some(candidate));
        }
    }
    (Color::WHITE, None)
}

/// 把表格的子盒整理成行，返回每一行在 `children` 里的下标路径。
///
/// HTML 的树构建会替我们补上 `tbody`，所以行一般裹在行组里；手写 DOM 时
/// 也可能直接挂在表格下，两种情况都要认。返回路径而不是引用，是为了让
/// 调用方还能同时用 `&mut self` 去布局单元格。
fn row_paths(children: &[LayoutBox]) -> Vec<Vec<usize>> {
    let mut paths = Vec::new();
    for (index, child) in children.iter().enumerate() {
        match child.kind {
            BoxKind::TableRow => paths.push(vec![index]),
            BoxKind::TableRowGroup => {
                for (inner, grandchild) in child.children.iter().enumerate() {
                    if grandchild.kind == BoxKind::TableRow {
                        paths.push(vec![index, inner]);
                    }
                }
            }
            _ => {}
        }
    }
    paths
}

/// 按下标路径取一行。
fn row_at<'a>(children: &'a [LayoutBox], path: &[usize]) -> &'a LayoutBox {
    match path {
        [index] => &children[*index],
        [outer, inner] => &children[*outer].children[*inner],
        _ => unreachable!("行的路径最多两级"),
    }
}

/// 按下标路径取一行，可变版本。
fn row_at_mut<'a>(children: &'a mut [LayoutBox], path: &[usize]) -> &'a mut LayoutBox {
    match path {
        [index] => &mut children[*index],
        [outer, inner] => &mut children[*outer].children[*inner],
        _ => unreachable!("行的路径最多两级"),
    }
}

/// 取一行里的单元格下标。
fn row_cells(row: &LayoutBox) -> Vec<usize> {
    row.children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.kind == BoxKind::TableCell)
        .map(|(index, _)| index)
        .collect()
}

/// 把计算样式翻译成文字排版体式。
fn text_style_of(style: &ComputedStyle) -> TextStyle {
    TextStyle {
        families: style.font_family.clone(),
        font_size: style.font_size_pixels() as f32,
        weight: style.font_weight,
        italic: style.italic,
        line_height: style.line_height,
        wrap: style.white_space.wraps(),
    }
}

impl LayoutBox {
    /// 把一个盒子连同子树平移到新位置。
    ///
    /// 行列盒在排版时用的是临时坐标，排完之后统一平移到位。
    pub fn translate_to(&mut self, x: f64, y: f64) {
        let dx = x - self.rect.x;
        let dy = y - self.rect.y;
        self.translate(dx, dy);
    }

    /// 按增量平移自身与全部后代。
    ///
    /// 行盒里的片段位置也要跟着挪。只挪 `rect` 是不够的：文字与行内盒的
    /// 位置存在 `lines` 里，漏了它们，文字就会留在排版时的临时坐标上——
    /// 带内边距的行内元素（比如 `<code>`）被搬到行上之后，框过去了、
    /// 里面的字还留在页面顶端，正是这个原因。
    pub fn translate(&mut self, dx: f64, dy: f64) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.rect = self.rect.translate(dx, dy);
        self.content = self.content.translate(dx, dy);
        self.clip = self.clip.translate(dx, dy);
        for line in &mut self.lines {
            line.top += dy;
            for fragment in &mut line.fragments {
                fragment.rect = fragment.rect.translate(dx, dy);
                fragment.baseline += dy;
            }
        }
        for child in &mut self.children {
            child.translate(dx, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;
    use crate::html::parse_document;
    use crate::style::compute_styles;

    /// 解析、算样式并布局。
    fn layout(html: &str, css: &str, width: f64, height: f64) -> LayoutTree {
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
        engine.layout(&document, &styles).expect("应当能布局出来")
    }

    /// 按 id 找到对应的盒子。
    fn box_of_id<'a>(tree: &'a LayoutTree, document: &Document, id: &str) -> &'a LayoutBox {
        let node = document
            .find_element(document.root(), |element| element.id() == Some(id))
            .unwrap_or_else(|| panic!("找不到 {id}"));
        tree.descendants()
            .into_iter()
            .find(|layout_box| layout_box.node == Some(node))
            .unwrap_or_else(|| panic!("{id} 没有对应的盒子"))
    }

    /// 只解析文档，供需要同时拿文档与盒子树的测试使用。
    fn document_of(html: &str) -> Document {
        parse_document(html)
    }

    #[test]
    fn block_fills_available_width() {
        let tree = layout("<div id=d>x</div>", "", 800.0, 600.0);
        let document = document_of("<div id=d>x</div>");
        let div = box_of_id(&tree, &document, "d");
        // body 有 8 像素外边距，div 又有 body 的内边距为零，所以宽度是 800-16。
        assert!(
            (div.rect.width - 784.0).abs() < 0.5,
            "实际宽度 {}",
            div.rect.width
        );
    }

    #[test]
    fn explicit_width_is_respected() {
        let tree = layout("<div id=d>x</div>", "#d { width: 200px }", 800.0, 600.0);
        let document = document_of("<div id=d>x</div>");
        assert!((box_of_id(&tree, &document, "d").rect.width - 200.0).abs() < 0.5);
    }

    #[test]
    fn percentage_width_resolves_against_parent() {
        let tree = layout(
            "<div id=p><div id=c>x</div></div>",
            "#c { width: 50% }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=p><div id=c>x</div></div>");
        let parent = box_of_id(&tree, &document, "p").rect.width;
        let child = box_of_id(&tree, &document, "c").rect.width;
        assert!((child - parent / 2.0).abs() < 0.5, "父 {parent} 子 {child}");
    }

    #[test]
    fn auto_margins_center_block() {
        let tree = layout(
            "<div id=d>x</div>",
            "#d { width: 100px; margin: 0 auto }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=d>x</div>");
        let div = box_of_id(&tree, &document, "d");
        // 左右外边距应当基本相等。
        assert!(
            (div.margin.left - div.margin.right).abs() < 0.5,
            "左 {} 右 {}",
            div.margin.left,
            div.margin.right
        );
        assert!(div.margin.left > 0.0);
    }

    #[test]
    fn siblings_stack_vertically() {
        let tree = layout(
            "<div id=a>a</div><div id=b>b</div>",
            "#a, #b { height: 50px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=a>a</div><div id=b>b</div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        assert!(b.rect.y >= a.rect.bottom() - 0.5, "b 应当在 a 下面");
    }

    #[test]
    fn adjacent_margins_collapse() {
        // 上下各 20 像素的外边距合并后只留 20，不是 40。
        let tree = layout(
            "<div id=a>a</div><div id=b>b</div>",
            "#a, #b { height: 20px } #a { margin-bottom: 20px } #b { margin-top: 20px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=a>a</div><div id=b>b</div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        let gap = b.rect.y - a.rect.bottom();
        assert!((gap - 20.0).abs() < 0.5, "间距应当是 20，实际 {gap}");
    }

    #[test]
    fn larger_margin_wins_when_collapsing() {
        let tree = layout(
            "<div id=a>a</div><div id=b>b</div>",
            "#a, #b { height: 20px } #a { margin-bottom: 30px } #b { margin-top: 10px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=a>a</div><div id=b>b</div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        assert!(((b.rect.y - a.rect.bottom()) - 30.0).abs() < 0.5);
    }

    #[test]
    fn text_creates_line_boxes() {
        let tree = layout("<p id=p>hello world</p>", "", 800.0, 600.0);
        let document = document_of("<p id=p>hello world</p>");
        let p = box_of_id(&tree, &document, "p");
        assert!(p.content.height > 0.0, "文本应当撑起高度");
        // 文本排版的结果落在行盒里，不是子盒子。
        assert!(!p.lines.is_empty(), "应当有行盒");
        assert!(
            p.lines
                .iter()
                .flat_map(|line| &line.fragments)
                .any(|fragment| matches!(fragment.kind, FragmentContent::Text(_))),
            "行盒里应当有文本片段"
        );
    }

    #[test]
    fn long_text_wraps_and_grows_height() {
        let text = "word ".repeat(200);
        let narrow = layout(&format!("<p id=p>{text}</p>"), "", 300.0, 600.0);
        let wide = layout(&format!("<p id=p>{text}</p>"), "", 2000.0, 600.0);
        let document = document_of(&format!("<p id=p>{text}</p>"));
        let narrow_p = box_of_id(&narrow, &document, "p");
        let wide_p = box_of_id(&wide, &document, "p");
        assert!(
            narrow_p.content.height > wide_p.content.height,
            "窄容器里的段落应当更高"
        );
    }

    #[test]
    fn padding_and_border_add_to_size() {
        let tree = layout(
            "<div id=d>x</div>",
            "#d { width: 100px; padding: 10px; border: 5px solid black }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=d>x</div>");
        let div = box_of_id(&tree, &document, "d");
        // 内容 100 + 内边距 20 + 边框 10。
        assert!(
            (div.rect.width - 130.0).abs() < 0.5,
            "实际 {}",
            div.rect.width
        );
    }

    #[test]
    fn margin_offsets_position() {
        let tree = layout(
            "<div id=d>x</div>",
            "#d { margin-left: 30px; margin-top: 40px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=d>x</div>");
        let div = box_of_id(&tree, &document, "d");
        // body 自身有 8 像素外边距，div 的上外边距会与 body 合并。
        assert!(div.rect.x >= 30.0);
        assert!(div.rect.y >= 8.0);
    }

    #[test]
    fn flex_row_lays_out_horizontally() {
        let tree = layout(
            "<div id=c><span id=a>a</span><span id=b>b</span></div>",
            "#c { display: flex } #a, #b { width: 100px; height: 30px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=c><span id=a>a</span><span id=b>b</span></div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        assert!(b.rect.x > a.rect.x, "第二项应当在右边");
        assert!((a.rect.y - b.rect.y).abs() < 0.5, "同一行应当对齐");
    }

    #[test]
    fn flex_grow_distributes_free_space() {
        let tree = layout(
            "<div id=c><span id=a>a</span><span id=b>b</span></div>",
            "#c { display: flex; width: 400px } #a, #b { flex-grow: 1; height: 20px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=c><span id=a>a</span><span id=b>b</span></div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        assert!(
            (a.rect.width - b.rect.width).abs() < 1.0,
            "两项应当等宽，实际 {} 与 {}",
            a.rect.width,
            b.rect.width
        );
        assert!(a.rect.width > 150.0, "伸展后应当明显变宽");
    }

    #[test]
    fn flex_justify_content_center() {
        let tree = layout(
            "<div id=c><span id=a>a</span></div>",
            "#c { display: flex; width: 400px; justify-content: center } #a { width: 100px; height: 20px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=c><span id=a>a</span></div>");
        let container = box_of_id(&tree, &document, "c");
        let item = box_of_id(&tree, &document, "a");
        let expected = container.content.x + (400.0 - 100.0) / 2.0;
        assert!(
            (item.rect.x - expected).abs() < 1.0,
            "居中位置应当在 {expected}，实际 {}",
            item.rect.x
        );
    }

    #[test]
    fn inline_blocks_advance_the_line() {
        // 同一行上的多个行内块必须横向排开。这条是回归测试：原子的宽度
        // 一度被写死成零，行游标不前进，几个块全叠在同一个位置上。
        let html = "<div id=a>甲</div><div id=b>乙</div><div id=c>丙</div>";
        let tree = layout(
            html,
            "div { display: inline-block; width: 50px; height: 20px }",
            800.0,
            600.0,
        );
        let document = document_of(html);
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        let c = box_of_id(&tree, &document, "c");
        assert!(b.rect.x > a.rect.x + 40.0, "第二个应当在第一个右边");
        assert!(c.rect.x > b.rect.x + 40.0, "第三个应当在第二个右边");
        // 同一行，纵坐标一样。
        assert!((a.rect.y - b.rect.y).abs() < 0.5, "应当在同一行上");
    }

    #[test]
    fn border_radius_resolves_against_the_shorter_side() {
        // 50% 在正方形上正好是圆，半径是边长的一半。
        let tree = layout(
            "<div id=d></div>",
            "#d { width: 60px; height: 60px; background: #2b6cb0; border-radius: 50% }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=d></div>");
        let div = box_of_id(&tree, &document, "d");
        let list = crate::paint::paint_tree(&tree);
        let radius = list
            .commands
            .iter()
            .find_map(|command| match command {
                crate::paint::DrawCommand::FillRect { corners, .. } if corners.top_left > 0.0 => {
                    Some(corners.top_left)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("应当画出圆角，盒子 {:?}", div.rect));
        assert!(
            (radius - 30.0).abs() < 0.5,
            "50% 应当是边长的一半，实际 {radius}"
        );
    }

    #[test]
    fn no_radius_means_square_corners() {
        let tree = layout(
            "<div id=d></div>",
            "#d { width: 60px; height: 60px; background: #2b6cb0 }",
            800.0,
            600.0,
        );
        let list = crate::paint::paint_tree(&tree);
        assert!(
            !list.commands.iter().any(|command| matches!(
                command,
                crate::paint::DrawCommand::FillRect { corners, .. } if corners.top_left > 0.0
            )),
            "没写圆角就应当是直角"
        );
    }

    #[test]
    fn table_cells_line_up_in_columns() {
        // 表格的单元格必须排成行列，不能像早先那样全叠在 x=0 上。
        let html = "<table id=t><tbody><tr><td id=a>A</td><td id=b>B</td></tr>\
                    <tr><td id=c>C</td><td id=d>D</td></tr></tbody></table>";
        let tree = layout(html, "", 800.0, 600.0);
        let document = document_of(html);
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        let c = box_of_id(&tree, &document, "c");
        let d = box_of_id(&tree, &document, "d");

        // 同一行的两个单元格横向并排，右边那个确实在右边。
        assert!(b.rect.x > a.rect.x + 1.0, "同一行应当横向排开");
        assert!(d.rect.x > c.rect.x + 1.0, "第二行同样");
        // 第二行在第一行下面。
        assert!(c.rect.y > a.rect.y + 1.0, "第二行应当在第一行下面");
        // 列对齐：上下两个单元格的左边在同一条竖线上。
        assert!(
            (a.rect.x - c.rect.x).abs() < 0.5,
            "同一列应当对齐，实际 {} 对 {}",
            a.rect.x,
            c.rect.x
        );
        assert!(
            (b.rect.x - d.rect.x).abs() < 0.5,
            "第二列也应当对齐，实际 {} 对 {}",
            b.rect.x,
            d.rect.x
        );
    }

    #[test]
    fn table_shrinks_to_its_content() {
        // 没有指定宽度时表格收到内容宽度，不该撑满容器。
        let html = "<table id=t><tbody><tr><td id=a>短</td></tr></tbody></table>";
        let tree = layout(html, "", 800.0, 600.0);
        let document = document_of(html);
        let table = box_of_id(&tree, &document, "t");
        assert!(
            table.rect.width < 400.0,
            "表格应当收到内容宽度，实际 {}",
            table.rect.width
        );
        assert!(table.rect.width > 0.0);
    }

    #[test]
    fn table_with_no_cells_does_not_blow_up() {
        let tree = layout("<table id=t></table>", "", 800.0, 600.0);
        let document = document_of("<table id=t></table>");
        let table = box_of_id(&tree, &document, "t");
        assert!(table.rect.height >= 0.0);
    }

    #[test]
    fn block_with_width_and_no_auto_margins_stays_left() {
        // 带明确宽度、外边距不是 auto 的块应当靠左。居中只属于
        // `margin: 0 auto` 那种写法。
        let tree = layout("<div id=d></div>", "#d { width: 200px }", 800.0, 600.0);
        let document = document_of("<div id=d></div>");
        let div = box_of_id(&tree, &document, "d");
        // 靠左是靠到包含块的左边，而 body 自己还有 8 像素的默认外边距。
        assert!(
            (div.rect.x - 8.0).abs() < 0.5,
            "没有 auto 外边距时块该靠左（body 左边距 8），实际 x={}",
            div.rect.x
        );
    }

    #[test]
    fn auto_margins_still_center() {
        // 上面那条改了之后，这个行为不能丢。
        let tree = layout(
            "<div id=d></div>",
            "#d { width: 200px; margin: 0 auto }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=d></div>");
        let div = box_of_id(&tree, &document, "d");
        assert!(
            (div.rect.x - 300.0).abs() < 0.5,
            "auto 外边距应当居中，实际 x={}",
            div.rect.x
        );
    }

    #[test]
    fn flex_column_stacks_vertically() {
        let tree = layout(
            "<div id=c><span id=a>a</span><span id=b>b</span></div>",
            "#c { display: flex; flex-direction: column } #a, #b { height: 30px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=c><span id=a>a</span><span id=b>b</span></div>");
        let a = box_of_id(&tree, &document, "a");
        let b = box_of_id(&tree, &document, "b");
        assert!(b.rect.y > a.rect.y, "列方向应当纵向堆叠");
    }

    #[test]
    fn inline_block_sits_on_line() {
        let tree = layout(
            "<p id=p>text <span id=s>box</span></p>",
            "#s { display: inline-block; width: 50px; height: 20px }",
            800.0,
            600.0,
        );
        let document = document_of("<p id=p>text <span id=s>box</span></p>");
        let span = box_of_id(&tree, &document, "s");
        assert!((span.rect.width - 50.0).abs() < 0.5);
        assert!(span.rect.x > 0.0, "应当排在文字后面");
    }

    #[test]
    fn inline_box_with_padding_keeps_its_text_in_place() {
        // 带内边距与背景的行内元素（比如 <code>）里，文字必须落在框里。
        // 这条是回归测试：曾经文字会跑到页面顶端，框留在原地是空的。
        let html = "<p id=p>前面 <code id=c>代码</code> 后面</p>";
        let tree = layout(
            html,
            "#c { background: #eee; padding: 2px 6px }",
            800.0,
            600.0,
        );
        let document = document_of(html);
        let paragraph = box_of_id(&tree, &document, "p");
        let code = box_of_id(&tree, &document, "c");

        assert!(
            code.rect.y >= paragraph.rect.y - 0.5,
            "行内盒应当落在段落里，实际 y={}，段落 y={}",
            code.rect.y,
            paragraph.rect.y
        );
        assert!(
            code.rect.y < paragraph.rect.y + 100.0,
            "行内盒不该跑出段落，实际 y={}",
            code.rect.y
        );

        // 框里要有文字片段，而且片段落在框的竖直范围内。
        let fragments: Vec<_> = code
            .descendants()
            .into_iter()
            .flat_map(|child| child.lines.iter())
            .flat_map(|line| line.fragments.iter())
            .collect();
        assert!(!fragments.is_empty(), "行内盒里应当有文字片段");
        for fragment in &fragments {
            let top = code.rect.y - 0.5;
            let bottom = code.rect.bottom() + 0.5;
            assert!(
                fragment.rect.y >= top && fragment.rect.y <= bottom,
                "文字片段跑到框外了：片段 y={}，框 {}..{}",
                fragment.rect.y,
                top,
                bottom
            );
        }
    }

    #[test]
    fn nested_blocks_position_correctly() {
        let tree = layout(
            "<div id=outer><div id=inner>x</div></div>",
            "#outer { padding: 20px } #inner { height: 40px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=outer><div id=inner>x</div></div>");
        let outer = box_of_id(&tree, &document, "outer");
        let inner = box_of_id(&tree, &document, "inner");
        assert!(inner.rect.x >= outer.rect.x + 20.0 - 0.5);
        assert!(inner.rect.y >= outer.rect.y + 20.0 - 0.5);
    }

    #[test]
    fn document_height_covers_content() {
        let tree = layout("<div style='height: 2000px'>tall</div>", "", 800.0, 600.0);
        assert!(tree.document_height() >= 2000.0);
    }

    #[test]
    fn root_fills_viewport_height() {
        let tree = layout("<p>x</p>", "", 800.0, 600.0);
        assert!(tree.root.rect.height >= 600.0 - 0.5);
    }

    #[test]
    fn hit_test_finds_deepest_box() {
        let tree = layout(
            "<div id=outer><p id=inner>text</p></div>",
            "#outer { padding: 50px } #inner { height: 40px }",
            800.0,
            600.0,
        );
        let document = document_of("<div id=outer><p id=inner>text</p></div>");
        let inner = box_of_id(&tree, &document, "inner");
        let hit = tree
            .hit_test(inner.rect.x + 1.0, inner.rect.y + 1.0)
            .expect("应当命中最深的盒子");
        assert_eq!(hit.node, inner.node);
    }

    #[test]
    fn empty_document_layouts() {
        let tree = layout("", "", 800.0, 600.0);
        assert!(tree.root.rect.height >= 600.0 - 0.5);
    }

    #[test]
    fn list_item_indent() {
        let tree = layout("<ul><li id=item>x</li></ul>", "", 800.0, 600.0);
        let document = document_of("<ul><li id=item>x</li></ul>");
        let item = box_of_id(&tree, &document, "item");
        // 默认样式里 ul 有 40 像素的左内边距。
        assert!(
            item.rect.x >= 40.0,
            "列表项应当有缩进，实际 {}",
            item.rect.x
        );
    }

    #[test]
    fn heading_is_larger_than_paragraph() {
        let tree = layout("<h1 id=h>title</h1><p id=p>body</p>", "", 800.0, 600.0);
        let document = document_of("<h1 id=h>title</h1><p id=p>body</p>");
        let heading = box_of_id(&tree, &document, "h");
        let paragraph = box_of_id(&tree, &document, "p");
        assert!(heading.content.height > paragraph.content.height);
    }
}
