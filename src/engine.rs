//! 内核门面：把解析、样式、布局、绘制串成一条线。
//!
//! 外壳只要拿一段 HTML 和视口尺寸喂进来，就能拿到一份可以直接交给渲染器
//! 的显示列表。中间每一步仍然可以单独调用，方便测试与调试。

use crate::css::{MediaContext, StyleSheet, parse_stylesheet_with_media};
use crate::dom::node::{Document, NodeId};
use crate::html::parse_document;
use crate::layout::engine::{LayoutContext, LayoutEngine, LayoutTree};
use crate::layout::geometry::Rect;
use crate::paint::{DisplayList, paint_tree, paint_tree_region};
use crate::style::{StyleMap, compute_styles_with_viewport};
use std::collections::HashMap;

/// 样式表的来源。
///
/// 文档里 `<style>` 与 `<link rel="stylesheet">` 会按出现顺序共同决定层叠，
/// 所以要把两类来源排在同一条队里，不能把外链统统挪到最后。
#[derive(Debug, Clone, PartialEq, Eq)]
enum SheetSource {
    /// 文档里的 `<style>` 元素，内容是它的文本。
    Inline(String),
    /// 外部样式表，值是绝对地址。
    Link(String),
}

/// 一次加载的完整状态。
pub struct Engine {
    /// 当前文档。
    document: Document,
    /// 样式表来源，按文档顺序。
    sources: Vec<SheetSource>,
    /// 已经取回的外部样式表，地址到内容的映射。
    linked: HashMap<String, String>,
    /// 作者样式表，按文档顺序排列。
    sheets: Vec<StyleSheet>,
    /// 布局引擎。
    layout_engine: LayoutEngine,
    /// 计算样式。
    styles: StyleMap,
    /// 布局结果，布局完成后才有值。
    tree: Option<LayoutTree>,
    /// 视口尺寸。
    viewport: (f64, f64),
    /// 文档地址，用来把相对地址补全。
    base_url: String,
}

impl Engine {
    /// 建一个空的内核，视口尺寸按给定值。
    pub fn new(viewport_width: f64, viewport_height: f64) -> Self {
        let viewport = (viewport_width, viewport_height);
        Self {
            document: Document::new(),
            sources: Vec::new(),
            linked: HashMap::new(),
            sheets: Vec::new(),
            layout_engine: LayoutEngine::new(LayoutContext {
                viewport_width,
                viewport_height,
                root_font_size: 16.0,
            }),
            styles: StyleMap::new(),
            tree: None,
            viewport,
            base_url: String::new(),
        }
    }

    /// 换一个视口尺寸，会按新尺寸重新算样式与布局。
    pub fn set_viewport(&mut self, width: f64, height: f64) {
        if self.viewport == (width, height) {
            return;
        }
        self.viewport = (width, height);
        self.layout_engine.set_context(LayoutContext {
            viewport_width: width,
            viewport_height: height,
            root_font_size: self.root_font_size(),
        });
        self.relayout();
    }

    /// 当前视口尺寸。
    pub fn viewport(&self) -> (f64, f64) {
        self.viewport
    }

    /// 加载一段 HTML，解析文档、收集样式并完成一次布局。
    pub fn load_html(&mut self, html: &str) {
        self.load_html_with_base(html, "");
    }

    /// 加载一段 HTML，并把相对地址按 `base_url` 补全。
    pub fn load_html_with_base(&mut self, html: &str, base_url: &str) {
        self.document = parse_document(html);
        self.base_url = base_url.to_string();
        // 换文档了，上一份文档取回的外部样式表不能再沿用。
        self.linked.clear();
        self.reload_styles();
    }

    /// 重新收集并解析文档里的样式表。
    pub fn reload_styles(&mut self) {
        self.sources = collect_sources(&self.document, self.viewport, &self.base_url);
        self.rebuild_sheets();
    }

    /// 按来源列表重新解析全部样式表并重新布局。
    fn rebuild_sheets(&mut self) {
        let media = MediaContext {
            width: self.viewport.0,
            height: self.viewport.1,
        };
        let mut sheets = Vec::new();
        for source in &self.sources {
            match source {
                SheetSource::Inline(css) => {
                    sheets.push(parse_stylesheet_with_media(css, media));
                }
                SheetSource::Link(url) => {
                    // 还没取回来的外链先跳过，取回之后会重新走一遍。
                    if let Some(css) = self.linked.get(url) {
                        sheets.push(parse_stylesheet_with_media(css, media));
                    }
                }
            }
        }
        self.sheets = sheets;
        self.relayout();
    }

    /// 文档里引用的外部样式表地址，已经补成绝对地址。
    ///
    /// 外壳拿到之后去取内容，取回一个就调一次
    /// [`Engine::set_linked_stylesheet`]。
    pub fn stylesheet_links(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|source| match source {
                SheetSource::Link(url) => Some(url.clone()),
                SheetSource::Inline(_) => None,
            })
            .collect()
    }

    /// 收下一份外部样式表的内容并重新布局。
    ///
    /// 地址不在文档引用列表里时忽略。
    pub fn set_linked_stylesheet(&mut self, url: &str, css: &str) {
        if !self
            .sources
            .iter()
            .any(|source| matches!(source, SheetSource::Link(linked) if linked == url))
        {
            return;
        }
        self.linked.insert(url.to_string(), css.to_string());
        self.rebuild_sheets();
    }

    /// 按当前的文档与样式重新算一遍样式和布局。
    pub fn relayout(&mut self) {
        let media = MediaContext {
            width: self.viewport.0,
            height: self.viewport.1,
        };
        self.styles = compute_styles_with_viewport(&self.document, &self.sheets, media);
        self.tree = self.layout_engine.layout(&self.document, &self.styles);
    }

    /// 当前文档。
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// 当前作者样式表。
    pub fn stylesheets(&self) -> &[StyleSheet] {
        &self.sheets
    }

    /// 当前计算样式表。
    pub fn styles(&self) -> &StyleMap {
        &self.styles
    }

    /// 当前布局树，还没布局时返回 `None`。
    pub fn layout_tree(&self) -> Option<&LayoutTree> {
        self.tree.as_ref()
    }

    /// 布局引擎，绘制阶段要拿它取字体。
    pub fn layout_engine_mut(&mut self) -> &mut LayoutEngine {
        &mut self.layout_engine
    }

    /// 整份文档的高度，用于滚动范围。
    pub fn document_height(&self) -> f64 {
        self.tree
            .as_ref()
            .map_or(self.viewport.1, LayoutTree::document_height)
    }

    /// 产出显示列表。
    pub fn display_list(&self) -> DisplayList {
        match &self.tree {
            Some(tree) => paint_tree(tree),
            None => DisplayList::new(self.viewport.0, self.viewport.1),
        }
    }

    /// 只绘制指定范围内的内容，滚动时用。
    pub fn display_list_region(&self, visible: Rect) -> DisplayList {
        match &self.tree {
            Some(tree) => paint_tree_region(tree, visible),
            None => DisplayList::new(self.viewport.0, self.viewport.1),
        }
    }

    /// 按坐标找出对应的元素，用于命中测试。
    pub fn element_at(&self, x: f64, y: f64) -> Option<NodeId> {
        let tree = self.tree.as_ref()?;
        let layout_box = tree.hit_test(x, y)?;
        layout_box.node
    }

    /// 按坐标找出压在该点上的文字片段来自哪个节点。
    ///
    /// 与 [`Engine::element_at`] 的区别在于行内内容：行内元素没有自己的
    /// 盒子，`element_at` 只会给出所在的块，而这里能给出真正的那段文字
    /// 属于谁——判断点在不在链接上要靠它。
    pub fn fragment_node_at(&self, x: f64, y: f64) -> Option<NodeId> {
        self.tree.as_ref()?.fragment_node_at(x, y)
    }

    /// 根元素的字号，`rem` 参照它。
    fn root_font_size(&self) -> f64 {
        let Some(root_element) = self
            .document
            .children(self.document.root())
            .iter()
            .copied()
            .find(|id| self.document.node(*id).as_element().is_some())
        else {
            return 16.0;
        };
        self.styles
            .get(&root_element)
            .map_or(16.0, crate::style::ComputedStyle::font_size_pixels)
    }
}

/// 按文档顺序收集样式表来源。
///
/// `<style>` 元素与 `<link rel="stylesheet">` 混在同一条队里，顺序就是层叠
/// 顺序。带 `media` 属性且不匹配当前视口的整条跳过。
fn collect_sources(document: &Document, viewport: (f64, f64), base_url: &str) -> Vec<SheetSource> {
    let media = MediaContext {
        width: viewport.0,
        height: viewport.1,
    };
    // 文档地址本身不合法时，相对地址就没法补全了。
    let base = url::Url::parse(base_url).ok();
    let mut sources = Vec::new();
    for node in document.descendants(document.root()) {
        let Some(element) = document.element(node) else {
            continue;
        };
        // `media` 属性写的是媒体查询，不匹配就整条不用。
        if let Some(media_attribute) = element.get_attribute("media")
            && !media_matches(media_attribute, media)
        {
            continue;
        }

        if element.is_html("style") {
            let css = document.text_content(node);
            if !css.trim().is_empty() {
                sources.push(SheetSource::Inline(css));
            }
            continue;
        }

        if element.is_html("link") {
            let rel = element.get_attribute("rel").unwrap_or("");
            if !rel
                .split_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
            {
                continue;
            }
            let Some(href) = element.get_attribute("href") else {
                continue;
            };
            if href.trim().is_empty() {
                continue;
            }
            // 相对地址按文档地址补全，补不出来就原样留着，取的时候会报错。
            let absolute = base
                .as_ref()
                .and_then(|base| base.join(href).ok())
                .map_or_else(|| href.to_string(), |url| url.to_string());
            sources.push(SheetSource::Link(absolute));
        }
    }
    sources
}

/// 判断 `media` 属性写的查询在当前视口下是否成立。
///
/// 属性里写的是没有外层括号的媒体查询列表，套一层 `@media` 让解析器处理。
fn media_matches(attribute: &str, media: MediaContext) -> bool {
    let trimmed = attribute.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("all")
        || trimmed.eq_ignore_ascii_case("screen")
    {
        return true;
    }
    let css = format!("@media {trimmed} {{ * {{ color: red }} }}");
    let probe = parse_stylesheet_with_media(&css, media);
    !probe.rules.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个内核并加载页面。
    fn engine(html: &str) -> Engine {
        let mut engine = Engine::new(800.0, 600.0);
        engine.load_html(html);
        engine
    }

    #[test]
    fn style_element_is_collected() {
        let engine = engine("<style>p { color: red }</style><p>x</p>");
        assert_eq!(engine.stylesheets().len(), 1);
        assert_eq!(engine.stylesheets()[0].rules.len(), 1);
    }

    #[test]
    fn multiple_style_elements_keep_document_order() {
        let engine =
            engine("<style>p { color: red }</style><style>p { color: blue }</style><p>x</p>");
        assert_eq!(engine.stylesheets().len(), 2);
    }

    #[test]
    fn author_style_overrides_user_agent() {
        let engine = engine("<style>body { margin: 0 }</style><body><p>x</p></body>");
        let root = engine.document().root();
        let body = engine
            .document()
            .find_element(root, |element| element.name == "body")
            .expect("应当有 body");
        assert_eq!(engine.styles()[&body].margin.top, crate::css::Length::ZERO);
    }

    #[test]
    fn background_colour_reaches_display_list() {
        let engine = engine("<style>div { background: red; height: 10px }</style><div>x</div>");
        let list = engine.display_list();
        assert!(
            list.commands.iter().any(|command| matches!(
                command,
                crate::paint::DrawCommand::FillRect { color, .. }
                    if *color == crate::css::Color::rgba(255, 0, 0, 255)
            )),
            "作者样式里的背景色应当出现在绘制命令里"
        );
    }

    /// 数一数某个颜色在显示列表里铺了多大面积。
    fn area_of(list: &DisplayList, colour: crate::css::Color) -> f64 {
        list.commands
            .iter()
            .filter_map(|command| match command {
                crate::paint::DrawCommand::FillRect { rect, color, .. } if *color == colour => {
                    Some(rect.width * rect.height)
                }
                _ => None,
            })
            .sum()
    }

    #[test]
    fn body_background_fills_the_whole_viewport() {
        // body 的背景要传播到画布。页面内容比视口短，少了这一步底下会露白。
        let cream = crate::css::Color::rgba(0xfd, 0xf6, 0xe3, 255);
        let engine = engine("<style>body { background: #fdf6e3; margin: 0 }</style><p>短内容</p>");
        let list = engine.display_list();
        let area = area_of(&list, cream);
        assert!(
            (area - 800.0 * 600.0).abs() < 1.0,
            "body 底色应当正好铺满视口，实际面积 {area}"
        );
    }

    #[test]
    fn body_background_is_not_painted_twice() {
        let cream = crate::css::Color::rgba(0xfd, 0xf6, 0xe3, 255);
        let engine = engine("<style>body { background: #fdf6e3; margin: 0 }</style><p>短内容</p>");
        let list = engine.display_list();
        let count = list
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    crate::paint::DrawCommand::FillRect { color, .. } if *color == cream
                )
            })
            .count();
        assert_eq!(count, 1, "传播到画布之后 body 自己那层不该再画一遍");
    }

    #[test]
    fn html_background_wins_over_body() {
        let red = crate::css::Color::rgba(255, 0, 0, 255);
        let blue = crate::css::Color::rgba(0, 0, 255, 255);
        let engine =
            engine("<style>html { background: red } body { background: blue }</style><p>x</p>");
        let list = engine.display_list();
        // 根元素有背景就轮不到 body，body 自己那块照常画。
        assert!((area_of(&list, red) - 800.0 * 600.0).abs() < 1.0);
        assert!(area_of(&list, blue) < 800.0 * 600.0);
    }

    #[test]
    fn transparent_everything_stays_white() {
        let white = crate::css::Color::WHITE;
        let engine = engine("<p>x</p>");
        let list = engine.display_list();
        assert!(
            (area_of(&list, white) - 800.0 * 600.0).abs() < 1.0,
            "没有背景时画布应当是白的"
        );
    }

    #[test]
    fn canvas_fill_follows_the_visible_region() {
        // 滚动之后视口上沿在页面坐标里是负的，画布底色要跟着可见范围走，
        // 不然下方会留出一条没盖住的地方。
        let cream = crate::css::Color::rgba(0xfd, 0xf6, 0xe3, 255);
        let engine = engine(
            "<style>body { background: #fdf6e3; margin: 0 } div { height: 2000px }</style><div></div>",
        );
        let visible = Rect::new(0.0, 700.0, 800.0, 600.0);
        let list = engine.display_list_region(visible);
        assert!(
            list.commands.iter().any(|command| matches!(
                command,
                crate::paint::DrawCommand::FillRect { rect, color, .. }
                    if *color == cream && rect.y >= visible.y && rect.bottom() <= visible.bottom()
            )),
            "画布底色应当铺在可见范围上"
        );
    }

    #[test]
    fn media_attribute_gates_whole_block() {
        let narrow = {
            let mut engine = Engine::new(400.0, 600.0);
            engine.load_html("<style media='(min-width: 900px)'>p { color: red }</style><p>x</p>");
            engine
        };
        assert!(
            narrow.stylesheets().is_empty(),
            "不匹配媒体查询的样式表不该收进来"
        );

        let wide = {
            let mut engine = Engine::new(1200.0, 600.0);
            engine.load_html("<style media='(min-width: 900px)'>p { color: red }</style><p>x</p>");
            engine
        };
        assert_eq!(wide.stylesheets().len(), 1);
    }

    #[test]
    fn media_all_attribute_is_accepted() {
        let engine = engine("<style media='all'>p { color: red }</style><p>x</p>");
        assert_eq!(engine.stylesheets().len(), 1);
    }

    #[test]
    fn empty_style_element_is_skipped() {
        let engine = engine("<style></style><p>x</p>");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn layout_produces_tree() {
        let engine = engine("<p>hello</p>");
        assert!(engine.layout_tree().is_some());
        assert!(engine.document_height() >= 600.0);
    }

    #[test]
    fn display_list_has_text() {
        let engine = engine("<p>hello world</p>");
        assert!(engine.display_list().all_text().contains("hello world"));
    }

    #[test]
    fn changing_viewport_relayouts() {
        let mut engine = engine("<style>p { width: 50% }</style><p id=p>x</p>");
        let narrow = engine.document_height();

        let root = engine.document().root();
        let paragraph = engine
            .document()
            .find_element(root, |element| element.id() == Some("p"))
            .expect("应当有 p");
        let width_before = engine.styles()[&paragraph].width;

        engine.set_viewport(200.0, 600.0);
        let width_after = engine.styles()[&paragraph].width;
        // 百分比宽度跟着视口变，但计算样式里存的还是百分比，所以看布局结果。
        assert_eq!(width_before, width_after);
        let _ = narrow;
        assert!(engine.layout_tree().is_some());
    }

    #[test]
    fn element_at_finds_deepest_node() {
        let engine = engine(
            "<style>#outer { padding: 40px } #inner { height: 20px }</style>\
             <div id=outer><p id=inner>text</p></div>",
        );
        let root = engine.document().root();
        let inner = engine
            .document()
            .find_element(root, |element| element.id() == Some("inner"))
            .expect("应当有 inner");
        let tree = engine.layout_tree().expect("应当有布局树");
        let layout_box = tree
            .descendants()
            .into_iter()
            .find(|layout_box| layout_box.node == Some(inner))
            .expect("应当有盒子");
        let hit = engine
            .element_at(layout_box.rect.x + 1.0, layout_box.rect.y + 1.0)
            .expect("应当命中");
        assert_eq!(hit, inner);
    }

    #[test]
    fn external_stylesheet_links_are_collected() {
        let engine = engine("<link rel=\"stylesheet\" href=\"/style.css\"><p>x</p>");
        let links = engine.stylesheet_links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], "/style.css");
    }

    #[test]
    fn relative_link_is_resolved_against_base() {
        let mut engine = Engine::new(800.0, 600.0);
        engine.load_html_with_base(
            "<link rel=\"stylesheet\" href=\"assets/a.css\"><p>x</p>",
            "https://example.com/page/index.html",
        );
        let links = engine.stylesheet_links();
        assert_eq!(links[0], "https://example.com/page/assets/a.css");
    }

    #[test]
    fn absolute_link_is_kept_as_is() {
        let mut engine = Engine::new(800.0, 600.0);
        engine.load_html_with_base(
            "<link rel=\"stylesheet\" href=\"https://cdn.example.com/a.css\">",
            "https://example.com/",
        );
        assert_eq!(
            engine.stylesheet_links()[0],
            "https://cdn.example.com/a.css"
        );
    }

    #[test]
    fn link_without_stylesheet_rel_is_ignored() {
        let engine = engine("<link rel=\"icon\" href=\"/favicon.ico\"><p>x</p>");
        assert!(engine.stylesheet_links().is_empty());
    }

    #[test]
    fn link_media_mismatch_is_skipped() {
        let engine =
            engine("<link rel=\"stylesheet\" media=\"(min-width: 5000px)\" href=\"/a.css\">");
        assert!(engine.stylesheet_links().is_empty());
    }

    #[test]
    fn linked_stylesheet_is_applied_and_ordered() {
        let mut engine = Engine::new(800.0, 600.0);
        // 内联在前、外链在后，外链应当盖住内联。
        engine.load_html(
            "<style>p { color: red }</style>\
             <link rel=\"stylesheet\" href=\"/a.css\">\
             <p id=p>x</p>",
        );
        let root = engine.document().root();
        let paragraph = engine
            .document()
            .find_element(root, |element| element.id() == Some("p"))
            .expect("应当有 p");
        assert_eq!(
            engine.styles()[&paragraph].color,
            crate::css::Color::rgba(255, 0, 0, 255)
        );

        // 外链取回来了，应当按文档顺序排在后面，压过内联样式。
        engine.set_linked_stylesheet("/a.css", "p { color: blue }");
        let paragraph = engine
            .document()
            .find_element(root, |element| element.id() == Some("p"))
            .expect("应当有 p");
        assert_eq!(
            engine.styles()[&paragraph].color,
            crate::css::Color::rgba(0, 0, 255, 255)
        );
    }

    #[test]
    fn unknown_linked_url_is_ignored() {
        let mut engine = Engine::new(800.0, 600.0);
        engine.load_html("<p>x</p>");
        // 文档里没有引用这个地址，塞进来也不该生效。
        engine.set_linked_stylesheet("/not-referenced.css", "p { color: red }");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn linked_stylesheets_are_dropped_on_new_document() {
        let mut engine = Engine::new(800.0, 600.0);
        engine.load_html("<link rel=\"stylesheet\" href=\"/a.css\"><p>x</p>");
        engine.set_linked_stylesheet("/a.css", "p { color: blue }");
        assert_eq!(engine.stylesheets().len(), 1);

        // 换一页之后，上一页取回的样式表不该继续影响新页面。
        engine.load_html("<link rel=\"stylesheet\" href=\"/b.css\"><p>y</p>");
        assert!(engine.stylesheets().is_empty());
    }

    #[test]
    fn empty_document_does_not_panic() {
        let engine = engine("");
        assert!(engine.layout_tree().is_some());
        let list = engine.display_list();
        assert!(!list.is_empty(), "至少要有一条清屏命令");
    }

    #[test]
    fn region_painting_uses_viewport() {
        let engine = engine("<style>div { height: 3000px }</style><div>x</div>");
        let list = engine.display_list_region(Rect::new(0.0, 0.0, 800.0, 600.0));
        assert!(!list.is_empty());
    }

    #[test]
    fn inline_styles_are_applied() {
        let engine = engine("<div style='height: 30px; background: blue'>x</div>");
        let list = engine.display_list();
        assert!(list.commands.iter().any(|command| matches!(
            command,
            crate::paint::DrawCommand::FillRect { color, .. }
                if *color == crate::css::Color::rgba(0, 0, 255, 255)
        )));
    }
}
