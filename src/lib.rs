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

/// 内核版本，来自仓库根的 `version.toml`。
pub const ENGINE_VERSION: &str = env!("YSU_KERNEL_VERSION");

#[cfg(test)]
mod tests {
    use super::ENGINE_VERSION;

    /// 内核版本号必须真的来自清单，而不是悄悄退回了 Cargo 里的号。
    ///
    /// `build.rs` 在清单里找不到那个键时会安静地退回 `CARGO_PKG_VERSION`，
    /// 界面上看不出来——版本号照样有，只是错的。这条把它拦下来。
    ///
    /// 清单不在时（这个 crate 被单独拿出去发布）跳过：那种情况下退回 Cargo
    /// 的版本号正是预期行为。
    #[test]
    fn version_comes_from_the_manifest() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../version.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            return;
        };
        let declared = text.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == "kernel_version").then(|| value.trim().trim_matches('"').to_string())
        });
        assert_eq!(
            declared.as_deref(),
            Some(ENGINE_VERSION),
            "清单里的 kernel_version 与嵌入程序的 ENGINE_VERSION 对不上"
        );
    }
}
