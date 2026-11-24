//! Runtime settings and the ticket signing key.

use std::path::Path;

use anyhow::{Context, bail};
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};

use crate::game_admin::GameAdmin;
use crate::ticket::{self, TicketClaims, TicketError};
use crate::tokens;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Scheme and host the site is served at, without a trailing slash.
    pub public_url: String,
    /// Shown on the privacy page as the way to reach the operator.
    pub operator_contact: Option<String>,
    pub session_days: i64,
    pub link_ttl_seconds: i64,
    pub link_poll_seconds: i64,
    pub max_pending_links: i64,
    /// Request header holding the client address, as a comma-separated list whose last
    /// entry the trusted proxy appended (`x-forwarded-for`). `None` uses the peer address.
    pub client_ip_header: Option<String>,
    /// Folder of the web fonts listed in fonts.txt; `None` serves none (pages fall back to
    /// system fonts).
    pub font_dir: Option<std::path::PathBuf>,
    /// Lowercase Google emails that always have the owner role; set by the operator, never
    /// changeable from the site.
    pub owner_emails: Vec<String>,
    /// The game server's admin API (`PORTAL_GAME_ADMIN_URL` and
    /// `PORTAL_GAME_ADMIN_SECRET_FILE`); `None` leaves the Game server admin section off.
    pub game_admin: Option<GameAdmin>,
}

impl Settings {
    pub fn new(public_url: &str) -> anyhow::Result<Self> {
        let public_url = public_url.trim_end_matches('/').to_owned();
        if !(public_url.starts_with("https://") || public_url.starts_with("http://localhost"))
            || public_url.contains(['?', '#'])
        {
            bail!("the public URL must be https:// (or http://localhost for local testing)");
        }
        Ok(Self {
            public_url,
            operator_contact: None,
            session_days: 30,
            link_ttl_seconds: 600,
            link_poll_seconds: 5,
            max_pending_links: 10_000,
            client_ip_header: None,
            font_dir: None,
            owner_emails: Vec::new(),
            game_admin: None,
        })
    }
}

pub struct TicketIssuer {
    key: SigningKey,
    pub key_id: u8,
    /// The default audience (`PORTAL_TICKET_AUDIENCE`), used when a launcher names none.
    pub audience: String,
    /// More game servers a launcher may ask for (`PORTAL_TICKET_EXTRA_AUDIENCES`).
    pub extra_audiences: Vec<String>,
    pub lifetime_seconds: u64,
}

/// Comma-separated audiences, each held to the same rule as the default one. Empty entries
/// are skipped.
pub fn parse_audiences(value: &str) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for entry in value.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        if !ticket::valid_audience(entry) {
            bail!(
                "ticket audience {entry:?} must be a lowercase host name of at most 63 characters"
            );
        }
        if !out.iter().any(|a| a == entry) {
            out.push(entry.to_owned());
        }
    }
    Ok(out)
}

impl TicketIssuer {
    pub fn new(
        key: SigningKey,
        key_id: u8,
        audience: &str,
        lifetime_seconds: u64,
    ) -> anyhow::Result<Self> {
        if !ticket::valid_audience(audience) {
            bail!("the ticket audience must be a lowercase host name of at most 63 characters");
        }
        if !(60..=ticket::MAX_LIFETIME_SECONDS).contains(&lifetime_seconds) {
            bail!(
                "the ticket lifetime must be 60 to {} seconds",
                ticket::MAX_LIFETIME_SECONDS
            );
        }
        Ok(Self {
            key,
            key_id,
            audience: audience.to_owned(),
            extra_audiences: Vec::new(),
            lifetime_seconds,
        })
    }

    /// Also issues tickets for these game servers when a launcher asks for one of them.
    pub fn with_extra_audiences(mut self, extra: &[String]) -> anyhow::Result<Self> {
        for audience in extra {
            if !ticket::valid_audience(audience) {
                bail!(
                    "ticket audience {audience:?} must be a lowercase host name of at most 63 characters"
                );
            }
            if *audience != self.audience && !self.extra_audiences.contains(audience) {
                self.extra_audiences.push(audience.clone());
            }
        }
        Ok(self)
    }

    /// The audience a ticket request gets: the default when it names none (or an empty one),
    /// otherwise the named one lowercased, if it is allowed. `None` for any other name.
    pub fn resolve_audience(&self, requested: Option<&str>) -> Option<&str> {
        let requested = match requested {
            None | Some("") => return Some(&self.audience),
            Some(r) => r.to_ascii_lowercase(),
        };
        std::iter::once(&self.audience)
            .chain(&self.extra_audiences)
            .find(|a| **a == requested)
            .map(String::as_str)
    }

    /// A ticket for `audience`, which the caller has resolved with `resolve_audience`.
    pub fn issue(
        &self,
        audience: &str,
        account_id: u32,
        player_name: &str,
        now: u64,
    ) -> Result<(Vec<u8>, u64), TicketError> {
        let claims = TicketClaims {
            key_id: self.key_id,
            issued_at: now,
            expires_at: now + self.lifetime_seconds,
            account_id,
            nonce: tokens::random_bytes(),
            audience: audience.to_owned(),
            player_name: player_name.to_owned(),
        };
        Ok((claims.sign(&self.key)?, claims.expires_at))
    }

    pub fn public_key_der(&self) -> Vec<u8> {
        public_key_der(&self.key)
    }
}

pub fn public_key_der(key: &SigningKey) -> Vec<u8> {
    key.verifying_key()
        .to_public_key_der()
        .expect("P-256 public keys encode")
        .into_vec()
}

pub fn load_signing_key(path: &Path) -> anyhow::Result<SigningKey> {
    let pem = std::fs::read_to_string(path)
        .with_context(|| format!("reading the ticket key {}", path.display()))?;
    SigningKey::from_pkcs8_pem(&pem)
        .context("the ticket key must be a PKCS#8 PEM P-256 private key")
}

pub fn generate_signing_key_pem() -> anyhow::Result<(String, Vec<u8>)> {
    let key = SigningKey::generate_from_rng(&mut rand::rng());
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .context("encoding the new key")?
        .to_string();
    Ok((pem, public_key_der(&key)))
}

pub fn read_secret_file(path: &Path) -> anyhow::Result<String> {
    let value = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .trim()
        .to_owned();
    if value.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn issuer() -> TicketIssuer {
        let key = SigningKey::from_slice(&Sha256::digest(b"config test key")).unwrap();
        TicketIssuer::new(key, 1, "play.example.test", 600).unwrap()
    }

    #[test]
    fn audiences() {
        assert_eq!(
            parse_audiences(" test.example.test, ,lab-2.example.test,test.example.test").unwrap(),
            vec!["test.example.test", "lab-2.example.test"]
        );
        assert!(parse_audiences("").unwrap().is_empty());
        for bad in ["Test.example.test", "a b", "x_y.test", &"a".repeat(64)] {
            assert!(parse_audiences(bad).is_err(), "{bad}");
        }
        let extra = parse_audiences("test.example.test,play.example.test").unwrap();
        let t = issuer().with_extra_audiences(&extra).unwrap();
        assert_eq!(t.extra_audiences, vec!["test.example.test"]);
        assert_eq!(t.resolve_audience(None), Some("play.example.test"));
        assert_eq!(t.resolve_audience(Some("")), Some("play.example.test"));
        assert_eq!(
            t.resolve_audience(Some("TEST.Example.test")),
            Some("test.example.test")
        );
        assert_eq!(
            t.resolve_audience(Some("play.example.test")),
            Some("play.example.test")
        );
        assert_eq!(t.resolve_audience(Some("other.example.test")), None);
        assert!(issuer().with_extra_audiences(&["Bad".into()]).is_err());
        assert_eq!(issuer().resolve_audience(Some("test.example.test")), None);
    }

    #[test]
    fn lifetime_range() {
        let key = || SigningKey::from_slice(&Sha256::digest(b"config test key")).unwrap();
        for ok in [60, 600, 3600, 43_200] {
            assert!(
                TicketIssuer::new(key(), 1, "play.example.test", ok).is_ok(),
                "{ok}"
            );
        }
        for bad in [0, 59, 43_201] {
            let err = TicketIssuer::new(key(), 1, "play.example.test", bad)
                .err()
                .unwrap_or_else(|| panic!("{bad} accepted"));
            assert!(err.to_string().contains("60 to 43200 seconds"), "{err}");
        }
        // A twelve-hour ticket is signed, verifies until its last second and expires on time.
        let t = TicketIssuer::new(key(), 1, "play.example.test", 43_200).unwrap();
        let mut keys = ticket::VerifyingKeys::default();
        keys.insert(1, *t.key.verifying_key());
        let (bytes, expires_at) = t.issue("play.example.test", 7, "Alpha", 1_000).unwrap();
        assert_eq!(expires_at, 44_200);
        assert!(ticket::verify(&bytes, &keys, "play.example.test", 44_199).is_ok());
        assert_eq!(
            ticket::verify(&bytes, &keys, "play.example.test", 44_200),
            Err(TicketError::Expired)
        );
    }

    #[test]
    fn tickets_carry_the_chosen_audience() {
        let t = issuer()
            .with_extra_audiences(&["test.example.test".into()])
            .unwrap();
        let mut keys = ticket::VerifyingKeys::default();
        keys.insert(1, *t.key.verifying_key());
        let (bytes, _) = t
            .issue("test.example.test", 7, "PlayerLinux", 1_000)
            .unwrap();
        let claims = ticket::verify(&bytes, &keys, "test.example.test", 1_001).unwrap();
        assert_eq!(claims.audience, "test.example.test");
        assert_eq!(
            ticket::verify(&bytes, &keys, "play.example.test", 1_001),
            Err(TicketError::WrongAudience)
        );
    }
}
