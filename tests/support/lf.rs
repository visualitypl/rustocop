//! RuboCop writes a corrected file with LF line endings throughout.

/// `bytes` with every `\r\n` turned into `\n`: what a corrected file holds.
pub fn lf(bytes: impl AsRef<[u8]>) -> Vec<u8> {
    let bytes = bytes.as_ref();
    (0..bytes.len())
        .filter(|&i| !(bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n')))
        .map(|i| bytes[i])
        .collect()
}

/// [`lf`] for text.
pub fn lf_str(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The expected file after `--fix`: [`lf`] when a correction changed
/// `before`, else `before`'s bytes unchanged.
pub fn fixed(before: impl AsRef<[u8]>, after: impl AsRef<[u8]>) -> Vec<u8> {
    if before.as_ref() == after.as_ref() {
        after.as_ref().to_vec()
    } else {
        lf(after)
    }
}
