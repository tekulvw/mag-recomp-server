//! Keeps `ticket-vectors.json` in step with the signer. Regenerate with
//! `PORTAL_WRITE_VECTORS=1 cargo test --test ticket_vectors`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mag_portal::config::public_key_der;
use mag_portal::ticket::{self, TicketClaims, TicketError, VerifyingKeys};
use p256::ecdsa::SigningKey;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const KEY_SEED: &str = "mag-portal ticket test vector key v1";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn key() -> SigningKey {
    SigningKey::from_slice(&Sha256::digest(KEY_SEED.as_bytes())).unwrap()
}

fn valid_claims() -> Vec<TicketClaims> {
    vec![
        TicketClaims {
            key_id: 1,
            issued_at: 1_800_000_000,
            expires_at: 1_800_000_600,
            account_id: 42,
            nonce: *b"0123456789abcdef",
            audience: "play.magtest.online".into(),
            player_name: "Alpha_Ranger-07".into(),
        },
        TicketClaims {
            key_id: 1,
            issued_at: 1_900_000_000,
            expires_at: 1_900_003_600,
            account_id: u32::MAX,
            nonce: [0xA5; 16],
            audience: "x".into(),
            player_name: "abc".into(),
        },
    ]
}

fn error_name(e: TicketError) -> &'static str {
    match e {
        TicketError::Malformed => "malformed",
        TicketError::InvalidField => "invalid_field",
        TicketError::UnknownKey => "unknown_key",
        TicketError::BadSignature => "bad_signature",
        TicketError::NotYetValid => "not_yet_valid",
        TicketError::Expired => "expired",
        TicketError::WrongAudience => "wrong_audience",
    }
}

fn build() -> Value {
    let key = key();
    let mut keys = VerifyingKeys::default();
    keys.insert(1, *key.verifying_key());
    let mut valid = Vec::new();
    let mut tickets = Vec::new();
    for c in valid_claims() {
        let t = c.sign(&key).unwrap();
        let verify_at = c.issued_at + 1;
        assert_eq!(
            ticket::verify(&t, &keys, &c.audience, verify_at).unwrap(),
            c
        );
        valid.push(json!({
            "key_id": c.key_id,
            "issued_at": c.issued_at,
            "expires_at": c.expires_at,
            "account_id": c.account_id,
            "nonce_hex": hex(&c.nonce),
            "audience": c.audience,
            "player_name": c.player_name,
            "length": t.len(),
            "signature_hex": hex(&t[t.len() - ticket::SIGNATURE_LEN..]),
            "ticket_sha256": hex(&Sha256::digest(&t)),
            "verify_at": verify_at,
        }));
        tickets.push((c, t));
    }
    let (c, t) = &tickets[0];
    let mut invalid = Vec::new();
    let mut case = |what: &str, mutation: Value, bytes: Vec<u8>, audience: &str, at: u64| {
        let e = ticket::verify(&bytes, &keys, audience, at).unwrap_err();
        invalid.push(json!({
            "case": what,
            "base": 0,
            "mutation": mutation,
            "audience": audience,
            "verify_at": at,
            "expect": error_name(e),
        }));
    };
    let mut flipped = t.clone();
    flipped[30] ^= 1;
    case(
        "nonce byte changed",
        json!({"xor_byte": {"offset": 30, "value": 1}}),
        flipped,
        &c.audience,
        c.issued_at,
    );
    let mut sig = t.clone();
    let last = sig.len() - 1;
    sig[last] ^= 0x80;
    case(
        "signature byte changed",
        json!({"xor_byte": {"offset": last, "value": 128}}),
        sig,
        &c.audience,
        c.issued_at,
    );
    let mut key_id = t.clone();
    key_id[5] = 2;
    case(
        "unknown key ID",
        json!({"set_byte": {"offset": 5, "value": 2}}),
        key_id,
        &c.audience,
        c.issued_at,
    );
    let mut short = t.clone();
    short.pop();
    case(
        "truncated",
        json!({"truncate": short.len()}),
        short,
        &c.audience,
        c.issued_at,
    );
    case("expired", json!(null), t.clone(), &c.audience, c.expires_at);
    case(
        "issued too far ahead",
        json!(null),
        t.clone(),
        &c.audience,
        c.issued_at - 61,
    );
    case(
        "other server",
        json!(null),
        t.clone(),
        "play.example.test",
        c.issued_at,
    );

    json!({
        "description": "Portal game ticket v1 vectors; see docs/portal-ticket.md. Rebuild each valid ticket from its fields, append the signature, check the SHA-256 and verify at verify_at. Invalid cases mutate valid ticket `base` and must be rejected.",
        "key": {
            "derivation": format!("P-256 private scalar = SHA-256 of the ASCII text '{KEY_SEED}'. Test use only."),
            "key_id": 1,
            "spki_base64": STANDARD.encode(public_key_der(&key)),
        },
        "valid": valid,
        "invalid": invalid,
    })
}

#[test]
fn vectors_file_matches_signer() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/ticket-vectors.json");
    let expected = serde_json::to_string_pretty(&build()).unwrap() + "\n";
    if std::env::var_os("PORTAL_WRITE_VECTORS").is_some() {
        std::fs::write(path, &expected).unwrap();
    }
    let actual = std::fs::read_to_string(path).expect("ticket-vectors.json exists");
    assert_eq!(actual, expected, "regenerate with PORTAL_WRITE_VECTORS=1");
}
