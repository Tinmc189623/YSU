//! 端到端演示：一段 HTML 走完解析、样式、布局、绘制。
//!
//! 运行：`cargo run -p ysu --example render_demo`

use ysu::Engine;

/// 演示用的页面，覆盖常见的排版场景。
const PAGE: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>演示</title>
<style>
  body { margin: 0; font-family: sans-serif; line-height: 1.5; }
  header { background: #2b6cb0; color: white; padding: 16px; }
  h1 { margin: 0; font-size: 24px; }
  main { padding: 20px; }
  .card { border: 1px solid #cbd5e0; padding: 12px; margin-bottom: 12px; }
  .row { display: flex; gap: 10px; }
  .box { flex: 1; background: #edf2f7; padding: 10px; text-align: center; }
  footer { background: #1a202c; color: white; padding: 10px; text-align: center; }
</style></head>
<body>
  <header><h1>YSU 渲染演示</h1></header>
  <main>
    <div class="card"><p>这是一段普通的段落文字，用来验证排版、断行与绘制命令是否正确。</p></div>
    <div class="row">
      <div class="box">左</div>
      <div class="box">中</div>
      <div class="box">右</div>
    </div>
    <ul><li>列表项一</li><li>列表项二</li></ul>
  </main>
  <footer>页脚</footer>
</body></html>"#;

/// 程序入口。
fn main() {
    let mut engine = Engine::new(900.0, 700.0);
    engine.load_html(PAGE);

    let list = engine.display_list();
    let (rects, borders, texts, images) = list.summary();

    println!("文档高度 {:.1}", engine.document_height());
    println!("样式表 {} 份", engine.stylesheets().len());
    println!(
        "绘制命令 {} 条：矩形 {rects} 边框 {borders} 文字 {texts} 图片 {images}",
        list.len()
    );
    println!("文字内容：{}", list.all_text());
}
