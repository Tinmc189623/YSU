//! 文本度量与断行。
//!
//! 字形整形、断行和字体回退交给 cosmic-text，这里只负责把 CSS 的字体制式
//! 翻译成它的输入，并把结果整理成布局阶段要用的行信息。

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap};

/// 一段文本的排版体式。
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// 字体族，按优先级排列。
    pub families: Vec<String>,
    /// 字号，单位像素。
    pub font_size: f32,
    /// 字重，100 到 900。
    pub weight: u16,
    /// 是否是斜体。
    pub italic: bool,
    /// 行高倍数。
    pub line_height: f64,
    /// 是否允许自动换行。
    pub wrap: bool,
}

impl Default for TextStyle {
    /// 与浏览器默认正文一致。
    fn default() -> Self {
        Self {
            families: vec!["sans-serif".to_string()],
            font_size: 16.0,
            weight: 400,
            italic: false,
            line_height: 1.2,
            wrap: true,
        }
    }
}

impl TextStyle {
    /// 行高的像素值。
    pub fn line_height_pixels(&self) -> f32 {
        (self.font_size * self.line_height as f32).max(self.font_size)
    }

    /// 把 CSS 的字体族名翻译成 cosmic-text 的字体族。
    ///
    /// 通用族名走内置枚举，其余按具体字体名查找，找不到时由 cosmic-text
    /// 的回退机制兜底。
    fn primary_family(&self) -> Family<'_> {
        let Some(name) = self.families.first() else {
            return Family::SansSerif;
        };
        match name.to_ascii_lowercase().as_str() {
            "serif" => Family::Serif,
            "sans-serif" => Family::SansSerif,
            "monospace" => Family::Monospace,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            "system-ui" | "-apple-system" => Family::SansSerif,
            _ => Family::Name(name),
        }
    }

    /// 组装 cosmic-text 需要的文字属性。
    pub fn attrs(&self) -> Attrs<'_> {
        let mut attrs = Attrs::new().family(self.primary_family());
        attrs = attrs.weight(Weight(self.weight));
        if self.italic {
            attrs = attrs.style(Style::Italic);
        }
        attrs
    }
}

/// 排版后的一行。
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// 这一行的文本内容，不含行尾换行。
    pub text: String,
    /// 这一行在原始文本里占用的字节数，含被去掉的换行符。
    ///
    /// 折行时要按它来切分剩余文本，用 `text` 的长度会少算换行的那一个字节。
    pub raw_len: usize,
    /// 行的宽度。
    pub width: f32,
    /// 行顶相对文本块顶部的位置。
    pub top: f32,
    /// 行高。
    pub height: f32,
    /// 基线相对文本块顶部的位置。
    pub baseline: f32,
}

/// 一段文本的排版结果。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    /// 各行的信息。
    pub lines: Vec<TextLine>,
    /// 整体宽度，取最宽的一行。
    pub width: f32,
    /// 整体高度，各行高度之和。
    pub height: f32,
    /// 第一行的基线相对顶部的位置。
    pub first_baseline: f32,
}

impl TextLayout {
    /// 只有一个空行时的排版结果，用于空元素占位。
    pub fn empty(line_height: f32) -> Self {
        Self {
            lines: Vec::new(),
            width: 0.0,
            height: line_height,
            first_baseline: line_height * 0.8,
        }
    }

    /// 文本是否没有任何内容。
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// 文本度量器，持有一份字体系统。
///
/// 字体系统加载了系统里的全部字体，创建一次即可，重复创建会很慢。
pub struct TextMeasurer {
    /// cosmic-text 的字体系统。
    font_system: FontSystem,
}

impl Default for TextMeasurer {
    /// 建一个加载了系统字体的度量器。
    fn default() -> Self {
        Self::new()
    }
}

impl TextMeasurer {
    /// 建一个度量器并加载系统字体。
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
        }
    }

    /// 取字体系统的可变引用，渲染阶段取字形位图时要用。
    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    /// 排版一段文本。
    ///
    /// `max_width` 为 `None` 表示不限制宽度，此时不换行。返回的行信息里
    /// 宽度与位置都已经算好，布局阶段直接使用。
    pub fn layout(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        let line_height = style.line_height_pixels();
        if text.is_empty() {
            return TextLayout::empty(line_height);
        }

        let metrics = Metrics::new(style.font_size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        // 不换行时给一个足够大的宽度，让所有内容排在一行里。
        buffer.set_size(max_width.or(Some(f32::MAX)), None);
        buffer.set_wrap(if style.wrap {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        buffer.set_text(text, &style.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut lines = Vec::new();
        let mut offset = 0.0f32;
        let mut widest = 0.0f32;
        let mut first_baseline = line_height * 0.8;

        for run in buffer.layout_runs() {
            let width = run.line_w;
            let height = run.line_height;
            // `LayoutRun::text` 给的是整段文本而不是这一行的内容，
            // 要按字形记录的字节区间自己切。
            let (start, end) = glyph_range(run.glyphs);
            let end = end.min(text.len());
            let start = start.min(end);
            let content = text.get(start..end).unwrap_or("").to_string();
            let mut raw_len = end;
            // 行尾的换行符也属于这一行消耗掉的内容。
            if text[end..].starts_with("\r\n") {
                raw_len += 2;
            } else if text[end..].starts_with('\n') {
                raw_len += 1;
            }

            // cosmic-text 给的 line_y 是基线位置，换成本地坐标。
            let baseline = offset + run.line_y;
            if lines.is_empty() {
                first_baseline = baseline;
            }
            lines.push(TextLine {
                text: content,
                raw_len,
                width,
                top: offset,
                height,
                baseline,
            });
            widest = widest.max(width);
            offset += height;
        }

        // 一行都没有时按空行占位，避免高度算成零。
        if lines.is_empty() {
            return TextLayout::empty(line_height);
        }

        TextLayout {
            lines,
            width: widest,
            height: offset,
            first_baseline,
        }
    }

    /// 度量一段文本的宽度，不做断行。
    pub fn measure_width(&mut self, text: &str, style: &TextStyle) -> f32 {
        let mut unbounded = style.clone();
        unbounded.wrap = false;
        self.layout(text, &unbounded, None).width
    }
}

/// 取一行里全部字形覆盖的字节区间。
///
/// 双向文本的字形按视觉顺序排列，所以取最小起点与最大终点。
fn glyph_range(glyphs: &[cosmic_text::LayoutGlyph]) -> (usize, usize) {
    let mut start = usize::MAX;
    let mut end = 0usize;
    for glyph in glyphs {
        start = start.min(glyph.start);
        end = end.max(glyph.end);
    }
    if start == usize::MAX {
        (0, 0)
    } else {
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个默认体式的度量器。
    fn measurer() -> TextMeasurer {
        TextMeasurer::new()
    }

    #[test]
    fn empty_text_still_takes_a_line() {
        let layout = measurer().layout("", &TextStyle::default(), Some(100.0));
        assert!(layout.is_empty());
        assert!(layout.height > 0.0);
    }

    #[test]
    fn single_line_is_measured() {
        let mut measurer = measurer();
        let layout = measurer.layout("hello world", &TextStyle::default(), Some(500.0));
        assert_eq!(layout.lines.len(), 1);
        assert!(layout.width > 0.0);
        assert!(layout.height >= 16.0);
    }

    #[test]
    fn long_text_wraps_to_multiple_lines() {
        let mut measurer = measurer();
        let text = "the quick brown fox jumps over the lazy dog and keeps running";
        let wide = measurer.layout(text, &TextStyle::default(), Some(2000.0));
        let narrow = measurer.layout(text, &TextStyle::default(), Some(120.0));
        assert_eq!(wide.lines.len(), 1);
        assert!(
            narrow.lines.len() > 1,
            "宽度受限时应当折行，实际 {} 行",
            narrow.lines.len()
        );
    }

    #[test]
    fn wrap_disabled_keeps_one_line() {
        let mut measurer = measurer();
        let style = TextStyle {
            wrap: false,
            ..TextStyle::default()
        };
        let layout = measurer.layout(
            "a very long sentence that would otherwise wrap",
            &style,
            Some(50.0),
        );
        assert_eq!(layout.lines.len(), 1);
    }

    #[test]
    fn explicit_newlines_create_lines() {
        let mut measurer = measurer();
        let layout = measurer.layout("one\ntwo\nthree", &TextStyle::default(), Some(500.0));
        assert_eq!(layout.lines.len(), 3);
    }

    #[test]
    fn line_positions_stack_up() {
        let mut measurer = measurer();
        let layout = measurer.layout("one\ntwo", &TextStyle::default(), Some(500.0));
        assert_eq!(layout.lines[0].top, 0.0);
        assert!(layout.lines[1].top > 0.0);
        assert!((layout.lines[1].top - layout.lines[0].height).abs() < 0.01);
    }

    #[test]
    fn taller_line_height_increases_block_height() {
        let mut measurer = measurer();
        let normal = TextStyle {
            line_height: 1.0,
            ..TextStyle::default()
        };
        let loose = TextStyle {
            line_height: 3.0,
            ..TextStyle::default()
        };
        let a = measurer.layout("x", &normal, Some(500.0));
        let b = measurer.layout("x", &loose, Some(500.0));
        assert!(b.height > a.height, "行高变大后文本块应当变高");
    }

    #[test]
    fn larger_font_measures_wider() {
        let mut measurer = measurer();
        let small = TextStyle {
            font_size: 12.0,
            ..TextStyle::default()
        };
        let large = TextStyle {
            font_size: 32.0,
            ..TextStyle::default()
        };
        let a = measurer.measure_width("hello", &small);
        let b = measurer.measure_width("hello", &large);
        assert!(b > a, "字号变大后宽度应当变大");
    }

    #[test]
    fn monospace_family_is_used() {
        let mut measurer = measurer();
        let style = TextStyle {
            families: vec!["monospace".to_string()],
            ..TextStyle::default()
        };
        // 等宽字体里每个字符宽度一致。
        let one = measurer.measure_width("i", &style);
        let two = measurer.measure_width("ii", &style);
        assert!((two - one * 2.0).abs() < 0.5, "等宽字体宽度应当成比例");
    }

    #[test]
    fn cjk_text_is_measured() {
        let mut measurer = measurer();
        let layout = measurer.layout("中文排版测试", &TextStyle::default(), Some(500.0));
        assert_eq!(layout.lines.len(), 1);
        assert!(layout.width > 0.0);
    }

    #[test]
    fn unknown_family_falls_back() {
        let mut measurer = measurer();
        let style = TextStyle {
            families: vec!["绝不存在的字体名".to_string()],
            ..TextStyle::default()
        };
        // 找不到时由回退字体接手，宽度仍然应当是正数。
        let layout = measurer.layout("fallback", &style, Some(500.0));
        assert!(layout.width > 0.0);
    }

    #[test]
    fn italic_and_bold_change_metrics_or_at_least_run() {
        let mut measurer = measurer();
        let bold = TextStyle {
            weight: 700,
            italic: true,
            ..TextStyle::default()
        };
        let layout = measurer.layout("styled", &bold, Some(500.0));
        assert_eq!(layout.lines.len(), 1);
        assert!(layout.width > 0.0);
    }

    #[test]
    fn first_baseline_is_within_first_line() {
        let mut measurer = measurer();
        let layout = measurer.layout("hello", &TextStyle::default(), Some(500.0));
        assert!(layout.first_baseline > 0.0);
        assert!(layout.first_baseline <= layout.lines[0].height + 0.01);
    }
}
