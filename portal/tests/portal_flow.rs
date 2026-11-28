//! End-to-end flows against a real PostgreSQL (`DATABASE_URL`, see portal/README.md) and a
//! fake OpenID provider that signs ID tokens with a freshly generated RSA key.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::extract::{Form, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use jsonwebtoken::{EncodingKey, Header};
use mag_portal::config::{Settings, TicketIssuer};
use mag_portal::db::{self, AssignOutcome, ClaimOutcome};
use mag_portal::game_admin::GameAdmin;
use mag_portal::oidc::{self, OidcClient, ProviderConfig};
use mag_portal::reports::Reports;
use mag_portal::status::{StatusServer, StatusService};

use mag_portal::ticket::{self, VerifyingKeys};
use mag_portal::web::{self, AppState, FLOW_COOKIE, SESSION_COOKIE};
use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use reqwest::StatusCode;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

mod support;

const CLIENT_ID: &str = "test-client";
const ISSUER: &str = "https://issuer.example.test";
const AUDIENCE: &str = "play.example.test";
/// A second game server the portal also issues tickets for (`PORTAL_TICKET_EXTRA_AUDIENCES`).
const EXTRA_AUDIENCE: &str = "test.example.test";

#[derive(Default)]
struct ProviderState {
    /// code -> (subject, nonce, PKCE challenge)
    codes: HashMap<String, (String, String, String)>,
    wrong_nonce: bool,
}

struct FakeProvider {
    key: EncodingKey,
    jwks: Value,
    state: Mutex<ProviderState>,
}

async fn token_endpoint(
    State(p): State<Arc<FakeProvider>>,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let st = p.state.lock().unwrap();
    let (sub, nonce, challenge) = st.codes.get(&form["code"]).ok_or(StatusCode::BAD_REQUEST)?;
    let verifier = &form["code_verifier"];
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != *challenge
        || form["client_id"] != CLIENT_ID
        || form["client_secret"] != "test-secret"
        || form["grant_type"] != "authorization_code"
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let nonce = if st.wrong_nonce {
        "not-the-nonce".to_owned()
    } else {
        nonce.clone()
    };
    let claims = json!({
        "iss": ISSUER, "aud": CLIENT_ID, "sub": sub, "iat": now, "exp": now + 300,
        "nonce": nonce, "email": format!("{sub}@example.test"), "email_verified": true,
    });
    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("k1".into());
    let id_token = jsonwebtoken::encode(&header, &claims, &p.key).unwrap();
    Ok(Json(
        json!({ "id_token": id_token, "access_token": "unused", "token_type": "Bearer" }),
    ))
}

async fn jwks_endpoint(State(p): State<Arc<FakeProvider>>) -> Json<Value> {
    Json(p.jwks.clone())
}

async fn serve(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    addr
}

fn b64_uint(bytes: &[u8]) -> String {
    let start = bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(bytes.len() - 1);
    URL_SAFE_NO_PAD.encode(&bytes[start..])
}

struct Harness {
    base: String,
    http: reqwest::Client,
    provider: Arc<FakeProvider>,
    pool: PgPool,
}

async fn harness(pool: PgPool) -> Harness {
    harness_with(pool, None).await
}

async fn harness_with(pool: PgPool, game_admin: Option<GameAdmin>) -> Harness {
    harness_full(pool, game_admin, None, web::Limits::default()).await
}

async fn harness_full(
    pool: PgPool,
    game_admin: Option<GameAdmin>,
    reports: Option<Arc<Reports>>,
    limits: web::Limits,
) -> Harness {
    let status = Arc::new(StatusService::new(Vec::new(), StdDuration::from_secs(15)));
    harness_status(pool, game_admin, reports, limits, status).await
}

async fn harness_status(
    pool: PgPool,
    game_admin: Option<GameAdmin>,
    reports: Option<Arc<Reports>>,
    limits: web::Limits,
    status: Arc<StatusService>,
) -> Harness {
    harness_status_names(pool, game_admin, reports, limits, status, false).await
}

async fn harness_status_names(
    pool: PgPool,
    game_admin: Option<GameAdmin>,
    reports: Option<Arc<Reports>>,
    limits: web::Limits,
    status: Arc<StatusService>,
    name_changes: bool,
) -> Harness {
    let rsa_key = rsa::PrivateKey::new(&mut rand::rng(), 2048).unwrap();
    let der = rsa_key.to_pkcs1_der().unwrap();
    let jwks = json!({ "keys": [{
        "kty": "RSA", "kid": "k1", "alg": "RS256", "use": "sig",
        "n": b64_uint(&rsa_key.n().to_be_bytes()), "e": b64_uint(&rsa_key.e().to_be_bytes()),
    }]});
    let provider = Arc::new(FakeProvider {
        key: EncodingKey::from_rsa_der(der.as_bytes()),
        jwks,
        state: Mutex::new(ProviderState::default()),
    });
    let provider_addr = serve(
        Router::new()
            .route("/token", post(token_endpoint))
            .route("/jwks", get(jwks_endpoint))
            .with_state(provider.clone()),
    )
    .await;
    let provider_url = format!("http://{provider_addr}");

    let mut settings = Settings::new("http://localhost").unwrap();
    settings.player_name_changes = name_changes;
    // Long enough that a loaded machine cannot stretch two back-to-back polls past it.
    settings.link_poll_seconds = 60;
    settings.owner_emails = vec!["owner@example.test".into()];
    let fonts = std::env::temp_dir().join(format!(
        "portal-fonts-{}",
        mag_portal::tokens::b64url(&mag_portal::tokens::random_bytes::<9>())
    ));
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::write(
        fonts.join("chakra-petch-latin-700-normal.woff2"),
        b"wOF2 fixture",
    )
    .unwrap();
    settings.font_dir = Some(fonts);
    settings.game_admin = game_admin;
    let ticket_key = SigningKey::from_slice(&Sha256::digest(b"portal flow test key")).unwrap();
    let state = Arc::new(AppState {
        pool: pool.clone(),
        oidc: OidcClient::new(
            ProviderConfig {
                provider: "google".into(),
                client_id: CLIENT_ID.into(),
                client_secret: "test-secret".into(),
                auth_url: format!("{provider_url}/authorize"),
                token_url: format!("{provider_url}/token"),
                jwks_url: format!("{provider_url}/jwks"),
                issuers: vec![ISSUER.into()],
                redirect_uri: "http://localhost/auth/google/callback".into(),
            },
            reqwest::Client::new(),
        ),
        tickets: TicketIssuer::new(ticket_key, 3, AUDIENCE, 600)
            .unwrap()
            .with_extra_audiences(&[EXTRA_AUDIENCE.to_owned()])
            .unwrap(),
        settings,
        limits,
        reports,
        status,
    });
    let addr = serve(web::router(state)).await;
    Harness {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        provider,
        pool,
    }
}

#[sqlx::test]
async fn rename_requires_enabled_feature_session_and_csrf(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let disabled = harness(pool.clone()).await;
    let session = disabled.sign_in("rename-person").await;
    let csrf = disabled.csrf(&session).await;
    disabled
        .post_form("/claim", &session, &[("csrf", &csrf), ("name", "Alpha")])
        .await;
    assert_eq!(
        disabled
            .post_form(
                "/account/name",
                &session,
                &[("csrf", &csrf), ("current", "Alpha"), ("name", "NewAlpha")]
            )
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let enabled = harness_status_names(
        pool.clone(),
        None,
        None,
        web::Limits::default(),
        Arc::new(StatusService::new(Vec::new(), StdDuration::from_secs(15))),
        true,
    )
    .await;
    assert_eq!(
        enabled
            .post_form(
                "/account/name",
                "invalid-session",
                &[("csrf", &csrf), ("current", "Alpha"), ("name", "NewAlpha")]
            )
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        enabled
            .post_form(
                "/account/name",
                &session,
                &[
                    ("csrf", "wrong"),
                    ("current", "Alpha"),
                    ("name", "NewAlpha")
                ]
            )
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let r = enabled
        .post_form(
            "/account/name",
            &session,
            &[("csrf", &csrf), ("current", "Alpha"), ("name", "NewAlpha")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let account = db::session(&pool, &session).await.unwrap().unwrap().account;
    assert_eq!(account.player_name.as_deref(), Some("NewAlpha"));
    assert_eq!(account.game_account_name.as_deref(), Some("alpha"));
    let token = "mgd_rename_ticket_fixture";
    sqlx::query("INSERT INTO devices(account_id,token_hash,label) VALUES($1,$2,'rename fixture')")
        .bind(account.id)
        .bind(mag_portal::tokens::sha256(token.as_bytes()).as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let (status, ticket) = enabled.ticket(token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ticket["account_id"], account.id);
    assert_eq!(ticket["player_name"], "NewAlpha");
    let bytes = STANDARD.decode(ticket["ticket"].as_str().unwrap()).unwrap();
    assert_eq!(&bytes[..5], b"MAGT\x01", "native ticket format stays v1");
    let audience_len = usize::from(bytes[42]);
    let name_start = 44 + audience_len;
    assert_eq!(&bytes[name_start..bytes.len() - 64], b"NewAlpha");
    let r = enabled
        .post_form(
            "/account/name",
            &session,
            &[
                ("csrf", &csrf),
                ("current", "NewAlpha"),
                ("name", "NEWALPHA"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert!(r.text().await.unwrap().contains("24 hours"));
    db::set_ban(&pool, "NewAlpha", Some("synthetic ban"))
        .await
        .unwrap();
    assert_eq!(
        enabled
            .post_form(
                "/account/name",
                &session,
                &[("csrf", &csrf), ("current", "NewAlpha"), ("name", "Other")]
            )
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    test_db.close().await;
}

fn set_cookies(r: &reqwest::Response) -> HashMap<String, String> {
    r.headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next()?.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn location(r: &reqwest::Response) -> String {
    r.headers()["location"].to_str().unwrap().to_owned()
}

fn query_param(url: &str, name: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let raw = query
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
        .unwrap();
    // The values the portal generates are URL-safe base64, so they need no decoding.
    raw.to_owned()
}

fn csrf_from(html: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = html.find(marker).unwrap() + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_owned()
}

impl Harness {
    async fn get(&self, path: &str, session: Option<&str>) -> reqwest::Response {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(s) = session {
            req = req.header("cookie", format!("{SESSION_COOKIE}={s}"));
        }
        req.send().await.unwrap()
    }

    async fn post_form(
        &self,
        path: &str,
        session: &str,
        form: &[(&str, &str)],
    ) -> reqwest::Response {
        self.http
            .post(format!("{}{path}", self.base))
            .header("cookie", format!("{SESSION_COOKIE}={session}"))
            .form(form)
            .send()
            .await
            .unwrap()
    }

    /// Signs in as `subject`, returning the session cookie value.
    async fn sign_in(&self, subject: &str) -> String {
        let start = self.get("/auth/google?next=%2Faccount", None).await;
        assert_eq!(start.status(), StatusCode::SEE_OTHER);
        let flow = set_cookies(&start)[FLOW_COOKIE].clone();
        let auth = location(&start);
        assert!(
            auth.contains("code_challenge_method=S256") && auth.contains("scope=openid%20email")
        );
        let state = query_param(&auth, "state");
        assert_eq!(state, flow);
        let code = format!("code-{subject}");
        self.provider.state.lock().unwrap().codes.insert(
            code.clone(),
            (
                subject.into(),
                query_param(&auth, "nonce"),
                query_param(&auth, "code_challenge"),
            ),
        );
        let callback = self
            .http
            .get(format!(
                "{}/auth/google/callback?code={code}&state={state}",
                self.base
            ))
            .header("cookie", format!("{FLOW_COOKIE}={flow}"))
            .send()
            .await
            .unwrap();
        assert_eq!(callback.status(), StatusCode::SEE_OTHER);
        assert_eq!(location(&callback), "/account");
        set_cookies(&callback)[SESSION_COOKIE].clone()
    }

    async fn csrf(&self, session: &str) -> String {
        csrf_from(
            &self
                .get("/account", Some(session))
                .await
                .text()
                .await
                .unwrap(),
        )
    }

    async fn poll(&self, device_code: &str) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/api/v1/device/token", self.base))
            .json(&json!({ "device_code": device_code }))
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap())
    }

    /// A ticket request with a raw body (`None`: no body at all).
    async fn ticket_with(&self, device_token: &str, body: Option<&str>) -> (StatusCode, Value) {
        let mut req = self
            .http
            .post(format!("{}/api/v1/ticket", self.base))
            .bearer_auth(device_token);
        if let Some(body) = body {
            req = req
                .header("content-type", "application/json")
                .body(body.to_owned());
        }
        let r = req.send().await.unwrap();
        (r.status(), r.json().await.unwrap())
    }

    async fn ticket(&self, device_token: &str) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/api/v1/ticket", self.base))
            .bearer_auth(device_token)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap())
    }
}

#[sqlx::test]
async fn sign_in_link_and_ticket(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;

    // Signed-out visitors are sent to sign in, and come back where they were going.
    let r = h.get("/account", None).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&r), "/auth/google?next=%2Faccount");
    let headers = r.headers();
    assert!(
        headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'none'")
    );
    assert_eq!(headers["x-frame-options"], "DENY");

    let session = h.sign_in("subject-a").await;
    let page = h
        .get("/account", Some(&session))
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("choose one") && page.contains("subject-a@example.test"));
    // Behind Cloudflare, addresses outside these markers become dead links (see base.html).
    let on = page.find("<!--email_off-->").unwrap();
    let off = page.find("<!--/email_off-->").unwrap();
    let at = page.find("subject-a@example.test").unwrap();
    assert!(
        on < at && at < off,
        "the email must sit inside the email_off markers"
    );
    let csrf = csrf_from(&page);

    // Claim a name; forms without the session's CSRF token are refused.
    let r = h
        .post_form("/claim", &session, &[("csrf", "wrong"), ("name", "Alpha")])
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form("/claim", &session, &[("csrf", &csrf), ("name", "9bad")])
        .await;
    assert!(r.text().await.unwrap().contains("starting with a letter"));
    let r = h
        .post_form(
            "/claim",
            &session,
            &[("csrf", &csrf), ("name", "Alpha"), ("next", "/account")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(
        h.get("/account", Some(&session))
            .await
            .text()
            .await
            .unwrap()
            .contains("<strong>Alpha</strong>")
    );

    // The launcher starts a link and polls while it waits.
    let r = h
        .http
        .post(format!("{}/api/v1/device/start", h.base))
        .json(&json!({ "client": "Test launcher (Linux)" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let start: Value = r.json().await.unwrap();
    let device_code = start["device_code"].as_str().unwrap().to_owned();
    let user_code = start["user_code"].as_str().unwrap().to_owned();
    assert_eq!(start["verification_uri"], "http://localhost/link");
    assert_eq!(start["interval"], 60);
    assert_eq!(
        h.poll(&device_code).await.1["error"],
        "authorization_pending"
    );
    assert_eq!(h.poll(&device_code).await.1["error"], "slow_down");
    assert_eq!(h.poll("mgl_nonsense").await.1["error"], "invalid_grant");

    // The player enters the code (any case, any separators), sees the launcher, approves.
    let typed = user_code.to_lowercase().replace('-', " ");
    let confirm = h
        .post_form("/link", &session, &[("csrf", &csrf), ("code", &typed)])
        .await;
    let html = confirm.text().await.unwrap();
    assert!(
        html.contains("Test launcher (Linux)") && html.contains(&user_code),
        "{html}"
    );
    // When it was requested, for the viewer's time zone.
    assert_eq!(support::page_times(&html), 1, "{html}");
    let r = h
        .post_form(
            "/link/decide",
            &session,
            &[
                ("csrf", &csrf),
                ("code", &user_code),
                ("decision", "approve"),
            ],
        )
        .await;
    assert!(r.text().await.unwrap().contains("Launcher linked"));

    let (status, linked) = h.poll(&device_code).await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked["player_name"], "Alpha");
    // The account page lists it with when it was linked; "never" used is words, not a time.
    let account = page_html(h.get("/account", Some(&session)).await).await;
    assert_eq!(support::page_times(&account), 1, "{account}");
    assert!(
        account.contains("UTC</time></td><td>never</td>"),
        "{account}"
    );
    let device_token = linked["device_token"].as_str().unwrap().to_owned();
    assert_eq!(
        h.poll(&device_code).await.1["error"],
        "invalid_grant",
        "a link completes once"
    );

    // The ticket verifies with the published key.
    let (status, issued) = h.ticket(&device_token).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let keys: Value = h
        .get("/api/v1/ticket-keys", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(keys["keys"][0]["id"], 3);
    let spki = STANDARD
        .decode(keys["keys"][0]["spki"].as_str().unwrap())
        .unwrap();
    let mut verifying = VerifyingKeys::default();
    verifying.insert(3, VerifyingKey::from_public_key_der(&spki).unwrap());
    let bytes = STANDARD.decode(issued["ticket"].as_str().unwrap()).unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
    let claims = ticket::verify(&bytes, &verifying, AUDIENCE, now).unwrap();
    let account = db::account_by_name(&h.pool, "alpha")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claims.player_name, "Alpha");
    assert_eq!(claims.account_id, account.id as u32);
    assert_eq!(claims.expires_at, issued["expires_at"].as_u64().unwrap());
    assert!(claims.expires_at > now && claims.expires_at <= now + 601);
    let (_, second) = h.ticket(&device_token).await;
    assert_ne!(
        second["ticket"], issued["ticket"],
        "every ticket has a fresh nonce"
    );

    // Bans stop tickets; lifting them restores them.
    assert!(
        db::set_ban(&h.pool, "ALPHA", Some("testing"))
            .await
            .unwrap()
    );
    let (status, body) = h.ticket(&device_token).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("banned"))
    );
    assert!(
        h.get("/account", Some(&session))
            .await
            .text()
            .await
            .unwrap()
            .contains("banned: testing")
    );
    assert!(db::set_ban(&h.pool, "alpha", None).await.unwrap());
    assert_eq!(h.ticket(&device_token).await.0, StatusCode::OK);

    // Unlinking the launcher revokes its token.
    let device = &db::devices(&h.pool, account.id).await.unwrap()[0];
    assert_eq!(device.label, "Test launcher (Linux)");
    assert!(device.last_used_at.is_some());
    let r = h
        .post_form(
            &format!("/devices/{}/revoke", device.id),
            &session,
            &[("csrf", &csrf)],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(h.ticket(&device_token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(h.ticket("mgd_made-up").await.0, StatusCode::UNAUTHORIZED);

    // Signing out ends the session.
    let r = h.post_form("/logout", &session, &[("csrf", &csrf)]).await;
    assert_eq!(set_cookies(&r)[SESSION_COOKIE], "");
    assert_eq!(
        h.get("/account", Some(&session)).await.status(),
        StatusCode::SEE_OTHER
    );
    test_db.close().await;
}

#[sqlx::test]
async fn names_stay_with_their_characters(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let a = h.sign_in("subject-a").await;
    let csrf_a = h.csrf(&a).await;
    h.post_form("/claim", &a, &[("csrf", &csrf_a), ("name", "Bravo")])
        .await;

    // Names are unique ignoring case.
    let b = h.sign_in("subject-b").await;
    let csrf_b = h.csrf(&b).await;
    let r = h
        .post_form("/claim", &b, &[("csrf", &csrf_b), ("name", "bravo")])
        .await;
    assert!(r.text().await.unwrap().contains("already has that name"));

    // Signing in again finds the same account.
    let a2 = h.sign_in("subject-a").await;
    assert!(
        h.get("/account", Some(&a2))
            .await
            .text()
            .await
            .unwrap()
            .contains("<strong>Bravo</strong>")
    );

    // A deleted account's name stays reserved.
    let r = h
        .post_form(
            "/account/delete",
            &a,
            &[("csrf", &csrf_a), ("confirm", "wrong")],
        )
        .await;
    assert!(r.text().await.unwrap().contains("nothing was deleted"));
    let r = h
        .post_form(
            "/account/delete",
            &a,
            &[("csrf", &csrf_a), ("confirm", "bravo")],
        )
        .await;
    assert!(r.text().await.unwrap().contains("Account deleted"));
    assert_eq!(
        h.get("/account", Some(&a2)).await.status(),
        StatusCode::SEE_OTHER,
        "all sessions end"
    );
    let r = h
        .post_form("/claim", &b, &[("csrf", &csrf_b), ("name", "BRAVO")])
        .await;
    assert!(r.text().await.unwrap().contains("reserved"));

    // The operator can hand it to its owner.
    let b_account = db::accounts_by_email(&h.pool, "subject-b@example.test")
        .await
        .unwrap();
    assert_eq!(
        db::assign_reserved_name(&h.pool, "bravo", b_account[0].id)
            .await
            .unwrap(),
        AssignOutcome::Assigned
    );
    assert!(
        h.get("/account", Some(&b))
            .await
            .text()
            .await
            .unwrap()
            .contains("<strong>Bravo</strong>")
    );
    assert_eq!(
        db::assign_reserved_name(&h.pool, "bravo", b_account[0].id)
            .await
            .unwrap(),
        AssignOutcome::NotReserved
    );

    // Imported names cannot be claimed first-come.
    assert_eq!(
        db::reserve_name(&h.pool, "Charlie", "existing player")
            .await
            .unwrap(),
        db::ReserveOutcome::Reserved
    );
    let c = h.sign_in("subject-c").await;
    let c_account = db::accounts_by_email(&h.pool, "subject-c@example.test")
        .await
        .unwrap();
    assert_eq!(
        db::claim_name(&h.pool, c_account[0].id, "charlie")
            .await
            .unwrap(),
        ClaimOutcome::Reserved
    );
    assert_eq!(
        db::reserve_name(&h.pool, "bravo", "x").await.unwrap(),
        db::ReserveOutcome::InUse
    );
    drop(c);
    test_db.close().await;
}

#[sqlx::test]
async fn sign_in_rejections(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let start = h.get("/auth/google", None).await;
    let flow = set_cookies(&start)[FLOW_COOKIE].clone();
    let auth = location(&start);
    let state = query_param(&auth, "state");
    h.provider.state.lock().unwrap().codes.insert(
        "c1".into(),
        (
            "subject-x".into(),
            query_param(&auth, "nonce"),
            query_param(&auth, "code_challenge"),
        ),
    );
    let callback = |cookie: Option<String>, state: String| {
        let mut req = h.http.get(format!(
            "{}/auth/google/callback?code=c1&state={state}",
            h.base
        ));
        if let Some(c) = cookie {
            req = req.header("cookie", format!("{FLOW_COOKIE}={c}"));
        }
        req.send()
    };
    // Another browser (no flow cookie) cannot finish the flow.
    assert_eq!(
        callback(None, state.clone()).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    // A token whose nonce does not match is refused, and the flow is used up.
    h.provider.state.lock().unwrap().wrong_nonce = true;
    let r = callback(Some(flow.clone()), state.clone()).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    assert!(!set_cookies(&r).contains_key(SESSION_COOKIE));
    h.provider.state.lock().unwrap().wrong_nonce = false;
    let r = callback(Some(flow), state).await.unwrap();
    assert_eq!(
        r.status(),
        StatusCode::BAD_REQUEST,
        "a flow completes at most once"
    );
    let cancelled = h
        .get("/auth/google/callback?error=access_denied", None)
        .await;
    assert!(
        cancelled
            .text()
            .await
            .unwrap()
            .contains("Sign-in cancelled")
    );
    assert!(
        db::accounts_by_email(&h.pool, "subject-x@example.test")
            .await
            .unwrap()
            .is_empty()
    );
    // Open redirects are not followed.
    let r = h.get("/auth/google?next=%2F%2Fevil.example", None).await;
    let flow = db::take_login_flow(&h.pool, &set_cookies(&r)[FLOW_COOKIE])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(flow.return_to, "/account");
    let _ = oidc::GOOGLE_ISSUERS;
    test_db.close().await;
}

#[sqlx::test]
async fn banned_accounts_cannot_link(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let s = h.sign_in("subject-d").await;
    let csrf = h.csrf(&s).await;
    h.post_form("/claim", &s, &[("csrf", &csrf), ("name", "Delta")])
        .await;
    db::set_ban(&h.pool, "delta", Some("cheating"))
        .await
        .unwrap();
    let r = h.get("/link", Some(&s)).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Unnamed accounts are sent to choose a name first.
    let u = h.sign_in("subject-e").await;
    let r = h.get("/link?code=BCDF-GHJK", Some(&u)).await;
    assert_eq!(location(&r), "/claim?next=%2Flink%3Fcode%3DBCDF-GHJK");
    // Denied links tell the launcher.
    let start: Value = h
        .http
        .post(format!("{}/api/v1/device/start", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let csrf_u = h.csrf(&u).await;
    h.post_form("/claim", &u, &[("csrf", &csrf_u), ("name", "Echo")])
        .await;
    let code = start["user_code"].as_str().unwrap();
    h.post_form(
        "/link/decide",
        &u,
        &[("csrf", &csrf_u), ("code", code), ("decision", "deny")],
    )
    .await;
    assert_eq!(
        h.poll(start["device_code"].as_str().unwrap()).await.1["error"],
        "access_denied"
    );
    assert_eq!(db::cleanup(&h.pool).await.unwrap(), 0);
    test_db.close().await;
}

#[sqlx::test]
async fn launcher_api_limits_and_errors(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let start = || h.http.post(format!("{}/api/v1/device/start", h.base));
    // A body that is not JSON gets a JSON error a launcher can read.
    let r = start()
        .header("content-type", "application/json")
        .body("{nope")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "invalid_request");
    let r = h
        .http
        .post(format!("{}/api/v1/device/token", h.base))
        .body("device_code=x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "invalid_request");
    // Ten link starts a minute per client, then 429.
    for _ in 0..10 {
        assert_eq!(start().send().await.unwrap().status(), StatusCode::OK);
    }
    let r = start().send().await.unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "rate_limited");
    test_db.close().await;
}

#[sqlx::test]
async fn banned_accounts_cannot_delete_themselves(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let s = h.sign_in("subject-f").await;
    let csrf = h.csrf(&s).await;
    h.post_form("/claim", &s, &[("csrf", &csrf), ("name", "Foxtrot")])
        .await;
    db::set_ban(&h.pool, "foxtrot", Some("cheating"))
        .await
        .unwrap();
    let r = h
        .post_form(
            "/account/delete",
            &s,
            &[("csrf", &csrf), ("confirm", "Foxtrot")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(
        db::account_by_name(&h.pool, "foxtrot")
            .await
            .unwrap()
            .is_some_and(|a| a.is_banned())
    );
    test_db.close().await;
}

/// A claim waits for any transaction holding the name's lock, so it sees a reservation that
/// transaction commits (the deletion path) instead of racing it.
#[sqlx::test]
async fn claims_wait_for_name_changes(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let a = db::upsert_identity(&pool, "google", "subject-g", None)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    db::lock_name(&mut tx, "Golf").await.unwrap();
    let claimer = {
        let pool = pool.clone();
        tokio::spawn(async move { db::claim_name(&pool, a.id, "golf").await.unwrap() })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!claimer.is_finished(), "the claim must wait for the lock");
    sqlx::query(
        "INSERT INTO reserved_names (name_key, name, reason) VALUES ('golf', 'Golf', 'test')",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(claimer.await.unwrap(), ClaimOutcome::Reserved);
    test_db.close().await;
}

async fn id_of(h: &Harness, subject: &str) -> i32 {
    db::accounts_by_email(&h.pool, &format!("{subject}@example.test"))
        .await
        .unwrap()[0]
        .id
}

#[sqlx::test]
async fn admin_panel_roles(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    // Signed out: sign in first. Signed in without a role: the panel does not exist.
    assert_eq!(h.get("/admin", None).await.status(), StatusCode::SEE_OTHER);
    let u = h.sign_in("player").await;
    let csrf_u = h.csrf(&u).await;
    h.post_form("/claim", &u, &[("csrf", &csrf_u), ("name", "Uniform")])
        .await;
    for path in ["/admin", "/admin/users", "/admin/staff", "/admin/names"] {
        assert_eq!(
            h.get(path, Some(&u)).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let player = id_of(&h, "player").await;
    let r = h
        .post_form(
            &format!("/admin/users/{player}/ban"),
            &u,
            &[("csrf", &csrf_u), ("reason", "x")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // The configured owner grants roles; forms need the CSRF token; owner is not grantable.
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let page = page_html(h.get("/admin", Some(&o)).await).await;
    assert!(page.contains("signed in as owner"));
    assert!(
        h.get("/account", Some(&o))
            .await
            .text()
            .await
            .unwrap()
            .contains("href=\"/admin\"")
    );
    assert!(
        !h.get("/account", Some(&u))
            .await
            .text()
            .await
            .unwrap()
            .contains("href=\"/admin\"")
    );
    let r = h
        .post_form(
            "/admin/staff/set",
            &o,
            &[
                ("csrf", "bad"),
                ("email", "mod@example.test"),
                ("role", "moderator"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            "/admin/staff/set",
            &o,
            &[
                ("csrf", &csrf_o),
                ("email", "boss@example.test"),
                ("role", "owner"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    for (email, role) in [
        ("Mod@Example.test", "moderator"),
        ("admin@example.test", "admin"),
    ] {
        let r = h
            .post_form(
                "/admin/staff/set",
                &o,
                &[("csrf", &csrf_o), ("email", email), ("role", role)],
            )
            .await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER, "{email}");
    }
    let staff = page_html(h.get("/admin/staff", Some(&o)).await).await;
    assert!(staff.contains("mod@example.test") && staff.contains("owner@example.test"));
    assert!(support::page_times(&staff) > 0, "{staff}");

    // Moderators: view and ban players, nothing above them, no names or staff management.
    let m = h.sign_in("mod").await;
    let csrf_m = h.csrf(&m).await;
    assert!(
        h.get("/admin/users?q=unif", Some(&m))
            .await
            .text()
            .await
            .unwrap()
            .contains("Uniform")
    );
    assert!(
        !h.get("/admin/users?q=%25", Some(&m))
            .await
            .text()
            .await
            .unwrap()
            .contains("Uniform"),
        "LIKE wildcards are literal"
    );
    assert!(
        h.get(&format!("/admin/users?q={player}"), Some(&m))
            .await
            .text()
            .await
            .unwrap()
            .contains("Uniform")
    );
    assert_eq!(
        h.get("/admin/staff", Some(&m)).await.status(),
        StatusCode::FORBIDDEN
    );
    let r = h
        .post_form(
            &format!("/admin/users/{player}/ban"),
            &m,
            &[("csrf", &csrf_m), ("reason", "griefing")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(
        db::account(&h.pool, player)
            .await
            .unwrap()
            .unwrap()
            .is_banned()
    );
    // The user page says since when, as a time, and the reason as text.
    let banned = page_html(h.get(&format!("/admin/users/{player}"), Some(&m)).await).await;
    assert!(
        banned.contains("<strong>banned</strong> since <time class=\"local-time\" datetime=\"")
            && banned.contains(" UTC</time>: griefing</dd>"),
        "{banned}"
    );
    assert!(
        support::page_times(&banned) >= 2,
        "joined and banned: {banned}"
    );
    let users = page_html(h.get("/admin/users?q=unif", Some(&m)).await).await;
    assert!(support::page_times(&users) >= 1, "{users}");
    let owner = id_of(&h, "owner").await;
    let admin = id_of(&h, "mod").await;
    for target in [owner, admin] {
        let r = h
            .post_form(
                &format!("/admin/users/{target}/ban"),
                &m,
                &[("csrf", &csrf_m), ("reason", "x")],
            )
            .await;
        assert_eq!(
            r.status(),
            StatusCode::FORBIDDEN,
            "moderators cannot ban staff at or above them"
        );
    }
    let r = h
        .post_form(
            &format!("/admin/users/{player}/delete"),
            &m,
            &[("csrf", &csrf_m), ("confirm", &player.to_string())],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            "/admin/names/reserve",
            &m,
            &[("csrf", &csrf_m), ("names", "Zulu"), ("reason", "")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            &format!("/admin/users/{player}/unban"),
            &m,
            &[("csrf", &csrf_m)],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);

    // Admins: grant moderator only, never act on admins or owners.
    let a = h.sign_in("admin").await;
    let csrf_a = h.csrf(&a).await;
    let r = h
        .post_form(
            "/admin/staff/set",
            &a,
            &[
                ("csrf", &csrf_a),
                ("email", "two@example.test"),
                ("role", "moderator"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let r = h
        .post_form(
            "/admin/staff/set",
            &a,
            &[
                ("csrf", &csrf_a),
                ("email", "three@example.test"),
                ("role", "admin"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            "/admin/staff/set",
            &a,
            &[
                ("csrf", &csrf_a),
                ("email", "owner@example.test"),
                ("role", "moderator"),
            ],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::FORBIDDEN,
        "owners cannot be demoted"
    );
    let r = h
        .post_form(
            "/admin/staff/remove",
            &a,
            &[("csrf", &csrf_a), ("email", "admin@example.test")],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::FORBIDDEN,
        "nobody removes a peer or themselves"
    );
    let r = h
        .post_form(
            "/admin/staff/remove",
            &a,
            &[("csrf", &csrf_a), ("email", "two@example.test")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let r = h
        .post_form(
            &format!("/admin/users/{owner}/ban"),
            &a,
            &[("csrf", &csrf_a), ("reason", "x")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // Admins manage names and delete accounts; deleted names stay reserved.
    let r = h
        .post_form(
            "/admin/names/reserve",
            &a,
            &[
                ("csrf", &csrf_a),
                ("names", "Zulu, Yankee 9bad Uniform"),
                ("reason", ""),
            ],
        )
        .await;
    let html = r.text().await.unwrap();
    assert!(html.contains("Zulu: reserved") && html.contains("Yankee: reserved"));
    assert!(
        html.contains("9bad: not a valid player name")
            && html.contains("Uniform: an account already has it")
    );
    let fresh = h.sign_in("fresh").await;
    let fresh_id = id_of(&h, "fresh").await;
    let r = h
        .post_form(
            &format!("/admin/users/{fresh_id}/assign-name"),
            &a,
            &[("csrf", &csrf_a), ("name", "zulu")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(
        h.get("/account", Some(&fresh))
            .await
            .text()
            .await
            .unwrap()
            .contains("<strong>Zulu</strong>")
    );
    h.post_form(
        "/admin/names/release",
        &a,
        &[("csrf", &csrf_a), ("name", "Yankee")],
    )
    .await;
    assert!(db::reserved_names(&h.pool).await.unwrap().is_empty());
    let r = h
        .post_form(
            &format!("/admin/users/{player}/delete"),
            &a,
            &[("csrf", &csrf_a), ("confirm", "nope")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = h
        .post_form(
            &format!("/admin/users/{player}/delete"),
            &a,
            &[("csrf", &csrf_a), ("confirm", &player.to_string())],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(db::account(&h.pool, player).await.unwrap().is_none());
    assert_eq!(
        db::reserved_names(&h.pool).await.unwrap()[0].name,
        "Uniform"
    );

    // Every change is in the audit log, newest first, each with its time.
    let log = page_html(h.get("/admin", Some(&o)).await).await;
    assert!(support::page_times(&log) >= 5, "{log}");
    for action in [
        "grant role",
        "ban",
        "unban",
        "remove role",
        "reserve name",
        "assign name",
        "release name",
        "delete account",
    ] {
        assert!(log.contains(&format!("<td>{action}</td>")), "{action}");
    }
    assert!(log.contains("mod@example.test") && log.contains("griefing"));
    // Revoked roles take effect at once.
    let r = h
        .post_form(
            "/admin/staff/remove",
            &o,
            &[("csrf", &csrf_o), ("email", "mod@example.test")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        h.get("/admin", Some(&m)).await.status(),
        StatusCode::NOT_FOUND
    );
    test_db.close().await;
}

/// The ticket in a ticket response, verified against the published key for `audience`.
async fn verify_issued(
    h: &Harness,
    issued: &Value,
    audience: &str,
) -> Result<ticket::TicketClaims, ticket::TicketError> {
    let keys: Value = h
        .get("/api/v1/ticket-keys", None)
        .await
        .json()
        .await
        .unwrap();
    let spki = STANDARD
        .decode(keys["keys"][0]["spki"].as_str().unwrap())
        .unwrap();
    let mut verifying = VerifyingKeys::default();
    verifying.insert(
        keys["keys"][0]["id"].as_u64().unwrap() as u8,
        VerifyingKey::from_public_key_der(&spki).unwrap(),
    );
    let bytes = STANDARD.decode(issued["ticket"].as_str().unwrap()).unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp() as u64;
    ticket::verify(&bytes, &verifying, audience, now)
}

/// A service account with one minted token, made through the database functions.
async fn service_with_token(pool: &PgPool, name: &str) -> (i32, String) {
    let db::ServiceCreateOutcome::Created { account_id, .. } =
        db::create_service_account(pool, name).await.unwrap()
    else {
        panic!("{name} not created");
    };
    let minted = db::mint_service_token(pool, account_id, "test")
        .await
        .unwrap()
        .unwrap();
    (account_id, minted.token)
}

#[sqlx::test]
async fn tickets_for_each_game_server(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let (id, token) = service_with_token(&h.pool, "PlayerLinux").await;

    // No body, an empty body, or no audience field: the default audience, as before.
    for body in [
        None,
        Some(""),
        Some("{}"),
        Some(r#"{"audience":null}"#),
        Some(r#"{"audience":""}"#),
    ] {
        let (status, issued) = h.ticket_with(&token, body).await;
        assert_eq!(status, StatusCode::OK, "{body:?}: {issued}");
        assert_eq!(issued["audience"], AUDIENCE, "{body:?}");
        let claims = verify_issued(&h, &issued, AUDIENCE).await.unwrap();
        assert_eq!(claims.audience, AUDIENCE);
        assert_eq!(claims.account_id, id as u32);
        assert_eq!(claims.player_name, "PlayerLinux");
    }

    // A configured extra audience, in any case; the ticket verifies only there.
    for requested in [EXTRA_AUDIENCE, "Test.Example.TEST"] {
        let (status, issued) = h
            .ticket_with(&token, Some(&json!({ "audience": requested }).to_string()))
            .await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        assert_eq!(issued["audience"], EXTRA_AUDIENCE);
        let claims = verify_issued(&h, &issued, EXTRA_AUDIENCE).await.unwrap();
        assert_eq!(claims.audience, EXTRA_AUDIENCE);
        assert_eq!(
            verify_issued(&h, &issued, AUDIENCE).await,
            Err(ticket::TicketError::WrongAudience)
        );
    }
    let (status, issued) = h
        .ticket_with(&token, Some(&json!({ "audience": AUDIENCE }).to_string()))
        .await;
    assert_eq!(
        (status, issued["audience"].as_str()),
        (StatusCode::OK, Some(AUDIENCE))
    );

    // Anything else is refused.
    for unknown in ["other.example.test", "play.example.test.evil", "a b"] {
        let (status, body) = h
            .ticket_with(&token, Some(&json!({ "audience": unknown }).to_string()))
            .await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("unknown_audience")),
            "{unknown}"
        );
    }
    for bad in ["nope", "[]", r#"{"audience":7}"#] {
        let (status, body) = h.ticket_with(&token, Some(bad)).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_request")),
            "{bad}"
        );
    }
    // Authentication still comes first.
    let (status, body) = h
        .ticket_with(
            "mgd_made-up",
            Some(&json!({ "audience": "other.example.test" }).to_string()),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("invalid_token"))
    );
    test_db.close().await;
}

#[sqlx::test]
async fn service_accounts_in_the_admin_panel(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    for (email, role) in [
        ("mod@example.test", "moderator"),
        ("admin@example.test", "admin"),
    ] {
        let r = h
            .post_form(
                "/admin/staff/set",
                &o,
                &[("csrf", &csrf_o), ("email", email), ("role", role)],
            )
            .await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER, "{email}");
    }
    let a = h.sign_in("admin").await;
    let csrf_a = h.csrf(&a).await;
    let m = h.sign_in("mod").await;
    let csrf_m = h.csrf(&m).await;
    let u = h.sign_in("player").await;
    let csrf_u = h.csrf(&u).await;
    h.post_form("/claim", &u, &[("csrf", &csrf_u), ("name", "Alpha")])
        .await;

    // Players do not see the section at all.
    assert_eq!(
        h.get("/admin/service-accounts", Some(&u)).await.status(),
        StatusCode::NOT_FOUND
    );
    let r = h
        .post_form(
            "/admin/service-accounts/create",
            &u,
            &[("csrf", &csrf_u), ("name", "Zed")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // A reserved test name is taken over, keeping its stored spelling.
    h.post_form(
        "/admin/names/reserve",
        &a,
        &[("csrf", &csrf_a), ("names", "PlayerLinux"), ("reason", "")],
    )
    .await;
    let r = h
        .post_form(
            "/admin/service-accounts/create",
            &a,
            &[("csrf", "bad"), ("name", "playerlinux")],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::FORBIDDEN,
        "forms need the CSRF token"
    );
    let r = h
        .post_form(
            "/admin/service-accounts/create",
            &m,
            &[("csrf", &csrf_m), ("name", "playerlinux")],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::FORBIDDEN,
        "moderators cannot create"
    );
    assert_eq!(db::reserved_names(&h.pool).await.unwrap().len(), 1);
    let r = h
        .post_form(
            "/admin/service-accounts/create",
            &a,
            &[("csrf", &csrf_a), ("name", "playerlinux")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(db::reserved_names(&h.pool).await.unwrap().is_empty());
    let svc = db::account_by_name(&h.pool, "PLAYERLINUX")
        .await
        .unwrap()
        .unwrap();
    assert!(svc.is_service() && svc.provider == "service" && svc.email.is_none());
    assert_eq!(svc.player_name.as_deref(), Some("PlayerLinux"));
    assert!(svc.subject.len() > 20);

    // Names a person holds, or another service account, or invalid names, are refused.
    for (name, needle) in [
        ("alpha", "already holds that name"),
        ("PlayerLinux", "already holds that name"),
        ("9bad", "starting with a letter"),
    ] {
        let r = h
            .post_form(
                "/admin/service-accounts/create",
                &a,
                &[("csrf", &csrf_a), ("name", name)],
            )
            .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{name}");
        assert!(r.text().await.unwrap().contains(needle), "{name}");
    }
    let alpha = db::account_by_name(&h.pool, "Alpha")
        .await
        .unwrap()
        .unwrap();
    assert!(!alpha.is_service());

    // Minting: admins only; the token is shown once, uncached, and stored hashed.
    let mint = format!("/admin/service-accounts/{}/mint", svc.id);
    let r = h
        .post_form(&mint, &m, &[("csrf", &csrf_m), ("label", "linux-ci")])
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(&mint, &a, &[("csrf", &csrf_a), ("label", " ")])
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = h
        .post_form(
            &format!("/admin/service-accounts/{}/mint", alpha.id),
            &a,
            &[("csrf", &csrf_a), ("label", "x")],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::NOT_FOUND,
        "people get no minted tokens"
    );
    let r = h
        .post_form(&mint, &a, &[("csrf", &csrf_a), ("label", "linux-ci")])
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let html = r.text().await.unwrap();
    let marker = "value=\"mgd_";
    let start = html.find(marker).unwrap() + "value=\"".len();
    let token = html[start..start + html[start..].find('"').unwrap()].to_owned();
    assert_eq!(token.len(), 4 + 43);
    assert!(html.contains("never shown again"));
    let devices = db::devices(&h.pool, svc.id).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].label, "linux-ci");
    let (hash,): (Vec<u8>,) = sqlx::query_as("SELECT token_hash FROM devices WHERE id = $1")
        .bind(devices[0].id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(hash, Sha256::digest(token.as_bytes()).to_vec());

    // The token gets tickets like any launcher's.
    let (status, issued) = h.ticket(&token).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    assert_eq!(issued["player_name"], "PlayerLinux");
    assert_eq!(issued["account_id"], svc.id);
    verify_issued(&h, &issued, AUDIENCE).await.unwrap();

    // Moderators see service accounts but cannot change them.
    let list = page_html(h.get("/admin/service-accounts", Some(&m)).await).await;
    assert!(list.contains("PlayerLinux") && list.contains("linux-ci"));
    // Created, and each token's minted and last-used times.
    assert!(support::page_times(&list) >= 3, "{list}");
    assert!(!list.contains("Mint token") && !list.contains("Revoke"));
    let user_page = h
        .get(&format!("/admin/users/{}", svc.id), Some(&m))
        .await
        .text()
        .await
        .unwrap();
    assert!(user_page.contains("Only admins can change service accounts."));
    assert!(!user_page.contains("Ban account"));
    let r = h
        .post_form(
            &format!("/admin/users/{}/ban", svc.id),
            &m,
            &[("csrf", &csrf_m), ("reason", "x")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            &format!("/admin/users/{}/devices/{}/revoke", svc.id, devices[0].id),
            &m,
            &[("csrf", &csrf_m)],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            &format!(
                "/admin/service-accounts/{}/devices/{}/revoke",
                svc.id, devices[0].id
            ),
            &m,
            &[("csrf", &csrf_m)],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(h.ticket(&token).await.0, StatusCode::OK);
    // Moderators still act on people.
    let r = h
        .post_form(
            &format!("/admin/users/{}/ban", alpha.id),
            &m,
            &[("csrf", &csrf_m), ("reason", "x")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);

    // The users list marks service accounts.
    let users = h
        .get("/admin/users?q=player", Some(&a))
        .await
        .text()
        .await
        .unwrap();
    assert!(users.contains("PlayerLinux") && users.contains("service account"));

    // Bans apply to service accounts (admins can ban them).
    let r = h
        .post_form(
            &format!("/admin/users/{}/ban", svc.id),
            &a,
            &[("csrf", &csrf_a), ("reason", "runaway bot")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let (status, body) = h.ticket(&token).await;
    assert_eq!(
        (status, body["reason"].as_str()),
        (StatusCode::FORBIDDEN, Some("runaway bot"))
    );
    h.post_form(
        &format!("/admin/users/{}/unban", svc.id),
        &a,
        &[("csrf", &csrf_a)],
    )
    .await;
    assert_eq!(h.ticket(&token).await.0, StatusCode::OK);

    // The token appears on no page after the mint page.
    for (path, session) in [
        ("/admin/service-accounts", &a),
        ("/admin/service-accounts", &o),
        (&format!("/admin/users/{}", svc.id)[..], &a),
        ("/admin/users", &a),
        ("/admin", &o),
        ("/admin/names", &a),
        ("/account", &a),
    ] {
        let html = h.get(path, Some(session)).await.text().await.unwrap();
        assert!(!html.contains(&token), "{path}");
        assert!(!html.contains(&token[4..20]), "{path}");
    }
    let r = h.get(&mint, Some(&a)).await;
    assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);

    // Revoking stops the token.
    let r = h
        .post_form(
            &format!(
                "/admin/service-accounts/{}/devices/{}/revoke",
                svc.id, devices[0].id
            ),
            &a,
            &[("csrf", &csrf_a)],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(h.ticket(&token).await.0, StatusCode::UNAUTHORIZED);

    // Deleting keeps the name reserved.
    let delete = format!("/admin/service-accounts/{}/delete", svc.id);
    let r = h
        .post_form(
            &delete,
            &m,
            &[("csrf", &csrf_m), ("confirm", "PlayerLinux")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(&delete, &a, &[("csrf", &csrf_a), ("confirm", "nope")])
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = h
        .post_form(
            &format!("/admin/service-accounts/{}/delete", alpha.id),
            &a,
            &[("csrf", &csrf_a), ("confirm", "Alpha")],
        )
        .await;
    assert_eq!(
        r.status(),
        StatusCode::NOT_FOUND,
        "people are not deleted here"
    );
    let r = h
        .post_form(
            &delete,
            &a,
            &[("csrf", &csrf_a), ("confirm", "playerlinux")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(db::account(&h.pool, svc.id).await.unwrap().is_none());
    assert_eq!(
        db::reserved_names(&h.pool).await.unwrap()[0].name,
        "PlayerLinux"
    );
    assert!(db::account(&h.pool, alpha.id).await.unwrap().is_some());

    // Every change is in the audit log.
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    for action in [
        "create service account",
        "mint service token",
        "revoke service token",
        "delete service account",
    ] {
        assert!(log.contains(&format!("<td>{action}</td>")), "{action}");
    }
    assert!(log.contains("took over the reserved name") && log.contains("linux-ci"));
    test_db.close().await;
}

#[sqlx::test]
async fn service_accounts_never_sign_in(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let (id, _) = service_with_token(&h.pool, "PlayerRetail").await;
    let svc = db::account(&h.pool, id).await.unwrap().unwrap();

    // A Google identity with the very same subject is a different, new person account.
    let s = h.sign_in(&svc.subject).await;
    let page = h.get("/account", Some(&s)).await.text().await.unwrap();
    assert!(page.contains("choose one") && !page.contains("PlayerRetail"));
    let person = db::session(&h.pool, &s).await.unwrap().unwrap().account;
    assert_ne!(person.id, id);
    assert!(!person.is_service());

    // Even a session row for a service account is never honoured.
    let forged = db::create_session(&h.pool, id, time::Duration::days(1))
        .await
        .unwrap();
    assert!(
        db::session(&h.pool, &forged.cookie)
            .await
            .unwrap()
            .is_none()
    );
    for path in ["/account", "/link", "/claim", "/admin"] {
        let r = h.get(path, Some(&forged.cookie)).await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER, "{path}");
        assert!(location(&r).starts_with("/auth/google"), "{path}");
    }

    // The schema keeps the two kinds apart.
    for sql in [
        "INSERT INTO accounts (provider, subject, kind, player_name, email)
         VALUES ('service', 'x1', 'service', 'Svc1', 'a@example.test')",
        "INSERT INTO accounts (provider, subject, kind) VALUES ('service', 'x2', 'service')",
        "INSERT INTO accounts (provider, subject, player_name) VALUES ('service', 'x3', 'Svc3')",
        "INSERT INTO accounts (provider, subject, kind, player_name)
         VALUES ('google', 'x4', 'service', 'Svc4')",
    ] {
        assert!(sqlx::query(sql).execute(&h.pool).await.is_err(), "{sql}");
    }
    test_db.close().await;
}

#[sqlx::test]
async fn service_account_commands(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::admin::{ServiceCommand, run_service};
    let h = harness(pool).await;
    async fn run(pool: &PgPool, command: ServiceCommand) -> anyhow::Result<String> {
        let mut out = Vec::new();
        run_service(pool, command, &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }
    assert_eq!(
        db::reserve_name(&h.pool, "Player", "test identity")
            .await
            .unwrap(),
        db::ReserveOutcome::Reserved
    );
    let u = h.sign_in("person").await;
    let csrf = h.csrf(&u).await;
    h.post_form("/claim", &u, &[("csrf", &csrf), ("name", "Kilo")])
        .await;

    let out = run(
        &h.pool,
        ServiceCommand::Create {
            name: "player".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        out.starts_with("created service account") && out.contains("(Player)"),
        "{out}"
    );
    assert!(db::reserved_names(&h.pool).await.unwrap().is_empty());
    for name in ["kilo", "Player", "1x"] {
        assert!(
            run(&h.pool, ServiceCommand::Create { name: name.into() })
                .await
                .is_err(),
            "{name}"
        );
    }

    let out = run(
        &h.pool,
        ServiceCommand::MintToken {
            name: "PLAYER".into(),
            label: "lab runner".into(),
        },
    )
    .await
    .unwrap();
    let token = out.trim().to_owned();
    assert!(token.starts_with("mgd_") && !token.contains(char::is_whitespace));
    for (name, label) in [("Kilo", "x"), ("Nobody", "x"), ("Player", "")] {
        assert!(
            run(
                &h.pool,
                ServiceCommand::MintToken {
                    name: name.into(),
                    label: label.into(),
                },
            )
            .await
            .is_err(),
            "{name} {label:?}"
        );
    }
    let (status, issued) = h.ticket(&token).await;
    assert_eq!(
        (status, issued["player_name"].as_str()),
        (StatusCode::OK, Some("Player"))
    );

    let list = run(&h.pool, ServiceCommand::List).await.unwrap();
    let svc = db::service_account_by_name(&h.pool, "player")
        .await
        .unwrap()
        .unwrap();
    let device = db::devices(&h.pool, svc.id).await.unwrap()[0].id;
    assert!(
        list.contains(&format!("Player\taccount {}", svc.id))
            && list.contains(&format!("device {device}\tlab runner"))
            && !list.contains(&token)
            && !list.contains("Kilo"),
        "{list}"
    );

    assert!(
        run(
            &h.pool,
            ServiceCommand::RevokeToken {
                name: "Player".into(),
                device_id: device + 100,
            },
        )
        .await
        .is_err()
    );
    let out = run(
        &h.pool,
        ServiceCommand::RevokeToken {
            name: "Player".into(),
            device_id: device,
        },
    )
    .await
    .unwrap();
    assert!(out.contains("revoked"));
    assert_eq!(h.ticket(&token).await.0, StatusCode::UNAUTHORIZED);

    // The commands are audit-logged under the command-line operator, without the token.
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT actor_email, action, detail FROM admin_log ORDER BY id")
            .fetch_all(&h.pool)
            .await
            .unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(
        actions,
        [
            "create service account",
            "mint service token",
            "revoke service token"
        ]
    );
    assert!(
        rows.iter()
            .all(|r| r.0 == mag_portal::staff::OPERATOR_ACTOR)
    );
    assert!(rows.iter().all(|r| !r.2.contains(&token[4..])));
    test_db.close().await;
}

#[sqlx::test]
async fn homepage_for_everyone(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let r = h.get("/", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    let html = r.text().await.unwrap();
    for needle in [
        "https://github.com/tekulvw/mag-recomp\"",
        "https://github.com/tekulvw/mag-recomp-server\"",
        "href=\"/auth/google\">Sign in with Google",
        "play.magtest.online",
        "Not affiliated",
    ] {
        assert!(html.contains(needle), "{needle}");
    }
    // Signed in, the page stays and offers the account instead.
    let s = h.sign_in("home").await;
    let html = h.get("/", Some(&s)).await.text().await.unwrap();
    let sign_in_button = "href=\"/auth/google\">Sign in with Google";
    assert!(
        html.contains("href=\"/claim\">Choose your player name") && !html.contains(sign_in_button)
    );
    let csrf = h.csrf(&s).await;
    h.post_form("/claim", &s, &[("csrf", &csrf), ("name", "Hotel")])
        .await;
    assert!(
        h.get("/", Some(&s))
            .await
            .text()
            .await
            .unwrap()
            .contains("Your account: Hotel")
    );
    test_db.close().await;
}

#[sqlx::test]
async fn fonts_and_page_policy(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let r = h
        .get("/fonts/chakra-petch-latin-700-normal.woff2", None)
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "font/woff2");
    assert!(
        r.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"wOF2 fixture");
    for bad in [
        "/fonts/missing-font.woff2",
        "/fonts/chakra-petch-latin-700-normal.ttf",
        "/fonts/..%2Fchakra-petch-latin-700-normal.woff2",
        "/fonts/Chakra-Petch.woff2",
    ] {
        assert_eq!(
            h.get(bad, None).await.status(),
            StatusCode::NOT_FOUND,
            "{bad}"
        );
    }
    let csp = h.get("/", None).await.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("font-src 'self'") && csp.contains("style-src 'self'"));
    assert!(!csp.contains("unsafe-inline"));
    // The policy blocks inline styles, so no template may rely on one.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("style=") && !text.contains("<style"),
            "{}",
            path.display()
        );
    }
    test_db.close().await;
}

#[sqlx::test]
async fn stylesheet_address_changes_with_its_content(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let version = mag_portal::web::style_version();
    let html = h.get("/", None).await.text().await.unwrap();
    assert!(
        html.contains(&format!("href=\"/style.css?v={version}\"")),
        "{html}"
    );
    let r = h.get(&format!("/style.css?v={version}"), None).await;
    assert_eq!(
        r.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(r.text().await.unwrap().contains("--accent"));
    for stale in ["/style.css", "/style.css?v=old"] {
        let r = h.get(stale, None).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            r.headers()["cache-control"],
            "public, max-age=300",
            "{stale}"
        );
    }
    test_db.close().await;
}

/// The page check the other tests run: what it accepts and what it refuses.
#[test]
fn page_time_check() {
    let instant = "<time class=\"local-time\" datetime=\"2026-11-28T05:25:47Z\" \
                   title=\"2026-11-28 05:25:47 UTC\">2026-11-28 05:25 UTC</time>";
    let page = format!(
        "<p data-generated=\"2026-11-28T05:25:47.5Z\">{instant} \
         <time datetime=\"2026-11-28\">2026-11-28</time> 2026-11-28 05:25 UTC 18:30</p>"
    );
    assert_eq!(support::page_times(&page), 2);
    assert_eq!(support::page_times("<p>No times.</p>"), 0);
    for bad in [
        "<td>2026-11-28 05:25</td>",
        "<td>2026-11-28T05:25:47</td>",
        "<time>2026-11-28 05:25 UTC</time>",
        "<time class=\"local-time\" datetime=\"2026-11-28T05:25:47\">x UTC</time>",
        "<time class=\"local-time\" datetime=\"2026-11-28T05:25:47Z\">Nov 28</time>",
        "<time datetime=\"2026-11-28T05:25:47Z\">2026-11-28 05:25 UTC</time>",
    ] {
        assert!(
            std::panic::catch_unwind(|| support::page_times(bad)).is_err(),
            "{bad}"
        );
    }
}

/// Every page loads the time script from the site itself (the policy allows no inline script),
/// at an address that changes with its content.
#[sqlx::test]
async fn time_script_on_every_page(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let version = mag_portal::web::time_script_version();
    for path in ["/", "/privacy", "/status"] {
        let html = page_html(h.get(path, None).await).await;
        assert!(
            html.contains(&format!(
                "<script src=\"/time.js?v={version}\" defer></script>"
            )),
            "{path}: {html}"
        );
    }
    let r = h.get(&format!("/time.js?v={version}"), None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    assert_eq!(
        r.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let script = r.text().await.unwrap();
    // It only sets text and titles: nothing is ever parsed as HTML.
    assert!(script.contains("Intl.DateTimeFormat") && !script.contains("innerHTML"));
    let r = h.get("/time.js", None).await;
    assert_eq!(r.headers()["cache-control"], "public, max-age=300");
    test_db.close().await;
}

// ---- Game server section, against a fake Medius admin API ----

const GAME_SECRET: &[u8] = b"game-admin-test-secret-0123456789abcdef";

#[derive(Clone, Copy)]
struct FakeCharacter {
    level: u32,
    experience: i64,
    skill_points: i64,
    session: &'static str,
}

#[derive(Default)]
struct MediusState {
    characters: HashMap<String, FakeCharacter>,
    nonces: std::collections::HashSet<String>,
    actors: Vec<String>,
    /// Signed `GET /admin/v1/status` requests answered.
    status_calls: u32,
    /// (account, level, apply) for each signed level request.
    level_calls: Vec<(String, u32, bool)>,
    /// Active voice sanctions, as `GET /admin/v1/voice` lists them.
    voice: Vec<Value>,
    /// (account, action, request body) for each signed voice request.
    voice_calls: Vec<(String, String, Value)>,
    /// Answer voice requests with 404, as a game server without voice moderation does.
    no_voice: bool,
    /// Make each applied change, then answer only after this long (live, 2026-11-27: the mute was
    /// saved but the answer came after the portal had given up).
    slow_apply: Option<std::time::Duration>,
    /// Make each applied change, then answer 503 `outcome_unknown`, as Medius does when its
    /// database stops answering during the commit.
    unknown_apply: bool,
}

struct FakeMedius {
    secret: Vec<u8>,
    state: Mutex<MediusState>,
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// [`fake_medius_answer`], late or with an unknown outcome for applied changes when asked.
async fn fake_medius(
    State(m): State<Arc<FakeMedius>>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let applies = method == axum::http::Method::POST
        && serde_json::from_slice::<Value>(&body).is_ok_and(|v| v["apply"] == json!(true));
    let (delay, unknown) = {
        let st = m.state.lock().unwrap();
        (st.slow_apply, st.unknown_apply)
    };
    let response = fake_medius_answer(State(m), method, uri, headers, body).await;
    if !applies || response.status() != StatusCode::OK {
        return response;
    }
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    if unknown {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "outcome_unknown", "message": "synthetic"})),
        )
            .into_response();
    }
    response
}

/// Checks the request's signature independently of the portal's code, then answers like the
/// Medius admin API contract.
async fn fake_medius_answer(
    State(m): State<Arc<FakeMedius>>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use hmac::{KeyInit, Mac};
    let reply = |status: StatusCode, value: Value| (status, Json(value)).into_response();
    let unauthorized = || reply(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"}));
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let (Some(ts), Some(nonce), Some(actor), Some(sig)) = (
        header("x-mag-admin-timestamp"),
        header("x-mag-admin-nonce"),
        header("x-mag-admin-actor"),
        header("x-mag-admin-signature"),
    ) else {
        return unauthorized();
    };
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if ts.parse::<i64>().map_or(true, |t| (now - t).abs() > 300)
        || !(16..=64).contains(&nonce.len())
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return unauthorized();
    }
    let path = uri.path_and_query().unwrap().as_str().to_owned();
    let signed = format!(
        "{method}\n{path}\n{ts}\n{nonce}\n{}",
        lower_hex(&Sha256::digest(&body))
    );
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&m.secret).unwrap();
    mac.update(signed.as_bytes());
    if lower_hex(&mac.finalize().into_bytes()) != sig {
        return unauthorized();
    }
    let mut st = m.state.lock().unwrap();
    if !st.nonces.insert(nonce) {
        return unauthorized();
    }
    st.actors.push(actor);
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method.as_str(), segments.as_slice()) {
        ("GET", ["admin", "v1", "status"]) => {
            st.status_calls += 1;
            reply(StatusCode::OK, fake_status())
        }
        ("GET", ["admin", "v1", "accounts"]) => {
            let accounts: Vec<Value> = st
                .characters
                .iter()
                .map(|(a, c)| {
                    json!({"account": a, "level": c.level, "experience": c.experience,
                           "skill_points": c.skill_points, "session": c.session})
                })
                .collect();
            reply(StatusCode::OK, json!({ "accounts": accounts }))
        }
        ("GET", ["admin", "v1", "voice"]) if !st.no_voice => {
            reply(StatusCode::OK, json!({ "sanctions": st.voice }))
        }
        ("POST", ["admin", "v1", "voice", account, action]) if !st.no_voice => {
            let (account, action) = (account.to_string(), action.to_string());
            let Ok(request) = serde_json::from_slice::<Value>(&body) else {
                return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"}));
            };
            st.voice_calls
                .push((account.clone(), action.clone(), request.clone()));
            let Some(apply) = request["apply"].as_bool() else {
                return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"}));
            };
            if !st.characters.contains_key(&account) {
                return reply(StatusCode::NOT_FOUND, json!({"error": "not_found"}));
            }
            let kind = action.trim_start_matches("un").to_owned();
            let lift = action.starts_with("un");
            let actor = st.actors.last().cloned().unwrap_or_default();
            let position = st
                .voice
                .iter()
                .position(|s| s["account"] == account.as_str() && s["kind"] == kind.as_str());
            let before = position.map(|i| st.voice[i].clone());
            if lift && before.is_none() {
                return reply(
                    StatusCode::CONFLICT,
                    json!({"error": "not_sanctioned",
                           "message": format!("not-sanctioned: {account} has no active voice {kind}")}),
                );
            }
            let after = (!lift).then(|| {
                json!({"account": account, "kind": kind, "reason": request["reason"],
                       "operator": actor, "created_at": "2026-11-27T12:00:00Z",
                       "expires_at": request["duration_minutes"].as_u64()
                           .map(|m| format!("2026-11-27T12:00:00Z+{m}m"))})
            });
            if apply {
                if let Some(i) = position {
                    st.voice.remove(i);
                }
                if let Some(a) = &after {
                    st.voice.push(a.clone());
                }
            }
            reply(
                StatusCode::OK,
                json!({"applied": apply, "account": account, "action": action, "kind": kind,
                       "before": before, "after": after}),
            )
        }
        ("GET", ["admin", "v1", "characters", "broken"]) => {
            reply(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "boom"}))
        }
        ("GET", ["admin", "v1", "characters", account]) => match st.characters.get(*account) {
            Some(c) => reply(
                StatusCode::OK,
                json!({"account": account, "level": c.level, "experience": c.experience,
                       "skill_points": c.skill_points,
                       "next_level_experience": (c.level < 70).then(|| (c.level + 1) * 1000),
                       "session": c.session}),
            ),
            None => reply(StatusCode::NOT_FOUND, json!({"error": "not_found"})),
        },
        ("POST", ["admin", "v1", "characters", account, "level"]) => {
            let account = account.to_string();
            let Ok(request) = serde_json::from_slice::<Value>(&body) else {
                return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"}));
            };
            let (Some(level), Some(apply)) =
                (request["level"].as_u64(), request["apply"].as_bool())
            else {
                return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"}));
            };
            let level = level as u32;
            st.level_calls.push((account.clone(), level, apply));
            let Some(c) = st.characters.get(&account).copied() else {
                return reply(StatusCode::NOT_FOUND, json!({"error": "not_found"}));
            };
            let refuse = |code: &str, message: String| {
                reply(
                    StatusCode::CONFLICT,
                    json!({"error": code, "message": message}),
                )
            };
            if c.session == "online" {
                return refuse("online", format!("{account} is online"));
            }
            if level < c.level {
                return refuse("lower_level", "levels only go up".into());
            }
            if level == c.level {
                return refuse("same_level", "already at that level".into());
            }
            if level > 70 {
                return refuse("level_out_of_range", "levels go up to 70".into());
            }
            let after = FakeCharacter {
                level,
                experience: i64::from(level) * 1000,
                skill_points: c.skill_points + i64::from(level - c.level) * 2,
                session: c.session,
            };
            let backup = apply.then(|| format!("{account}-backup-1.json"));
            if apply {
                st.characters.insert(account.clone(), after);
            }
            reply(
                StatusCode::OK,
                json!({"applied": apply, "account": account, "from_level": c.level,
                       "to_level": level, "experience_before": c.experience,
                       "experience_after": after.experience,
                       "skill_points_before": c.skill_points,
                       "skill_points_after": after.skill_points, "backup": backup}),
            )
        }
        _ => reply(StatusCode::NOT_FOUND, json!({"error": "not_found"})),
    }
}

/// A registry with an open Sabotage world in a match and a faulted Acquisition world; counts
/// only, as the Medius contract promises.
fn fake_status() -> Value {
    json!({
        "medius_version": "9.8.7", "gameplay_version": "6.5.4", "started_at": "2026-11-26T10:00:00Z",
        "uptime_seconds": 7260, "mode": "registry", "players_online": 4, "queued": 2,
        "queued_by_game_type": [2, 0, 0], "dme_ports": {"capacity": 32, "in_use": 4},
        "human_limit": 32,
        "worlds": [
            // A 64-Character row where every seat may be human, held to 32 by the ports.
            {"worker_id": "sabotage", "state": "open", "game_types": [0], "human_capacity": 64,
             "human_limit": 32, "world_size": 64, "bot_fill": 64,
             "pending": 0, "match": {"profile_id": "synthetic-sab-raven-sver", "game_type": 0,
             "attacking_faction": 1, "defending_faction": 2, "humans": 3, "attacking_humans": 2,
             "defending_humans": 1, "human_capacity": 64, "human_limit": 32, "bots": 61,
             "bot_fill": 64, "world_capacity": 64}},
            // As an older game server reports it: the row cap only.
            {"worker_id": "acquisition", "state": "faulted", "game_types": [1],
             "human_capacity": 32, "pending": 0, "match": null},
            {"worker_id": "domination", "state": "open", "game_types": [2], "human_capacity": 32,
             "human_limit": 32, "world_size": 256, "bot_fill": 256, "pending": 0, "match": null}
        ]
    })
}

async fn fake_medius_server() -> (Arc<FakeMedius>, String) {
    let mut state = MediusState::default();
    for (account, level, session) in [
        ("alpha", 5, "offline"),
        ("bravo", 12, "online"),
        ("charlie", 3, "unknown"),
    ] {
        state.characters.insert(
            account.into(),
            FakeCharacter {
                level,
                experience: i64::from(level) * 1000,
                skill_points: 10,
                session,
            },
        );
    }
    let medius = Arc::new(FakeMedius {
        secret: GAME_SECRET.to_vec(),
        state: Mutex::new(state),
    });
    let addr = serve(
        Router::new()
            .fallback(fake_medius)
            .with_state(medius.clone()),
    )
    .await;
    (medius, format!("http://{addr}"))
}

/// A response's status and body; a page's times are checked on the way (see `page_times`).
async fn body(r: reqwest::Response) -> (StatusCode, String) {
    let status = r.status();
    let text = r.text().await.unwrap();
    if text.starts_with("<!doctype html>") {
        support::page_times(&text);
    }
    (status, text)
}

/// A page's HTML, with its times checked.
async fn page_html(r: reqwest::Response) -> String {
    let text = r.text().await.unwrap();
    support::page_times(&text);
    text
}

#[sqlx::test]
async fn game_server_not_configured(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let u = h.sign_in("player").await;
    assert_eq!(
        h.get("/admin/game", Some(&u)).await.status(),
        StatusCode::NOT_FOUND
    );
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let (status, page) = body(h.get("/admin/game", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("not configured") && page.contains("href=\"/admin/game\""));
    let (status, page) = body(h.get("/admin/game/characters/alpha", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("not configured") && !page.contains("Preview change"));
    let r = h
        .post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    let player = id_of(&h, "player").await;
    let page = h
        .get(&format!("/admin/users/{player}"), Some(&o))
        .await
        .text()
        .await
        .unwrap();
    assert!(!page.contains("/admin/game/characters/"));
    test_db.close().await;
}

#[sqlx::test]
async fn game_server_characters_and_levels(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let (medius, url) = fake_medius_server().await;
    let h = harness_with(
        pool,
        Some(GameAdmin::new(&url, GAME_SECRET.to_vec()).unwrap()),
    )
    .await;
    let calls = || medius.state.lock().unwrap().level_calls.clone();
    let level_of = |a: &str| medius.state.lock().unwrap().characters[a].level;

    // A player named Alpha, who has no role: the section does not exist for them.
    let u = h.sign_in("player").await;
    let csrf_u = h.csrf(&u).await;
    h.post_form("/claim", &u, &[("csrf", &csrf_u), ("name", "Alpha")])
        .await;
    let player = id_of(&h, "player").await;
    for path in ["/admin/game", "/admin/game/characters/alpha"] {
        assert_eq!(
            h.get(path, Some(&u)).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let r = h
        .post_form(
            "/admin/game/characters/alpha/level/confirm",
            &u,
            &[("csrf", &csrf_u), ("level", "20")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let r = h
        .post_form(
            "/admin/staff/set",
            &o,
            &[
                ("csrf", &csrf_o),
                ("email", "mod@example.test"),
                ("role", "moderator"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);

    // Moderators see the list and characters, linked to portal accounts, but cannot set levels.
    let m = h.sign_in("mod").await;
    let csrf_m = h.csrf(&m).await;
    let (status, page) = body(h.get("/admin/game", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("href=\"/admin/game\""));
    assert!(page.contains("href=\"/admin/game/characters/alpha\""));
    assert!(page.contains(&format!("href=\"/admin/users/{player}\"")));
    assert!(page.contains("bravo") && page.contains("<strong>online</strong>"));
    assert!(page.contains("charlie") && page.contains("unknown"));
    assert!(
        medius
            .state
            .lock()
            .unwrap()
            .actors
            .contains(&"mod@example.test".to_owned())
    );
    let (status, page) = body(h.get("/admin/game/characters/alpha", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("5000") && page.contains("6000") && page.contains("offline"));
    assert!(page.contains(&format!("href=\"/admin/users/{player}\"")));
    assert!(!page.contains("Preview change"));
    for path in [
        "/admin/game/characters/alpha/level",
        "/admin/game/characters/alpha/level/confirm",
    ] {
        let r = h
            .post_form(path, &m, &[("csrf", &csrf_m), ("level", "20")])
            .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert!(calls().is_empty());

    // The portal account page links to the character.
    let page = h
        .get(&format!("/admin/users/{player}"), Some(&o))
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("href=\"/admin/game/characters/alpha\""));

    // Admins and above: the form needs the CSRF token.
    let (_, page) = body(h.get("/admin/game/characters/alpha", Some(&o)).await).await;
    assert!(page.contains("Preview change"));
    for path in [
        "/admin/game/characters/alpha/level",
        "/admin/game/characters/alpha/level/confirm",
    ] {
        let r = h
            .post_form(path, &o, &[("csrf", "bad"), ("level", "20")])
            .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
    }
    let r = h
        .post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "twenty")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(calls().is_empty());

    // A dry run shows the plan and changes nothing.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Nothing has been saved yet") && page.contains("Apply level 20"));
    assert!(page.contains("<td>5</td><td>20</td>"));
    assert!(page.contains("<td>5000</td><td>20000</td>"));
    assert!(page.contains("<td>10</td><td>40</td>"));
    assert!(page.contains("action=\"/admin/game/characters/alpha/level/confirm\""));
    assert_eq!(level_of("alpha"), 5);
    assert_eq!(calls(), vec![("alpha".to_owned(), 20, false)]);

    // Confirming applies it and writes the audit log.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level/confirm",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Level set.") && page.contains("alpha-backup-1.json"));
    assert_eq!(level_of("alpha"), 20);
    assert_eq!(calls().last().unwrap(), &("alpha".to_owned(), 20, true));
    assert_eq!(
        medius.state.lock().unwrap().actors.last().unwrap(),
        "owner@example.test"
    );
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert!(log.contains("set level") && log.contains("5→20, backup alpha-backup-1.json"));
    let (status, page) = body(h.get("/admin/game/characters/Alpha", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("20000"));

    // Refusals are shown plainly, with what to do; nothing is logged.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/bravo/level",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(page.contains("refused the change") && page.contains("bravo is online"));
    assert!(page.contains("quit the game") && !page.contains("Apply level"));
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "3")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(page.contains("lower_level") && page.contains("only be raised"));
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert_eq!(log.matches("set level").count(), 1);

    // Unknown characters, bad names and broken answers.
    let (status, page) = body(h.get("/admin/game/characters/nobody", Some(&o)).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(page.contains("no character"));
    let r = h
        .post_form(
            "/admin/game/characters/nobody/level",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        h.get("/admin/game/characters/a%20b", Some(&o))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let (status, page) = body(h.get("/admin/game/characters/broken", Some(&o)).await).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(page.contains("unexpected answer: HTTP 500"));
    test_db.close().await;
}

#[sqlx::test]
async fn game_server_errors_are_shown(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    // The portal holds a different secret than the game server: every call gets 401.
    let (medius, url) = fake_medius_server().await;
    let h = harness_with(
        pool.clone(),
        Some(GameAdmin::new(&url, b"a-different-secret-0123456789abcdef".to_vec()).unwrap()),
    )
    .await;
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    for path in ["/admin/game", "/admin/game/characters/alpha"] {
        let (status, page) = body(h.get(path, Some(&o)).await).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}");
        assert!(
            page.contains("rejected the portal&#39;s admin credentials")
                || page.contains("rejected the portal's admin credentials"),
            "{page}"
        );
        assert!(!page.contains("a-different-secret"));
    }
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level/confirm",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(page.contains("admin credentials") && page.contains("Nothing was changed"));
    assert!(medius.state.lock().unwrap().level_calls.is_empty());

    // Nothing listening: the page says the game server cannot be reached.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let h = harness_with(
        pool,
        Some(GameAdmin::new(&closed_url, GAME_SECRET.to_vec()).unwrap()),
    )
    .await;
    let o = h.sign_in("owner").await;
    let (status, page) = body(h.get("/admin/game", Some(&o)).await).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(page.contains("Could not reach the game server"), "{page}");
    test_db.close().await;
}

// ---- Game server section with several characters per account ----

/// One character of a fake Medius that keeps character slots.
#[derive(Clone)]
struct SlotCharacter {
    account: &'static str,
    character_id: u32,
    name: &'static str,
    slot: u8,
    faction: u8,
    level: u32,
    deleted_at: Option<&'static str>,
}

impl SlotCharacter {
    fn json(&self) -> Value {
        json!({"character_id": self.character_id, "name": self.name, "slot": self.slot,
               "faction": self.faction, "level": self.level,
               "experience": i64::from(self.level) * 1000, "skill_points": 10,
               "deleted": self.deleted_at.is_some(), "deleted_at": self.deleted_at})
    }
}

#[derive(Default)]
struct SlotMediusState {
    characters: Vec<SlotCharacter>,
    /// The path and query of each request, and its body.
    requests: Vec<(String, String)>,
}

/// Answers like the Medius admin API with character slots: one account entry per character,
/// `?slot=` on the character read and `slot` in the level body (signatures are checked by the
/// other fake; this one looks at what the portal asks for).
async fn fake_slot_medius(
    State(m): State<Arc<Mutex<SlotMediusState>>>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let reply = |status: StatusCode, value: Value| (status, Json(value)).into_response();
    let mut st = m.lock().unwrap();
    let text = String::from_utf8_lossy(&body).into_owned();
    st.requests
        .push((uri.path_and_query().unwrap().to_string(), text.clone()));
    let query_slot = uri
        .query()
        .and_then(|q| q.strip_prefix("slot="))
        .map(|n| n.parse::<u8>().unwrap());
    let segments: Vec<&str> = uri.path().trim_start_matches('/').split('/').collect();
    let live = |st: &SlotMediusState, account: &str| -> Vec<SlotCharacter> {
        let mut v: Vec<SlotCharacter> = st
            .characters
            .iter()
            .filter(|c| c.account == account && c.deleted_at.is_none())
            .cloned()
            .collect();
        v.sort_by_key(|c| c.slot);
        v
    };
    match (method.as_str(), segments.as_slice()) {
        ("GET", ["admin", "v1", "accounts"]) => {
            let accounts: Vec<Value> = st
                .characters
                .iter()
                .map(|c| {
                    let mut v = c.json();
                    v["account"] = c.account.into();
                    v["session"] = "offline".into();
                    v
                })
                .collect();
            reply(StatusCode::OK, json!({ "accounts": accounts }))
        }
        // The character page also shows the account's voice sanctions; this server has none.
        ("GET", ["admin", "v1", "voice"]) => reply(StatusCode::OK, json!({ "sanctions": [] })),
        ("GET", ["admin", "v1", "characters", account]) => {
            let live = live(&st, account);
            let chosen = match query_slot {
                Some(n) => live.iter().find(|c| c.slot == n),
                None => live.first(),
            };
            let Some(c) = chosen else {
                return reply(StatusCode::NOT_FOUND, json!({"error": "not_found"}));
            };
            let all: Vec<Value> = st
                .characters
                .iter()
                .filter(|x| x.account == *account)
                .map(SlotCharacter::json)
                .collect();
            let mut v = c.json();
            v["account"] = (*account).into();
            v["session"] = "offline".into();
            v["next_level_experience"] = (i64::from(c.level + 1) * 1000).into();
            v["characters"] = all.into();
            reply(StatusCode::OK, v)
        }
        ("POST", ["admin", "v1", "characters", account, "level"]) => {
            let account = account.to_string();
            let request: Value = serde_json::from_str(&text).unwrap();
            let (level, apply) = (
                request["level"].as_u64().unwrap() as u32,
                request["apply"].as_bool().unwrap(),
            );
            let live = live(&st, &account);
            let chosen = match request.get("slot").and_then(Value::as_u64) {
                Some(n) => live.iter().find(|c| u64::from(c.slot) == n),
                None if live.len() > 1 => {
                    return reply(
                        StatusCode::CONFLICT,
                        json!({"error": "ambiguous_character",
                               "message": format!("{account} has {} live characters; name a slot", live.len())}),
                    );
                }
                None => live.first(),
            };
            let Some(c) = chosen.cloned() else {
                return reply(StatusCode::NOT_FOUND, json!({"error": "not_found"}));
            };
            if apply {
                let id = c.character_id;
                st.characters
                    .iter_mut()
                    .find(|x| x.character_id == id)
                    .unwrap()
                    .level = level;
            }
            reply(
                StatusCode::OK,
                json!({"applied": apply, "account": account, "from_level": c.level,
                       "to_level": level, "experience_before": i64::from(c.level) * 1000,
                       "experience_after": i64::from(level) * 1000, "skill_points_before": 10,
                       "skill_points_after": 20,
                       "backup": apply.then(|| format!("{account}-backup-1.json")),
                       "character_id": c.character_id, "name": c.name, "slot": c.slot}),
            )
        }
        _ => reply(StatusCode::NOT_FOUND, json!({"error": "not_found"})),
    }
}

#[sqlx::test]
async fn game_server_character_slots(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let medius = Arc::new(Mutex::new(SlotMediusState {
        characters: vec![
            SlotCharacter {
                account: "alpha",
                character_id: 11,
                name: "Alpha",
                slot: 0,
                faction: 1,
                level: 5,
                deleted_at: None,
            },
            SlotCharacter {
                account: "alpha",
                character_id: 12,
                name: "AlphaTwo",
                slot: 2,
                faction: 2,
                level: 30,
                deleted_at: None,
            },
            SlotCharacter {
                account: "alpha",
                character_id: 9,
                name: "AlphaOld",
                slot: 1,
                faction: 0,
                level: 2,
                deleted_at: Some("2026-11-20T08:30:00Z"),
            },
            SlotCharacter {
                account: "bravo",
                character_id: 20,
                name: "Bravo",
                slot: 0,
                faction: 0,
                level: 7,
                deleted_at: None,
            },
        ],
        requests: Vec::new(),
    }));
    let addr = serve(
        Router::new()
            .fallback(fake_slot_medius)
            .with_state(medius.clone()),
    )
    .await;
    let h = harness_with(
        pool,
        Some(GameAdmin::new(&format!("http://{addr}"), GAME_SECRET.to_vec()).unwrap()),
    )
    .await;
    // The last request other than the voice sanctions read the character page makes after it.
    let last_request = || {
        medius
            .lock()
            .unwrap()
            .requests
            .iter()
            .rev()
            .find(|(path, _)| path != "/admin/v1/voice")
            .cloned()
            .unwrap()
    };
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;

    // The list has a row per character, the deleted one marked and not linked to a slot.
    let (status, page) = body(h.get("/admin/game", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("<th>Slot</th>"));
    assert!(page.contains("href=\"/admin/game/characters/alpha?slot=0\">Alpha</a>"));
    assert!(page.contains("href=\"/admin/game/characters/alpha?slot=2\">AlphaTwo</a>"));
    assert!(page.contains("AlphaOld <span class=\"pill\">deleted</span>"));
    assert!(page.contains("href=\"/admin/game/characters/bravo?slot=0\">Bravo</a>"));
    assert!(!page.contains("?slot=1"));

    // Without a slot the page shows the lowest live slot, and lists every character.
    let (status, page) = body(h.get("/admin/game/characters/alpha", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(last_request().0, "/admin/v1/characters/alpha");
    let paths: Vec<String> = medius
        .lock()
        .unwrap()
        .requests
        .iter()
        .rev()
        .take(2)
        .map(|r| r.0.clone())
        .collect();
    assert_eq!(
        paths,
        ["/admin/v1/voice", "/admin/v1/characters/alpha"],
        "the page reads the character, then its sanctions"
    );
    assert!(page.contains("<h2>Slot 0 · Alpha</h2>"));
    assert!(page.contains("href=\"/admin/game/characters/alpha?slot=2\">AlphaTwo</a>"));
    assert!(page.contains(
        "<span class=\"pill\">deleted</span> <time class=\"local-time\" \
         datetime=\"2026-11-20T08:30:00Z\" title=\"2026-11-20 08:30:00 UTC\">2026-11-20 08:30 UTC</time>"
    ));
    assert!(page.contains("<input type=\"hidden\" name=\"slot\" value=\"0\">"));

    let (status, page) = body(h.get("/admin/game/characters/alpha?slot=2", Some(&o)).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(last_request().0, "/admin/v1/characters/alpha?slot=2");
    assert!(page.contains("<h2>Slot 2 · AlphaTwo</h2>") && page.contains("<dd>SVER</dd>"));
    assert!(page.contains("<input type=\"hidden\" name=\"slot\" value=\"2\">"));

    // A free slot (the deleted character's) and a slot that cannot exist.
    let (status, page) = body(h.get("/admin/game/characters/alpha?slot=1", Some(&o)).await).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(page.contains("no live character in slot 1 of alpha"));
    let requests = medius.lock().unwrap().requests.len();
    let (status, _) = body(h.get("/admin/game/characters/alpha?slot=7", Some(&o)).await).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(medius.lock().unwrap().requests.len(), requests);

    // Raising a level names the slot, through the preview and the confirmation.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "40"), ("slot", "2")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(last_request().1, r#"{"level":40,"apply":false,"slot":2}"#);
    assert!(
        page.contains("Character in slot 2: AlphaTwo.") && page.contains("<td>30</td><td>40</td>")
    );
    assert!(page.contains("<input type=\"hidden\" name=\"slot\" value=\"2\">"));
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level/confirm",
            &o,
            &[("csrf", &csrf_o), ("level", "40"), ("slot", "2")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(last_request().1, r#"{"level":40,"apply":true,"slot":2}"#);
    assert!(page.contains("Level set."));
    assert!(page.contains("href=\"/admin/game/characters/alpha?slot=2\">Back to alpha"));
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert!(
        log.contains("slot 2 (AlphaTwo): 30→40, backup alpha-backup-1.json"),
        "{log}"
    );
    {
        let st = medius.lock().unwrap();
        let levels: Vec<u32> = st.characters.iter().map(|c| c.level).collect();
        assert_eq!(levels, [5, 40, 2, 7]);
    }

    // No slot on an account with two live characters: refused, with what to do.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "10")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(last_request().1, r#"{"level":10,"apply":false}"#);
    assert!(page.contains("ambiguous_character") && page.contains("choose the character"));
    let (status, _) = body(
        h.post_form(
            "/admin/game/characters/alpha/level",
            &o,
            &[("csrf", &csrf_o), ("level", "10"), ("slot", "5")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // One live character and no slot: the game server picks it, and the confirmation keeps to it.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/bravo/level",
            &o,
            &[("csrf", &csrf_o), ("level", "10")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Character in slot 0: Bravo."));
    assert!(page.contains("<input type=\"hidden\" name=\"slot\" value=\"0\">"));
    test_db.close().await;
}

// ---- Launcher reports ----

mod report_support {
    use super::*;
    use mag_portal::report_store::{DirStore, ReportStore};
    use std::io::Write;

    pub fn temp_dir(what: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "portal-{what}-{}",
            mag_portal::tokens::b64url(&mag_portal::tokens::random_bytes::<9>())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn dir_reports(dir: &std::path::Path) -> Arc<Reports> {
        Arc::new(Reports::new(Arc::new(DirStore::new(dir)), 30))
    }

    /// Report uploads without the per-address limit getting in the way.
    pub fn roomy_limits() -> web::Limits {
        web::Limits {
            reports: mag_portal::ratelimit::RateLimiter::new(
                100_000,
                std::time::Duration::from_secs(60),
            ),
            ..web::Limits::default()
        }
    }

    pub fn gzip(text: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(text).unwrap();
        enc.finish().unwrap()
    }

    pub fn new_id() -> String {
        let mut b = mag_portal::tokens::random_bytes::<16>();
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let hex = lower_hex(&b);
        format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
    }

    pub fn report(id: &str, logs: &[(&str, &[u8])]) -> Value {
        let logs: serde_json::Map<String, Value> = logs
            .iter()
            .map(|(name, text)| {
                (
                    name.to_string(),
                    json!({ "encoding": "gzip+base64", "data": STANDARD.encode(gzip(text)) }),
                )
            })
            .collect();
        json!({
            "schema_version": 1,
            "report_id": id,
            "trigger": "slow_frames",
            "client": { "version": "1.4.0", "platform": "linux", "gpu": "synthetic" },
            "session": { "server": "play.example.test", "duration_s": 61 },
            "metrics": { "frame_ms": { "p50": 16.6, "p99": 48.0 }, "hitches": 3 },
            "logs": logs,
        })
    }

    /// A person account holding `name`, with a launcher token.
    pub async fn person_with_token(pool: &PgPool, subject: &str, name: &str) -> (i32, String) {
        let a = db::upsert_identity(
            pool,
            "google",
            subject,
            Some(&format!("{subject}@example.test")),
        )
        .await
        .unwrap();
        assert_eq!(
            db::claim_name(pool, a.id, name).await.unwrap(),
            ClaimOutcome::Claimed
        );
        let (token, hash) = mag_portal::tokens::new_secret(db::DEVICE_TOKEN_PREFIX);
        sqlx::query("INSERT INTO devices (account_id, token_hash, label) VALUES ($1, $2, 'test')")
            .bind(a.id)
            .bind(&hash[..])
            .execute(pool)
            .await
            .unwrap();
        (a.id, token)
    }

    impl Harness {
        pub async fn send_report(&self, token: Option<&str>, body: Vec<u8>) -> reqwest::Response {
            let mut req = self
                .http
                .post(format!("{}/api/v1/reports", self.base))
                .header("content-type", "application/json")
                .body(body);
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            req.send().await.unwrap()
        }

        pub async fn report(&self, token: &str, body: &Value) -> (StatusCode, Value) {
            let r = self
                .send_report(Some(token), serde_json::to_vec(body).unwrap())
                .await;
            (r.status(), r.json().await.unwrap())
        }
    }

    pub async fn run_cli(
        pool: &PgPool,
        store: Option<&dyn ReportStore>,
        command: mag_portal::admin::ReportsCommand,
    ) -> anyhow::Result<String> {
        let mut out = Vec::new();
        mag_portal::admin::run_reports(pool, store, command, &mut out).await?;
        Ok(String::from_utf8(out).unwrap())
    }
}

use report_support::*;

#[sqlx::test]
async fn reports_are_stored_once(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let (_, token) = person_with_token(&h.pool, "rep-a", "Romeo").await;
    let (_, other) = person_with_token(&h.pool, "rep-b", "Sierra").await;
    let id = new_id();
    let body = report(&id, &[("launcher.log", b"frame 1 slow\n")]);

    let (status, first) = h.report(&token, &body).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["report_id"], id);
    let at = first["received_at"].as_str().unwrap().to_owned();
    // Resending is recognised and stores nothing new.
    let (status, again) = h.report(&token, &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, first);
    // Upper case IDs are the same report.
    let (status, _) = h
        .report(&token, &report(&id.to_ascii_uppercase(), &[]))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, conflict) = h.report(&other, &body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "report_id_conflict");

    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM reports")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let (names, bytes, ip_like): (Vec<String>, i64, i64) = sqlx::query_as(
        "SELECT log_names, log_bytes,
                (SELECT count(*) FROM information_schema.columns
                 WHERE table_name = 'reports' AND (column_name LIKE '%ip%' OR column_name LIKE '%addr%'))
         FROM reports",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(names, vec!["launcher.log"]);
    assert_eq!(ip_like, 0, "no client address is kept");
    // The log is stored compressed, as sent, under its date and report ID.
    let path = dir.join(&at[..10]).join(&id).join("launcher.log.gz");
    let stored = std::fs::read(&path).unwrap();
    assert_eq!(stored.len() as i64, bytes);
    assert_eq!(
        mag_portal::reports::decompress(&stored, 1 << 20).unwrap(),
        b"frame 1 slow\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

#[sqlx::test]
async fn reports_refuse_bad_requests(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let (id, token) = person_with_token(&h.pool, "rep-c", "Tango").await;

    // No token, an unknown token, a revoked one.
    let (_, revoked) = person_with_token(&h.pool, "rep-c2", "Tango2").await;
    let revoked_account = db::account_by_name(&h.pool, "Tango2")
        .await
        .unwrap()
        .unwrap();
    let device = db::devices(&h.pool, revoked_account.id).await.unwrap()[0].id;
    assert!(
        db::revoke_device(&h.pool, revoked_account.id, device)
            .await
            .unwrap()
    );
    let body = serde_json::to_vec(&report(&new_id(), &[])).unwrap();
    for t in [
        None,
        Some("mgd_nope"),
        Some("other"),
        Some(revoked.as_str()),
    ] {
        let r = h.send_report(t, body.clone()).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(r.json::<Value>().await.unwrap()["error"], "invalid_token");
    }

    let cases: Vec<(&str, Value)> = vec![
        ("trigger", {
            let mut v = report(&new_id(), &[]);
            v["trigger"] = json!("explosion");
            v
        }),
        ("name", report(&new_id(), &[("Launcher.log", b"x")])),
        ("not gzip", {
            let mut v = report(&new_id(), &[("a.log", b"x")]);
            v["logs"]["a.log"]["data"] = json!(STANDARD.encode(b"plain text"));
            v
        }),
        (
            "too many logs",
            report(
                &new_id(),
                &(0..9)
                    .map(|i| {
                        (
                            ["l0", "l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8"][i],
                            &b"x"[..],
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
        ),
        ("client size", {
            let mut v = report(&new_id(), &[]);
            v["client"]["pad"] = json!("x".repeat(17 << 10));
            v
        }),
        ("metrics size", {
            let mut v = report(&new_id(), &[]);
            v["metrics"]["pad"] = json!("x".repeat(65 << 10));
            v
        }),
        ("unknown field", {
            let mut v = report(&new_id(), &[]);
            v["extra"] = json!(true);
            v
        }),
        ("metrics array", {
            let mut v = report(&new_id(), &[]);
            v["metrics"] = json!([1, 2]);
            v
        }),
        ("platform", {
            let mut v = report(&new_id(), &[]);
            v["client"]["platform"] = json!("ps3");
            v
        }),
        ("id", report("not-a-uuid", &[])),
    ];
    for (what, body) in cases {
        let (status, v) = h.report(&token, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {v}");
        assert_eq!(v["error"], "invalid_report", "{what}");
        assert!(
            v["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "{what}"
        );
    }
    let r = h.send_report(Some(&token), b"{not json".to_vec()).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // Over 4 MiB: 413 before anything else is looked at.
    let r = h.send_report(Some(&token), vec![b' '; (4 << 20) + 1]).await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "too_large");

    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM reports")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        0,
        "nothing written"
    );

    // Banned accounts are refused.
    staff_ban(&h.pool, id).await;
    let (status, v) = h.report(&token, &report(&new_id(), &[])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(v["error"], "banned");
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

async fn staff_ban(pool: &PgPool, id: i32) {
    assert!(
        mag_portal::staff::set_ban_by_id(pool, id, Some("test"))
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn reports_have_daily_allowances(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let (_, person) = person_with_token(&h.pool, "rep-d", "Uniform").await;
    let (_, service) = service_with_token(&h.pool, "PlayerLinux").await;

    let first = new_id();
    for i in 0..20 {
        let id = if i == 0 { first.clone() } else { new_id() };
        let (status, v) = h.report(&person, &report(&id, &[])).await;
        assert_eq!(status, StatusCode::CREATED, "{i}: {v}");
    }
    let r = h
        .send_report(
            Some(&person),
            serde_json::to_vec(&report(&new_id(), &[])).unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = r.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((86_000..=86_400).contains(&retry), "{retry}");
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "rate_limited");
    // A resend of a stored report still answers, and does not count.
    assert_eq!(
        h.report(&person, &report(&first, &[])).await.0,
        StatusCode::OK
    );
    // Once the oldest report ages out of the window, another fits.
    sqlx::query(
        "UPDATE reports SET received_at = received_at - interval '25 hours' WHERE id = $1::uuid",
    )
    .bind(&first)
    .execute(&h.pool)
    .await
    .unwrap();
    assert_eq!(
        h.report(&person, &report(&new_id(), &[])).await.0,
        StatusCode::CREATED
    );

    // Service accounts get 200 a day.
    for i in 0..200 {
        let (status, v) = h.report(&service, &report(&new_id(), &[])).await;
        assert_eq!(status, StatusCode::CREATED, "{i}: {v}");
    }
    let r = h
        .send_report(
            Some(&service),
            serde_json::to_vec(&report(&new_id(), &[])).unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r.headers().contains_key("retry-after"));
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

#[sqlx::test]
async fn reports_off_and_per_address_limit(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    // Off: 503 for everyone, before the token is looked at.
    let h = harness(pool.clone()).await;
    let (_, token) = person_with_token(&h.pool, "rep-e", "Victor").await;
    let (status, v) = h.report(&token, &report(&new_id(), &[])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["error"], "reports_disabled");

    let dir = temp_dir("reports");
    let limits = web::Limits {
        reports: mag_portal::ratelimit::RateLimiter::new(2, std::time::Duration::from_secs(60)),
        ..web::Limits::default()
    };
    let h = harness_full(pool, None, Some(dir_reports(&dir)), limits).await;
    for _ in 0..2 {
        assert_eq!(
            h.report(&token, &report(&new_id(), &[])).await.0,
            StatusCode::CREATED
        );
    }
    let r = h
        .send_report(
            Some(&token),
            serde_json::to_vec(&report(&new_id(), &[])).unwrap(),
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.headers()["retry-after"], "60");
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

/// Ages a folder past the sweep's grace period.
fn age(path: &std::path::Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

#[sqlx::test]
async fn expired_reports_and_orphans_are_purged(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::report_store::log_key;
    let dir = temp_dir("reports");
    let service = dir_reports(&dir);
    let h = harness_full(pool, None, Some(service.clone()), roomy_limits()).await;
    let (_, token) = person_with_token(&h.pool, "rep-f", "Whiskey").await;

    // A fresh report stays.
    let fresh = new_id();
    let (status, v) = h
        .report(&token, &report(&fresh, &[("a.log", b"fresh")]))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let today = v["received_at"].as_str().unwrap()[..10].to_owned();

    // An expired report: a row and its log from 40 days ago.
    let old = new_id();
    let old_at = time::OffsetDateTime::now_utc() - time::Duration::days(40);
    let old_key = log_key(old_at, &old, "a.log");
    service.store.put(&old_key, gzip(b"old")).await.unwrap();
    let (account,): (i32,) =
        sqlx::query_as("SELECT id FROM accounts WHERE player_name = 'Whiskey'")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO reports (id, account_id, received_at, trigger, platform, client_version,
                              client, log_names, log_bytes)
         VALUES ($1::uuid, $2, $3, 'crash', 'macos', '1.0', '{}'::jsonb, '{a.log}', 3)",
    )
    .bind(&old)
    .bind(account)
    .bind(old_at)
    .execute(&h.pool)
    .await
    .unwrap();

    // Orphans: an old one today (a failed write), a new one today (maybe being written), and a
    // whole day folder from long ago.
    let stale = new_id();
    service
        .store
        .put(&format!("{today}/{stale}/x.gz"), gzip(b"stale"))
        .await
        .unwrap();
    age(&dir.join(&today).join(&stale));
    let young = new_id();
    service
        .store
        .put(&format!("{today}/{young}/x.gz"), gzip(b"young"))
        .await
        .unwrap();
    service
        .store
        .put(&format!("2000-01-01/{}/x.gz", new_id()), gzip(b"ancient"))
        .await
        .unwrap();

    let purged = mag_portal::reports::purge(&h.pool, &service).await.unwrap();
    assert_eq!(purged.reports, 1);
    assert_eq!(purged.orphans, 2);
    let ids: Vec<(String,)> = sqlx::query_as("SELECT id::text FROM reports")
        .fetch_all(&h.pool)
        .await
        .unwrap();
    assert_eq!(ids, vec![(fresh.clone(),)]);
    assert!(service.store.get(&old_key).await.unwrap().is_none());
    assert!(!dir.join(&old_key).parent().unwrap().exists());
    assert!(!dir.join(&today).join(&stale).exists());
    assert!(dir.join(&today).join(&young).exists());
    assert!(dir.join(&today).join(&fresh).join("a.log.gz").exists());
    assert!(!dir.join("2000-01-01").exists());
    // Deleting an account removes its reports' rows and logs.
    let s = h.sign_in("rep-f").await;
    let csrf = h.csrf(&s).await;
    let r = h
        .post_form(
            "/account/delete",
            &s,
            &[("csrf", &csrf), ("confirm", "Whiskey")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(!dir.join(&today).join(&fresh).exists());
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM reports")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

#[sqlx::test]
async fn report_commands(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::admin::ReportsCommand;
    use mag_portal::report_store::{DirStore, ReportStore};
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let store = DirStore::new(&dir);
    let store: Option<&dyn ReportStore> = Some(&store);
    let (_, token) = person_with_token(&h.pool, "rep-g", "Xray").await;
    let id = new_id();
    let log_text = b"line one\nline two\n".repeat(1000);
    assert_eq!(
        h.report(&token, &report(&id, &[("launcher.log", &log_text)]))
            .await
            .0,
        StatusCode::CREATED
    );
    let mut crash = report(&new_id(), &[]);
    crash["trigger"] = json!("crash");
    crash["client"]["platform"] = json!("windows");
    assert_eq!(h.report(&token, &crash).await.0, StatusCode::CREATED);

    let list = |account: Option<&str>, trigger: Option<&str>, platform: Option<&str>| {
        ReportsCommand::List {
            account: account.map(Into::into),
            trigger: trigger.map(Into::into),
            platform: platform.map(Into::into),
            since: None,
            days: Some(1),
            has_comment: false,
            limit: 50,
        }
    };
    let out = run_cli(&h.pool, None, list(Some("xRAY"), None, None))
        .await
        .unwrap();
    assert_eq!(out.lines().count(), 2, "{out}");
    let out = run_cli(
        &h.pool,
        None,
        list(None, Some("slow_frames"), Some("linux")),
    )
    .await
    .unwrap();
    assert!(
        out.starts_with(&id) && out.contains("Xray") && out.contains("1 logs"),
        "{out}"
    );
    assert_eq!(out.lines().count(), 1);
    assert!(
        run_cli(&h.pool, None, list(Some("nobody"), None, None))
            .await
            .unwrap()
            .is_empty()
    );
    let since = ReportsCommand::List {
        account: None,
        trigger: None,
        platform: None,
        since: Some(time::OffsetDateTime::now_utc() + time::Duration::hours(1)),
        days: None,
        has_comment: false,
        limit: 50,
    };
    assert!(run_cli(&h.pool, None, since).await.unwrap().is_empty());

    let out = run_cli(
        &h.pool,
        None,
        ReportsCommand::Show {
            report_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(
        out.contains("trigger slow_frames") && out.contains("\"p99\": 48.0"),
        "{out}"
    );
    assert!(out.contains("logs launcher.log"), "{out}");
    assert!(out.contains("\ncomment none\n"), "{out}");
    assert!(
        run_cli(
            &h.pool,
            None,
            ReportsCommand::Show {
                report_id: new_id()
            }
        )
        .await
        .is_err()
    );

    let log = |name: &str| ReportsCommand::Log {
        report_id: id.clone(),
        name: name.into(),
    };
    assert!(
        run_cli(&h.pool, None, log("launcher.log")).await.is_err(),
        "needs the store"
    );
    let out = run_cli(&h.pool, store, log("launcher.log")).await.unwrap();
    assert_eq!(out.as_bytes(), &log_text[..]);
    assert!(run_cli(&h.pool, store, log("other.log")).await.is_err());

    let out = run_cli(&h.pool, None, ReportsCommand::Stats { days: 14 })
        .await
        .unwrap();
    let today = mag_portal::report_store::day(time::OffsetDateTime::now_utc());
    assert!(
        out.contains(&format!("{today}\tcrash\twindows\t1")),
        "{out}"
    );
    assert!(
        out.contains(&format!("{today}\tslow_frames\tlinux\t1")),
        "{out}"
    );

    let out = run_cli(
        &h.pool,
        store,
        ReportsCommand::Delete {
            report_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(out.starts_with("deleted report"));
    assert!(!dir.join(&today).join(&id).exists());
    assert!(
        run_cli(
            &h.pool,
            store,
            ReportsCommand::Delete {
                report_id: id.clone()
            }
        )
        .await
        .is_err()
    );
    let actions: Vec<(String, String)> =
        sqlx::query_as("SELECT action, actor_email FROM admin_log ORDER BY id")
            .fetch_all(&h.pool)
            .await
            .unwrap();
    assert_eq!(
        actions,
        vec![
            (
                "read report log".into(),
                mag_portal::staff::OPERATOR_ACTOR.into()
            ),
            (
                "delete report".into(),
                mag_portal::staff::OPERATOR_ACTOR.into()
            ),
        ]
    );
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

#[sqlx::test]
async fn report_comments(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::admin::ReportsCommand;
    use mag_portal::staff::{self, Role};
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let (_, token) = person_with_token(&h.pool, "rep-k", "Kilo").await;
    let manual = |comment: Value| {
        let mut v = report(&new_id(), &[]);
        v["trigger"] = json!("manual");
        v["session"]["user_comment"] = comment;
        v
    };
    let text = "Stutter at the <b>gate</b> & \"spawn\"\r\n\u{1b}[31msecond line\u{202e}\nthird \u{4e2d}\u{6587} line, long enough to need cutting short";
    let with = manual(json!(text));
    let with_id = with["report_id"].as_str().unwrap().to_owned();
    assert_eq!(h.report(&token, &with).await.0, StatusCode::CREATED);
    let odd = manual(json!({"text": "not a string"}));
    let odd_id = odd["report_id"].as_str().unwrap().to_owned();
    assert_eq!(h.report(&token, &odd).await.0, StatusCode::CREATED);
    let blank = manual(json!(" \n\t "));
    assert_eq!(h.report(&token, &blank).await.0, StatusCode::CREATED);
    let plain = report(&new_id(), &[]);
    assert_eq!(h.report(&token, &plain).await.0, StatusCode::CREATED);

    // Stored exactly as sent.
    let (stored,): (Option<String>,) =
        sqlx::query_as("SELECT session->>'user_comment' FROM reports WHERE id = $1::uuid")
            .bind(&with_id)
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(stored.as_deref(), Some(text));

    // Too long: refused, and nothing stored. 1000 characters of three bytes each fit.
    let (status, v) = h
        .report(&token, &manual(json!("\u{4e2d}".repeat(1001))))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], "invalid_report");
    assert_eq!(v["detail"], "user_comment too long");
    assert_eq!(
        h.report(&token, &manual(json!("\u{4e2d}".repeat(1000))))
            .await
            .0,
        StatusCode::CREATED
    );

    // The CLI list shows a one-line preview; --has-comment keeps only real comments.
    let list = |has_comment| ReportsCommand::List {
        account: Some("kilo".into()),
        trigger: None,
        platform: None,
        since: None,
        days: None,
        has_comment,
        limit: 50,
    };
    let preview = mag_portal::reports::comment_preview(text, 60);
    assert_eq!(
        preview,
        "Stutter at the <b>gate</b> & \"spawn\" [31msecond line third \u{4e2d}…"
    );
    let out = run_cli(&h.pool, None, list(false)).await.unwrap();
    assert_eq!(out.lines().count(), 5, "{out}");
    let line = out.lines().find(|l| l.starts_with(&with_id)).unwrap();
    assert!(line.ends_with(&format!("\t{preview}")), "{line}");
    let line = out.lines().find(|l| l.starts_with(&odd_id)).unwrap();
    assert!(line.ends_with("\t-"), "{line}");
    let out = run_cli(&h.pool, None, list(true)).await.unwrap();
    assert_eq!(out.lines().count(), 2, "{out}");
    assert!(out.contains(&with_id) && !out.contains(&odd_id), "{out}");

    // The CLI show prints the whole comment, line by line, without terminal controls.
    let show = |id: &str| ReportsCommand::Show {
        report_id: id.into(),
    };
    let out = run_cli(&h.pool, None, show(&with_id)).await.unwrap();
    assert!(
        out.contains(
            "\ncomment\n  Stutter at the <b>gate</b> & \"spawn\"\n  [31msecond line\n  third \u{4e2d}\u{6587} line, long enough to need cutting short\nclient "
        ),
        "{out}"
    );
    let out = run_cli(&h.pool, None, show(&odd_id)).await.unwrap();
    assert!(out.contains("\ncomment none\n"), "{out}");

    // The panel lists a preview and shows the whole comment, escaped.
    let m = h.sign_in("mod").await;
    let granter = id_of(&h, "mod").await;
    staff::set_staff(&h.pool, "mod@example.test", Role::Moderator, granter)
        .await
        .unwrap();
    let (status, list) = body(h.get("/admin/reports", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains(&with_id) && list.contains(&odd_id), "{list}");
    // 14 UTC days, which never shift, and each report's received time.
    assert!(support::page_times(&list) >= 14 + 2, "{list}");
    assert!(list.contains("<th>Day (UTC)</th>"));
    assert!(
        list.contains("Stutter at the &#60;b&#62;gate&#60;/b&#62; &#38;")
            && !list.contains("<b>gate"),
        "{list}"
    );
    assert!(!list.contains("cutting") && list.contains("third \u{4e2d}…"));
    let (_, filtered) = body(h.get("/admin/reports?comment=yes", Some(&m)).await).await;
    assert!(filtered.contains(&with_id) && !filtered.contains(&odd_id));
    assert!(filtered.contains("<option value=\"yes\" selected>"));
    let (status, detail) = body(h.get(&format!("/admin/reports/{with_id}"), Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(support::page_times(&detail), 1, "received: {detail}");
    assert!(
        detail.contains(
            "<p class=\"comment\">Stutter at the &#60;b&#62;gate&#60;/b&#62; &#38; &#34;spawn&#34;\n[31msecond line\nthird \u{4e2d}\u{6587} line, long enough to need cutting short</p>"
        ),
        "{detail}"
    );
    assert!(!detail.contains("<b>gate") && !detail.contains('\u{1b}'));
    let (_, detail) = body(h.get(&format!("/admin/reports/{odd_id}"), Some(&m)).await).await;
    assert!(!detail.contains("class=\"comment\""), "{detail}");
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

#[sqlx::test]
async fn reports_in_the_admin_panel(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::staff::{self, Role};
    let dir = temp_dir("reports");
    let h = harness_full(pool, None, Some(dir_reports(&dir)), roomy_limits()).await;
    let (_, token) = person_with_token(&h.pool, "rep-h", "Yankee").await;
    let id = new_id();
    assert_eq!(
        h.report(&token, &report(&id, &[("launcher.log", b"<b>hello</b>\n")]))
            .await
            .0,
        StatusCode::CREATED
    );

    let u = h.sign_in("nobody").await;
    let m = h.sign_in("mod").await;
    let a = h.sign_in("admin").await;
    let granter = id_of(&h, "admin").await;
    staff::set_staff(&h.pool, "mod@example.test", Role::Moderator, granter)
        .await
        .unwrap();
    staff::set_staff(&h.pool, "admin@example.test", Role::Admin, granter)
        .await
        .unwrap();
    let page = format!("/admin/reports/{id}");
    let download = format!("/admin/reports/{id}/logs/launcher.log");

    // Not staff: the pages do not exist.
    for path in ["/admin/reports", page.as_str()] {
        assert_eq!(h.get(path, Some(&u)).await.status(), StatusCode::NOT_FOUND);
    }
    let csrf_u = h.csrf(&u).await;
    assert_eq!(
        h.post_form(&download, &u, &[("csrf", &csrf_u)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Moderators see counts, the list and the details, but cannot download logs.
    let (status, list) = body(h.get("/admin/reports", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains(&format!("href=\"/admin/reports/{id}\"")) && list.contains("Yankee"));
    let (_, filtered) = body(h.get("/admin/reports?trigger=crash", Some(&m)).await).await;
    assert!(!filtered.contains(&id) && filtered.contains("No reports match"));
    let (_, filtered) = body(
        h.get("/admin/reports?account=yankee&platform=linux", Some(&m))
            .await,
    )
    .await;
    assert!(filtered.contains(&id));
    let (status, detail) = body(h.get(&page, Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        detail.contains("p99") && detail.contains("Yankee"),
        "{detail}"
    );
    assert!(detail.contains("admins only") && !detail.contains(&download));
    let csrf_m = h.csrf(&m).await;
    assert_eq!(
        h.post_form(&download, &m, &[("csrf", &csrf_m)])
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.get(&format!("/admin/reports/{}", new_id()), Some(&m))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Admins download the log as text, once the form's CSRF token checks out.
    let (_, detail) = body(h.get(&page, Some(&a)).await).await;
    assert!(detail.contains(&format!("action=\"{download}\"")));
    let csrf_a = csrf_from(&detail);
    assert_eq!(
        h.post_form(&download, &a, &[("csrf", "bad")])
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let r = h.post_form(&download, &a, &[("csrf", &csrf_a)]).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "text/plain; charset=utf-8");
    assert_eq!(
        r.headers()["content-disposition"],
        format!("attachment; filename=\"report-{id}-launcher.log.txt\"").as_str()
    );
    assert_eq!(r.text().await.unwrap(), "<b>hello</b>\n");
    let missing = format!("/admin/reports/{id}/logs/other.log");
    assert_eq!(
        h.post_form(&missing, &a, &[("csrf", &csrf_a)])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let (action, target): (String, String) =
        sqlx::query_as("SELECT action, target FROM admin_log ORDER BY id DESC LIMIT 1")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(action, "download report log");
    assert_eq!(target, format!("report {id}"));
    std::fs::remove_dir_all(&dir).unwrap();
    test_db.close().await;
}

// ---- S3 report store against an in-process fake ----

#[derive(Default)]
struct FakeS3 {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    /// Every request: method, path.
    requests: Mutex<Vec<(String, String)>>,
}

const S3_ACCESS: &str = "AKIDSYNTHETIC";
const S3_SECRET: &str = "synthetic-secret-never-printed";

async fn fake_s3(
    State(s3): State<Arc<FakeS3>>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = uri.path().to_owned();
    s3.requests
        .lock()
        .unwrap()
        .push((method.to_string(), path.clone()));
    let query = uri.query().unwrap_or("");
    let signed_query = query.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256")
        && query.contains(&format!("X-Amz-Credential={S3_ACCESS}%2F"))
        && query.contains("X-Amz-Signature=");
    let signed_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("AWS4-HMAC-SHA256"));
    if !(signed_query || signed_header) {
        return (axum::http::StatusCode::FORBIDDEN, "unsigned").into_response();
    }
    // Path style: /<bucket>[/<key>].
    let Some(rest) = path.strip_prefix("/mag-validation") else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let key = rest.trim_start_matches('/').to_owned();
    let mut objects = s3.objects.lock().unwrap();
    match (method.as_str(), key.is_empty()) {
        ("HEAD", true) => axum::http::StatusCode::OK.into_response(),
        ("PUT", false) => {
            if headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("application/gzip")
            {
                return axum::http::StatusCode::BAD_REQUEST.into_response();
            }
            objects.insert(key, body.to_vec());
            axum::http::StatusCode::OK.into_response()
        }
        ("GET", false) => match objects.get(&key) {
            Some(data) => data.clone().into_response(),
            None => axum::http::StatusCode::NOT_FOUND.into_response(),
        },
        ("DELETE", false) => {
            objects.remove(&key);
            axum::http::StatusCode::NO_CONTENT.into_response()
        }
        _ => axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn s3_args(
    endpoint: &str,
    bucket: &str,
    dir: &std::path::Path,
) -> mag_portal::report_store::StoreArgs {
    let access = dir.join("access");
    let secret = dir.join("secret");
    std::fs::write(&access, format!("{S3_ACCESS}\n")).unwrap();
    std::fs::write(&secret, S3_SECRET).unwrap();
    mag_portal::report_store::StoreArgs {
        report_store: Some(mag_portal::report_store::StoreKind::S3),
        report_dir: None,
        report_s3_endpoint: Some(endpoint.into()),
        report_s3_bucket: Some(bucket.into()),
        report_s3_region: "us-east-1".into(),
        report_s3_access_key_file: Some(access),
        report_s3_secret_key_file: Some(secret),
    }
}

#[sqlx::test]
async fn reports_in_an_s3_bucket(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    use mag_portal::admin::ReportsCommand;
    let s3 = Arc::new(FakeS3::default());
    let addr = serve(Router::new().fallback(fake_s3).with_state(s3.clone())).await;
    let endpoint = format!("http://{addr}");
    let keys = temp_dir("s3-keys");

    let store = s3_args(&endpoint, "mag-validation", &keys).open().unwrap();
    store.check().await.unwrap();
    // A missing bucket fails the startup check without printing the keys.
    let err = s3_args(&endpoint, "missing", &keys)
        .open()
        .unwrap()
        .check()
        .await
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("does not exist"), "{text}");
    assert!(
        !text.contains(S3_SECRET) && !text.contains(S3_ACCESS),
        "{text}"
    );
    // Settings that are missing are named.
    let mut partial = s3_args(&endpoint, "mag-validation", &keys);
    partial.report_s3_secret_key_file = None;
    assert!(
        format!("{:#}", partial.open().err().unwrap()).contains("PORTAL_REPORT_S3_SECRET_KEY_FILE")
    );

    let service = Arc::new(Reports::new(store.clone(), 30));
    let h = harness_full(pool, None, Some(service), roomy_limits()).await;
    let (_, token) = person_with_token(&h.pool, "rep-s3", "Zulu").await;
    let id = new_id();
    let (status, v) = h
        .report(
            &token,
            &report(
                &id,
                &[("launcher.log", b"from s3\n"), ("audio.log", b"xrun\n")],
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let day = &v["received_at"].as_str().unwrap()[..10];
    let key = format!("reports/{day}/{id}/launcher.log.gz");
    assert_eq!(
        mag_portal::reports::decompress(&s3.objects.lock().unwrap()[&key], 1 << 20).unwrap(),
        b"from s3\n"
    );
    assert_eq!(s3.objects.lock().unwrap().len(), 2);
    assert!(
        s3.requests
            .lock()
            .unwrap()
            .contains(&("PUT".into(), format!("/mag-validation/{key}")))
    );

    let out = run_cli(
        &h.pool,
        Some(store.as_ref()),
        ReportsCommand::Log {
            report_id: id.clone(),
            name: "launcher.log".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(out, "from s3\n");
    run_cli(
        &h.pool,
        Some(store.as_ref()),
        ReportsCommand::Delete {
            report_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(s3.objects.lock().unwrap().is_empty());
    assert!(
        s3.requests
            .lock()
            .unwrap()
            .contains(&("DELETE".into(), format!("/mag-validation/{key}")))
    );

    // When the upload fails, no row is written and the logs already uploaded are removed.
    let broken = s3_args(&endpoint, "missing", &keys).open().unwrap();
    let service = Reports::new(broken, 30);
    let account = db::account_by_name(&h.pool, "Zulu").await.unwrap().unwrap();
    let parsed = mag_portal::reports::parse(
        &serde_json::to_vec(&report(&new_id(), &[("a.log", b"x")])).unwrap(),
    )
    .unwrap();
    assert!(
        mag_portal::reports::submit(&h.pool, &service, &account, parsed)
            .await
            .is_err()
    );
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM reports")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    std::fs::remove_dir_all(&keys).unwrap();
    test_db.close().await;
}

/// A pool whose database never answers: the address accepts connections (into the listen
/// backlog) but nothing reads them, so every acquire runs out its timeout.
async fn silent_database() -> (tokio::net::TcpListener, PgPool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy_with(
            PgConnectOptions::new()
                .host("127.0.0.1")
                .port(port)
                .username("portal")
                .database("portal"),
        );
    (listener, pool)
}

#[tokio::test]
async fn unavailable_database_answers_busy() {
    let (_listener, pool) = silent_database().await;
    let h = harness(pool).await;
    let started = std::time::Instant::now();

    // The launcher API answers a retryable 503 busy in JSON, after the pool's timeout.
    let token = format!("{}unknown", db::DEVICE_TOKEN_PREFIX);
    let r = h
        .http
        .post(format!("{}/api/v1/ticket", h.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "error": "busy" }));
    let r = h
        .http
        .get(format!("{}/api/v1/social", h.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "busy");

    // Pages keep the error page, as 503 with Retry-After.
    let r = h.get("/account", Some("mgs_unknown")).await;
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(r.text().await.unwrap().contains("Busy"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "each request waits only the pool's acquire timeout"
    );
}

/// Makes new connections to the test's database read-only, as during a failover, and returns
/// a pool of such connections.
async fn read_only_pool(pool: &PgPool, connect: PgConnectOptions) -> PgPool {
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET default_transaction_read_only = on', \
         current_database()); END $$",
    )
    .execute(pool)
    .await
    .unwrap();
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(connect)
        .await
        .unwrap()
}

#[sqlx::test]
async fn tickets_while_the_database_is_read_only(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect.clone()).await;
    let (_, token) = service_with_token(&pool, "PlayerLinux").await;
    let read_only = read_only_pool(&pool, connect).await;
    let h = harness(read_only.clone()).await;

    // Recording the device's use fails, but the ticket is still issued.
    let (status, issued) = h.ticket(&token).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    assert_eq!(issued["player_name"], "PlayerLinux");
    verify_issued(&h, &issued, AUDIENCE).await.unwrap();
    let (last_used,): (Option<time::OffsetDateTime>,) =
        sqlx::query_as("SELECT last_used_at FROM devices")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(last_used, None);

    // Calls that must write answer a retryable 503 busy instead of failing.
    let r = h
        .http
        .put(format!("{}/api/v1/social/presence", h.base))
        .bearer_auth(&token)
        .json(&json!({ "data_b64": STANDARD.encode([1u8, 2]), "state": "online" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.headers()["retry-after"], "2");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "error": "busy" }));
    read_only.close().await;
    test_db.close().await;
}

#[sqlx::test]
async fn api_failures_answer_json(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let (_, token) = service_with_token(&h.pool, "PlayerLinux").await;
    // A failure that is not the database being unavailable stays a 500, in JSON for the API.
    sqlx::query("ALTER TABLE devices RENAME TO devices_elsewhere")
        .execute(&h.pool)
        .await
        .unwrap();
    let (status, body) = h.ticket(&token).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, json!({ "error": "server_error" }));
    test_db.close().await;
}

// ---- Server status on the home page, /status and /api/v1/status ----

fn status_server(id: &str, url: &str, staff_only: bool) -> StatusServer {
    StatusServer {
        id: id.into(),
        name: format!("Server {id}"),
        host: Some(format!("{id}.example.test")),
        staff_only,
        admin: GameAdmin::new(url, GAME_SECRET.to_vec()).unwrap(),
    }
}

#[sqlx::test]
async fn server_status_is_public_cached_and_names_nobody(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let (medius, url) = fake_medius_server().await;
    let status = Arc::new(StatusService::new(
        vec![
            status_server("live", &url, false),
            // Nothing listens on the discard port: this server is down.
            status_server("modes", "http://127.0.0.1:9", false),
            status_server("lab", &url, true),
        ],
        StdDuration::from_secs(15),
    ));
    let h = harness_status(
        pool.clone(),
        Some(GameAdmin::new(&url, GAME_SECRET.to_vec()).unwrap()),
        None,
        web::Limits::default(),
        status.clone(),
    )
    .await;
    let calls = || medius.state.lock().unwrap().status_calls;

    // Before the first poll the home page still renders, and says it is checking.
    let (code, page) = body(h.get("/", None).await).await;
    assert_eq!(code, StatusCode::OK);
    assert!(
        page.contains("id=\"status\"") && page.contains("Checking"),
        "{page}"
    );

    status.refresh(&pool).await;
    assert_eq!(calls(), 2, "one poll each for live and the staff-only lab");

    // Anonymous: no cookie at all. The status block is on the home page itself.
    let (code, page) = body(h.get("/", None).await).await;
    assert_eq!(code, StatusCode::OK);
    assert!(page.contains("id=\"status\""), "{page}");
    for shown in [
        "Server live",
        "live.example.test",
        "Some servers or worlds are closed",
        "Degraded",
        "Sabotage",
        "Raven</strong> attacking <strong>SVER",
        "Humans 3 / 32 · bots fill the rest (64-player match)",
        "Humans 0 / 32 · bots fill the rest (256-player match)",
        "4 · up to 32 humans at once across all modes",
        "2 humans attacking, 1 defending",
        "synthetic-sab-raven-sver",
        "Acquisition",
        "9.8.7",
        "6.5.4",
        "2 h 1 min",
        "4 of 32",
        "Server modes",
        "The game server did not answer.",
    ] {
        assert!(page.contains(shown), "home page lacks {shown:?}");
    }
    // Counts only: the fake game server's characters never appear, nor do admin links or the
    // staff-only server, nor the game servers' addresses.
    for hidden in [
        "alpha",
        "bravo",
        "charlie",
        "/admin",
        "Server lab",
        "127.0.0.1",
        &url,
        // A world size or row cap is never shown as a human cap.
        "/ 64",
        "of 64",
        "Up to 64",
    ] {
        assert!(!page.contains(hidden), "home page shows {hidden:?}");
    }
    // It updates itself through the page script, and needs no full-page reload; the block
    // above is the server-rendered fallback for browsers without scripts.
    assert!(
        page.contains("id=\"status-live\"")
            && page.contains("id=\"status-servers\"")
            && page.contains("id=\"status-banner\"")
            && page.contains(&format!(
                "<script src=\"/status.js?v={}\" defer></script>",
                mag_portal::status::script_version()
            )),
        "{page}"
    );
    assert!(!page.contains("http-equiv=\"refresh\""));

    let (code, page) = body(h.get("/status", None).await).await;
    assert_eq!(code, StatusCode::OK);
    assert!(page.contains("synthetic-sab-raven-sver") && page.contains("/status.js?v="));
    assert!(!page.contains("http-equiv=\"refresh\""));
    assert!(!page.contains("alpha") && !page.contains("Server lab"));

    // The script is same-origin, allowed by the policy, and builds nodes from text only.
    let home = h.get("/", None).await;
    let policy = home.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        policy.contains("script-src 'self'") && policy.contains("connect-src 'self'"),
        "{policy}"
    );
    assert!(!policy.contains("unsafe-inline"), "{policy}");
    let r = h
        .get(
            &format!("/status.js?v={}", mag_portal::status::script_version()),
            None,
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(
        r.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let script = r.text().await.unwrap();
    assert!(script.contains("textContent") && script.contains("visibilitychange"));
    for unsafe_sink in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
    ] {
        assert!(
            !script.contains(unsafe_sink),
            "the script uses {unsafe_sink}"
        );
    }

    let r = h.get("/api/v1/status", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    let text = r.text().await.unwrap();
    for hidden in [
        "alpha",
        "bravo",
        "charlie",
        "Server lab",
        "lab.example.test",
        "127.0.0.1",
    ] {
        assert!(!text.contains(hidden), "JSON shows {hidden:?}: {text}");
    }
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["state"], "degraded");
    let live = &json["servers"][0];
    assert_eq!(live["state"], "degraded");
    assert_eq!(live["players_online"], 4);
    assert_eq!(live["dme_ports"]["in_use"], 4);
    assert_eq!(live["worlds"][0]["queued"], 2);
    assert_eq!(live["worlds"][0]["match"]["bots"], 61);
    assert_eq!(live["worlds"][0]["match"]["attackers"], "Raven");
    assert_eq!(live["worlds"][1]["open"], false);
    // The fields the page script renders, with the same wording as the page.
    assert_eq!(json["headline"], "Some servers or worlds are closed");
    assert_eq!(live["state_label"], "Degraded");
    assert_eq!(live["human_limit"], 32);
    assert_eq!(
        live["players_text"],
        "4 · up to 32 humans at once across all modes"
    );
    assert_eq!(live["worlds"][0]["human_capacity"], 64);
    assert_eq!(live["worlds"][0]["human_limit"], 32);
    assert_eq!(live["worlds"][0]["world_size"], 64);
    assert_eq!(
        live["worlds"][0]["match"]["humans_text"],
        "Humans 3 / 32 · bots fill the rest (64-player match)"
    );
    // An older game server's row cap is held to the port cap too.
    assert_eq!(live["worlds"][1]["human_limit"], 32);
    assert_eq!(
        live["worlds"][2]["humans_text"],
        "Humans 0 / 32 · bots fill the rest (256-player match)"
    );
    assert_eq!(json["servers"][1]["state"], "down");
    assert_eq!(json["servers"].as_array().unwrap().len(), 2);

    // Public traffic reads the cache: many page views, no new game server requests.
    for path in ["/", "/status", "/api/v1/status"] {
        for _ in 0..5 {
            assert_eq!(h.get(path, None).await.status(), StatusCode::OK);
        }
    }
    assert_eq!(calls(), 2, "page views reached the game server");

    // Staff see the staff-only server on the admin status page; players do not.
    let u = h.sign_in("player").await;
    assert_eq!(
        h.get("/admin/status", Some(&u)).await.status(),
        StatusCode::NOT_FOUND
    );
    let (_, page) = body(h.get("/", Some(&u)).await).await;
    assert!(page.contains("id=\"status\"") && !page.contains("Server lab"));
    let o = h.sign_in("owner").await;
    let (code, page) = body(h.get("/admin/status", Some(&o)).await).await;
    assert_eq!(code, StatusCode::OK);
    assert!(page.contains("Server lab") && page.contains("Server live"));
    assert!(!page.contains("alpha"));
    test_db.close().await;
}

#[sqlx::test]
async fn maintenance_banner(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let status = Arc::new(StatusService::new(Vec::new(), StdDuration::from_secs(15)));
    let h = harness_status(
        pool.clone(),
        None,
        None,
        web::Limits::default(),
        status.clone(),
    )
    .await;
    let message = "Restarting the game servers for an update";
    let (_, page) = body(h.get("/", None).await).await;
    assert!(!page.contains("class=\"banner\""));

    // Only admins and owners set it.
    let u = h.sign_in("player").await;
    let csrf_u = h.csrf(&u).await;
    let r = h
        .post_form(
            "/admin/status/banner",
            &u,
            &[("csrf", &csrf_u), ("message", message), ("restart_at", "")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(
        mag_portal::status::load_banner(&pool)
            .await
            .unwrap()
            .is_none()
    );

    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let r = h
        .post_form(
            "/admin/status/banner",
            &o,
            &[("csrf", &csrf_o), ("message", "   "), ("restart_at", "")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = h
        .post_form(
            "/admin/status/banner",
            &o,
            &[
                ("csrf", &csrf_o),
                ("message", message),
                ("restart_at", "soon"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = h
        .post_form(
            "/admin/status/banner",
            &o,
            &[("csrf", "wrong"), ("message", message), ("restart_at", "")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .post_form(
            "/admin/status/banner",
            &o,
            &[
                ("csrf", &csrf_o),
                ("message", message),
                ("restart_at", "2030-01-02T18:30"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);

    // Shown publicly at once, without signing in: in UTC, labelled, for the time script.
    for path in ["/", "/status"] {
        let (_, page) = body(h.get(path, None).await).await;
        assert!(
            page.contains(message)
                && page.contains(
                    "Back by <time class=\"local-time\" datetime=\"2030-01-02T18:30:00Z\" \
                     title=\"2030-01-02 18:30:00 UTC\">2030-01-02 18:30 UTC</time>."
                ),
            "{path}: {page}"
        );
    }
    // The form holds it in UTC for the script to show in local time, and says the zone.
    let (_, page) = body(h.get("/admin/status", Some(&o)).await).await;
    assert!(
        page.contains("value=\"2030-01-02T18:30\" data-utc data-tz-field=\"restart-tz\"")
            && page.contains("<span data-zone-for=\"restart-at\">UTC</span>"),
        "{page}"
    );
    // Typed in local time five hours behind UTC, as the script sends it: the same instant.
    let r = h
        .post_form(
            "/admin/status/banner",
            &o,
            &[
                ("csrf", &csrf_o),
                ("message", message),
                ("restart_at", "2030-01-02T13:30"),
                ("tz", "300"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        mag_portal::status::load_banner(&pool)
            .await
            .unwrap()
            .unwrap()
            .restart_expected_at,
        Some(time::macros::datetime!(2030-01-02 18:30 UTC))
    );
    // A refused form comes back with the time in UTC, as the field holds it.
    let (code, page) = body(
        h.post_form(
            "/admin/status/banner",
            &o,
            &[
                ("csrf", &csrf_o),
                ("message", " "),
                ("restart_at", "2030-01-02T13:30"),
                ("tz", "300"),
            ],
        )
        .await,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(
        page.contains("value=\"2030-01-02T18:30\" data-utc"),
        "{page}"
    );
    let json: Value = h.get("/api/v1/status", None).await.json().await.unwrap();
    assert_eq!(json["banner"]["message"], message);
    assert_eq!(
        json["banner"]["restart_expected_at"],
        "2030-01-02T18:30:00Z"
    );
    let (_, dashboard) = body(h.get("/admin", Some(&o)).await).await;
    // The audit log keeps its text, with the time labelled UTC, and its own time to show.
    assert!(dashboard.contains("set status banner"));
    assert!(dashboard.contains("(restart expected 2030-01-02 18:30 UTC)"));

    // A restarted portal loads it from the database.
    let fresh = StatusService::new(Vec::new(), StdDuration::from_secs(15));
    fresh.refresh(&pool).await;
    assert_eq!(fresh.banner().unwrap().message, message);

    let r = h
        .post_form("/admin/status/banner/clear", &o, &[("csrf", &csrf_o)])
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let (_, page) = body(h.get("/", None).await).await;
    assert!(!page.contains(message));
    assert!(
        mag_portal::status::load_banner(&pool)
            .await
            .unwrap()
            .is_none()
    );
    let (_, dashboard) = body(h.get("/admin", Some(&o)).await).await;
    assert!(dashboard.contains("clear status banner"));
    test_db.close().await;
}

/// A change the game server applies but does not confirm in time is shown as possibly applied,
/// never as "could not reach … nothing was changed", and the page reads the state back.
#[sqlx::test]
async fn game_server_unconfirmed_changes(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let (medius, url) = fake_medius_server().await;
    // Short synthetic timeouts: reads 1 s, writes 2 s; the game server answers writes after 3 s.
    let api = GameAdmin::new(&url, GAME_SECRET.to_vec())
        .unwrap()
        .with_timeouts(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(2),
        );
    let h = harness_with(pool, Some(api)).await;
    medius.state.lock().unwrap().slow_apply = Some(std::time::Duration::from_secs(3));
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;

    // A dry run is not slowed and still works.
    let fields = [
        ("csrf", csrf_o.as_str()),
        ("action", "mute"),
        ("duration", "60"),
        ("reason", "synthetic spam"),
    ];
    let (status, page) = body(
        h.post_form("/admin/game/characters/alpha/voice", &o, &fields)
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    // The applied mute times out: the page says it may have been applied and shows it active.
    let started = std::time::Instant::now();
    let (status, page) = body(
        h.post_form("/admin/game/characters/alpha/voice/confirm", &o, &fields)
            .await,
    )
    .await;
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{page}");
    assert!(
        page.contains("The request timed out; it may have been applied")
            && page.contains("no answer within 2 seconds")
            && page.contains("Refresh to check"),
        "{page}"
    );
    assert!(!page.contains("Could not reach") && !page.contains("Nothing was changed"));
    assert!(
        page.contains("Now on the game server") && page.contains("synthetic spam"),
        "the mute read back: {page}"
    );
    assert_eq!(medius.state.lock().unwrap().voice.len(), 1);
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert!(
        log.contains("voice mute") && log.contains("unconfirmed"),
        "{log}"
    );

    // A level change the same way.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/level/confirm",
            &o,
            &[("csrf", &csrf_o), ("level", "20")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{page}");
    assert!(page.contains("it may have been applied") && !page.contains("Nothing was changed"));
    assert!(page.contains("now reports level 20"), "{page}");

    // The game server's own "outcome unknown" answer is shown the same way.
    {
        let mut st = medius.state.lock().unwrap();
        st.slow_apply = None;
        st.unknown_apply = true;
    }
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/voice/confirm",
            &o,
            &[("csrf", &csrf_o), ("action", "unmute")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{page}");
    assert!(
        page.contains("it may have been applied")
            && page.contains("database stopped answering")
            && page.contains("has no active voice mute or ban"),
        "{page}"
    );

    // Nothing listening: a write that never left is still "could not reach … nothing was changed".
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let h = harness_with(
        h.pool.clone(),
        Some(GameAdmin::new(&closed_url, GAME_SECRET.to_vec()).unwrap()),
    )
    .await;
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/voice/confirm",
            &o,
            &[("csrf", &csrf_o), ("action", "unmute")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(page.contains("Could not reach") && page.contains("Nothing was changed"));
    test_db.close().await;
}

#[sqlx::test]
async fn game_server_voice_moderation(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let (medius, url) = fake_medius_server().await;
    let h = harness_with(
        pool,
        Some(GameAdmin::new(&url, GAME_SECRET.to_vec()).unwrap()),
    )
    .await;
    let calls = || medius.state.lock().unwrap().voice_calls.clone();
    let active = || medius.state.lock().unwrap().voice.clone();

    // Players cannot reach it at all.
    let u = h.sign_in("player").await;
    let csrf_u = h.csrf(&u).await;
    let r = h
        .post_form(
            "/admin/game/characters/alpha/voice/confirm",
            &u,
            &[("csrf", &csrf_u), ("action", "mute"), ("duration", "30")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Moderators may moderate voice (they cannot set levels).
    let o = h.sign_in("owner").await;
    let csrf_o = h.csrf(&o).await;
    let r = h
        .post_form(
            "/admin/staff/set",
            &o,
            &[
                ("csrf", &csrf_o),
                ("email", "mod@example.test"),
                ("role", "moderator"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let m = h.sign_in("mod").await;
    let csrf_m = h.csrf(&m).await;
    let (status, page) = body(h.get("/admin/game/characters/alpha", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Not voice muted or banned") && page.contains("Preview voice change"));
    assert!(page.contains("action=\"/admin/game/characters/alpha/voice\""));
    let (_, page) = body(h.get("/admin/game", Some(&m)).await).await;
    assert!(page.contains("No account is voice muted or banned"));

    // The CSRF token is required; bad input is refused before the game server is asked.
    let r = h
        .post_form(
            "/admin/game/characters/alpha/voice",
            &m,
            &[("csrf", "bad"), ("action", "mute"), ("duration", "30")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    for form in [
        [("action", "silence"), ("duration", "30"), ("reason", "")],
        [("action", "mute"), ("duration", "soon"), ("reason", "")],
        [("action", "mute"), ("duration", "0"), ("reason", "")],
        [("action", "mute"), ("duration", "5256001"), ("reason", "")],
        [
            ("action", "mute"),
            ("duration", "30"),
            ("reason", "caf\u{e9}"),
        ],
    ] {
        let mut fields = vec![("csrf", csrf_m.as_str())];
        fields.extend(form);
        let r = h
            .post_form("/admin/game/characters/alpha/voice", &m, &fields)
            .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{form:?}");
    }
    assert!(calls().is_empty());

    // A dry run shows the plan and changes nothing.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/voice",
            &m,
            &[
                ("csrf", &csrf_m),
                ("action", "mute"),
                ("duration", "30"),
                ("reason", "synthetic spam"),
            ],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Nothing has been saved yet") && page.contains("Apply voice mute"));
    assert!(page.contains("action=\"/admin/game/characters/alpha/voice/confirm\""));
    assert!(page.contains("name=\"duration\" value=\"30\"") && page.contains("synthetic spam"));
    assert!(active().is_empty());
    assert_eq!(
        calls()[0].2,
        json!({"apply": false, "reason": "synthetic spam", "duration_minutes": 30})
    );

    // Confirming applies it, under the moderator's name, and writes the audit log.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/voice/confirm",
            &m,
            &[
                ("csrf", &csrf_m),
                ("action", "mute"),
                ("duration", "30"),
                ("reason", "synthetic spam"),
            ],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Voice mute applied."));
    assert_eq!(active().len(), 1);
    assert_eq!(active()[0]["operator"], "mod@example.test");
    // Until lifted: no duration is sent.
    let r = h
        .post_form(
            "/admin/game/characters/charlie/voice/confirm",
            &m,
            &[("csrf", &csrf_m), ("action", "ban"), ("duration", "")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(calls().last().unwrap().2, json!({"apply": true}));
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert!(log.contains("voice mute") && log.contains("reason: synthetic spam"));
    assert!(log.contains("voice ban") && log.contains("ends until lifted"));

    // The list and the character page show the sanctions, with a lift button.
    let (_, page) = body(h.get("/admin/game", Some(&m)).await).await;
    assert!(page.contains("Voice mutes and bans") && page.contains("synthetic spam"));
    assert!(page.contains("until lifted"));
    // Since, from the game server's UTC time.
    assert!(page.contains("datetime=\"2026-11-27T12:00:00Z\""), "{page}");
    let (_, page) = body(h.get("/admin/game/characters/alpha", Some(&m)).await).await;
    assert!(page.contains("Lift mute") && page.contains("value=\"unmute\""));
    assert!(!page.contains("Lift ban"));

    // Lifting: a refusal is explained and logs nothing; a lift is logged.
    let (status, page) = body(
        h.post_form(
            "/admin/game/characters/alpha/voice",
            &m,
            &[("csrf", &csrf_m), ("action", "unban")],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(page.contains("not_sanctioned") && page.contains("nothing to lift"));
    let r = h
        .post_form(
            "/admin/game/characters/alpha/voice/confirm",
            &m,
            &[("csrf", &csrf_m), ("action", "unmute"), ("duration", "30")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(calls().last().unwrap().2, json!({"apply": true}));
    assert_eq!(active().len(), 1);
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
    assert!(log.contains("voice unmute") && log.contains("lifted mute"));
    assert_eq!(log.matches("voice unban").count(), 0);

    // Unknown characters, and a game server without voice moderation.
    let r = h
        .post_form(
            "/admin/game/characters/nobody/voice",
            &m,
            &[("csrf", &csrf_m), ("action", "mute"), ("duration", "30")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    medius.state.lock().unwrap().no_voice = true;
    let (status, page) = body(h.get("/admin/game/characters/alpha", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("does not support voice moderation")
            && !page.contains("Preview voice change")
    );
    let (status, page) = body(h.get("/admin/game", Some(&m)).await).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("does not support voice moderation"));
    test_db.close().await;
}
