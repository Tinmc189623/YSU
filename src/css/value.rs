//! CSS 的值类型：长度、颜色与关键字。

use std::fmt;

/// 长度单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    /// 无单位的零，只有 `0` 可以不带单位。
    None,
    /// 像素。
    Px,
    /// 相对当前元素字号。
    Em,
    /// 相对根元素字号。
    Rem,
    /// 相对视口宽度。
    Vw,
    /// 相对视口高度。
    Vh,
    /// 取视口宽高中较小的那个。
    Vmin,
    /// 取视口宽高中较大的那个。
    Vmax,
    /// 相对父元素同方向尺寸，用于宽高与内外边距。
    Percent,
    /// 磅，1pt 等于 4/3 像素。
    Pt,
    /// 派卡，1pc 等于 16 像素。
    Pc,
    /// 英寸，1in 等于 96 像素。
    In,
    /// 厘米。
    Cm,
    /// 毫米。
    Mm,
    /// 四分之一毫米。
    Q,
    /// 相对当前字体里 `0` 的宽度。
    Ch,
    /// 相对当前字体的 x 高度。
    Ex,
    /// `auto`。
    ///
    /// 它不是长度，但在「这一边交给布局自己决定」这个意义上和长度同处一个
    /// 位置，所以放在这里当一种单位。解析成像素时是零，需要区分的地方用
    /// [`Length::is_auto`] 判断——外边距的居中就是靠它区分的。
    Auto,
}

impl LengthUnit {
    /// 按单位名解析，无法识别返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "px" => Self::Px,
            "em" => Self::Em,
            "rem" => Self::Rem,
            "vw" => Self::Vw,
            "vh" => Self::Vh,
            "vmin" => Self::Vmin,
            "vmax" => Self::Vmax,
            "%" => Self::Percent,
            "pt" => Self::Pt,
            "pc" => Self::Pc,
            "in" => Self::In,
            "cm" => Self::Cm,
            "mm" => Self::Mm,
            "q" => Self::Q,
            "ch" => Self::Ch,
            "ex" => Self::Ex,
            _ => return None,
        })
    }

    /// 单位在此视口下的像素数，相对单位返回 `None`。
    pub fn absolute_pixels(self) -> Option<f64> {
        Some(match self {
            Self::None | Self::Px => 1.0,
            Self::Pt => 4.0 / 3.0,
            Self::Pc => 16.0,
            Self::In => 96.0,
            Self::Cm => 96.0 / 2.54,
            Self::Mm => 96.0 / 25.4,
            Self::Q => 96.0 / 101.6,
            _ => return None,
        })
    }
}

/// 带单位的长度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// 数值。
    pub value: f64,
    /// 单位。
    pub unit: LengthUnit,
}

impl Default for Length {
    /// 零长度，与 `Sides` 这类容器的默认值保持一致。
    fn default() -> Self {
        Self::ZERO
    }
}

impl Length {
    /// 零长度。
    pub const ZERO: Self = Self {
        value: 0.0,
        unit: LengthUnit::None,
    };

    /// `auto`。
    pub const AUTO: Self = Self {
        value: 0.0,
        unit: LengthUnit::Auto,
    };

    /// 是不是 `auto`。
    pub fn is_auto(&self) -> bool {
        self.unit == LengthUnit::Auto
    }

    /// 按像素构造。
    pub const fn px(value: f64) -> Self {
        Self {
            value,
            unit: LengthUnit::Px,
        }
    }

    /// 按百分比构造，`value` 是 0 到 100 之间的数。
    pub const fn percent(value: f64) -> Self {
        Self {
            value,
            unit: LengthUnit::Percent,
        }
    }

    /// 是否是零。
    pub fn is_zero(&self) -> bool {
        // `auto` 的值虽然是零，但意思是「交给布局决定」，不是「零」。
        // 两者混为一谈会让「外边距为零」与「外边距为 auto」分不开。
        self.value == 0.0 && self.unit != LengthUnit::Auto
    }

    /// 是否是百分比。
    pub fn is_percent(&self) -> bool {
        self.unit == LengthUnit::Percent
    }
}

impl fmt::Display for Length {
    /// 输出成 CSS 写法。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.unit == LengthUnit::None {
            return write!(f, "{}", super::selector::number_to_css_string(self.value));
        }
        if self.unit == LengthUnit::Auto {
            return f.write_str("auto");
        }
        let unit = match self.unit {
            LengthUnit::None => "",
            LengthUnit::Px => "px",
            LengthUnit::Em => "em",
            LengthUnit::Rem => "rem",
            LengthUnit::Vw => "vw",
            LengthUnit::Vh => "vh",
            LengthUnit::Vmin => "vmin",
            LengthUnit::Vmax => "vmax",
            LengthUnit::Percent => "%",
            LengthUnit::Pt => "pt",
            LengthUnit::Pc => "pc",
            LengthUnit::In => "in",
            LengthUnit::Cm => "cm",
            LengthUnit::Mm => "mm",
            LengthUnit::Q => "q",
            LengthUnit::Ch => "ch",
            LengthUnit::Ex => "ex",
            // 上面已经提前返回，这里到不了。
            LengthUnit::Auto => "auto",
        };
        write!(
            f,
            "{}{unit}",
            super::selector::number_to_css_string(self.value)
        )
    }
}

/// 颜色，按 8 位 RGBA 存放。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    /// 红。
    pub red: u8,
    /// 绿。
    pub green: u8,
    /// 蓝。
    pub blue: u8,
    /// 不透明度，255 表示完全不透明。
    pub alpha: u8,
}

impl Color {
    /// 完全透明。
    pub const TRANSPARENT: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 0,
    };
    /// 黑色。
    pub const BLACK: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 255,
    };
    /// 白色。
    pub const WHITE: Self = Self {
        red: 255,
        green: 255,
        blue: 255,
        alpha: 255,
    };

    /// 按 RGBA 分量构造。
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// 是否是全透明，全透明的元素不用绘制。
    pub fn is_transparent(&self) -> bool {
        self.alpha == 0
    }

    /// 取归一化的 RGBA，交给渲染器用。
    pub fn to_f32_array(&self) -> [f32; 4] {
        [
            f32::from(self.red) / 255.0,
            f32::from(self.green) / 255.0,
            f32::from(self.blue) / 255.0,
            f32::from(self.alpha) / 255.0,
        ]
    }

    /// 按比例调整不透明度，返回新颜色。
    pub fn with_opacity(&self, opacity: f64) -> Self {
        let alpha = (f64::from(self.alpha) * opacity.clamp(0.0, 1.0)).round();
        Self {
            alpha: alpha.clamp(0.0, 255.0) as u8,
            ..*self
        }
    }
}

impl fmt::Display for Color {
    /// 输出成 `#rrggbb` 或 `#rrggbbaa` 的形式。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.alpha == 255 {
            write!(f, "#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
        } else {
            write!(
                f,
                "#{:02x}{:02x}{:02x}{:02x}",
                self.red, self.green, self.blue, self.alpha
            )
        }
    }
}

/// 解析颜色文本，支持十六进制、`rgb()`、`rgba()`、`hsl()`、`hsla()` 与具名颜色。
pub fn parse_color(text: &str) -> Option<Color> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(hex) = trimmed.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(arguments) = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_functional_color(arguments, false);
    }
    if let Some(arguments) = lower
        .strip_prefix("hsl(")
        .or_else(|| lower.strip_prefix("hsla("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_functional_color(arguments, true);
    }

    if lower == "transparent" {
        return Some(Color::TRANSPARENT);
    }
    lookup_named_color(&lower)
}

/// 解析十六进制颜色。
fn parse_hex_color(hex: &str) -> Option<Color> {
    let digits: Vec<char> = hex.chars().collect();
    let expand = |c: char| -> Option<u8> {
        let value = c.to_digit(16)? as u8;
        Some(value * 16 + value)
    };
    let pair = |a: char, b: char| -> Option<u8> {
        Some((a.to_digit(16)? as u8) * 16 + b.to_digit(16)? as u8)
    };

    match digits.len() {
        3 => Some(Color::rgba(
            expand(digits[0])?,
            expand(digits[1])?,
            expand(digits[2])?,
            255,
        )),
        4 => Some(Color::rgba(
            expand(digits[0])?,
            expand(digits[1])?,
            expand(digits[2])?,
            expand(digits[3])?,
        )),
        6 => Some(Color::rgba(
            pair(digits[0], digits[1])?,
            pair(digits[2], digits[3])?,
            pair(digits[4], digits[5])?,
            255,
        )),
        8 => Some(Color::rgba(
            pair(digits[0], digits[1])?,
            pair(digits[2], digits[3])?,
            pair(digits[4], digits[5])?,
            pair(digits[6], digits[7])?,
        )),
        _ => None,
    }
}

/// 解析 `rgb()` 与 `hsl()` 的参数字符串。
fn parse_functional_color(arguments: &str, is_hsl: bool) -> Option<Color> {
    let parts: Vec<&str> = arguments
        .split([',', '/', ' '])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }

    let alpha = match parts.get(3) {
        Some(text) => parse_alpha(text)?,
        None => 1.0,
    };

    if is_hsl {
        let hue = parts[0].parse::<f64>().ok()?;
        let saturation = parse_percentage(parts[1])?;
        let lightness = parse_percentage(parts[2])?;
        let (red, green, blue) = hsl_to_rgb(hue, saturation, lightness);
        return Some(Color::rgba(
            red,
            green,
            blue,
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        ));
    }

    let red = parse_channel(parts[0])?;
    let green = parse_channel(parts[1])?;
    let blue = parse_channel(parts[2])?;
    Some(Color::rgba(
        red,
        green,
        blue,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ))
}

/// 解析 0 到 255 的通道值，也接受百分比形式。
fn parse_channel(text: &str) -> Option<u8> {
    if let Some(percent) = text.strip_suffix('%') {
        let value = percent.parse::<f64>().ok()?;
        return Some((value / 100.0 * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    let value = text.parse::<f64>().ok()?;
    Some(value.round().clamp(0.0, 255.0) as u8)
}

/// 解析百分比，返回 0 到 1 之间的数。
fn parse_percentage(text: &str) -> Option<f64> {
    let value = text.strip_suffix('%')?.parse::<f64>().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}

/// 解析 alpha 值，接受 0 到 1 的小数或百分比。
fn parse_alpha(text: &str) -> Option<f64> {
    if let Some(percent) = text.strip_suffix('%') {
        let value = percent.parse::<f64>().ok()?;
        return Some((value / 100.0).clamp(0.0, 1.0));
    }
    let value = text.parse::<f64>().ok()?;
    Some(value.clamp(0.0, 1.0))
}

/// 把 HSL 转成 RGB，色相按度，饱和度与亮度是 0 到 1。
fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (u8, u8, u8) {
    let hue = hue.rem_euclid(360.0) / 360.0;
    if saturation == 0.0 {
        let channel = (lightness * 255.0).round().clamp(0.0, 255.0) as u8;
        return (channel, channel, channel);
    }

    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;

    let channel = |mut t: f64| -> u8 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let value = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    };

    (
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
    )
}

/// 按名字查颜色，表按名字排序，用二分查找。
pub fn lookup_named_color(name: &str) -> Option<Color> {
    let index = NAMED_COLORS
        .binary_search_by(|(candidate, _)| candidate.cmp(&name))
        .ok()?;
    let (_, packed) = NAMED_COLORS[index];
    Some(Color::rgba(
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
        255,
    ))
}

/// CSS 具名颜色表，按名字的字节序排列。
///
/// 名字是标准里的写法，不含大写，因此可以用原样比较做二分查找。
pub static NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(parse_color("#fff"), Some(Color::WHITE));
        assert_eq!(parse_color("#000000"), Some(Color::BLACK));
        assert_eq!(parse_color("#ff0000"), Some(Color::rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("#f00f"), Some(Color::rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("#00ff0080"), Some(Color::rgba(0, 255, 0, 128)));
    }

    #[test]
    fn invalid_hex_returns_none() {
        assert_eq!(parse_color("#gg0000"), None);
        assert_eq!(parse_color("#12345"), None);
    }

    #[test]
    fn rgb_functions() {
        assert_eq!(
            parse_color("rgb(255, 0, 0)"),
            Some(Color::rgba(255, 0, 0, 255))
        );
        assert_eq!(
            parse_color("rgba(0, 0, 255, 0.5)"),
            Some(Color::rgba(0, 0, 255, 128))
        );
        assert_eq!(
            parse_color("rgb(100%, 0%, 0%)"),
            Some(Color::rgba(255, 0, 0, 255))
        );
        // 空格分隔且用斜杠写 alpha 的新语法。
        assert_eq!(
            parse_color("rgb(0 128 0 / 0.5)"),
            Some(Color::rgba(0, 128, 0, 128))
        );
    }

    #[test]
    fn hsl_functions() {
        assert_eq!(
            parse_color("hsl(0, 100%, 50%)"),
            Some(Color::rgba(255, 0, 0, 255))
        );
        assert_eq!(
            parse_color("hsl(120, 100%, 50%)"),
            Some(Color::rgba(0, 255, 0, 255))
        );
        assert_eq!(
            parse_color("hsl(240, 100%, 50%)"),
            Some(Color::rgba(0, 0, 255, 255))
        );
        // 饱和度为 0 时是灰阶。
        assert_eq!(
            parse_color("hsl(0, 0%, 50%)"),
            Some(Color::rgba(128, 128, 128, 255))
        );
    }

    #[test]
    fn named_colors() {
        assert_eq!(parse_color("red"), Some(Color::rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("WHITE"), Some(Color::WHITE));
        assert_eq!(
            parse_color("rebeccapurple"),
            Some(Color::rgba(0x66, 0x33, 0x99, 255))
        );
        assert_eq!(parse_color("nosuchcolor"), None);
    }

    #[test]
    fn transparent_keyword() {
        assert_eq!(parse_color("transparent"), Some(Color::TRANSPARENT));
        assert!(Color::TRANSPARENT.is_transparent());
    }

    #[test]
    fn named_color_table_is_sorted() {
        for window in NAMED_COLORS.windows(2) {
            assert!(
                window[0].0 < window[1].0,
                "{} 与 {} 的顺序不对",
                window[0].0,
                window[1].0
            );
        }
    }

    #[test]
    fn named_color_table_has_all_entries_findable() {
        for (name, _) in NAMED_COLORS {
            assert!(lookup_named_color(name).is_some(), "{name} 查不到");
        }
    }

    #[test]
    fn color_to_f32() {
        let values = Color::rgba(255, 128, 0, 255).to_f32_array();
        assert!((values[0] - 1.0).abs() < 1e-6);
        assert!((values[1] - 0.502).abs() < 0.01);
        assert!((values[2]).abs() < 1e-6);
    }

    #[test]
    fn opacity_scales_alpha() {
        let faded = Color::BLACK.with_opacity(0.5);
        assert_eq!(faded.alpha, 128);
        assert_eq!(faded.red, 0);
    }

    #[test]
    fn color_display() {
        assert_eq!(Color::WHITE.to_string(), "#ffffff");
        assert_eq!(Color::rgba(0, 0, 0, 128).to_string(), "#00000080");
    }

    #[test]
    fn length_units() {
        assert_eq!(LengthUnit::from_name("px"), Some(LengthUnit::Px));
        assert_eq!(LengthUnit::from_name("REM"), None);
        assert_eq!(LengthUnit::Px.absolute_pixels(), Some(1.0));
        assert_eq!(LengthUnit::In.absolute_pixels(), Some(96.0));
        assert_eq!(LengthUnit::Em.absolute_pixels(), None);
    }

    #[test]
    fn length_display() {
        assert_eq!(Length::px(12.0).to_string(), "12px");
        assert_eq!(Length::percent(50.0).to_string(), "50%");
        assert_eq!(Length::ZERO.to_string(), "0");
        assert!(Length::ZERO.is_zero());
    }
}
