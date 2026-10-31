//! SYNTHETIC public bot identities. No game catalog or real-person list is embedded.
use std::sync::OnceLock;
pub const BOT_NAME_COUNT: usize = 256;
/// No real-person identities are distributed in the public package.
pub const ZIPPER_NAME_COUNT: usize = 0;
pub const MAX_NAME_BYTES: usize = 32;
fn names() -> &'static Vec<String> {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (0..BOT_NAME_COUNT)
            .map(|i| {
                format!(
                    "Synthetic Bot {}{}",
                    (b'A' + (i / 26) as u8) as char,
                    (b'A' + (i % 26) as u8) as char
                )
            })
            .collect()
    })
}
/// Stable slot identity; unsupported slots never wrap.
pub fn name_for(index: u16) -> Option<&'static str> {
    names().get(usize::from(index)).map(String::as_str)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_names_are_unique_stable_and_bounded() {
        let names: std::collections::BTreeSet<_> = (0..BOT_NAME_COUNT as u16)
            .map(|i| name_for(i).unwrap())
            .collect();
        assert_eq!(names.len(), BOT_NAME_COUNT);
        for i in 0..=u16::MAX {
            assert_eq!(name_for(i).is_some(), usize::from(i) < BOT_NAME_COUNT);
            assert_eq!(name_for(i), name_for(i));
        }
        assert!(names.iter().all(|n| n.len() <= MAX_NAME_BYTES
            && n.bytes().all(|b| b.is_ascii_alphabetic() || b == b' ')));
    }
}
