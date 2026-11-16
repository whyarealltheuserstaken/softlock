//! Just enough of a PNG to read the size `screencapture` wrote.

pub fn png_size(bytes: &[u8]) -> Option<(i32, i32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0 || height == 0 || width > 100_000 || height > 100_000 {
        return None;
    }
    Some((width as i32, height as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn ihdr_size_is_read() {
        assert_eq!(png_size(&header(1024, 768)), Some((1024, 768)));
    }

    #[test]
    fn a_truncated_or_empty_image_is_rejected() {
        assert!(png_size(b"\x89PNG\r\n\x1a\n").is_none());
        assert!(png_size(&header(0, 10)).is_none());
        assert!(png_size(b"not a png............").is_none());
    }
}
