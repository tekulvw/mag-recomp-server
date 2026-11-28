//! Console linking against throwaway PostgreSQL databases and the real HTTP router.
//! Synthetic accounts, sessions and verified-console proof rows are created directly;
//! Google sign-in and device tickets remain covered by portal_flow.rs.

use std::net::SocketAddr;
use std::sync::Arc;

use mag_portal::config::{Settings, TicketIssuer};
use mag_portal::console_links::{self, RedeemOutcome};
use mag_portal::db::{self, ClaimOutcome};
use mag_portal::oidc::{OidcClient, ProviderConfig};
use mag_portal::web::{self, AppState, SESSION_COOKIE};
use p256::ecdsa::SigningKey;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

mod support;

struct Harness {
    base: String,
    http: reqwest::Client,
}

async fn harness(pool: PgPool) -> Harness {
    harness_with(pool, web::Limits::default()).await
}

async fn harness_with(pool: PgPool, limits: web::Limits) -> Harness {
    let mut settings = Settings::new("http://localhost").unwrap();
    settings.owner_emails = vec!["owner@example.test".into()];
    let ticket_key = SigningKey::from_slice(&Sha256::digest(b"portal console test key")).unwrap();
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
        status: Arc::new(mag_portal::status::StatusService::new(
            Vec::new(),
            std::time::Duration::from_secs(15),
        )),
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
    }
}

const CODE: &str = "0123456789abcdef0123456789abcdef";

#[sqlx::test(migrations = "./migrations")]
async fn schema_stores_only_hashed_proofs(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let inserted = sqlx::query("INSERT INTO console_link_proofs (code_hash, provider, subject, online_id, expires_at) VALUES ($1, 'psn', '256:42', 'DifferentName', now() + interval '5 minutes')")
        .bind(Sha256::digest(CODE.as_bytes()).as_slice()).execute(&pool).await;
    assert!(
        inserted.is_ok(),
        "hashed console proofs must be stored: {inserted:?}"
    );
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn console_page_requires_portal_authentication(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool).await;
    let response = h
        .http
        .get(format!("{}/account/console", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(
        response.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("/auth/google?next=")
    );
    test_db.close().await;
}

const OTHER_CODE: &str = "abcdef0123456789abcdef0123456789";

async fn person(pool: &PgPool, subject: &str, name: Option<&str>) -> i32 {
    let a = db::upsert_identity(pool, "google", subject, None)
        .await
        .unwrap();
    if let Some(name) = name {
        assert_eq!(
            db::claim_name(pool, a.id, name).await.unwrap(),
            ClaimOutcome::Claimed
        );
    }
    a.id
}

async fn proof(pool: &PgPool, code: &str, subject: &str, name: &str, expired: bool) {
    sqlx::query("INSERT INTO console_link_proofs (code_hash, provider, subject, online_id, expires_at) VALUES ($1, 'psn', $2, $3, clock_timestamp() + make_interval(secs => $4))")
        .bind(Sha256::digest(code.as_bytes()).as_slice()).bind(subject).bind(name)
        .bind(if expired { -1.0f64 } else { 300.0 }).execute(pool).await.unwrap();
}

async fn proof_exists(pool: &PgPool, code: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM console_link_proofs WHERE code_hash = $1)")
        .bind(Sha256::digest(code.as_bytes()).as_slice())
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn different_name_redemption_does_not_grant_same_name_account(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("PortalOwner")).await;
    let namesake = person(&pool, "namesake", Some("SonyName")).await;
    proof(&pool, CODE, "256:42", "SonyName", false).await;
    assert_eq!(
        console_links::redeem(&pool, owner, &format!("  {}\n", CODE.to_uppercase()))
            .await
            .unwrap(),
        RedeemOutcome::Linked
    );
    assert!(!proof_exists(&pool, CODE).await);
    assert_eq!(
        console_links::list(&pool, owner).await.unwrap()[0].subject,
        "256:42"
    );
    assert!(
        console_links::list(&pool, namesake)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db::account(&pool, owner)
            .await
            .unwrap()
            .unwrap()
            .player_name
            .as_deref(),
        Some("PortalOwner")
    );
    assert_eq!(
        console_links::redeem(&pool, namesake, CODE).await.unwrap(),
        RedeemOutcome::InvalidProof
    );
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_malformed_and_reused_proofs_are_denied(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("Owner")).await;
    proof(&pool, CODE, "256:42", "SonyName", true).await;
    for raw in [
        CODE,
        "",
        "0123456789abcdef",
        "g123456789abcdef0123456789abcdef",
        "0123456789abcdef 123456789abcdef",
    ] {
        assert_eq!(
            console_links::redeem(&pool, owner, raw).await.unwrap(),
            RedeemOutcome::InvalidProof
        );
    }
    assert!(console_links::list(&pool, owner).await.unwrap().is_empty());
    assert!(db::cleanup(&pool).await.unwrap() >= 1);
    assert!(!proof_exists(&pool, CODE).await);
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn conflicts_leave_proof_redeemable_after_unlink(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let a = person(&pool, "a", Some("Alpha")).await;
    let b = person(&pool, "b", Some("Bravo")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    assert_eq!(
        console_links::redeem(&pool, a, CODE).await.unwrap(),
        RedeemOutcome::Linked
    );
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    assert_eq!(
        console_links::redeem(&pool, b, CODE).await.unwrap(),
        RedeemOutcome::Conflict
    );
    assert!(proof_exists(&pool, CODE).await);
    proof(&pool, OTHER_CODE, "256:43", "SonyB", false).await;
    assert_eq!(
        console_links::redeem(&pool, a, OTHER_CODE).await.unwrap(),
        RedeemOutcome::Conflict
    );
    assert!(proof_exists(&pool, OTHER_CODE).await);
    assert!(!console_links::unlink(&pool, b).await.unwrap());
    assert_eq!(console_links::list(&pool, a).await.unwrap().len(), 1);
    assert!(console_links::unlink(&pool, a).await.unwrap());
    assert_eq!(
        console_links::redeem(&pool, b, CODE).await.unwrap(),
        RedeemOutcome::Linked
    );
    assert_eq!(
        console_links::redeem(&pool, a, OTHER_CODE).await.unwrap(),
        RedeemOutcome::Linked
    );
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_redemption_has_exactly_one_owner(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let a = person(&pool, "a", Some("Alpha")).await;
    let b = person(&pool, "b", Some("Bravo")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    let (left, right) = tokio::join!(
        console_links::redeem(&pool, a, CODE),
        console_links::redeem(&pool, b, CODE)
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|&&o| o == RedeemOutcome::Linked)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|&&o| o == RedeemOutcome::InvalidProof)
            .count(),
        1
    );
    assert_eq!(
        console_links::list(&pool, a).await.unwrap().len()
            + console_links::list(&pool, b).await.unwrap().len(),
        1
    );
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn ineligible_accounts_keep_proofs_and_deletion_cascades(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let banned = person(&pool, "banned", Some("Banned")).await;
    db::set_ban(&pool, "Banned", Some("synthetic"))
        .await
        .unwrap();
    let unnamed = person(&pool, "unnamed", None).await;
    let service = match db::create_service_account(&pool, "Service").await.unwrap() {
        db::ServiceCreateOutcome::Created { account_id, .. } => account_id,
        o => panic!("unexpected: {o:?}"),
    };
    let deleted = person(&pool, "deleted", Some("Deleted")).await;
    db::delete_account(&pool, deleted).await.unwrap();
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    for id in [banned, unnamed, service, deleted] {
        assert_eq!(
            console_links::redeem(&pool, id, CODE).await.unwrap(),
            RedeemOutcome::IneligibleAccount
        );
        assert!(proof_exists(&pool, CODE).await);
    }
    db::set_ban(&pool, "Banned", None).await.unwrap();
    assert_eq!(
        console_links::redeem(&pool, banned, CODE).await.unwrap(),
        RedeemOutcome::Linked
    );
    db::delete_account(&pool, banned).await.unwrap();
    assert!(console_links::list(&pool, banned).await.unwrap().is_empty());
    test_db.close().await;
}

async fn browser_session(pool: &PgPool, id: i32) -> (String, String) {
    let cookie = db::create_session(pool, id, time::Duration::hours(1))
        .await
        .unwrap()
        .cookie;
    let csrf = db::session(pool, &cookie)
        .await
        .unwrap()
        .unwrap()
        .csrf_token;
    (format!("{SESSION_COOKIE}={cookie}"), csrf)
}

#[sqlx::test(migrations = "./migrations")]
async fn browser_controls_require_csrf_and_only_unlink_current_account(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let a = person(&pool, "a", Some("Alpha")).await;
    let b = person(&pool, "b", Some("Bravo")).await;
    let (cookie, csrf) = browser_session(&pool, a).await;
    let (other_cookie, other_csrf) = browser_session(&pool, b).await;
    let h = harness(pool.clone()).await;
    proof(&pool, CODE, "256:42", "<SonyA>", false).await;
    let post = |path: &'static str, cookie: String, csrf: String| {
        h.http
            .post(format!("{}{path}", h.base))
            .header("cookie", cookie)
            .form(&[
                ("csrf", csrf),
                ("code", CODE.to_owned()),
                ("account_id", a.to_string()),
            ])
    };
    let account = h
        .http
        .get(format!("{}/account", h.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(account.contains("href=\"/account/console\""));
    assert_eq!(
        post("/account/console", cookie.clone(), "wrong".into())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert!(proof_exists(&pool, CODE).await);
    assert_eq!(
        post("/account/console", cookie.clone(), csrf.clone())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    let page = h
        .http
        .get(format!("{}/account/console", h.base))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("&#60;SonyA&#62;"));
    assert!(!page.contains(CODE));
    let other_page = h
        .http
        .get(format!("{}/account/console", h.base))
        .header("cookie", &other_cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!other_page.contains("SonyA"));
    assert_eq!(
        post("/account/console/unlink", cookie.clone(), "wrong".into())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post("/account/console/unlink", other_cookie, other_csrf)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(console_links::list(&pool, a).await.unwrap().len(), 1);
    assert_eq!(
        post("/account/console/unlink", cookie, csrf)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert!(console_links::list(&pool, a).await.unwrap().is_empty());
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_distinct_proofs_cannot_link_two_identities_to_one_account(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("Owner")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    proof(&pool, OTHER_CODE, "256:43", "SonyB", false).await;
    let (left, right) = tokio::join!(
        console_links::redeem(&pool, owner, CODE),
        console_links::redeem(&pool, owner, OTHER_CODE)
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|&&o| o == RedeemOutcome::Linked)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|&&o| o == RedeemOutcome::Conflict)
            .count(),
        1
    );
    assert_eq!(console_links::list(&pool, owner).await.unwrap().len(), 1);
    assert_ne!(
        proof_exists(&pool, CODE).await,
        proof_exists(&pool, OTHER_CODE).await
    );
    test_db.close().await;
}

async fn wait_for_account_lock(pool: &PgPool) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE 'SELECT kind, player_name%')")
                .fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("redemption must wait for the account lock");
}

#[sqlx::test(migrations = "./migrations")]
async fn redemption_rechecks_ban_after_waiting_for_account_lock(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("Owner")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let task_pool = pool.clone();
    let redeem = tokio::spawn(async move {
        console_links::redeem(&task_pool, owner, CODE)
            .await
            .unwrap()
    });
    wait_for_account_lock(&pool).await;
    sqlx::query("UPDATE accounts SET banned_at = clock_timestamp() WHERE id = $1")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(redeem.await.unwrap(), RedeemOutcome::IneligibleAccount);
    assert!(proof_exists(&pool, CODE).await);
    assert!(console_links::list(&pool, owner).await.unwrap().is_empty());
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn expiry_uses_time_after_account_lock_wait(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("Owner")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let task_pool = pool.clone();
    let redeem = tokio::spawn(async move {
        console_links::redeem(&task_pool, owner, CODE)
            .await
            .unwrap()
    });
    wait_for_account_lock(&pool).await;
    // Expiry is after the waiting transaction started, but before it acquires the lock.
    sqlx::query(
        "UPDATE console_link_proofs SET expires_at = clock_timestamp() - interval '1 microsecond'",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(redeem.await.unwrap(), RedeemOutcome::InvalidProof);
    assert!(console_links::list(&pool, owner).await.unwrap().is_empty());
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn browser_banned_unnamed_service_and_missing_csrf_cannot_acquire_link(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let named = person(&pool, "named", Some("Named")).await;
    let unnamed = person(&pool, "unnamed", None).await;
    let (named_cookie, named_csrf) = browser_session(&pool, named).await;
    let (unnamed_cookie, unnamed_csrf) = browser_session(&pool, unnamed).await;
    let service = match db::create_service_account(&pool, "Service").await.unwrap() {
        db::ServiceCreateOutcome::Created { account_id, .. } => account_id,
        o => panic!("unexpected: {o:?}"),
    };
    let service_cookie = db::create_session(&pool, service, time::Duration::hours(1))
        .await
        .unwrap()
        .cookie;
    let h = harness(pool.clone()).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    for path in ["/account/console", "/account/console/unlink"] {
        let response = h
            .http
            .post(format!("{}{path}", h.base))
            .header("cookie", &named_cookie)
            .form(&[("code", CODE)])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let response = h
            .http
            .post(format!("{}{path}", h.base))
            .form(&[("csrf", "x"), ("code", CODE)])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
    db::set_ban(&pool, "Named", Some("synthetic"))
        .await
        .unwrap();
    for (cookie, csrf) in [(named_cookie, named_csrf), (unnamed_cookie, unnamed_csrf)] {
        let response = h
            .http
            .post(format!("{}/account/console", h.base))
            .header("cookie", cookie)
            .form(&[("csrf", csrf), ("code", CODE.to_owned())])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = h
        .http
        .post(format!("{}/account/console", h.base))
        .header("cookie", format!("{SESSION_COOKIE}={service_cookie}"))
        .form(&[("csrf", "x"), ("code", CODE)])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert!(proof_exists(&pool, CODE).await);
    test_db.close().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn expiry_is_rechecked_after_unchanged_proof_lock_is_released(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let owner = person(&pool, "owner", Some("Owner")).await;
    proof(&pool, CODE, "256:42", "SonyA", false).await;
    sqlx::query(
        "UPDATE console_link_proofs SET expires_at = clock_timestamp() + interval '1 second'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT code_hash FROM console_link_proofs FOR UPDATE")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let task_pool = pool.clone();
    let redeem = tokio::spawn(async move {
        console_links::redeem(&task_pool, owner, CODE)
            .await
            .unwrap()
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%FROM console_link_proofs%')")
                .fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("redemption must wait for the proof row lock");
    // Let this unchanged row expire while the redeemer is blocked. Updating its expiry
    // would trigger PostgreSQL's row-version predicate recheck and conceal this race.
    sqlx::query("SELECT pg_sleep(GREATEST(0, extract(epoch FROM expires_at - clock_timestamp()))::double precision + 0.02) FROM console_link_proofs")
        .execute(&pool).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(redeem.await.unwrap(), RedeemOutcome::InvalidProof);
    assert!(console_links::list(&pool, owner).await.unwrap().is_empty());
    assert!(proof_exists(&pool, CODE).await);
    test_db.close().await;
}
