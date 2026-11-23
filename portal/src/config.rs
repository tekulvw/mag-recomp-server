//! Runtime settings and the ticket signing key.

use std::path::Path;

use anyhow::{Context, bail};
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};

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
    /// Lowercase Google emails that always have the owner role; set by the operator, never
    /// changeable from the site.
    pub owner_emails: Vec<String>,
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
            owner_emails: Vec::new(),
        })
    }
}

pub struct TicketIssuer {
    key: SigningKey,
    pub key_id: u8,
    pub audience: String,
    pub lifetime_seconds: u64,
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
            bail!("the ticket lifetime must be 60 to 3600 seconds");
        }
        Ok(Self {
            key,
            key_id,
            audience: audience.to_owned(),
            lifetime_seconds,
        })
    }

    pub fn issue(
        &self,
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
            audience: self.audience.clone(),
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
