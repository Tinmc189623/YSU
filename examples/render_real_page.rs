//! 取一个真实网站，走完解析、样式、布局、绘制，并报告结果。
//!
//! 用来验证内核面对真实世界的 HTML 与 CSS 不会崩、能排出东西。
//!
//! 运行：`cargo run -p ysu --example render_real_page -- https://example.com/`

use std::time::Instant;
use ysu::Engine;
use ysu::net::{Client, decode};

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com/".to_string());

    let client = Client::new();
    let started = Instant::now();
    let response = match client.fetch(&url) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("取回失败：{error}");
            std::process::exit(1);
        }
    };
    let fetch_time = started.elapsed();

    let content_type = response.header("content-type").map(str::to_string);
    let html = decode(&response.body, content_type.as_deref());
    println!(
        "取回 {} ：{} {}，{} 字节，耗时 {:?}",
        response.url,
        response.status,
        response.reason,
        response.body.len(),
        fetch_time
    );

    let (width, height) = (1280.0, 800.0);
    let mut engine = Engine::new(width, height);

    let started = Instant::now();
    engine.load_html_with_base(&html, response.url.as_str());
    let layout_time = started.elapsed();

    // 页面引用的外部样式表要自己去取，取回来再重新排版。
    let links = engine.stylesheet_links();
    if !links.is_empty() {
        println!("外部样式表 {} 份", links.len());
    }
    for url in &links {
        match client.fetch(url) {
            Ok(sheet) => {
                let content_type = sheet.header("content-type").map(str::to_string);
                let css = decode(&sheet.body, content_type.as_deref());
                println!("  取回 {url}：{} 字节", sheet.body.len());
                engine.set_linked_stylesheet(url, &css);
            }
            Err(error) => println!("  取回 {url} 失败：{error}"),
        }
    }

    let started = Instant::now();
    let list = engine.display_list();
    let paint_time = started.elapsed();

    let (rects, borders, texts, images) = list.summary();
    println!("样式表 {} 份", engine.stylesheets().len());
    println!("文档高度 {:.0}", engine.document_height());
    println!("布局耗时 {layout_time:?}，绘制耗时 {paint_time:?}");
    println!(
        "绘制命令 {} 条：矩形 {rects} 边框 {borders} 文字 {texts} 图片 {images}",
        list.len()
    );

    // 把文字命令连同行位置打出来，确认断行位置合理、没有丢空格。
    println!("文字片段（最多 12 条）：");
    for command in list.commands.iter().take(40) {
        if let ysu::paint::DrawCommand::Text { rect, text, .. } = command {
            println!("  y={:>6.1} x={:>6.1} {:?}", rect.y, rect.x, text);
        }
    }

    if list.is_empty() {
        eprintln!("没有产出任何绘制命令");
        std::process::exit(1);
    }
}
