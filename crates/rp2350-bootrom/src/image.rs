//! Synthetic boot ROM image construction
//! 合成 Boot ROM 镜像构建

// 镜像构建器按固定偏移写入字段，越界属于布局常量写错，直接 panic 更容易定位。
// The image builder writes fields at fixed offsets; an out-of-range write means a
// layout constant is wrong, so panicking surfaces the mistake immediately.
#![allow(clippy::missing_panics_doc)]

/// Mutable image buffer with little-endian field helpers
/// 带小端字段辅助方法的可变镜像缓冲区
#[derive(Debug)]
pub struct RomImageBuilder {
    bytes: Vec<u8>,
}

impl RomImageBuilder {
    /// Create a zeroed image of `size` bytes / 创建一个 `size` 字节的清零镜像
    #[must_use]
    pub fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
        }
    }

    /// Write a little-endian `u16` / 写入小端 `u16`
    pub fn write_u16(&mut self, offset: usize, value: u16) {
        self.bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian `u32` / 写入小端 `u32`
    pub fn write_u32(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Copy a byte slice into the image / 将字节切片复制进镜像
    pub fn write_slice(&mut self, offset: usize, bytes: &[u8]) {
        self.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// Write a Thumb `udf #id` trap stub / 写入 Thumb `udf #id` 陷阱桩
    ///
    /// The host dispatcher keys on the stub address, so the immediate is only a
    /// debugging aid when no dispatcher is installed.
    /// 宿主分发器以桩地址为键，因此立即数仅在没有分发器时用作调试线索。
    pub fn trap_stub(&mut self, offset: usize, id: u16) {
        self.write_u16(offset, 0xDE00 | (id & 0xFF));
    }

    /// Consume the builder and return the image bytes / 消耗构建器并返回镜像字节
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_little_endian() {
        let mut image = RomImageBuilder::new(16);
        image.write_u32(0, 0x0002_754D);
        image.write_u16(8, 0x0075);
        image.trap_stub(12, 7);
        let bytes = image.finish();
        assert_eq!(&bytes[..4], &[0x4D, 0x75, 0x02, 0x00]);
        assert_eq!(&bytes[8..10], &[0x75, 0x00]);
        assert_eq!(&bytes[12..14], &[0x07, 0xDE]);
    }
}
