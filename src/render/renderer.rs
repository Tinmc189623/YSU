//! wgpu 渲染器：把显示列表翻译成绘制调用。
//!
//! 渲染器不创建 GPU 设备，设备与队列由外壳传进来。矩形、边框与文字各走
//! 一条实例化的管线，裁剪用渲染通道的剪刀矩形实现。
//!
//! 命令的顺序必须保持，所以每当裁剪范围变化、或者在矩形与文字之间切换时
//! 就断一次批次。批次小一些，但绘制顺序与显示列表完全一致。

use super::atlas::GlyphAtlas;
use super::shaders::{GLOBALS_WGSL, GLYPH_WGSL, RECT_WGSL};
use crate::css::value::Color;
use crate::layout::geometry::{Edges, Rect};
use crate::layout::text::{TextMeasurer, TextStyle};
use crate::paint::{Corners, DisplayList, DrawCommand, LineStyle};
use cosmic_text::{Buffer, Metrics, Shaping, SwashCache, Wrap};

/// 全局参数的字节数：视口、滚动、缩放，补齐到 32 字节。
const GLOBALS_SIZE: u64 = 32;

/// 一个批次的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    /// 矩形与边框。
    Rects,
    /// 文字。
    Glyphs,
}

/// 一次绘制调用。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Batch {
    /// 走哪条管线。
    kind: BatchKind,
    /// 剪刀矩形，物理像素，依次是 x、y、宽、高。
    scissor: Option<[u32; 4]>,
    /// 起始实例下标。
    first: u32,
    /// 实例个数。
    count: u32,
}

/// 渲染器。
pub struct Renderer {
    /// 矩形管线。
    rect_pipeline: wgpu::RenderPipeline,
    /// 文字管线。
    glyph_pipeline: wgpu::RenderPipeline,
    /// 全局参数缓冲。
    globals_buffer: wgpu::Buffer,
    /// 全局参数绑定组。
    globals_bind_group: wgpu::BindGroup,
    /// 字形图集。
    atlas: GlyphAtlas,
    /// 图集绑定组。
    atlas_bind_group: wgpu::BindGroup,
    /// 矩形实例缓冲。
    rect_buffer: wgpu::Buffer,
    /// 文字实例缓冲。
    glyph_buffer: wgpu::Buffer,
    /// 矩形缓冲能装多少个实例。
    rect_capacity: u32,
    /// 文字缓冲能装多少个实例。
    glyph_capacity: u32,
    /// 字形位图的缓存。
    swash_cache: SwashCache,
    /// 视口尺寸，逻辑像素。
    viewport: (f32, f32),
    /// 滚动偏移，逻辑像素。
    scroll: (f32, f32),
    /// 物理像素与逻辑像素的比例。
    scale: f64,
    /// 页面缩放倍数，只作用于页面内容，不影响浏览器界面。
    zoom: f64,
    /// 交换链纹理格式。
    format: wgpu::TextureFormat,
}

impl Renderer {
    /// 建一个渲染器。
    ///
    /// `format` 是交换链的纹理格式，管线要与它匹配。
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("全局参数布局"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("全局参数"),
            size: GLOBALS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("全局参数绑定组"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let atlas = GlyphAtlas::new(device, super::atlas::DEFAULT_ATLAS_SIZE);
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("图集布局"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("图集采样器"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("图集绑定组"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
            ],
        });

        // 矩形管线的实例布局：位置尺寸、颜色、圆角。
        let rect_layout = wgpu::VertexBufferLayout {
            array_stride: 12 * 4,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
            ],
        };
        let glyph_layout = wgpu::VertexBufferLayout {
            array_stride: 12 * 4,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x4,
                1 => Float32x4,
                2 => Float32x4,
            ],
        };

        let rect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("矩形着色器"),
            source: wgpu::ShaderSource::Wgsl(format!("{GLOBALS_WGSL}\n{RECT_WGSL}").into()),
        });
        let glyph_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("文字着色器"),
            source: wgpu::ShaderSource::Wgsl(format!("{GLOBALS_WGSL}\n{GLYPH_WGSL}").into()),
        });

        let rect_pipeline = create_pipeline(
            device,
            "矩形管线",
            &rect_shader,
            ("vs_rect", "fs_rect"),
            format,
            &[&globals_layout],
            &rect_layout,
        );
        let glyph_pipeline = create_pipeline(
            device,
            "文字管线",
            &glyph_shader,
            ("vs_glyph", "fs_glyph"),
            format,
            &[&globals_layout, &atlas_layout],
            &glyph_layout,
        );

        let (rect_buffer, rect_capacity) = create_instance_buffer(device, "矩形实例", 1024);
        let (glyph_buffer, glyph_capacity) = create_instance_buffer(device, "文字实例", 1024);

        let mut renderer = Self {
            rect_pipeline,
            glyph_pipeline,
            globals_buffer,
            globals_bind_group,
            atlas,
            atlas_bind_group,
            rect_buffer,
            glyph_buffer,
            rect_capacity,
            glyph_capacity,
            swash_cache: SwashCache::new(),
            viewport: (width as f32, height as f32),
            scroll: (0.0, 0.0),
            scale: 1.0,
            zoom: 1.0,
            format,
        };
        renderer.update_globals(queue);
        renderer
    }

    /// 交换链纹理格式。
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// 更新视口尺寸，逻辑像素。
    pub fn resize(&mut self, queue: &wgpu::Queue, width: f32, height: f32) {
        self.viewport = (width.max(1.0), height.max(1.0));
        self.update_globals(queue);
    }

    /// 一次设定视口、滚动、缩放与像素比例。
    ///
    /// 一帧里要画两次（页面与浏览器界面），每次画之前都用这个换一遍参数。
    /// 像素比例必须和视口一起给：它决定剪刀矩形与字形栅格化的尺寸，两者
    /// 不同步就会画出半屏内容。
    pub fn set_view(
        &mut self,
        queue: &wgpu::Queue,
        viewport: (f32, f32),
        scroll: (f32, f32),
        zoom: f32,
        scale: f32,
    ) {
        self.viewport = (viewport.0.max(1.0), viewport.1.max(1.0));
        self.scroll = scroll;
        self.zoom = f64::from(zoom).clamp(0.1, 10.0);
        self.scale = f64::from(scale).max(0.1);
        self.update_globals(queue);
    }

    /// 当前缩放倍数。
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// 设置滚动偏移，逻辑像素。
    pub fn set_scroll(&mut self, queue: &wgpu::Queue, x: f32, y: f32) {
        self.scroll = (x, y);
        self.update_globals(queue);
    }

    /// 把全局参数写进缓冲。
    fn update_globals(&mut self, queue: &wgpu::Queue) {
        let mut data = Vec::with_capacity(GLOBALS_SIZE as usize);
        push_f32(&mut data, self.viewport.0);
        push_f32(&mut data, self.viewport.1);
        push_f32(&mut data, self.scroll.0);
        push_f32(&mut data, self.scroll.1);
        push_f32(&mut data, self.zoom as f32);
        // 补三个零，凑够十六字节对齐。
        push_f32(&mut data, 0.0);
        push_f32(&mut data, 0.0);
        push_f32(&mut data, 0.0);
        queue.write_buffer(&self.globals_buffer, 0, &data);
    }

    /// 绘制一帧，先清屏再画。
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        list: &DisplayList,
        measurer: &mut TextMeasurer,
    ) {
        self.draw(device, queue, target, list, measurer, true);
    }

    /// 在已经画好的内容上叠加一层，不清屏。
    ///
    /// 外壳先画页面，再用这个把浏览器界面盖上去。
    pub fn render_over(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        list: &DisplayList,
        measurer: &mut TextMeasurer,
    ) {
        self.draw(device, queue, target, list, measurer, false);
    }

    /// 绘制一帧。
    fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        list: &DisplayList,
        measurer: &mut TextMeasurer,
        clear: bool,
    ) {
        let mut rect_instances: Vec<u8> = Vec::new();
        let mut glyph_instances: Vec<u8> = Vec::new();
        let mut batches: Vec<Batch> = Vec::new();

        let mut scissor: Option<[u32; 4]> = None;
        let mut current: Option<Batch> = None;

        for command in &list.commands {
            match command {
                DrawCommand::PushClip { rect } => {
                    // 裁剪范围变了要断一次批次。
                    if let Some(batch) = current.take() {
                        batches.push(batch);
                    }
                    scissor = self.scissor_of(*rect);
                }
                DrawCommand::PopClip => {
                    if let Some(batch) = current.take() {
                        batches.push(batch);
                    }
                    scissor = None;
                }
                DrawCommand::FillRect {
                    rect,
                    color,
                    corners,
                } => {
                    self.ensure_rect_batch(&mut current, &mut batches, scissor);
                    push_rect(&mut rect_instances, *rect, *color, *corners, 1.0);
                    if let Some(batch) = current.as_mut() {
                        batch.count += 1;
                    }
                }
                DrawCommand::Border {
                    rect,
                    widths,
                    color,
                    style,
                    ..
                } => {
                    self.ensure_rect_batch(&mut current, &mut batches, scissor);
                    let count = push_border(&mut rect_instances, *rect, *widths, *color, *style);
                    if let Some(batch) = current.as_mut() {
                        batch.count += count;
                    }
                }
                DrawCommand::Text {
                    rect,
                    baseline,
                    text,
                    color,
                    font,
                    underline,
                } => {
                    self.ensure_glyph_batch(&mut current, &mut batches, scissor);
                    let count = self.rasterize_text(
                        measurer,
                        &mut glyph_instances,
                        *rect,
                        *baseline,
                        text,
                        *color,
                        font,
                        *underline,
                    );
                    if let Some(batch) = current.as_mut() {
                        batch.count += count;
                    }
                }
                // 图片需要解码与纹理加载，尚未实现。
                DrawCommand::Image { .. } => {}
            }
        }
        if let Some(batch) = current {
            batches.push(batch);
        }

        // 图集与实例缓冲都要在编码之前准备好。
        self.atlas.upload(queue);
        self.upload_instances(device, queue, &rect_instances, &glyph_instances);

        if batches.is_empty() {
            return;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("绘制编码器"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("页面绘制"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // 显示列表自带清屏矩形，这里清成白色只为兜底；
                        // 叠加那一层则要保留已经画好的内容。
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::WHITE)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let surface = self.surface_size();
            for batch in &batches {
                if batch.count == 0 {
                    continue;
                }
                match batch.kind {
                    BatchKind::Rects => {
                        pass.set_pipeline(&self.rect_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(
                            0,
                            self.rect_buffer
                                .slice(..(batch.count as u64) * 48 + (batch.first as u64) * 48),
                        );
                        if let Some(rect) = batch.scissor {
                            pass.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
                        } else {
                            pass.set_scissor_rect(0, 0, surface.0, surface.1);
                        }
                        pass.draw(0..6, batch.first..batch.first + batch.count);
                    }
                    BatchKind::Glyphs => {
                        pass.set_pipeline(&self.glyph_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                        pass.set_vertex_buffer(
                            0,
                            self.glyph_buffer
                                .slice(..(batch.count as u64) * 48 + (batch.first as u64) * 48),
                        );
                        if let Some(rect) = batch.scissor {
                            pass.set_scissor_rect(rect[0], rect[1], rect[2], rect[3]);
                        } else {
                            pass.set_scissor_rect(0, 0, surface.0, surface.1);
                        }
                        pass.draw(0..6, batch.first..batch.first + batch.count);
                    }
                }
            }
        }
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// 表面尺寸，物理像素。
    fn surface_size(&self) -> (u32, u32) {
        (
            (f64::from(self.viewport.0) * self.scale).round().max(1.0) as u32,
            (f64::from(self.viewport.1) * self.scale).round().max(1.0) as u32,
        )
    }

    /// 把页面坐标的裁剪矩形转成物理像素的剪刀矩形。
    fn scissor_of(&self, rect: Rect) -> Option<[u32; 4]> {
        scissor_rect(
            rect,
            self.scroll,
            self.zoom,
            self.scale,
            self.surface_size(),
        )
    }

    /// 确保当前批次是矩形批次，不是就断一个。
    fn ensure_rect_batch(
        &self,
        current: &mut Option<Batch>,
        batches: &mut Vec<Batch>,
        scissor: Option<[u32; 4]>,
    ) {
        let need_new = match current {
            Some(batch) => batch.kind != BatchKind::Rects || batch.scissor != scissor,
            None => true,
        };
        if need_new {
            if let Some(batch) = current.take() {
                batches.push(batch);
            }
            let first = batches
                .iter()
                .filter(|batch| batch.kind == BatchKind::Rects)
                .map(|batch| batch.first + batch.count)
                .max()
                .unwrap_or(0);
            *current = Some(Batch {
                kind: BatchKind::Rects,
                scissor,
                first,
                count: 0,
            });
        }
    }

    /// 确保当前批次是文字批次。
    fn ensure_glyph_batch(
        &self,
        current: &mut Option<Batch>,
        batches: &mut Vec<Batch>,
        scissor: Option<[u32; 4]>,
    ) {
        let need_new = match current {
            Some(batch) => batch.kind != BatchKind::Glyphs || batch.scissor != scissor,
            None => true,
        };
        if need_new {
            if let Some(batch) = current.take() {
                batches.push(batch);
            }
            let first = batches
                .iter()
                .filter(|batch| batch.kind == BatchKind::Glyphs)
                .map(|batch| batch.first + batch.count)
                .max()
                .unwrap_or(0);
            *current = Some(Batch {
                kind: BatchKind::Glyphs,
                scissor,
                first,
                count: 0,
            });
        }
    }

    /// 把栅格化出来的字形拼进实例数据，返回新增的实例个数。
    #[allow(clippy::too_many_arguments)]
    fn rasterize_text(
        &mut self,
        measurer: &mut TextMeasurer,
        out: &mut Vec<u8>,
        rect: Rect,
        baseline: f64,
        text: &str,
        color: Color,
        font: &TextStyle,
        underline: bool,
    ) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let font_system = measurer.font_system_mut();
        // 字形按物理像素栅格化。高分屏下如果还按逻辑尺寸画位图，再放大到
        // 屏幕上就会发虚，中文字尤其明显。
        let raster_scale = (self.scale * self.zoom).max(1.0) as f32;
        let metrics = Metrics::new(
            font.font_size,
            (font.font_size * font.line_height as f32).max(font.font_size),
        );
        let mut buffer = Buffer::new(font_system, metrics);
        // 显示列表里的文本已经是断好行的一行，这里不再换行。
        buffer.set_size(None, None);
        buffer.set_wrap(Wrap::None);
        buffer.set_text(text, &font.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        let linear = to_linear(color);
        let mut count = 0u32;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, run.line_y), raster_scale);
                // 取出位图后立刻复制一份，好让 swash 缓存的借用到此结束，
                // 后面才能接着改图集。
                let image = match self.swash_cache.get_image(font_system, physical.cache_key) {
                    Some(image) => image.clone(),
                    None => continue,
                };
                let (width, height) = (image.placement.width, image.placement.height);
                if width == 0 || height == 0 {
                    continue;
                }
                if !matches!(image.content, cosmic_text::SwashContent::Mask) {
                    // 彩色字形（emoji）走的是 RGBA 位图，图集只存覆盖率，暂不支持。
                    continue;
                }
                // physical 给的是缩放之后的坐标，除以倍率换回逻辑单位，
                // 位置与尺寸就都落在页面的坐标系里。
                let ratio = f64::from(raster_scale);
                let x = rect.x + (f64::from(physical.x) + f64::from(image.placement.left)) / ratio;
                let y = rect.y + (f64::from(physical.y) - f64::from(image.placement.top)) / ratio;
                let Some(entry) = self
                    .atlas
                    .insert(physical.cache_key, width, height, &image.data)
                else {
                    continue;
                };
                let quad = Rect::new(x, y, f64::from(width) / ratio, f64::from(height) / ratio);
                push_glyph(out, quad, entry.uv, linear);
                count += 1;
            }
        }

        // 下划线按文字宽度画一条细线。
        if underline {
            let underline_rect = Rect::new(
                rect.x,
                baseline + 2.0,
                rect.width.max(0.0),
                (font.font_size as f64 * 0.06).max(1.0),
            );
            push_rect(out, underline_rect, color, Corners::default(), 1.0);
            count += 1;
        }

        count
    }

    /// 把实例数据写进显存，容量不够就重建缓冲。
    fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rects: &[u8],
        glyphs: &[u8],
    ) {
        let rect_count = (rects.len() / 48) as u32;
        if rect_count > self.rect_capacity {
            let (buffer, capacity) =
                create_instance_buffer(device, "矩形实例", rect_count.next_power_of_two());
            self.rect_buffer = buffer;
            self.rect_capacity = capacity;
        }
        if !rects.is_empty() {
            queue.write_buffer(&self.rect_buffer, 0, rects);
        }

        let glyph_count = (glyphs.len() / 48) as u32;
        if glyph_count > self.glyph_capacity {
            let (buffer, capacity) =
                create_instance_buffer(device, "文字实例", glyph_count.next_power_of_two());
            self.glyph_buffer = buffer;
            self.glyph_capacity = capacity;
        }
        if !glyphs.is_empty() {
            queue.write_buffer(&self.glyph_buffer, 0, glyphs);
        }
    }
}

/// 建一条实例化的渲染管线。
fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    entry_points: (&str, &str),
    format: wgpu::TextureFormat,
    bind_group_layouts: &[&wgpu::BindGroupLayout],
    vertex_layout: &wgpu::VertexBufferLayout<'_>,
) -> wgpu::RenderPipeline {
    // 绑定组布局在 wgpu 30 里是可选的，用不上的槽位填 None。
    let layouts: Vec<Option<&wgpu::BindGroupLayout>> = bind_group_layouts
        .iter()
        .map(|layout| Some(*layout))
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(entry_points.0),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(vertex_layout.clone())],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // 文字与矩形的朝向一致，不剔除任何面。
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_points.1),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// 建一个实例缓冲，返回缓冲与能装的实例数。
fn create_instance_buffer(
    device: &wgpu::Device,
    label: &str,
    capacity: u32,
) -> (wgpu::Buffer, u32) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::from(capacity) * 48,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (buffer, capacity)
}

/// 往字节流里写一个 f32，小端序与 GPU 一致。
fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_ne_bytes());
}

/// 把页面坐标的裁剪矩形换算成物理像素的剪刀矩形。
///
/// 换算和顶点走的是同一套：**先减滚动偏移，再乘缩放与像素比**，两步都不能少。
/// 顶点在着色器里算的是 `(页面坐标 - 滚动) * 缩放`；剪刀矩形要是漏掉减滚动
/// 这一步，滚动之后裁剪范围会整体错位——页面往上走多少，屏幕顶部就被白白
/// 裁掉多少，看起来就是一片空白。
///
/// 返回 `None` 表示整个矩形都在可见范围之外，调用方按「不设裁剪」处理。
fn scissor_rect(
    rect: Rect,
    scroll: (f32, f32),
    zoom: f64,
    scale: f64,
    surface: (u32, u32),
) -> Option<[u32; 4]> {
    let factor = scale * zoom;
    let x = ((rect.x - f64::from(scroll.0)) * factor).floor().max(0.0);
    let y = ((rect.y - f64::from(scroll.1)) * factor).floor().max(0.0);
    let right = ((rect.right() - f64::from(scroll.0)) * factor)
        .ceil()
        .min(f64::from(surface.0));
    let bottom = ((rect.bottom() - f64::from(scroll.1)) * factor)
        .ceil()
        .min(f64::from(surface.1));
    if right <= x || bottom <= y {
        // 空裁剪等于什么都看不见，用一个零尺寸的矩形表示。
        return Some([0, 0, 0, 0]);
    }
    Some([x as u32, y as u32, (right - x) as u32, (bottom - y) as u32])
}

/// 追加一个矩形实例。
fn push_rect(out: &mut Vec<u8>, rect: Rect, color: Color, corners: Corners, opacity: f64) {
    let linear = to_linear(color.with_opacity(opacity));
    // 位置与尺寸。
    push_f32(out, rect.x as f32);
    push_f32(out, rect.y as f32);
    push_f32(out, rect.width.max(0.0) as f32);
    push_f32(out, rect.height.max(0.0) as f32);
    // 颜色。
    for channel in linear {
        push_f32(out, channel);
    }
    // 四个角的圆角半径，顺序是左上、右上、右下、左下。
    push_f32(out, corners.top_left as f32);
    push_f32(out, corners.top_right as f32);
    push_f32(out, corners.bottom_right as f32);
    push_f32(out, corners.bottom_left as f32);
}

/// 追加一个字形实例。
fn push_glyph(out: &mut Vec<u8>, rect: Rect, uv: [f32; 4], color: [f32; 4]) {
    push_f32(out, rect.x as f32);
    push_f32(out, rect.y as f32);
    push_f32(out, rect.width.max(0.0) as f32);
    push_f32(out, rect.height.max(0.0) as f32);
    for value in uv {
        push_f32(out, value);
    }
    for channel in color {
        push_f32(out, channel);
    }
}

/// 把边框拆成若干个矩形实例，返回实例个数。
///
/// 虚线按 3 倍线宽一段来切，点线用正方形点，双线画成两条细线。
fn push_border(
    out: &mut Vec<u8>,
    rect: Rect,
    widths: Edges,
    color: Color,
    style: LineStyle,
) -> u32 {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return 0;
    }
    let mut count = 0u32;
    // 四条边的范围：上下横跨整宽，左右夹在上下边之间。
    let inner_height = (rect.height - widths.top - widths.bottom).max(0.0);
    let inner_width = (rect.width - widths.left - widths.right).max(0.0);

    let sides = [
        (
            Rect::new(rect.x, rect.y, rect.width, widths.top),
            true,
            widths.top,
        ),
        (
            Rect::new(
                rect.right() - widths.right,
                rect.y + widths.top,
                widths.right,
                inner_height,
            ),
            false,
            widths.right,
        ),
        (
            Rect::new(
                rect.x + widths.left,
                rect.bottom() - widths.bottom,
                inner_width,
                widths.bottom,
            ),
            true,
            widths.bottom,
        ),
        (
            Rect::new(rect.x, rect.y + widths.top, widths.left, inner_height),
            false,
            widths.left,
        ),
    ];

    for (side, horizontal, thickness) in sides {
        if thickness <= 0.0 || side.width <= 0.0 || side.height <= 0.0 {
            continue;
        }
        count += push_border_side(out, side, horizontal, thickness, color, style);
    }
    count
}

/// 画一条边，按线型决定切成几段。
fn push_border_side(
    out: &mut Vec<u8>,
    side: Rect,
    horizontal: bool,
    thickness: f64,
    color: Color,
    style: LineStyle,
) -> u32 {
    let length = if horizontal { side.width } else { side.height };
    match style {
        LineStyle::Solid => {
            push_rect(out, side, color, Corners::default(), 1.0);
            1
        }
        LineStyle::Double => {
            // 两条细线，中间留出空隙。
            let thin = (thickness / 3.0).max(0.5);
            if horizontal {
                let top = Rect::new(side.x, side.y, side.width, thin);
                let bottom = Rect::new(side.x, side.bottom() - thin, side.width, thin);
                push_rect(out, top, color, Corners::default(), 1.0);
                push_rect(out, bottom, color, Corners::default(), 1.0);
            } else {
                let left = Rect::new(side.x, side.y, thin, side.height);
                let right = Rect::new(side.right() - thin, side.y, thin, side.height);
                push_rect(out, left, color, Corners::default(), 1.0);
                push_rect(out, right, color, Corners::default(), 1.0);
            }
            2
        }
        LineStyle::Dashed | LineStyle::Dotted => {
            // 虚线一段走 3 倍线宽，点线用正方形，两者间距都等于线宽。
            let segment = if style == LineStyle::Dotted {
                thickness
            } else {
                thickness * 3.0
            };
            let gap = thickness.max(1.0);
            let step = segment + gap;
            if step <= 0.0 || length <= 0.0 {
                return 0;
            }
            let mut count = 0u32;
            let mut offset = 0.0;
            while offset < length {
                let size = segment.min(length - offset);
                let piece = if horizontal {
                    Rect::new(side.x + offset, side.y, size, thickness)
                } else {
                    Rect::new(side.x, side.y + offset, thickness, size)
                };
                push_rect(out, piece, color, Corners::default(), 1.0);
                count += 1;
                offset += step;
            }
            count
        }
    }
}

/// 把 sRGB 颜色转成线性空间的 RGBA。
///
/// 交换链通常是 sRGB 格式，GPU 在写入时会做一次编码，所以这里要先解码，
/// 否则颜色会整体偏亮。
fn to_linear(color: Color) -> [f32; 4] {
    [
        srgb_to_linear(color.red),
        srgb_to_linear(color.green),
        srgb_to_linear(color.blue),
        f32::from(color.alpha) / 255.0,
    ]
}

/// 单个通道的 sRGB 到线性换算。
fn srgb_to_linear(value: u8) -> f32 {
    let normalized = f32::from(value) / 255.0;
    if normalized <= 0.04045 {
        normalized / 12.92
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_conversion_endpoints() {
        assert!((srgb_to_linear(0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn srgb_conversion_midpoint_is_darker_than_input() {
        // 线性空间里中灰比 sRGB 的中灰要暗。
        let mid = srgb_to_linear(128);
        assert!(mid < 0.5, "实际 {mid}");
        assert!(mid > 0.2);
    }

    #[test]
    fn color_to_linear_keeps_alpha() {
        let linear = to_linear(Color::rgba(255, 0, 0, 128));
        assert!((linear[0] - 1.0).abs() < 1e-6);
        assert!((linear[1]).abs() < 1e-6);
        assert!((linear[3] - 0.50196).abs() < 0.01);
    }

    #[test]
    fn scissor_subtracts_the_scroll_offset() {
        // 顶点在着色器里减了滚动量，剪刀矩形也必须减，否则滚动之后
        // 屏幕上方会被整块裁掉。
        let surface = (1000, 800);
        // 滚动为零时，页面坐标直接就是屏幕坐标。
        assert_eq!(
            scissor_rect(
                Rect::new(10.0, 20.0, 100.0, 50.0),
                (0.0, 0.0),
                1.0,
                1.0,
                surface
            ),
            Some([10, 20, 100, 50])
        );
        // 滚动 200 之后，同一个盒子上移 200 像素。
        assert_eq!(
            scissor_rect(
                Rect::new(10.0, 220.0, 100.0, 50.0),
                (0.0, 200.0),
                1.0,
                1.0,
                surface
            ),
            Some([10, 20, 100, 50])
        );
    }

    #[test]
    fn scissor_drops_boxes_scrolled_out_of_view() {
        let surface = (1000, 800);
        // 整个盒子滚到视口上方去了，裁剪区域是空的。
        assert_eq!(
            scissor_rect(
                Rect::new(0.0, 0.0, 100.0, 50.0),
                (0.0, 200.0),
                1.0,
                1.0,
                surface
            ),
            Some([0, 0, 0, 0])
        );
    }

    #[test]
    fn scissor_scales_with_zoom_and_pixel_ratio() {
        let surface = (2000, 1600);
        // 两倍缩放叠加两倍像素比，范围乘四。
        assert_eq!(
            scissor_rect(
                Rect::new(10.0, 10.0, 100.0, 50.0),
                (0.0, 0.0),
                2.0,
                2.0,
                surface
            ),
            Some([40, 40, 400, 200])
        );
    }

    #[test]
    fn scissor_is_clamped_to_the_surface() {
        let surface = (100, 100);
        // 超出目标纹理的部分要夹掉，否则会画出边界之外。
        assert_eq!(
            scissor_rect(
                Rect::new(50.0, 50.0, 500.0, 500.0),
                (0.0, 0.0),
                1.0,
                1.0,
                surface
            ),
            Some([50, 50, 50, 50])
        );
    }

    #[test]
    fn push_rect_writes_twelve_floats() {
        let mut out = Vec::new();
        push_rect(
            &mut out,
            Rect::new(1.0, 2.0, 3.0, 4.0),
            Color::BLACK,
            Corners::uniform(2.0),
            1.0,
        );
        assert_eq!(out.len(), 48);
        let first = f32::from_ne_bytes([out[0], out[1], out[2], out[3]]);
        assert_eq!(first, 1.0);
    }

    #[test]
    fn push_glyph_writes_twelve_floats() {
        let mut out = Vec::new();
        push_glyph(
            &mut out,
            Rect::new(0.0, 0.0, 4.0, 4.0),
            [0.0, 0.0, 0.1, 0.1],
            [1.0, 1.0, 1.0, 1.0],
        );
        assert_eq!(out.len(), 48);
    }

    #[test]
    fn solid_border_makes_four_rects() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Edges::uniform(2.0),
            Color::BLACK,
            LineStyle::Solid,
        );
        assert_eq!(count, 4);
        assert_eq!(out.len(), 4 * 48);
    }

    #[test]
    fn zero_width_border_makes_nothing() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Edges::default(),
            Color::BLACK,
            LineStyle::Solid,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn double_border_makes_eight_rects() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Edges::uniform(3.0),
            Color::BLACK,
            LineStyle::Double,
        );
        assert_eq!(count, 8);
    }

    #[test]
    fn dashed_border_splits_into_segments() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Edges::uniform(2.0),
            Color::BLACK,
            LineStyle::Dashed,
        );
        // 一段 6 像素加 2 像素间隔，100 像素的边上有十几段。
        assert!(count > 20, "实际 {count} 段");
    }

    #[test]
    fn dotted_border_segments_are_square() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Edges::uniform(4.0),
            Color::BLACK,
            LineStyle::Dotted,
        );
        // 点长与间隔都是 4 像素，步长 8。上边横跨整宽 40，得 5 个点；
        // 另外三条边要扣掉两端边框的宽度，只剩 32，各得 4 个点。
        assert_eq!(count, 5 + 4 + 4 + 4);
    }

    #[test]
    fn empty_rect_makes_no_border() {
        let mut out = Vec::new();
        let count = push_border(
            &mut out,
            Rect::new(0.0, 0.0, 0.0, 0.0),
            Edges::uniform(2.0),
            Color::BLACK,
            LineStyle::Solid,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn instance_stride_matches_shader_layout() {
        // 着色器按每实例 12 个 f32 读取，两处必须一致。
        let mut out = Vec::new();
        push_rect(
            &mut out,
            Rect::default(),
            Color::BLACK,
            Corners::default(),
            1.0,
        );
        assert_eq!(out.len() % 48, 0);
        assert_eq!(out.len() / 48, 1);
    }
}
