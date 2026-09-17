//! 离屏渲染验证：把一页渲染到纹理，再把像素读回来核对。
//!
//! 这个例子不需要窗口，能在没有显示服务的环境里跑，用来确认着色器、
//! 图集与批次划分真的产出了正确的像素。
//!
//! 运行：`cargo run -p ysu --example render_headless`

use ysu::Engine;
use ysu::render::Renderer;

/// 渲染目标的尺寸，物理像素。
const WIDTH: u32 = 400;
const HEIGHT: u32 = 300;

/// 演示用的页面，颜色都是纯色，方便核对像素。
const PAGE: &str = r#"<!DOCTYPE html>
<html><head><style>
  body { margin: 0; }
  .bar { background: #ff0000; height: 60px; }
  .box { background: #0000ff; width: 100px; height: 40px; margin-left: 20px; margin-top: 10px; }
</style></head>
<body>
  <div class="bar"></div>
  <div class="box"></div>
  <p style="margin: 0; color: #00ff00; font-size: 20px">文字</p>
</body></html>"#;

/// 程序入口。
fn main() -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|error| format!("找不到图形适配器：{error}"))?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("离屏渲染设备"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|error| format!("创建设备失败：{error}"))?;

    let info = adapter.get_info();
    println!("适配器：{}（{:?}）", info.name, info.backend);

    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("离屏目标"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut engine = Engine::new(f64::from(WIDTH), f64::from(HEIGHT));
    engine.load_html(PAGE);
    let list = engine.display_list();
    let (rects, borders, texts, _) = list.summary();
    println!(
        "绘制命令 {} 条：矩形 {rects} 边框 {borders} 文字 {texts}",
        list.len()
    );

    let mut renderer = Renderer::new(&device, &queue, format, WIDTH, HEIGHT);
    let measurer = engine.layout_engine_mut().measurer_mut();
    renderer.render(&device, &queue, &view, &list, measurer);

    let pixels = read_back(&device, &queue, &target, WIDTH, HEIGHT)?;
    report(&pixels);

    let failures = check(&pixels);
    if failures.is_empty() {
        println!("像素核对通过");
        Ok(())
    } else {
        for failure in &failures {
            eprintln!("像素核对失败：{failure}");
        }
        Err(format!("{} 项不符", failures.len()))
    }
}

/// 把渲染结果读回内存。
///
/// 纹理拷贝每行要按 256 字节对齐，所以缓冲区比实际像素宽。
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    const ALIGNMENT: u32 = 256;
    let unpadded = width * 4;
    let padded = unpadded.div_ceil(ALIGNMENT) * ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("回读缓冲"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("回读编码器"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| format!("等待回读失败：{error}"))?;
    receiver
        .recv()
        .map_err(|error| format!("回读通道断开：{error}"))?
        .map_err(|error| format!("映射缓冲失败：{error}"))?;

    let data = slice
        .get_mapped_range()
        .map_err(|error| format!("读取映射内存失败：{error}"))?;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    buffer.unmap();
    Ok(pixels)
}

/// 取某个像素的 RGB。
fn pixel(pixels: &[u8], x: u32, y: u32) -> (u8, u8, u8) {
    let index = ((y * WIDTH + x) * 4) as usize;
    (pixels[index], pixels[index + 1], pixels[index + 2])
}

/// 打印几个关键位置的采样值，便于人工核对。
fn report(pixels: &[u8]) {
    for (name, x, y) in [
        ("顶部横条（应为红）", 200, 20),
        ("横条下方（应为白）", 200, 100),
        ("蓝色方块（应为蓝）", 60, 90),
        ("方块右侧（应为白）", 200, 90),
    ] {
        let (r, g, b) = pixel(pixels, x, y);
        println!("  {name}: #{r:02x}{g:02x}{b:02x} @ ({x},{y})");
    }
}

/// 核对关键位置的像素，返回不符项。
fn check(pixels: &[u8]) -> Vec<String> {
    let mut failures = Vec::new();

    let (r, g, b) = pixel(pixels, 200, 20);
    if r < 200 || g > 60 || b > 60 {
        failures.push(format!("顶部横条应当是红色，实际 #{r:02x}{g:02x}{b:02x}"));
    }

    let (r, g, b) = pixel(pixels, 60, 90);
    if b < 200 || r > 60 || g > 60 {
        failures.push(format!("蓝色方块应当是蓝色，实际 #{r:02x}{g:02x}{b:02x}"));
    }

    let (r, g, b) = pixel(pixels, 390, 290);
    if r < 240 || g < 240 || b < 240 {
        failures.push(format!("空白处应当是白色，实际 #{r:02x}{g:02x}{b:02x}"));
    }

    // 绿色文字所在的行里应当能找到明显偏绿的像素。
    let mut found_text = false;
    for y in 120..180 {
        for x in 0..200 {
            let (r, g, b) = pixel(pixels, x, y);
            if g > 120 && r < 120 && b < 120 {
                found_text = true;
                break;
            }
        }
        if found_text {
            break;
        }
    }
    if !found_text {
        failures.push("文字区域里没有找到绿色像素".to_string());
    }

    failures
}
