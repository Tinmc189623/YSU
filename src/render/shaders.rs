//! 渲染用到的着色器源码。
//!
//! 矩形与边框走同一个实例化的圆角矩形管线，字形走一张单通道图集。
//! 顶点着色器直接由 `vertex_index` 生成两个三角形，没有顶点缓冲。

/// 页面坐标到裁剪空间的换算，矩形与字形共用。
pub const GLOBALS_WGSL: &str = r#"
struct Globals {
    // 视口尺寸，逻辑像素。
    viewport: vec2<f32>,
    // 滚动偏移，页面坐标减去它就是屏幕坐标。
    scroll: vec2<f32>,
    // 页面缩放倍数，画浏览器界面时是一。
    zoom: f32,
    // 补到十六字节对齐。
    //
    // 这里必须用三个 f32，不能用 vec3<f32>：vec3 的对齐要求是 16 字节，
    // 会把整个结构体撑到 48 字节，而 Rust 那边只按 32 字节写入，绑定大小
    // 对不上，wgpu 会直接报校验错误。
    _padding0: f32,
    _padding1: f32,
    _padding2: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// 把页面坐标换算成裁剪空间坐标。先减滚动再乘缩放。
fn to_clip(page: vec2<f32>) -> vec4<f32> {
    let screen = (page - globals.scroll) * globals.zoom;
    let x = screen.x / globals.viewport.x * 2.0 - 1.0;
    let y = 1.0 - screen.y / globals.viewport.y * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}
"#;

/// 圆角矩形的顶点与片元着色器。
///
/// 圆角用带符号距离场算，边缘按屏幕空间导数做一点点羽化，看起来不会锯齿。
pub const RECT_WGSL: &str = r#"
struct RectInstance {
    // 位置与尺寸，页面坐标。
    @location(0) rect: vec4<f32>,
    // 填充色，线性空间。
    @location(1) color: vec4<f32>,
    // 左上、右上、右下、左下四个角的圆角半径。
    @location(2) corners: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    // 相对矩形左上角的局部坐标，片元里算距离用。
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) corners: vec4<f32>,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_rect(@builtin(vertex_index) index: u32, instance: RectInstance) -> VsOut {
    // 两个三角形拼成一个矩形。
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let unit = corners[index];
    let local = unit * instance.rect.zw;
    let page = instance.rect.xy + local;

    var out: VsOut;
    out.position = to_clip(page);
    out.local = local;
    out.size = instance.rect.zw;
    out.corners = instance.corners;
    out.color = instance.color;
    return out;
}

// 按象限挑圆角半径。四个分量依次是左上、右上、右下、左下。
fn corner_radius(center: vec2<f32>, corners: vec4<f32>) -> f32 {
    let is_left = center.x < 0.0;
    let is_top = center.y < 0.0;
    if (is_top) {
        // 上排：左边取左上，右边取右上。
        return select(corners.y, corners.x, is_left);
    }
    // 下排：左边取左下，右边取右下。
    return select(corners.z, corners.w, is_left);
}

// 圆角矩形的有符号距离：负数在内部，零在边界上。
//
// 只算外部的距离是不够的，那样盒子内部会一律得到零，覆盖度上不去，
// 整个矩形会变成半透明。最后那一项负责把内部的距离压成负值。
fn rounded_distance(local: vec2<f32>, size: vec2<f32>, corners: vec4<f32>) -> f32 {
    let half = size * 0.5;
    let center = local - half;
    let radius = corner_radius(center, corners);
    let clamped = min(radius, min(half.x, half.y));
    let q = abs(center) - half + vec2<f32>(clamped);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - clamped;
}

@fragment
fn fs_rect(input: VsOut) -> @location(0) vec4<f32> {
    let distance = rounded_distance(input.local, input.size, input.corners);
    // 用屏幕空间导数把边缘羽化到一个像素以内。
    let width = max(fwidth(distance), 0.0001);
    let coverage = clamp(0.5 - distance / width, 0.0, 1.0);
    return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
"#;

/// 字形的顶点与片元着色器。
pub const GLYPH_WGSL: &str = r#"
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct GlyphInstance {
    // 位置与尺寸，页面坐标。
    @location(0) rect: vec4<f32>,
    // 图集里的纹理坐标，依次是左上与右下。
    @location(1) uv: vec4<f32>,
    // 文字颜色，线性空间。
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_glyph(@builtin(vertex_index) index: u32, instance: GlyphInstance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let unit = corners[index];
    let page = instance.rect.xy + unit * instance.rect.zw;
    let uv = instance.uv.xy + unit * (instance.uv.zw - instance.uv.xy);

    var out: VsOut;
    out.position = to_clip(page);
    out.uv = uv;
    out.color = instance.color;
    return out;
}

@fragment
fn fs_glyph(input: VsOut) -> @location(0) vec4<f32> {
    // 图集存的是覆盖率，颜色由实例带进来。
    let coverage = textureSample(atlas, atlas_sampler, input.uv).r;
    return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
"#;
