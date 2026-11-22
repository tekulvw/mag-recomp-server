//! Random secrets and the codes people type.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};

/// Letters people can read back and type without confusing them (RFC 8628, section 6.1).
const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
pub const USER_CODE_LEN: usize = 8;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::rng().fill_bytes(&mut out);
    out
}

pub fn b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// A 256-bit secret with a readable prefix, and the hash the database stores in its place.
pub fn new_secret(prefix: &str) -> (String, [u8; 32]) {
    let token = format!("{prefix}{}", b64url(&random_bytes::<32>()));
    let hash = sha256(token.as_bytes());
    (token, hash)
}

/// A fresh device-link code, normalized (no separator).
pub fn new_user_code() -> String {
    let mut rng = rand::rng();
    (0..USER_CODE_LEN)
        .map(|_| {
            // 256 is not a multiple of 20; reject the top values to keep the choice uniform.
            loop {
                let b = rng.next_u32() as u8;
                if (b as usize) < 240 {
                    break USER_CODE_ALPHABET[b as usize % 20] as char;
                }
            }
        })
        .collect()
}

/// What a person typed, as a code to look up: letters uppercased, everything else dropped.
/// Returns `None` unless exactly eight letters from the alphabet remain.
pub fn normalize_user_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == USER_CODE_LEN && code.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)))
        .then_some(code)
}

/// `BCDF-GHJK` for display.
pub fn format_user_code(code: &str) -> String {
    if code.len() == USER_CODE_LEN {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes() {
        for _ in 0..200 {
            let code = new_user_code();
            assert_eq!(normalize_user_code(&code).as_deref(), Some(code.as_str()));
            let shown = format_user_code(&code);
            assert_eq!(normalize_user_code(&shown.to_lowercase()), Some(code));
        }
        assert_eq!(normalize_user_code("bcdf ghjk"), Some("BCDFGHJK".into()));
        assert_eq!(normalize_user_code("BCDF-GHJA"), None);
        assert_eq!(normalize_user_code("BCDFGHJ"), None);
    }

    #[test]
    fn secrets() {
        let (a, ha) = new_secret("mgd_");
        let (b, _) = new_secret("mgd_");
        assert!(a.starts_with("mgd_") && a.len() == 4 + 43);
        assert_ne!(a, b);
        assert_eq!(sha256(a.as_bytes()), ha);
    }
}
