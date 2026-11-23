//! End-to-end flows against a real PostgreSQL (`DATABASE_URL`, see portal/README.md) and a
//! fake OpenID provider that signs ID tokens with a freshly generated RSA key.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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

const CLIENT_ID: &str = "test-client";
const ISSUER: &str = "https://issuer.example.test";
const AUDIENCE: &str = "play.example.test";

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
    settings.link_poll_seconds = 2;
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
        tickets: TicketIssuer::new(ticket_key, 3, AUDIENCE, 600).unwrap(),
        settings,
        limits: web::Limits::default(),
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
async fn sign_in_link_and_ticket(pool: PgPool) {
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
    assert_eq!(start["interval"], 2);
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
}

#[sqlx::test]
async fn names_stay_with_their_characters(pool: PgPool) {
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
}

#[sqlx::test]
async fn sign_in_rejections(pool: PgPool) {
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
}

#[sqlx::test]
async fn banned_accounts_cannot_link(pool: PgPool) {
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
}

#[sqlx::test]
async fn launcher_api_limits_and_errors(pool: PgPool) {
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
}

#[sqlx::test]
async fn banned_accounts_cannot_delete_themselves(pool: PgPool) {
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
}

/// A claim waits for any transaction holding the name's lock, so it sees a reservation that
/// transaction commits (the deletion path) instead of racing it.
#[sqlx::test]
async fn claims_wait_for_name_changes(pool: PgPool) {
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
}

async fn id_of(h: &Harness, subject: &str) -> i32 {
    db::accounts_by_email(&h.pool, &format!("{subject}@example.test"))
        .await
        .unwrap()[0]
        .id
}

#[sqlx::test]
async fn admin_panel_roles(pool: PgPool) {
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
    let page = h.get("/admin", Some(&o)).await.text().await.unwrap();
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
    let staff = h.get("/admin/staff", Some(&o)).await.text().await.unwrap();
    assert!(staff.contains("mod@example.test") && staff.contains("owner@example.test"));

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

    // Every change is in the audit log, newest first.
    let log = h.get("/admin", Some(&o)).await.text().await.unwrap();
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
}

#[sqlx::test]
async fn homepage_for_everyone(pool: PgPool) {
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
}

#[sqlx::test]
async fn fonts_and_page_policy(pool: PgPool) {
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
}

#[sqlx::test]
async fn stylesheet_address_changes_with_its_content(pool: PgPool) {
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
    /// (account, level, apply) for each signed level request.
    level_calls: Vec<(String, u32, bool)>,
}

struct FakeMedius {
    secret: Vec<u8>,
    state: Mutex<MediusState>,
}

fn lower_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks the request's signature independently of the portal's code, then answers like the
/// Medius admin API contract.
async fn fake_medius(
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

async fn body(r: reqwest::Response) -> (StatusCode, String) {
    (r.status(), r.text().await.unwrap())
}

#[sqlx::test]
async fn game_server_not_configured(pool: PgPool) {
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
}

#[sqlx::test]
async fn game_server_characters_and_levels(pool: PgPool) {
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
}

#[sqlx::test]
async fn game_server_errors_are_shown(pool: PgPool) {
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
}
