//! Content detection for the OfficeDoc row of DESIGN.md 3.3.

use crate::{package, OfficeError, OfficeKind};

/// Decide from content whether `bytes` are an Office package and which
/// kind. `Ok(None)` for anything that is not one (including plain ZIPs).
/// An OLE compound file (`D0 CF 11 E0 ...`) is how OOXML is wrapped when a
/// password is set, so it is reported as [`OfficeError::Encrypted`].
///
/// Only the central directory and one small entry (`[Content_Types].xml`
/// or `mimetype`) are read; the rest of the package stays compressed.
pub fn detect(bytes: &[u8]) -> Result<Option<OfficeKind>, OfficeError> {
    Ok(package::open(bytes)?.map(|(_, kind)| kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_zip_is_none_and_cfb_is_encrypted() {
        assert_eq!(detect(b"%PDF-1.4").unwrap(), None);
        assert_eq!(detect(b"").unwrap(), None);
        let mut cfb = package::CFB_MAGIC.to_vec();
        cfb.extend_from_slice(&[0; 504]);
        assert_eq!(detect(&cfb), Err(OfficeError::Encrypted));
    }
}
