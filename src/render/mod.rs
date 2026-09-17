//! 渲染：把显示列表交给 GPU。
//!
//! 渲染器不创建 GPU 设备，`Device` 与 `Queue` 由外壳传进来，这样内核可以
//! 脱离窗口单独测试，设备的生命周期也只归外壳管。

pub mod atlas;
pub mod renderer;
pub mod shaders;

pub use atlas::{AtlasEntry, DEFAULT_ATLAS_SIZE, GlyphAtlas};
pub use renderer::Renderer;
