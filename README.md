# YSU

网页渲染内核。把 HTML 与 CSS 变成可以交给 GPU 绘制的显示列表，不依赖系统自带的网页控件。

## 数据怎么流动

一条单向管线，每一步的产物是下一步的输入：

```
html::Tokenizer    →  记号流
html::TreeBuilder  →  节点树
css::Parser        →  样式表
style::Cascade     →  每个节点的计算样式
layout::Tree       →  带尺寸和位置的盒子树
paint::DisplayList →  绘制命令
render::Renderer   →  wgpu 的绘制调用
```

中间每一步都能单独调用，测试与排查都从这里入手。渲染器只负责把显示列表翻译成绘制
调用，换渲染后端只动 `render/` 一个模块。

内核不创建 GPU 设备，也不生产文档。设备与队列由调用方建好后传进来，这样内核可以
脱离窗口单独跑；页面内容也一律由调用方给出——起始页、空白页、出错提示都属于外壳，
内核只解析递给它的那一份。

## 构建与测试

需要 Rust 1.98 或更高，工具链由 `rust-toolchain.toml` 固定。

```bash
cargo build --release
cargo test
```

看渲染结果不用开窗口：

```bash
# 渲染对照页并导出成图片
cargo run --example render_demo -- /tmp/frame.ppm

# 完整走一遍加载流程：取页面、取它引用的样式表、应用、渲染
cargo run --example render_real_page -- https://example.com/ /tmp/out.ppm
```

## 合规现状

判据用的是 [html5lib-tests](https://github.com/html5lib/html5lib-tests) 的树构建
用例——各浏览器与 Servo 用的同一份，不是为本项目定制的。用例收在
`tests/html5lib/`，来源与许可见那里的 README。

```bash
cargo test --test html5lib_tree -- --nocapture
```

| | |
| --- | --- |
| 用例总数 | 1792 |
| 通过 | 1368 |
| 失败 | 424 |
| 其中片段解析 | 192 条，通过 159 |

失败详情写在 `CARGO_TARGET_TMPDIR/html5lib-failures.txt`。

**这个测试现在是红的，它报的就是真实差距。** 已知缺口：

- `template` 的内容没有单独建模（约 111 条）。规范要求模板内容放在一个独立的片段
  里，导出时写成一个 `content` 节点；现在模板的子节点直接挂在元素下面。
- 格式化元素的收养机构算法还有边角没对齐（约 137 条），多数是同一个名字的格式化
  元素层层嵌套的情形。
- 表格与寄养（约 40 条）、`select`（约 21 条）。
- 样式表那边还有一批：`calc()` 与 `var()` 没做，`@import`、`@supports`、`@page`
  没接，`position` 与 `float` 解析进了计算样式但没有布局侧的消费方。
- 网页脚本不执行，`<script>` 会被解析进 DOM 但没人跑它。

## 许可

Apache License 2.0，全文见 [LICENSE](LICENSE)。

Copyright © 2026 Nexsteaduser. All Rights Reserved.
