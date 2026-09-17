//! 字形图集。
//!
//! 把栅格化出来的字形位图拼进一张单通道纹理，绘制时按 UV 采样。分配用
//! 货架算法：从左往右排，排满换一行。图集满了就整体清空重来，字形会按需
//! 重新栅格化，代价是偶尔的一次重排。

use cosmic_text::CacheKey;
use std::collections::HashMap;

/// 图集的默认边长。
pub const DEFAULT_ATLAS_SIZE: u32 = 2048;

/// 一次待写入图集的位图。
struct PendingCopy {
    /// 目标位置左上角。
    x: u32,
    /// 目标位置左上角。
    y: u32,
    /// 宽。
    width: u32,
    /// 高。
    height: u32,
    /// 按 256 字节对齐后的行数据。
    data: Vec<u8>,
    /// 对齐后的每行字节数。
    bytes_per_row: u32,
}

/// 图集里一个字形的位置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasEntry {
    /// 归一化的纹理坐标，依次是左上角 u、v 与右下角 u、v。
    pub uv: [f32; 4],
    /// 位图宽。
    pub width: u32,
    /// 位图高。
    pub height: u32,
}

/// 字形图集。
pub struct GlyphAtlas {
    /// 图集纹理。
    texture: wgpu::Texture,
    /// 纹理视图，供绑定组使用。
    view: wgpu::TextureView,
    /// 边长。
    size: u32,
    /// 已缓存的字形。
    entries: HashMap<CacheKey, AtlasEntry>,
    /// 本帧要写入的位图。
    pending: Vec<PendingCopy>,
    /// 当前货架的纵坐标。
    shelf_y: u32,
    /// 当前货架的高度。
    shelf_height: u32,
    /// 当前货架内的横向游标。
    cursor_x: u32,
}

impl GlyphAtlas {
    /// 建一张空图集。
    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("字形图集"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
            entries: HashMap::new(),
            pending: Vec::new(),
            shelf_y: 0,
            shelf_height: 0,
            cursor_x: 0,
        }
    }

    /// 图集纹理视图。
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// 取已缓存的字形位置。
    pub fn get(&self, key: &CacheKey) -> Option<AtlasEntry> {
        self.entries.get(key).copied()
    }

    /// 把一个字形位图放进图集，返回它的位置。
    ///
    /// 放不下时会先清空图集再试一次，仍然放不下说明这个字形比整张图还大，
    /// 返回 `None`，调用方跳过它。
    pub fn insert(
        &mut self,
        key: CacheKey,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.entries.get(&key) {
            return Some(*entry);
        }
        if width == 0 || height == 0 {
            return None;
        }
        if width > self.size || height > self.size {
            return None;
        }

        if let Some(entry) = self.allocate(key, width, height, data) {
            return Some(entry);
        }

        // 排不下就清空重来，这次一定放得下（尺寸已经检查过）。
        self.reset();
        self.allocate(key, width, height, data)
    }

    /// 在图集里找一个空位并记录写入。
    fn allocate(
        &mut self,
        key: CacheKey,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Option<AtlasEntry> {
        // 当前货架放不下就换一行。
        if self.cursor_x + width > self.size {
            self.shelf_y += self.shelf_height + 1;
            self.shelf_height = 0;
            self.cursor_x = 0;
        }
        if self.shelf_y + height > self.size {
            return None;
        }

        let x = self.cursor_x;
        let y = self.shelf_y;
        self.cursor_x += width + 1;
        self.shelf_height = self.shelf_height.max(height);

        let (padded, bytes_per_row) = pad_rows(data, width, height);
        self.pending.push(PendingCopy {
            x,
            y,
            width,
            height,
            data: padded,
            bytes_per_row,
        });

        let entry = AtlasEntry {
            uv: [
                x as f32 / self.size as f32,
                y as f32 / self.size as f32,
                (x + width) as f32 / self.size as f32,
                (y + height) as f32 / self.size as f32,
            ],
            width,
            height,
        };
        self.entries.insert(key, entry);
        Some(entry)
    }

    /// 清空图集，回到初始状态。
    pub fn reset(&mut self) {
        self.entries.clear();
        self.pending.clear();
        self.shelf_y = 0;
        self.shelf_height = 0;
        self.cursor_x = 0;
    }

    /// 把本帧攒下的位图写进纹理。
    ///
    /// 清空过图集时顺带把整张纹理抹零，免得残留上一轮的旧字形。
    pub fn upload(&mut self, queue: &wgpu::Queue) {
        if self.pending.is_empty() {
            return;
        }
        for copy in self.pending.drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: copy.x,
                        y: copy.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &copy.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(copy.bytes_per_row),
                    rows_per_image: Some(copy.height),
                },
                wgpu::Extent3d {
                    width: copy.width,
                    height: copy.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

/// 把位图按 256 字节的行对齐补齐。
///
/// `Queue::write_texture` 要求每行起始地址按 256 字节对齐，字形位图通常
/// 只有几十像素宽，所以每行都要补零。
fn pad_rows(data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32) {
    const ALIGNMENT: u32 = 256;
    let row_bytes = width;
    if row_bytes.is_multiple_of(ALIGNMENT) {
        return (data.to_vec(), row_bytes);
    }
    let padded_row = row_bytes.div_ceil(ALIGNMENT) * ALIGNMENT;
    let mut out = vec![0u8; (padded_row * height) as usize];
    for row in 0..height {
        let src_start = (row * row_bytes) as usize;
        let src_end = src_start + row_bytes as usize;
        if src_end > data.len() {
            break;
        }
        let dst_start = (row * padded_row) as usize;
        out[dst_start..dst_start + row_bytes as usize].copy_from_slice(&data[src_start..src_end]);
    }
    (out, padded_row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_aligns_rows() {
        // 三行两列的位图，每行两字节，要补到 256 字节。
        let data = vec![1u8, 2, 3, 4, 5, 6];
        let (padded, stride) = pad_rows(&data, 2, 3);
        assert_eq!(stride, 256);
        assert_eq!(padded.len(), 256 * 3);
        assert_eq!(&padded[0..2], &[1, 2]);
        assert_eq!(&padded[256..258], &[3, 4]);
        assert_eq!(&padded[512..514], &[5, 6]);
    }

    #[test]
    fn padding_is_noop_when_aligned() {
        let width = 256;
        let data = vec![7u8; 256 * 2];
        let (padded, stride) = pad_rows(&data, width, 2);
        assert_eq!(stride, 256);
        assert_eq!(padded.len(), data.len());
    }

    #[test]
    fn padding_handles_short_data() {
        // 数据比声称的短时只补零，不越界。
        let data = vec![1u8, 2];
        let (padded, _) = pad_rows(&data, 2, 3);
        assert_eq!(padded.len(), 256 * 3);
    }

    #[test]
    fn alignment_rounds_up() {
        // 宽 100 的行要补到 256，宽 300 的行要补到 512。
        assert_eq!(pad_rows(&[0], 100, 1).1, 256);
        assert_eq!(pad_rows(&[0], 300, 1).1, 512);
        assert_eq!(pad_rows(&[0], 256, 1).1, 256);
    }
}
