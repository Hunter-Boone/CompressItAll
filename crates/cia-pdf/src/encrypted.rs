//! Detect an `/Encrypt` entry without trusting the parser: lopdf removes the
//! key from the trailer once it has (or has not) managed to decrypt, and may
//! refuse to load a file whose encryption dictionary is nonsense. Scanning the
//! raw bytes outside stream bodies catches both.

/// True if a `/Encrypt` name appears anywhere outside `stream ... endstream`
/// bodies. The name only legitimately occurs as a key of the trailer or of a
/// cross-reference stream dictionary, both of which live outside stream data.
pub fn declares_encrypt(bytes: &[u8]) -> bool {
    let needle = b"/Encrypt";
    let mut in_stream = false;
    let mut i = 0;
    while i < bytes.len() {
        if in_stream {
            if bytes[i..].starts_with(b"endstream") {
                in_stream = false;
                i += 9;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(needle) {
            let after = bytes.get(i + needle.len()).copied();
            if after.is_none_or(|c| !is_regular(c)) {
                return true;
            }
            i += needle.len();
            continue;
        }
        if bytes[i..].starts_with(b"stream") && (i == 0 || !is_regular(bytes[i - 1])) {
            let next = bytes.get(i + 6).copied();
            if matches!(next, Some(b'\r') | Some(b'\n')) {
                in_stream = true;
                i += 6;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// PDF "regular" character: anything that is not whitespace or a delimiter.
fn is_regular(c: u8) -> bool {
    !matches!(
        c,
        b'\0'
            | b'\t'
            | b'\n'
            | b'\x0c'
            | b'\r'
            | b' '
            | b'('
            | b')'
            | b'<'
            | b'>'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'/'
            | b'%'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_trailer_key() {
        assert!(declares_encrypt(
            b"trailer\n<< /Size 3 /Encrypt 5 0 R /Root 1 0 R >>"
        ));
        assert!(declares_encrypt(
            b"trailer\n<</Encrypt<</Filter/Standard>>/Root 1 0 R>>"
        ));
    }

    #[test]
    fn ignores_metadata_flag_and_stream_bodies() {
        assert!(!declares_encrypt(b"<< /EncryptMetadata false >>"));
        assert!(!declares_encrypt(
            b"4 0 obj\n<< /Length 9 >>\nstream\n/Encrypt \nendstream\nendobj"
        ));
        assert!(!declares_encrypt(b"%PDF-1.4 nothing here"));
    }
}
