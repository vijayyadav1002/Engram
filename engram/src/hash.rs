use crate::error::Error;
use std::path::Path;

/// Lowercase hex blake3 digest of `bytes`.
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Hash file contents with blake3; IO errors map to [`Error::Io`].
pub fn blake3_file(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path)?;
    Ok(blake3_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_stable_hex() {
        assert_eq!(blake3_hex(b"hi").len(), 64);
        assert_eq!(blake3_hex(b"hi"), blake3_hex(b"hi"));
        assert_ne!(blake3_hex(b"hi"), blake3_hex(b"ho"));
    }
}
