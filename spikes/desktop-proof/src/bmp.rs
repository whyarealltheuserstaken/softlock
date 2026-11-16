//! Uncompressed 32-bit BMP writer for proof artifacts.

use std::fs;
use std::path::Path;

use crate::verdict::BgraImage;

pub fn write_bmp(path: &Path, image: &BgraImage) -> std::io::Result<()> {
    let row_bytes = image.width.saturating_mul(4);
    let pixel_bytes = row_bytes.saturating_mul(image.height);
    if image.pixels.len() != pixel_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "image buffer does not match its dimensions",
        ));
    }
    let offset = 54u32;
    let file_size = offset.saturating_add(pixel_bytes as u32);
    let mut bytes = Vec::with_capacity(file_size as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&file_size.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(image.width as i32).to_le_bytes());
    bytes.extend_from_slice(&(image.height as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    bytes.extend_from_slice(&[0u8; 16]);
    for y in (0..image.height).rev() {
        let start = y * row_bytes;
        bytes.extend_from_slice(&image.pixels[start..start + row_bytes]);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::{BgraImage, Rgb};

    #[test]
    fn bmp_starts_with_the_bitmap_signature() {
        let image = BgraImage::solid(2, 2, Rgb { r: 1, g: 2, b: 3 });
        let path = std::env::temp_dir().join(format!("p7-bmp-{}-test.bmp", std::process::id()));
        write_bmp(&path, &image).expect("write bmp");
        let bytes = fs::read(&path).expect("read bmp");
        let _ = fs::remove_file(&path);
        assert_eq!(&bytes[..2], b"BM");
        assert!(bytes.len() > 54);
    }
}
