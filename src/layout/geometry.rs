//! 布局用的几何类型。
//!
//! 坐标一律相对页面左上角，单位是逻辑像素。

/// 一个点。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// 横坐标。
    pub x: f64,
    /// 纵坐标。
    pub y: f64,
}

impl Point {
    /// 按坐标构造。
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// 一个尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// 宽度。
    pub width: f64,
    /// 高度。
    pub height: f64,
}

impl Size {
    /// 按宽高构造。
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// 一个矩形，位置指左上角。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// 左上角横坐标。
    pub x: f64,
    /// 左上角纵坐标。
    pub y: f64,
    /// 宽度。
    pub width: f64,
    /// 高度。
    pub height: f64,
}

impl Rect {
    /// 按位置与尺寸构造。
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 从一个点与尺寸构造。
    pub const fn from_point(point: Point, size: Size) -> Self {
        Self::new(point.x, point.y, size.width, size.height)
    }

    /// 右边界的横坐标。
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    /// 下边界的纵坐标。
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// 中心点。
    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// 矩形是否包含某个点。
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// 两个矩形是否相交。
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    /// 四个方向的边缘值，与另一个矩形取交集。
    ///
    /// 内容溢出但没有设置裁剪时，绘制阶段用它判断哪些部分落在可见范围内。
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    }

    /// 向外扩张指定的边距。
    pub fn expand(&self, edges: Edges) -> Rect {
        Rect::new(
            self.x - edges.left,
            self.y - edges.top,
            self.width + edges.horizontal(),
            self.height + edges.vertical(),
        )
    }

    /// 向内收缩指定的边距。
    pub fn shrink(&self, edges: Edges) -> Rect {
        Rect::new(
            self.x + edges.left,
            self.y + edges.top,
            (self.width - edges.horizontal()).max(0.0),
            (self.height - edges.vertical()).max(0.0),
        )
    }

    /// 平移矩形。
    pub fn translate(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// 四个方向的边缘值，用于内外边距与边框。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    /// 上。
    pub top: f64,
    /// 右。
    pub right: f64,
    /// 下。
    pub bottom: f64,
    /// 左。
    pub left: f64,
}

impl Edges {
    /// 四边取同一个值。
    pub const fn uniform(value: f64) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// 横向合计。
    pub fn horizontal(&self) -> f64 {
        self.left + self.right
    }

    /// 纵向合计。
    pub fn vertical(&self) -> f64 {
        self.top + self.bottom
    }

    /// 四边是否都是零。
    pub fn is_zero(&self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_bounds() {
        let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(rect.right(), 40.0);
        assert_eq!(rect.bottom(), 60.0);
        assert_eq!(rect.center(), Point::new(25.0, 40.0));
    }

    #[test]
    fn rect_contains_point_half_open() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(rect.contains(Point::new(0.0, 0.0)));
        assert!(rect.contains(Point::new(9.9, 9.9)));
        // 右边界与下边界不算在内，避免相邻元素重复命中。
        assert!(!rect.contains(Point::new(10.0, 5.0)));
        assert!(!rect.contains(Point::new(5.0, 10.0)));
        assert!(!rect.contains(Point::new(-1.0, 5.0)));
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let overlap = a.intersection(&b);
        assert_eq!(overlap, Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersects(&b));

        let c = Rect::new(20.0, 20.0, 5.0, 5.0);
        assert!(!a.intersects(&c));
        assert_eq!(a.intersection(&c).width, 0.0);
    }

    #[test]
    fn rect_expand_and_shrink_round_trip() {
        let rect = Rect::new(10.0, 10.0, 100.0, 50.0);
        let edges = Edges {
            top: 1.0,
            right: 2.0,
            bottom: 3.0,
            left: 4.0,
        };
        let expanded = rect.expand(edges);
        assert_eq!(expanded, Rect::new(6.0, 9.0, 106.0, 54.0));
        assert_eq!(expanded.shrink(edges), rect);
    }

    #[test]
    fn shrink_never_goes_negative() {
        let rect = Rect::new(0.0, 0.0, 5.0, 5.0);
        let shrunk = rect.shrink(Edges::uniform(10.0));
        assert_eq!(shrunk.width, 0.0);
        assert_eq!(shrunk.height, 0.0);
    }

    #[test]
    fn rect_translate() {
        let rect = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(rect.translate(10.0, 20.0), Rect::new(11.0, 22.0, 3.0, 4.0));
    }

    #[test]
    fn edges_totals() {
        let edges = Edges {
            top: 1.0,
            right: 2.0,
            bottom: 3.0,
            left: 4.0,
        };
        assert_eq!(edges.horizontal(), 6.0);
        assert_eq!(edges.vertical(), 4.0);
        assert!(!edges.is_zero());
        assert!(Edges::default().is_zero());
    }
}
