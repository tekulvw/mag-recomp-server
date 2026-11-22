//! Player names, as the game's sign-in field accepts them.

pub const MIN_LEN: usize = 3;
pub const MAX_LEN: usize = 15;

/// 3 to 15 ASCII letters, digits, `-` or `_`, starting with a letter.
pub fn valid_player_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (MIN_LEN..=MAX_LEN).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}

/// The key names are unique under: the game server keys accounts by lowercase name.
pub fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule() {
        for ok in ["abc", "Alpha_Ranger-7", "z12345678901234"] {
            assert!(valid_player_name(ok), "{ok}");
        }
        for bad in [
            "",
            "ab",
            "1abc",
            "_abc",
            "abc def",
            "abcdefghijklmnop",
            "näme",
            "abc!",
        ] {
            assert!(!valid_player_name(bad), "{bad}");
        }
        assert_eq!(name_key("Alpha_Ranger"), "alpha_ranger");
    }
}
