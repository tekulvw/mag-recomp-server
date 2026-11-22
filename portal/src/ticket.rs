//! Game tickets, version 1. The byte layout and verification rules are specified in
//! `docs/portal-ticket.md`; this module is the reference signer and verifier.

use std::collections::BTreeMap;

use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};

use crate::names;

pub const MAGIC: &[u8; 4] = b"MAGT";
pub const VERSION: u8 = 1;
pub const NONCE_LEN: usize = 16;
pub const SIGNATURE_LEN: usize = 64;
/// Fixed bytes: header, times, account, nonce, two length bytes and the signature.
pub const FIXED_LEN: usize = 4 + 1 + 1 + 8 + 8 + 4 + NONCE_LEN + 1 + 1 + SIGNATURE_LEN;
pub const MAX_LIFETIME_SECONDS: u64 = 3600;
pub const MAX_FUTURE_SKEW_SECONDS: u64 = 60;
pub const MAX_AUDIENCE_LEN: usize = 63;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketClaims {
    pub key_id: u8,
    pub issued_at: u64,
    pub expires_at: u64,
    pub account_id: u32,
    pub nonce: [u8; NONCE_LEN],
    pub audience: String,
    pub player_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    #[error("ticket is malformed")]
    Malformed,
    #[error("ticket field is out of range")]
    InvalidField,
    #[error("ticket key is unknown")]
    UnknownKey,
    #[error("ticket signature is invalid")]
    BadSignature,
    #[error("ticket is not yet valid")]
    NotYetValid,
    #[error("ticket has expired")]
    Expired,
    #[error("ticket is for another server")]
    WrongAudience,
}

/// A host name as the audience field allows it: 1 to 63 lowercase letters, digits, `.` and `-`.
pub fn valid_audience(audience: &str) -> bool {
    !audience.is_empty()
        && audience.len() <= MAX_AUDIENCE_LEN
        && audience
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

impl TicketClaims {
    fn check_fields(&self) -> Result<(), TicketError> {
        let lifetime = self.expires_at.checked_sub(self.issued_at);
        if self.account_id == 0
            || !valid_audience(&self.audience)
            || !names::valid_player_name(&self.player_name)
            || !matches!(lifetime, Some(1..=MAX_LIFETIME_SECONDS))
        {
            return Err(TicketError::InvalidField);
        }
        Ok(())
    }

    /// The bytes the signature covers.
    pub fn signed_bytes(&self) -> Result<Vec<u8>, TicketError> {
        self.check_fields()?;
        let mut out = Vec::with_capacity(FIXED_LEN + self.audience.len() + self.player_name.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.key_id);
        out.extend_from_slice(&self.issued_at.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out.extend_from_slice(&self.account_id.to_be_bytes());
        out.extend_from_slice(&self.nonce);
        out.push(self.audience.len() as u8);
        out.extend_from_slice(self.audience.as_bytes());
        out.push(self.player_name.len() as u8);
        out.extend_from_slice(self.player_name.as_bytes());
        Ok(out)
    }

    /// The complete ticket: signed bytes followed by a low-`s` P-256 signature.
    pub fn sign(&self, key: &SigningKey) -> Result<Vec<u8>, TicketError> {
        let mut out = self.signed_bytes()?;
        let signature: Signature = key.sign(&out);
        out.extend_from_slice(&signature.normalize_s().to_bytes());
        Ok(out)
    }
}

/// Verifying keys by key ID.
#[derive(Debug, Clone, Default)]
pub struct VerifyingKeys(BTreeMap<u8, VerifyingKey>);

impl VerifyingKeys {
    pub fn insert(&mut self, key_id: u8, key: VerifyingKey) {
        self.0.insert(key_id, key);
    }
    pub fn iter(&self) -> impl Iterator<Item = (&u8, &VerifyingKey)> {
        self.0.iter()
    }
}

fn split_ticket(bytes: &[u8]) -> Result<(TicketClaims, &[u8], &[u8]), TicketError> {
    let malformed = TicketError::Malformed;
    if bytes.len() < FIXED_LEN || &bytes[0..4] != MAGIC || bytes[4] != VERSION {
        return Err(malformed);
    }
    let u64_at = |at: usize| u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
    let audience_len = bytes[42] as usize;
    if !(1..=MAX_AUDIENCE_LEN).contains(&audience_len) {
        return Err(malformed);
    }
    let name_len_at = 43 + audience_len;
    let name_len = *bytes.get(name_len_at).ok_or(malformed)? as usize;
    if !(names::MIN_LEN..=names::MAX_LEN).contains(&name_len)
        || bytes.len() != FIXED_LEN + audience_len + name_len
    {
        return Err(malformed);
    }
    let text = |range: std::ops::Range<usize>| {
        std::str::from_utf8(&bytes[range])
            .map(str::to_owned)
            .map_err(|_| malformed)
    };
    let signed_len = bytes.len() - SIGNATURE_LEN;
    let claims = TicketClaims {
        key_id: bytes[5],
        issued_at: u64_at(6),
        expires_at: u64_at(14),
        account_id: u32::from_be_bytes(bytes[22..26].try_into().unwrap()),
        nonce: bytes[26..42].try_into().unwrap(),
        audience: text(43..name_len_at)?,
        player_name: text(name_len_at + 1..signed_len)?,
    };
    Ok((claims, &bytes[..signed_len], &bytes[signed_len..]))
}

/// Verifies everything except nonce reuse, which needs the caller's memory of accepted
/// nonces (rule 6 of the specification).
pub fn verify(
    bytes: &[u8],
    keys: &VerifyingKeys,
    audience: &str,
    now: u64,
) -> Result<TicketClaims, TicketError> {
    let (claims, signed, signature) = split_ticket(bytes)?;
    let key = keys.0.get(&claims.key_id).ok_or(TicketError::UnknownKey)?;
    let signature = Signature::from_slice(signature).map_err(|_| TicketError::BadSignature)?;
    key.verify(signed, &signature)
        .map_err(|_| TicketError::BadSignature)?;
    claims.check_fields()?;
    if claims.issued_at > now.saturating_add(MAX_FUTURE_SKEW_SECONDS) {
        return Err(TicketError::NotYetValid);
    }
    if claims.expires_at <= now {
        return Err(TicketError::Expired);
    }
    if claims.audience != audience {
        return Err(TicketError::WrongAudience);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn test_key() -> SigningKey {
        let seed = Sha256::digest(b"mag-portal ticket test vector key v1");
        SigningKey::from_slice(&seed).unwrap()
    }

    fn claims() -> TicketClaims {
        TicketClaims {
            key_id: 1,
            issued_at: 1_800_000_000,
            expires_at: 1_800_000_600,
            account_id: 42,
            nonce: *b"0123456789abcdef",
            audience: "play.example.test".into(),
            player_name: "Alpha_Ranger-7".into(),
        }
    }

    fn keys() -> VerifyingKeys {
        let mut keys = VerifyingKeys::default();
        keys.insert(1, *test_key().verifying_key());
        keys
    }

    #[test]
    fn round_trip_and_length() {
        let c = claims();
        let ticket = c.sign(&test_key()).unwrap();
        assert_eq!(
            ticket.len(),
            FIXED_LEN + c.audience.len() + c.player_name.len()
        );
        assert_eq!(ticket.len(), 108 + 17 + 14);
        let got = verify(&ticket, &keys(), "play.example.test", c.issued_at + 1).unwrap();
        assert_eq!(got, c);
    }

    #[test]
    fn every_flipped_bit_is_rejected() {
        let ticket = claims().sign(&test_key()).unwrap();
        for i in 0..ticket.len() {
            for bit in 0..8 {
                let mut bad = ticket.clone();
                bad[i] ^= 1 << bit;
                assert!(
                    verify(&bad, &keys(), "play.example.test", 1_800_000_001).is_err(),
                    "byte {i} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn time_audience_and_key_rules() {
        let c = claims();
        let ticket = c.sign(&test_key()).unwrap();
        let k = keys();
        let aud = "play.example.test";
        assert_eq!(
            verify(&ticket, &k, aud, c.expires_at),
            Err(TicketError::Expired)
        );
        assert!(verify(&ticket, &k, aud, c.expires_at - 1).is_ok());
        assert_eq!(
            verify(&ticket, &k, aud, c.issued_at - MAX_FUTURE_SKEW_SECONDS - 1),
            Err(TicketError::NotYetValid)
        );
        assert!(verify(&ticket, &k, aud, c.issued_at - MAX_FUTURE_SKEW_SECONDS).is_ok());
        assert_eq!(
            verify(&ticket, &k, "other.example.test", c.issued_at),
            Err(TicketError::WrongAudience)
        );
        assert_eq!(
            verify(&ticket, &VerifyingKeys::default(), aud, c.issued_at),
            Err(TicketError::UnknownKey)
        );
        let mut truncated = ticket.clone();
        truncated.pop();
        assert_eq!(
            verify(&truncated, &k, aud, c.issued_at),
            Err(TicketError::Malformed)
        );
    }

    #[test]
    fn high_s_signatures_verify() {
        let c = claims();
        let mut ticket = c.sign(&test_key()).unwrap();
        let at = ticket.len() - SIGNATURE_LEN;
        let low = Signature::from_slice(&ticket[at..]).unwrap();
        let s = -*low.s();
        let high = Signature::from_scalars(low.r().to_bytes(), s.to_bytes()).unwrap();
        ticket.truncate(at);
        ticket.extend_from_slice(&high.to_bytes());
        assert!(verify(&ticket, &keys(), &c.audience, c.issued_at).is_ok());
    }

    #[test]
    fn invalid_fields_are_not_signed() {
        let key = test_key();
        let mut c = claims();
        c.account_id = 0;
        assert_eq!(c.sign(&key), Err(TicketError::InvalidField));
        let mut c = claims();
        c.expires_at = c.issued_at + MAX_LIFETIME_SECONDS + 1;
        assert_eq!(c.sign(&key), Err(TicketError::InvalidField));
        let mut c = claims();
        c.player_name = "9lives".into();
        assert_eq!(c.sign(&key), Err(TicketError::InvalidField));
        let mut c = claims();
        c.audience = "Play.Example".into();
        assert_eq!(c.sign(&key), Err(TicketError::InvalidField));
    }
}
