//! OpenID Connect sign-in (Google): authorization code flow with PKCE, state and nonce.

use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::tokens;

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
pub const GOOGLE_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

/// Refetch keys on an unknown key ID no more often than this.
const JWKS_REFRESH_FLOOR: Duration = Duration::from_secs(60);
const JWKS_MAX_AGE: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Stored with each account; the identity is (provider, subject).
    pub provider: String,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub issuers: Vec<String>,
    pub redirect_uri: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("identity provider request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("identity provider refused the code: {0}")]
    TokenEndpoint(String),
    #[error("ID token rejected: {0}")]
    BadToken(String),
}

/// What the portal keeps from a verified sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub email: Option<String>,
}

/// Values a sign-in flow keeps until the provider redirects back.
pub struct FlowSecrets {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

impl FlowSecrets {
    pub fn new() -> Self {
        Self {
            state: tokens::b64url(&tokens::random_bytes::<32>()),
            nonce: tokens::b64url(&tokens::random_bytes::<32>()),
            pkce_verifier: tokens::b64url(&tokens::random_bytes::<32>()),
        }
    }
}

impl Default for FlowSecrets {
    fn default() -> Self {
        Self::new()
    }
}

pub fn pkce_challenge(verifier: &str) -> String {
    tokens::b64url(&tokens::sha256(verifier.as_bytes()))
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
}

struct CachedKeys {
    keys: JwkSet,
    fetched: Instant,
}

pub struct OidcClient {
    config: ProviderConfig,
    http: reqwest::Client,
    keys: Mutex<Option<CachedKeys>>,
}

impl OidcClient {
    pub fn new(config: ProviderConfig, http: reqwest::Client) -> Self {
        Self {
            config,
            http,
            keys: Mutex::new(None),
        }
    }

    pub fn provider(&self) -> &str {
        &self.config.provider
    }

    pub fn authorize_url(&self, flow: &FlowSecrets) -> String {
        let query = [
            ("response_type", "code"),
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("scope", "openid email"),
            ("state", flow.state.as_str()),
            ("nonce", flow.nonce.as_str()),
            ("code_challenge", &pkce_challenge(&flow.pkce_verifier)),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ];
        let encoded: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{k}={}", form_urlencode(v)))
            .collect();
        format!("{}?{}", self.config.auth_url, encoded.join("&"))
    }

    /// Exchanges the code and verifies the ID token it returns.
    pub async fn complete(
        &self,
        code: &str,
        pkce_verifier: &str,
        nonce: &str,
    ) -> Result<Identity, OidcError> {
        let response = self
            .http
            .post(&self.config.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", self.config.redirect_uri.as_str()),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.as_str()),
                ("code_verifier", pkce_verifier),
            ])
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(OidcError::TokenEndpoint(status.to_string()));
        }
        let token: TokenResponse = response.json().await?;
        self.verify_id_token(&token.id_token, nonce).await
    }

    pub async fn verify_id_token(
        &self,
        id_token: &str,
        nonce: &str,
    ) -> Result<Identity, OidcError> {
        let bad = |m: &str| OidcError::BadToken(m.to_owned());
        let header = jsonwebtoken::decode_header(id_token).map_err(|e| bad(&e.to_string()))?;
        if header.alg != Algorithm::RS256 {
            return Err(bad("unexpected algorithm"));
        }
        let kid = header.kid.ok_or_else(|| bad("no key ID"))?;
        let key = self.key(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.config.client_id]);
        validation.set_issuer(&self.config.issuers);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
        validation.leeway = 60;
        let data = jsonwebtoken::decode::<IdClaims>(id_token, &key, &validation)
            .map_err(|e| bad(&e.to_string()))?;
        let claims = data.claims;
        let nonce_ok = claims
            .nonce
            .as_deref()
            .is_some_and(|n| subtle::ConstantTimeEq::ct_eq(n.as_bytes(), nonce.as_bytes()).into());
        if !nonce_ok {
            return Err(bad("nonce mismatch"));
        }
        if claims.sub.is_empty() || claims.sub.len() > 255 {
            return Err(bad("bad subject"));
        }
        // Keep an email only when the provider vouches for it.
        let email = claims.email.filter(|_| claims.email_verified == Some(true));
        Ok(Identity {
            subject: claims.sub,
            email,
        })
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, OidcError> {
        let mut cache = self.keys.lock().await;
        let stale = cache
            .as_ref()
            .is_none_or(|c| c.fetched.elapsed() > JWKS_MAX_AGE);
        let missing = cache.as_ref().is_some_and(|c| c.keys.find(kid).is_none());
        let may_refresh = cache
            .as_ref()
            .is_none_or(|c| c.fetched.elapsed() > JWKS_REFRESH_FLOOR);
        if stale || (missing && may_refresh) {
            let keys: JwkSet = self
                .http
                .get(&self.config.jwks_url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            *cache = Some(CachedKeys {
                keys,
                fetched: Instant::now(),
            });
        }
        let jwk = cache
            .as_ref()
            .and_then(|c| c.keys.find(kid))
            .ok_or_else(|| OidcError::BadToken("unknown key ID".into()))?;
        DecodingKey::from_jwk(jwk).map_err(|e| OidcError::BadToken(e.to_string()))
    }
}

/// `application/x-www-form-urlencoded` value encoding.
pub fn form_urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn urlencode() {
        assert_eq!(form_urlencode("a b/c?d=é"), "a%20b%2Fc%3Fd%3D%C3%A9");
    }
}
