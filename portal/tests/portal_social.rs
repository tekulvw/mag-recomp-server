//! Friends, requests, blocks and presence (docs/portal-social.md) against a real PostgreSQL
//! (`DATABASE_URL`, see portal/README.md). Accounts, sessions and launcher tokens are made
//! directly in the database; sign-in itself is covered by portal_flow.rs.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mag_portal::config::{Settings, TicketIssuer};
use mag_portal::db::{self, ClaimOutcome};
use mag_portal::oidc::{OidcClient, ProviderConfig};
use mag_portal::ratelimit::RateLimiter;
use mag_portal::web::{self, AppState, SESSION_COOKIE};
use p256::ecdsa::SigningKey;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

mod support;

struct Harness {
    base: String,
    http: reqwest::Client,
    pool: PgPool,
}

/// One player: account ID, launcher token and web session cookie.
struct Player {
    id: i32,
    token: String,
    session: String,
}

async fn harness(pool: PgPool) -> Harness {
    harness_with(pool, web::Limits::default()).await
}

async fn harness_with(pool: PgPool, limits: web::Limits) -> Harness {
    let mut settings = Settings::new("http://localhost").unwrap();
    settings.owner_emails = vec!["owner@example.test".into()];
    let ticket_key = SigningKey::from_slice(&Sha256::digest(b"portal social test key")).unwrap();
    let unused = "http://127.0.0.1:9".to_owned();
    let state = Arc::new(AppState {
        pool: pool.clone(),
        oidc: OidcClient::new(
            ProviderConfig {
                provider: "google".into(),
                client_id: "unused".into(),
                client_secret: "unused".into(),
                auth_url: unused.clone(),
                token_url: unused.clone(),
                jwks_url: unused,
                issuers: vec!["https://issuer.example.test".into()],
                redirect_uri: "http://localhost/auth/google/callback".into(),
            },
            reqwest::Client::new(),
        ),
        tickets: TicketIssuer::new(ticket_key, 1, "play.example.test", 600).unwrap(),
        settings,
        limits,
        reports: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            web::router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    Harness {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        pool,
    }
}

fn roomy() -> web::Limits {
    let big = || RateLimiter::new(100_000, Duration::from_secs(60));
    web::Limits {
        presence: big(),
        social_changes: big(),
        social_reads: big(),
        lookup: big(),
        ..web::Limits::default()
    }
}

fn csrf_from(html: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = html.find(marker).unwrap() + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_owned()
}

impl Harness {
    /// A person (`subject`) holding `name` (unnamed when `None`), with a token and a session.
    async fn person(&self, subject: &str, name: Option<&str>) -> Player {
        let a = db::upsert_identity(
            &self.pool,
            "google",
            subject,
            Some(&format!("{subject}@example.test")),
        )
        .await
        .unwrap();
        if let Some(name) = name {
            assert_eq!(
                db::claim_name(&self.pool, a.id, name).await.unwrap(),
                ClaimOutcome::Claimed
            );
        }
        let (token, hash) = mag_portal::tokens::new_secret(db::DEVICE_TOKEN_PREFIX);
        sqlx::query("INSERT INTO devices (account_id, token_hash, label) VALUES ($1, $2, 'test')")
            .bind(a.id)
            .bind(&hash[..])
            .execute(&self.pool)
            .await
            .unwrap();
        let session = db::create_session(&self.pool, a.id, time::Duration::days(1))
            .await
            .unwrap()
            .cookie;
        Player {
            id: a.id,
            token,
            session,
        }
    }

    async fn named(&self, name: &str) -> Player {
        self.person(&name.to_ascii_lowercase(), Some(name)).await
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut req = self.http.request(method, format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        req.send().await.unwrap()
    }

    /// Status and JSON body (`Null` when there is none).
    async fn api(
        &self,
        method: Method,
        path: &str,
        p: &Player,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let r = self.call(method, path, Some(&p.token), body).await;
        let status = r.status();
        let text = r.text().await.unwrap();
        let v = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        };
        (status, v)
    }

    async fn social(&self, p: &Player) -> Value {
        let (status, v) = self.api(Method::GET, "/api/v1/social", p, None).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v
    }

    async fn request(&self, p: &Player, name: &str) -> (StatusCode, Value) {
        self.api(
            Method::POST,
            "/api/v1/social/requests",
            p,
            Some(json!({ "name": name })),
        )
        .await
    }

    async fn presence(&self, p: &Player, body: Value) -> StatusCode {
        self.api(Method::PUT, "/api/v1/social/presence", p, Some(body))
            .await
            .0
    }

    async fn page(&self, path: &str, p: &Player) -> reqwest::Response {
        self.http
            .get(format!("{}{path}", self.base))
            .header("cookie", format!("{SESSION_COOKIE}={}", p.session))
            .send()
            .await
            .unwrap()
    }

    async fn form(&self, path: &str, p: &Player, form: &[(&str, &str)]) -> reqwest::Response {
        self.http
            .post(format!("{}{path}", self.base))
            .header("cookie", format!("{SESSION_COOKIE}={}", p.session))
            .form(form)
            .send()
            .await
            .unwrap()
    }

    async fn csrf(&self, p: &Player) -> String {
        csrf_from(&self.page("/friends", p).await.text().await.unwrap())
    }

    async fn befriend(&self, a: &Player, b: &Player, b_name: &str, a_name: &str) {
        assert_eq!(self.request(a, b_name).await.1["status"], "requested");
        assert_eq!(self.request(b, a_name).await.1["status"], "accepted");
    }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_owned())
        .collect()
}

fn friend<'a>(social: &'a Value, name: &str) -> &'a Value {
    social["friends"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not a friend: {social}"))
}

#[sqlx::test]
async fn request_accept_and_presence(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;

    let empty = h.social(&alpha).await;
    assert_eq!(
        empty,
        json!({ "self": "Alpha", "friends": [], "incoming": [], "outgoing": [], "blocked": [],
                "poll_seconds": 15 })
    );

    // Names match in any case and come back canonical.
    let (status, v) = h.request(&alpha, "bRAVO").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(v, json!({ "status": "requested", "name": "Bravo" }));
    // Asking again changes nothing.
    assert_eq!(h.request(&alpha, "Bravo").await.1["status"], "requested");
    assert_eq!(names(&h.social(&alpha).await["outgoing"]), ["Bravo"]);
    assert_eq!(names(&h.social(&bravo).await["incoming"]), ["Alpha"]);

    // Presence is not shared before the request is accepted.
    let blob = STANDARD.encode([0u8, 1, 2, 0xff]);
    assert_eq!(
        h.presence(&alpha, json!({ "data_b64": blob, "state": "online" }))
            .await,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.social(&bravo).await["friends"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let (status, _) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/alpha/accept",
            &bravo,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let b = h.social(&bravo).await;
    assert!(names(&b["incoming"]).is_empty());
    let a_seen = friend(&b, "Alpha");
    assert_eq!(a_seen["online"], true);
    assert_eq!(a_seen["presence"]["data_b64"], blob);
    let at = a_seen["presence"]["updated_at"].as_str().unwrap();
    time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).unwrap();
    let a = h.social(&alpha).await;
    assert!(names(&a["outgoing"]).is_empty());
    assert_eq!(
        friend(&a, "Bravo"),
        &json!({ "name": "Bravo", "online": false, "presence": null })
    );

    // A second accept finds nothing waiting; asking a friend again is a conflict.
    let (status, v) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/Alpha/accept",
            &bravo,
            None,
        )
        .await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("no_request"))
    );
    let (status, v) = h.request(&alpha, "Bravo").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::CONFLICT, Some("already_friends"))
    );

    // A new blob replaces the old one; the state defaults to online.
    let blob2 = STANDARD.encode(b"second");
    assert_eq!(
        h.presence(&alpha, json!({ "data_b64": blob2 })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        friend(&h.social(&bravo).await, "Alpha")["presence"]["data_b64"],
        blob2
    );
    // An empty blob is still online.
    assert_eq!(
        h.presence(&alpha, json!({ "data_b64": "" })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(friend(&h.social(&bravo).await, "Alpha")["online"], true);
    test_db.close().await;
}

#[sqlx::test]
async fn crossing_requests_become_friends(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    h.befriend(&alpha, &bravo, "Bravo", "alpha").await;
    for p in [&alpha, &bravo] {
        let v = h.social(p).await;
        assert_eq!(v["friends"].as_array().unwrap().len(), 1);
        assert!(names(&v["incoming"]).is_empty() && names(&v["outgoing"]).is_empty());
    }
    test_db.close().await;
}

#[sqlx::test]
async fn decline_cancel_and_remove(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;

    // Decline: the request is gone for both.
    h.request(&alpha, "Bravo").await;
    let (status, _) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/Alpha/decline",
            &bravo,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(names(&h.social(&alpha).await["outgoing"]).is_empty());
    assert!(names(&h.social(&bravo).await["incoming"]).is_empty());

    // Cancel: the sender takes it back.
    h.request(&alpha, "Bravo").await;
    let (status, _) = h
        .api(
            Method::DELETE,
            "/api/v1/social/requests/Bravo",
            &alpha,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(names(&h.social(&bravo).await["incoming"]).is_empty());
    // Repeating either is harmless.
    for (method, path, p) in [
        (Method::DELETE, "/api/v1/social/requests/Bravo", &alpha),
        (
            Method::POST,
            "/api/v1/social/requests/Alpha/decline",
            &bravo,
        ),
        (Method::DELETE, "/api/v1/social/friends/Bravo", &alpha),
        (Method::DELETE, "/api/v1/social/blocks/Bravo", &alpha),
    ] {
        assert_eq!(
            h.api(method, path, p, None).await.0,
            StatusCode::NO_CONTENT,
            "{path}"
        );
    }

    // Remove: either side ends the friendship for both.
    h.befriend(&alpha, &bravo, "Bravo", "Alpha").await;
    let (status, _) = h
        .api(Method::DELETE, "/api/v1/social/friends/alpha", &bravo, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for p in [&alpha, &bravo] {
        assert!(h.social(p).await["friends"].as_array().unwrap().is_empty());
    }
    test_db.close().await;
}

#[sqlx::test]
async fn blocks_hide_presence_and_requests(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    let charlie = h.named("Charlie").await;
    h.befriend(&alpha, &bravo, "Bravo", "Alpha").await;
    h.request(&alpha, "Charlie").await;
    h.request(&charlie, "Bravo").await;
    h.presence(&bravo, json!({ "data_b64": "AAE=" })).await;
    h.presence(&alpha, json!({ "data_b64": "AAE=" })).await;
    assert_eq!(friend(&h.social(&alpha).await, "Bravo")["online"], true);

    // Blocking ends the friendship, so neither sees the other's presence.
    let (status, _) = h
        .api(Method::PUT, "/api/v1/social/blocks/bravo", &alpha, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Blocking twice is harmless.
    assert_eq!(
        h.api(Method::PUT, "/api/v1/social/blocks/Bravo", &alpha, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let a = h.social(&alpha).await;
    assert!(a["friends"].as_array().unwrap().is_empty());
    assert_eq!(names(&a["blocked"]), ["Bravo"]);
    let b = h.social(&bravo).await;
    assert!(b["friends"].as_array().unwrap().is_empty());
    assert!(names(&b["blocked"]).is_empty());

    // The blocker is refused; the blocked player's request looks sent but never arrives.
    let (status, v) = h.request(&alpha, "Bravo").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("blocked"))
    );
    let (status, v) = h.request(&bravo, "Alpha").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(v["status"], "requested");
    assert_eq!(names(&h.social(&bravo).await["outgoing"]), ["Alpha"]);
    assert!(names(&h.social(&alpha).await["incoming"]).is_empty());
    let (status, _) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/Bravo/accept",
            &alpha,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Blocking also drops requests both ways.
    h.api(Method::PUT, "/api/v1/social/blocks/Charlie", &alpha, None)
        .await;
    assert!(names(&h.social(&charlie).await["incoming"]).is_empty());
    assert!(names(&h.social(&alpha).await["outgoing"]).is_empty());
    assert_eq!(
        names(&h.social(&alpha).await["blocked"]),
        ["Bravo", "Charlie"]
    );
    // Other players are untouched.
    assert_eq!(names(&h.social(&bravo).await["incoming"]), ["Charlie"]);

    // Unblocking lets a request held while blocked show up.
    let (status, _) = h
        .api(Method::DELETE, "/api/v1/social/blocks/Bravo", &alpha, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let a = h.social(&alpha).await;
    assert_eq!(names(&a["blocked"]), ["Charlie"]);
    assert_eq!(names(&a["incoming"]), ["Bravo"]);
    assert_eq!(h.request(&alpha, "Bravo").await.1["status"], "accepted");
    test_db.close().await;
}

#[sqlx::test]
async fn presence_expires_and_goes_offline(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    let stranger = h.named("Stranger").await;
    h.befriend(&alpha, &bravo, "Bravo", "Alpha").await;

    h.presence(&alpha, json!({ "data_b64": "AQID" })).await;
    h.presence(&stranger, json!({ "data_b64": "AQID" })).await;
    assert_eq!(friend(&h.social(&bravo).await, "Alpha")["online"], true);

    // Explicitly offline: cleared at once.
    assert_eq!(
        h.presence(&alpha, json!({ "state": "offline" })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        friend(&h.social(&bravo).await, "Alpha"),
        &json!({ "name": "Alpha", "online": false, "presence": null })
    );
    let rows: (i64,) = sqlx::query_as("SELECT count(*) FROM presence WHERE account_id = $1")
        .bind(alpha.id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);

    // Not refreshed within 60 s: offline, and the cleanup task drops it.
    h.presence(&alpha, json!({ "data_b64": "AQID" })).await;
    sqlx::query("UPDATE presence SET updated_at = now() - interval '61 seconds'")
        .execute(&h.pool)
        .await
        .unwrap();
    assert_eq!(friend(&h.social(&bravo).await, "Alpha")["online"], false);
    assert!(db::cleanup(&h.pool).await.unwrap() >= 2);
    let rows: (i64,) = sqlx::query_as("SELECT count(*) FROM presence")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);

    // Bad bodies.
    let max = STANDARD.encode(vec![7u8; 1024]);
    assert_eq!(
        h.presence(&alpha, json!({ "data_b64": max })).await,
        StatusCode::NO_CONTENT
    );
    let over = STANDARD.encode(vec![7u8; 1025]);
    let (status, v) = h
        .api(
            Method::PUT,
            "/api/v1/social/presence",
            &alpha,
            Some(json!({ "data_b64": over })),
        )
        .await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::PAYLOAD_TOO_LARGE, Some("too_large"))
    );
    let huge = "A".repeat(8192);
    assert_eq!(
        h.presence(&alpha, json!({ "data_b64": huge })).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for bad in [
        json!({ "data_b64": "not base64!" }),
        json!({ "state": "online" }),
        json!({ "data_b64": "AA==", "state": "away" }),
        json!(["AA=="]),
    ] {
        assert_eq!(
            h.presence(&alpha, bad.clone()).await,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    test_db.close().await;
}

#[sqlx::test]
async fn auth_and_name_errors(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let alpha = h.named("Alpha").await;
    let unnamed = h.person("unnamed", None).await;

    for (method, path) in [
        (Method::GET, "/api/v1/social"),
        (Method::PUT, "/api/v1/social/presence"),
        (Method::POST, "/api/v1/social/requests"),
        (Method::POST, "/api/v1/social/requests/Alpha/accept"),
        (Method::POST, "/api/v1/social/requests/Alpha/decline"),
        (Method::DELETE, "/api/v1/social/requests/Alpha"),
        (Method::DELETE, "/api/v1/social/friends/Alpha"),
        (Method::PUT, "/api/v1/social/blocks/Alpha"),
        (Method::DELETE, "/api/v1/social/blocks/Alpha"),
        (Method::GET, "/api/v1/social/lookup?name=Alpha"),
    ] {
        let body = Some(json!({ "name": "Alpha", "data_b64": "" }));
        for token in [None, Some("mgd_nope"), Some("not-a-token")] {
            let r = h.call(method.clone(), path, token, body.clone()).await;
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{path}");
            assert_eq!(r.json::<Value>().await.unwrap()["error"], "invalid_token");
        }
        let r = h
            .call(method.clone(), path, Some(&unnamed.token), body.clone())
            .await;
        assert_eq!(r.status(), StatusCode::CONFLICT, "{path}");
        assert_eq!(r.json::<Value>().await.unwrap()["error"], "no_player_name");
    }

    let (status, v) = h.request(&alpha, "Nobody").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknown_name"))
    );
    let (status, v) = h.request(&alpha, "not a name!").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::NOT_FOUND, Some("unknown_name"))
    );
    let (status, v) = h.request(&alpha, "ALPHA").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("self"))
    );
    let (status, _) = h
        .api(Method::PUT, "/api/v1/social/blocks/Alpha", &alpha, None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = h
        .api(
            Method::DELETE,
            "/api/v1/social/friends/Nobody",
            &alpha,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in [json!({}), json!({ "name": 7 }), json!("Alpha")] {
        let (status, v) = h
            .api(Method::POST, "/api/v1/social/requests", &alpha, Some(bad))
            .await;
        assert_eq!(
            (status, v["error"].as_str()),
            (StatusCode::BAD_REQUEST, Some("invalid_request"))
        );
    }

    // Banned accounts cannot use the API; service accounts can.
    let bravo = h.named("Bravo").await;
    assert!(
        mag_portal::staff::set_ban_by_id(&h.pool, bravo.id, Some("test"))
            .await
            .unwrap()
    );
    let (status, v) = h.api(Method::GET, "/api/v1/social", &bravo, None).await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("banned"))
    );
    let db::ServiceCreateOutcome::Created { account_id, .. } =
        db::create_service_account(&h.pool, "PlayerLinux")
            .await
            .unwrap()
    else {
        panic!("service account not created");
    };
    let minted = db::mint_service_token(&h.pool, account_id, "test")
        .await
        .unwrap()
        .unwrap();
    let service = Player {
        id: account_id,
        token: minted.token,
        session: String::new(),
    };
    assert_eq!(h.request(&service, "Alpha").await.0, StatusCode::CREATED);
    assert_eq!(
        h.request(&alpha, "playerlinux").await.1["status"],
        "accepted"
    );
    assert_eq!(h.social(&service).await["self"], "PlayerLinux");
    test_db.close().await;
}

#[sqlx::test]
async fn lookup_finds_exact_names(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let limits = web::Limits {
        lookup: RateLimiter::new(5, Duration::from_secs(60)),
        ..roomy()
    };
    let h = harness_with(pool, limits).await;
    let alpha = h.named("Alpha").await;
    h.named("Alphabet").await;
    let (status, v) = h
        .api(
            Method::GET,
            "/api/v1/social/lookup?name=alphabet",
            &alpha,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v, json!({ "name": "Alphabet", "exists": true }));
    for path in [
        "/api/v1/social/lookup?name=Alph",
        "/api/v1/social/lookup?name=Alph%25",
        "/api/v1/social/lookup",
    ] {
        let (status, v) = h.api(Method::GET, path, &alpha, None).await;
        assert_eq!(
            (status, v["error"].as_str()),
            (StatusCode::NOT_FOUND, Some("unknown_name")),
            "{path}"
        );
    }
    // Five lookups a minute in this harness.
    let (status, _) = h
        .api(
            Method::GET,
            "/api/v1/social/lookup?name=Alpha",
            &alpha,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let r = h
        .call(
            Method::GET,
            "/api/v1/social/lookup?name=Alpha",
            Some(&alpha.token),
            None,
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.headers()["retry-after"], "60");
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "rate_limited");
    test_db.close().await;
}

/// Accounts `Filler1..=n` holding names, made in one statement.
async fn fillers(pool: &PgPool, n: i32) -> Vec<i32> {
    sqlx::query_as::<_, (i32,)>(
        "INSERT INTO accounts (provider, subject, player_name)
         SELECT 'google', 'filler-' || g, 'Filler' || g FROM generate_series(1, $1) g
         RETURNING id",
    )
    .bind(n)
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(id,)| id)
    .collect()
}

#[sqlx::test]
async fn list_limits(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness_with(pool, roomy()).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    let ids = fillers(&h.pool, 260).await;

    // 50 requests may wait at once.
    for i in 1..=50 {
        let (status, v) = h.request(&alpha, &format!("Filler{i}")).await;
        assert_eq!(status, StatusCode::CREATED, "{i}: {v}");
    }
    let (status, v) = h.request(&alpha, "Filler51").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::CONFLICT, Some("request_limit"))
    );
    // A repeat of a waiting request still answers.
    assert_eq!(h.request(&alpha, "Filler50").await.0, StatusCode::CREATED);
    h.api(
        Method::DELETE,
        "/api/v1/social/requests/Filler50",
        &alpha,
        None,
    )
    .await;
    assert_eq!(h.request(&alpha, "Filler51").await.0, StatusCode::CREATED);

    // 200 friends at most: no new requests, no accepts, no crossing requests.
    sqlx::query(
        "INSERT INTO friendships (account_lo, account_hi)
         SELECT least($1, f), greatest($1, f) FROM unnest($2::int[]) f",
    )
    .bind(bravo.id)
    .bind(&ids[60..260])
    .execute(&h.pool)
    .await
    .unwrap();
    assert_eq!(
        h.social(&bravo).await["friends"].as_array().unwrap().len(),
        200
    );
    let (status, v) = h.request(&bravo, "Filler1").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::CONFLICT, Some("friend_limit"))
    );
    h.api(
        Method::DELETE,
        "/api/v1/social/requests/Filler49",
        &alpha,
        None,
    )
    .await;
    let (status, v) = h.request(&alpha, "Bravo").await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let (status, v) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/Alpha/accept",
            &bravo,
            None,
        )
        .await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::CONFLICT, Some("friend_limit"))
    );
    let (status, v) = h.request(&bravo, "Alpha").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::CONFLICT, Some("friend_limit"))
    );
    // Making room lets the accept through.
    h.api(
        Method::DELETE,
        "/api/v1/social/friends/Filler200",
        &bravo,
        None,
    )
    .await;
    let (status, _) = h
        .api(
            Method::POST,
            "/api/v1/social/requests/Alpha/accept",
            &bravo,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    test_db.close().await;
}

#[sqlx::test]
async fn per_account_rate_limits(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let limits = web::Limits {
        presence: RateLimiter::new(3, Duration::from_secs(60)),
        social_changes: RateLimiter::new(2, Duration::from_secs(60)),
        social_reads: RateLimiter::new(2, Duration::from_secs(60)),
        ..roomy()
    };
    let h = harness_with(pool, limits).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    h.named("Charlie").await;
    h.named("Delta").await;

    for _ in 0..3 {
        assert_eq!(
            h.presence(&alpha, json!({ "data_b64": "AA==" })).await,
            StatusCode::NO_CONTENT
        );
    }
    let r = h
        .call(
            Method::PUT,
            "/api/v1/social/presence",
            Some(&alpha.token),
            Some(json!({ "data_b64": "AA==" })),
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.headers()["retry-after"], "60");
    // Each account has its own allowance, and each kind of call its own limit.
    assert_eq!(
        h.presence(&bravo, json!({ "data_b64": "AA==" })).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(h.request(&alpha, "Charlie").await.0, StatusCode::CREATED);
    assert_eq!(
        h.api(Method::PUT, "/api/v1/social/blocks/Delta", &alpha, None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let (status, v) = h.request(&alpha, "Bravo").await;
    assert_eq!(
        (status, v["error"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    // The Friends page shares the change allowance.
    let csrf = csrf_from(&h.page("/friends", &alpha).await.text().await.unwrap());
    let r = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf), ("name", "Bravo")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    // Reads: the page above was one, so one more API read fits.
    assert_eq!(
        h.api(Method::GET, "/api/v1/social", &alpha, None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        h.api(Method::GET, "/api/v1/social", &alpha, None).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        h.page("/friends", &alpha).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    test_db.close().await;
}

#[sqlx::test]
async fn friends_page_flows(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness_with(pool, roomy()).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    let charlie = h.named("Charlie").await;

    // Signed out: sign in first. Unnamed: choose a name first.
    let r = h
        .http
        .get(format!("{}/friends", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()["location"], "/auth/google?next=%2Ffriends");
    let unnamed = h.person("unnamed", None).await;
    let r = h.page("/friends", &unnamed).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()["location"], "/claim?next=%2Ffriends");
    // The navigation links the page.
    let account = h.page("/account", &alpha).await.text().await.unwrap();
    assert!(account.contains("href=\"/friends\""));

    let csrf_a = h.csrf(&alpha).await;
    let csrf_b = h.csrf(&bravo).await;
    // Forms need the session's CSRF token.
    let r = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", "bad"), ("name", "Bravo")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = h
        .form("/friends/Bravo/block", &alpha, &[("csrf", &csrf_b)])
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(names(&h.social(&alpha).await["blocked"]).is_empty());

    // Add by name; mistakes are explained.
    let page = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf_a), ("name", "nobody")],
        )
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("Nobody is called nobody."), "{page}");
    let page = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf_a), ("name", "alpha")],
        )
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("That is you."));
    let page = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf_a), ("name", "bravo")],
        )
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("Friend request sent to Bravo."));
    assert!(page.contains("action=\"/friends/Bravo/cancel\""));

    // Bravo sees it and accepts.
    let page = h.page("/friends", &bravo).await.text().await.unwrap();
    assert!(page.contains("Requests for you") && page.contains("action=\"/friends/Alpha/accept\""));
    let r = h
        .form("/friends/Alpha/accept", &bravo, &[("csrf", &csrf_b)])
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers()["location"], "/friends");
    let page = h.page("/friends", &bravo).await.text().await.unwrap();
    assert!(
        page.contains("Your friends · 1/200")
            && page.contains("<span class=\"quiet\">offline</span>")
    );
    h.presence(&alpha, json!({ "data_b64": "AA==" })).await;
    let page = h.page("/friends", &bravo).await.text().await.unwrap();
    assert!(
        page.contains("<span class=\"status-on\">online</span>") && page.contains("1 playing now.")
    );
    // The opaque presence is never shown.
    assert!(!page.contains("AA=="));

    // Decline and cancel.
    h.request(&charlie, "Alpha").await;
    let r = h
        .form("/friends/Charlie/decline", &alpha, &[("csrf", &csrf_a)])
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(names(&h.social(&charlie).await["outgoing"]).is_empty());
    h.request(&alpha, "Charlie").await;
    h.form("/friends/Charlie/cancel", &alpha, &[("csrf", &csrf_a)])
        .await;
    assert!(names(&h.social(&charlie).await["incoming"]).is_empty());

    // Block from the list and by name, then unblock; remove a friend.
    let page = h
        .form("/friends/Bravo/block", &alpha, &[("csrf", &csrf_a)])
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("You block Bravo now."));
    assert!(page.contains("action=\"/friends/Bravo/unblock\""));
    assert!(
        h.social(&bravo).await["friends"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let page = h
        .form(
            "/friends/block",
            &alpha,
            &[("csrf", &csrf_a), ("name", "charlie")],
        )
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("You block Charlie now."));
    let page = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf_a), ("name", "Bravo")],
        )
        .await
        .text()
        .await
        .unwrap();
    assert!(page.contains("You block Bravo. Unblock them first."));
    h.form("/friends/Bravo/unblock", &alpha, &[("csrf", &csrf_a)])
        .await;
    assert_eq!(names(&h.social(&alpha).await["blocked"]), ["Charlie"]);
    h.befriend(&alpha, &bravo, "Bravo", "Alpha").await;
    let r = h
        .form("/friends/Bravo/remove", &alpha, &[("csrf", &csrf_a)])
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(
        h.social(&bravo).await["friends"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let r = h
        .form("/friends/Bravo/explode", &alpha, &[("csrf", &csrf_a)])
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Banned players cannot use the page.
    assert!(
        mag_portal::staff::set_ban_by_id(&h.pool, alpha.id, Some("test"))
            .await
            .unwrap()
    );
    assert_eq!(
        h.page("/friends", &alpha).await.status(),
        StatusCode::FORBIDDEN
    );
    let r = h
        .form(
            "/friends/add",
            &alpha,
            &[("csrf", &csrf_a), ("name", "Bravo")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The privacy page explains presence.
    let privacy = h.page("/privacy", &bravo).await.text().await.unwrap();
    assert!(privacy.contains("Friends and status") && privacy.contains("60 seconds"));
    test_db.close().await;
}

#[sqlx::test]
async fn staff_see_counts_and_deletion_cascades(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness_with(pool, roomy()).await;
    let alpha = h.named("Alpha").await;
    let bravo = h.named("Bravo").await;
    let charlie = h.named("Charlie").await;
    let owner = h.person("owner", Some("Owner")).await;
    let player = h.named("Player").await;
    h.befriend(&alpha, &bravo, "Bravo", "Alpha").await;
    h.request(&charlie, "Alpha").await;
    h.request(&alpha, "Player").await;
    h.api(Method::PUT, "/api/v1/social/blocks/Owner", &alpha, None)
        .await;
    h.presence(&alpha, json!({ "data_b64": "AA==" })).await;

    let page = h
        .page(&format!("/admin/users/{}", alpha.id), &owner)
        .await
        .text()
        .await
        .unwrap();
    assert!(
        page.contains("1 (requests: 1 in, 1 out; blocks: 1)"),
        "{page}"
    );
    // Players without a staff role do not get the admin page.
    assert_eq!(
        h.page(&format!("/admin/users/{}", alpha.id), &player)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    assert!(db::delete_account(&h.pool, alpha.id).await.unwrap());
    for table in [
        "SELECT count(*) FROM friendships",
        "SELECT count(*) FROM friend_requests",
        "SELECT count(*) FROM blocks",
        "SELECT count(*) FROM presence",
    ] {
        let (n,): (i64,) = sqlx::query_as(table).fetch_one(&h.pool).await.unwrap();
        assert_eq!(n, 0, "{table}");
    }
    assert!(
        h.social(&bravo).await["friends"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    test_db.close().await;
}
