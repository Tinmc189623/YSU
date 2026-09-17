//! # YSU
//!
//! YSU 是网页渲染内核，负责把 HTML 与 CSS 变成可以交给 GPU 绘制的
//! 显示列表。
//!
//! 数据沿一条单向管线流动，每一步的产物是下一步的输入：
//!
//! ```text
//! html::Tokenizer   →  记号流
//! dom::TreeBuilder  →  节点树
//! css::Parser       →  样式表
//! style::Cascade    →  每个节点的计算样式
//! layout::Tree      →  带尺寸和位置的盒子树
//! paint::DisplayList →  绘制命令
//! ```
//!
//! 内核不创建 GPU 设备。渲染器由外壳拿到设备后构造，把 `Device` 和
//! `Queue` 交给 YSU 使用。

pub mod address;
pub mod css;
pub mod dom;
pub mod engine;
pub mod html;
pub mod layout;
pub mod net;
pub mod paint;
pub mod render;
pub mod style;

pub use engine::Engine;

/// 内核名称。
pub const ENGINE_NAME: &str = "YSU";

/// 内核版本。
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
