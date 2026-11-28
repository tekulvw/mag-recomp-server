//! Player reports (docs/portal-player-reports.md) against a real PostgreSQL (`DATABASE_URL`,
//! see portal/README.md): filing, the daily allowance, refusals, the moderators' queue and
//! review, the audit log, and what reporters may and may not see. Accounts and sessions are
//! made directly in the database; sign-in itself is covered by portal_flow.rs.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use mag_portal::config::{Settings, TicketIssuer};
use mag_portal::db::{self, ClaimOutcome};
use mag_portal::game_admin::GameAdmin;
use mag_portal::oidc::{OidcClient, ProviderConfig};
use mag_portal::web::{self, AppState, SESSION_COOKIE};
use p256::ecdsa::SigningKey;
use reqwest::StatusCode;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

mod support;

const GAME_SECRET: &[u8] = b"synthetic-player-report-secret-0123456789";

struct Harness {
    base: String,
    http: reqwest::Client,
    pool: PgPool,
}

struct Player {
    id: i32,
    session: String,
}

/// A stand-in for the game server's admin API that lists characters. Signatures are checked in
/// portal_flow.rs; here only the list matters. `None` answers 500, as a broken server would.
struct FakeGame {
    accounts: Mutex<Option<Vec<String>>>,
    calls: Mutex<u32>,
}

async fn fake_game_handler(State(g): State<Arc<FakeGame>>) -> axum::response::Response {
    use axum::response::IntoResponse;
    *g.calls.lock().unwrap() += 1;
    match g.accounts.lock().unwrap().clone() {
        Some(list) => {
            let accounts: Vec<Value> = list
                .iter()
                .map(|a| {
                    json!({"account": a, "level": 1, "experience": 0, "skill_points": 0,
                           "session": "offline"})
                })
                .collect();
            Json(json!({ "accounts": accounts })).into_response()
        }
        None => (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response(),
    }
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

async fn fake_game(accounts: Option<&[&str]>) -> (Arc<FakeGame>, GameAdmin) {
    let game = Arc::new(FakeGame {
        accounts: Mutex::new(accounts.map(|a| a.iter().map(|s| s.to_string()).collect())),
        calls: Mutex::new(0),
    });
    let addr = serve(
        Router::new()
            .route("/admin/v1/accounts", axum::routing::get(fake_game_handler))
            .with_state(game.clone()),
    )
    .await;
    let admin = GameAdmin::new(&format!("http://{addr}"), GAME_SECRET.to_vec()).unwrap();
    (game, admin)
}

async fn harness(pool: PgPool, per_day: i64, game_admin: Option<GameAdmin>) -> Harness {
    let mut settings = Settings::new("http://localhost").unwrap();
    settings.owner_emails = vec!["owner@example.test".into()];
    settings.player_reports_per_day = per_day;
    settings.game_admin = game_admin;
    let ticket_key = SigningKey::from_slice(&Sha256::digest(b"player report test key")).unwrap();
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
        limits: web::Limits::default(),
        reports: None,
        status: Arc::new(mag_portal::status::StatusService::new(
            Vec::new(),
            std::time::Duration::from_secs(15),
        )),
    });
    let addr = serve(web::router(state)).await;
    Harness {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        pool,
    }
}

fn csrf_from(html: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = html.find(marker).unwrap() + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_owned()
}

impl Harness {
    /// A person signed in as `subject`, holding `name` (unnamed when `None`).
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
        let session = db::create_session(&self.pool, a.id, time::Duration::days(1))
            .await
            .unwrap()
            .cookie;
        Player { id: a.id, session }
    }

    async fn named(&self, name: &str) -> Player {
        self.person(&name.to_ascii_lowercase(), Some(name)).await
    }

    async fn get(&self, path: &str, p: Option<&Player>) -> (StatusCode, String) {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(p) = p {
            req = req.header("cookie", format!("{SESSION_COOKIE}={}", p.session));
        }
        let r = req.send().await.unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        if text.starts_with("<!doctype html>") {
            support::page_times(&text);
        }
        (status, text)
    }

    async fn csrf(&self, p: &Player) -> String {
        let (status, html) = self.get("/account", Some(p)).await;
        assert_eq!(status, StatusCode::OK);
        csrf_from(&html)
    }

    async fn post(&self, path: &str, p: &Player, form: &[(&str, &str)]) -> reqwest::Response {
        self.http
            .post(format!("{}{path}", self.base))
            .header("cookie", format!("{SESSION_COOKIE}={}", p.session))
            .form(form)
            .send()
            .await
            .unwrap()
    }

    /// Files a report as `p` and returns the status and body.
    async fn report(
        &self,
        p: &Player,
        name: &str,
        reason: &str,
        description: &str,
        when: &str,
    ) -> (StatusCode, String) {
        let csrf = self.csrf(p).await;
        let r = self
            .post(
                "/reports/new",
                p,
                &[
                    ("csrf", &csrf),
                    ("name", name),
                    ("reason", reason),
                    ("description", description),
                    ("when", when),
                    ("tz", ""),
                ],
            )
            .await;
        let status = r.status();
        if status == StatusCode::SEE_OTHER {
            assert_eq!(r.headers()["location"], "/reports?filed=1");
        }
        (status, r.text().await.unwrap())
    }

    async fn count(&self, sql: &'static str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(sql).fetch_one(&self.pool).await.unwrap();
        n
    }

    async fn grant(&self, email: &str, role: &str) {
        sqlx::query("INSERT INTO staff (email_key, role) VALUES ($1, $2)")
            .bind(email)
            .bind(role)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn report_ids(&self) -> Vec<i64> {
        sqlx::query_as::<_, (i64,)>("SELECT id FROM player_reports ORDER BY id")
            .fetch_all(&self.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|(id,)| id)
            .collect()
    }
}

#[sqlx::test]
async fn players_file_reports_and_see_only_their_status(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool, 5, None).await;
    let alice = h.named("Alice").await;
    let bravo = h.named("Bravo").await;

    // Linked from the account page, not from the public pages.
    let (_, account) = h.get("/account", Some(&alice)).await;
    assert!(account.contains("href=\"/reports/new\"") && account.contains("href=\"/reports\""));
    let (_, home) = h.get("/", None).await;
    assert!(!home.contains("/reports/new"));
    let (_, status) = h.get("/status", None).await;
    assert!(!status.contains("/reports/new"));

    // Signed out: sign in first. Unnamed: choose a name first.
    let (s, _) = h.get("/reports/new", None).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let unnamed = h.person("nobody", None).await;
    let r = h
        .http
        .get(format!("{}/reports/new", h.base))
        .header("cookie", format!("{SESSION_COOKIE}={}", unnamed.session))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(
        r.headers()["location"]
            .to_str()
            .unwrap()
            .starts_with("/claim?next=")
    );

    // The form: external script only (the policy forbids inline ones), every reason offered.
    let (s, form) = h.get("/reports/new?name=Bravo", Some(&alice)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(form.contains("<script src=\"/report.js?v="));
    assert!(!form.contains("<script>"));
    for reason in [
        "cheating",
        "harassment",
        "griefing",
        "offensive_name",
        "other",
    ] {
        assert!(form.contains(&format!("value=\"{reason}\"")), "{reason}");
    }
    assert!(form.contains("value=\"Bravo\""));
    // The incident time is typed in local time; the shared time script names the zone and sends
    // its offset. Without it the label says UTC, as the server then reads it.
    assert!(form.contains("<span data-zone-for=\"report-when\">UTC</span>"));
    assert!(form.contains("id=\"report-when\" value=\"\" data-tz-field=\"report-tz\""));
    assert!(form.contains("<script src=\"/time.js?v="));
    let script = h
        .http
        .get(format!("{}/report.js", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(script.status(), StatusCode::OK);
    assert!(
        script.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    assert!(!script.text().await.unwrap().contains("innerHTML"));

    // Any case finds the player; the stored spelling is theirs. The time is taken as UTC
    // without the form script's time zone.
    let hour_ago = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    let typed = hour_ago
        .format(time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]"
        ))
        .unwrap();
    let (s, _) = h
        .report(
            &alice,
            "bRAVO",
            "harassment",
            "  Kept shouting slurs\r\nover voice on Sabotage.  ",
            &typed,
        )
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (occurred,): (Option<time::OffsetDateTime>,) =
        sqlx::query_as("SELECT occurred_at FROM player_reports")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(
        occurred.map(|t| t.unix_timestamp() / 60),
        Some(hour_ago.unix_timestamp() / 60)
    );
    let row: (Option<i32>, String, String, Option<i32>, String, String, String) = sqlx::query_as(
        "SELECT reporter_account, target_key, target_name, target_account, reason, description, status
         FROM player_reports",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            Some(alice.id),
            "bravo".into(),
            "Bravo".into(),
            Some(bravo.id),
            "harassment".into(),
            "Kept shouting slurs\nover voice on Sabotage.".into(),
            "open".into()
        )
    );

    let (s, mine) = h.get("/reports?filed=1", Some(&alice)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(mine.contains("sent to the moderators"));
    assert!(mine.contains("Bravo") && mine.contains("Harassment or voice abuse"));
    assert!(mine.contains("<td>open</td>"));
    assert_eq!(support::page_times(&mine), 1, "filed: {mine}");
    // Names only: no emails or account numbers of anyone.
    assert!(!mine.contains("bravo@example.test") && !mine.contains("/admin/users/"));

    // Bravo sees nothing about it.
    let (_, theirs) = h.get("/reports", Some(&bravo)).await;
    assert!(theirs.contains("You have not reported anyone.") && !theirs.contains("Alice"));

    // Banned players cannot report.
    sqlx::query("UPDATE accounts SET banned_at = now(), ban_reason = 'x' WHERE id = $1")
        .bind(bravo.id)
        .execute(&h.pool)
        .await
        .unwrap();
    let (s, _) = h.get("/reports/new", Some(&bravo)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    test_db.close().await;
}

#[sqlx::test]
async fn refusals_and_the_daily_allowance(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool, 2, None).await;
    let alice = h.named("Alice").await;
    for name in ["Bravo", "Charlie", "Delta"] {
        h.named(name).await;
    }

    // Not yourself, in any spelling.
    for me in ["Alice", "alice", " ALICE "] {
        let (s, page) = h.report(&alice, me, "cheating", "x", "").await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{me}");
        assert!(page.contains("You cannot report yourself."));
    }
    // Everything else the form needs.
    let long = "x".repeat(1001);
    for (name, reason, description, when, text) in [
        ("", "cheating", "x", "", "Enter the name"),
        ("Nobody", "cheating", "x", "", "Nobody is called Nobody."),
        ("a b", "cheating", "x", "", "Enter the name"),
        ("Bravo", "", "x", "", "Choose a reason."),
        ("Bravo", "rudeness", "x", "", "Choose a reason."),
        ("Bravo", "cheating", " \r\n ", "", "Describe what happened"),
        ("Bravo", "cheating", &long, "", "1000 characters"),
        (
            "Bravo",
            "cheating",
            "x",
            "last tuesday",
            "within the last year",
        ),
        (
            "Bravo",
            "cheating",
            "x",
            "2001-01-01T00:00",
            "within the last year",
        ),
    ] {
        let (s, page) = h.report(&alice, name, reason, description, when).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{name} {reason} {when}");
        assert!(page.contains(text), "{text}: {page}");
    }
    // A bad form token changes nothing.
    let r = h
        .post(
            "/reports/new",
            &alice,
            &[
                ("csrf", "nope"),
                ("name", "Bravo"),
                ("reason", "cheating"),
                ("description", "x"),
            ],
        )
        .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert_eq!(h.count("SELECT count(*) FROM player_reports").await, 0);

    // One open report per player: a second one is refused and does not count.
    let (s, _) = h.report(&alice, "Bravo", "cheating", "aimbot", "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, page) = h.report(&alice, "bravo", "griefing", "again", "").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(page.contains("already have an open report about Bravo"));
    // The form keeps what was typed.
    assert!(page.contains(">again</textarea>"));

    // Two a day here; the third is refused.
    let (s, _) = h.report(&alice, "Charlie", "other", "spam", "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, page) = h.report(&alice, "Delta", "other", "spam", "").await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(page.contains("2 reports in 24 hours"));
    assert_eq!(h.count("SELECT count(*) FROM player_reports").await, 2);

    // Older reports no longer count.
    sqlx::query("UPDATE player_reports SET created_at = now() - interval '25 hours'")
        .execute(&h.pool)
        .await
        .unwrap();
    let (s, _) = h.report(&alice, "Delta", "other", "spam", "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    test_db.close().await;
}

#[sqlx::test]
async fn name_suggestions_show_names_only(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    // The game server also has a character without a portal account, and a reserved name.
    let (game, admin) = fake_game(Some(&["bravo", "brawler", "legacy_one", "alice"])).await;
    let h = harness(pool, 5, Some(admin)).await;
    let alice = h.named("Alice").await;
    let bravo = h.named("Bravo").await;
    db::reserve_name(&h.pool, "Bramble", "existing player")
        .await
        .unwrap();

    let suggest = |q: &'static str| {
        let h = &h;
        let alice = &alice;
        async move {
            let (s, body) = h.get(&format!("/reports/players?q={q}"), Some(alice)).await;
            assert_eq!(s, StatusCode::OK, "{body}");
            assert!(
                !body.contains('@') && !body.contains("example.test"),
                "{body}"
            );
            let v: Value = serde_json::from_str(&body).unwrap();
            v["players"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };
    // Portal spellings first where they exist; prefix matches before the rest.
    assert_eq!(suggest("BRA").await, ["Bramble", "Bravo", "brawler"]);
    assert_eq!(suggest("ra").await, ["Bramble", "Bravo", "brawler"]);
    assert_eq!(suggest("legacy").await, ["legacy_one"]);
    // Never yourself; nothing for one letter or text that cannot be a name.
    assert!(suggest("ali").await.is_empty());
    assert!(suggest("b").await.is_empty());
    assert!(suggest("%25").await.is_empty());
    // The game server's list is fetched once and reused.
    assert_eq!(*game.calls.lock().unwrap(), 1);

    // Signed out: no names.
    let (s, body) = h.get("/reports/players?q=bra", None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("Bravo"));

    // Without the script, the search box lists names to pick.
    let (s, page) = h.get("/reports/new?q=bra", Some(&alice)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains("href=\"/reports/new?name=Bravo\""));
    assert!(page.contains("href=\"/reports/new?name=brawler\""));
    assert!(!page.contains("bravo@example.test"));

    // Characters only the game server has can be reported, without a portal account.
    let (s, _) = h
        .report(&alice, "legacy_one", "offensive_name", "name", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, _) = h
        .report(&alice, "Bramble", "cheating", "wallhack", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let rows: Vec<(String, String, Option<i32>)> = sqlx::query_as(
        "SELECT target_key, target_name, target_account FROM player_reports ORDER BY id",
    )
    .fetch_all(&h.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            ("legacy_one".into(), "legacy_one".into(), None),
            ("bramble".into(), "Bramble".into(), None)
        ]
    );

    // A game server that cannot answer: names it alone knows cannot be checked.
    *game.accounts.lock().unwrap() = None;
    let (game2, admin2) = fake_game(None).await;
    let h2 = harness(h.pool.clone(), 5, Some(admin2)).await;
    let (s, page) = h2.report(&alice, "brawler", "cheating", "x", "").await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(page.contains("cannot be reached"));
    // Portal names still work, and suggestions fall back to them.
    let (s, _) = h2.report(&alice, "Bravo", "cheating", "x", "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (_, body) = h2.get("/reports/players?q=bra", Some(&alice)).await;
    assert_eq!(body, r#"{"players":["Bramble","Bravo"]}"#);
    assert!(*game2.calls.lock().unwrap() >= 1);
    let _ = bravo;
    test_db.close().await;
}

#[sqlx::test]
async fn moderators_review_and_every_change_is_audited(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let (_, admin) = fake_game(Some(&["bravo"])).await;
    let h = harness(pool, 5, Some(admin)).await;
    let alice = h.named("Alice").await;
    let bravo = h.named("Bravo").await;
    let charlie = h.named("Charlie").await;
    h.grant("mod@example.test", "moderator").await;
    let moderator = h.person("mod", Some("Moddy")).await;

    let (s, _) = h
        .report(&alice, "Bravo", "griefing", "Team-killed me 6 times.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, _) = h
        .report(&charlie, "Bravo", "cheating", "Shot through walls.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let ids = h.report_ids().await;
    let first = ids[0];

    // Players cannot see the queue or change reports: for them it does not exist.
    for path in [
        "/admin/player-reports".to_owned(),
        format!("/admin/player-reports/{first}"),
    ] {
        let (s, _) = h.get(&path, Some(&alice)).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
    }
    let csrf_a = h.csrf(&alice).await;
    let r = h
        .post(
            &format!("/admin/player-reports/{first}/review"),
            &alice,
            &[("csrf", &csrf_a), ("action", "dismissed"), ("note", "x")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let (s, _) = h.get("/admin/player-reports", None).await;
    assert_eq!(s, StatusCode::SEE_OTHER);

    // Moderators: the queue, newest first, with the other report count and the reporter.
    let (s, page) = h.get("/admin/player-reports", Some(&moderator)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(page.contains("2 open"));
    let charlie_row = page.find("Shot through walls.").unwrap();
    let alice_row = page.find("Team-killed me 6 times.").unwrap();
    assert!(charlie_row < alice_row, "newest first");
    assert!(page.contains(&format!("href=\"/admin/users/{}\">Alice</a>", alice.id)));
    assert!(page.contains("<td>1 (1 open)</td>"));
    assert_eq!(support::page_times(&page), 2, "filed: {page}");
    let (_, filtered) = h
        .get("/admin/player-reports?reason=cheating", Some(&moderator))
        .await;
    assert!(filtered.contains("Shot through walls.") && !filtered.contains("Team-killed"));
    let (_, dashboard) = h.get("/admin", Some(&moderator)).await;
    assert!(dashboard.contains("href=\"/admin/player-reports\">2</a>"));

    // The report links to the character's tools and the user page.
    let (s, detail) = h
        .get(&format!("/admin/player-reports/{first}"), Some(&moderator))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(detail.contains("href=\"/admin/game/characters/bravo\""));
    assert!(detail.contains(&format!("href=\"/admin/users/{}\"", bravo.id)));
    assert!(
        detail.contains("Shot through walls."),
        "other reports listed"
    );

    // A note is required to close a report; the form token too.
    let csrf_m = h.csrf(&moderator).await;
    let review = |action: &'static str, note: &'static str, csrf: String| {
        let h = &h;
        let moderator = &moderator;
        async move {
            h.post(
                &format!("/admin/player-reports/{first}/review"),
                moderator,
                &[("csrf", &csrf), ("action", action), ("note", note)],
            )
            .await
            .status()
        }
    };
    assert_eq!(
        review("actioned", "  ", csrf_m.clone()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        review("banished", "x", csrf_m.clone()).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        review("actioned", "x", "bad".into()).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.count("SELECT count(*) FROM admin_log WHERE action LIKE '%player report%'")
            .await,
        0
    );

    assert_eq!(
        review("actioned", "Voice muted 7 days.", csrf_m.clone()).await,
        StatusCode::SEE_OTHER
    );
    let (_, detail) = h
        .get(&format!("/admin/player-reports/{first}"), Some(&moderator))
        .await;
    assert!(detail.contains("Voice muted 7 days.") && detail.contains("mod@example.test"));
    assert!(detail.contains("Reopen"));
    // Filed, happened and reviewed are times; the reviewer is text beside them.
    assert!(support::page_times(&detail) >= 3, "{detail}");
    assert!(
        detail.contains(" UTC</time> by mod@example.test</dd>"),
        "{detail}"
    );

    // The reporter sees "reviewed", never the note or the decision.
    let (_, mine) = h.get("/reports", Some(&alice)).await;
    assert!(mine.contains("<td>reviewed</td>"));
    assert!(!mine.contains("Voice muted") && !mine.contains("actioned"));
    assert!(!mine.contains("mod@example.test") && !mine.contains("Moddy"));

    assert_eq!(
        review("open", "", csrf_m.clone()).await,
        StatusCode::SEE_OTHER
    );
    let (_, mine) = h.get("/reports", Some(&alice)).await;
    assert!(mine.contains("<td>open</td>"));
    assert_eq!(
        review("dismissed", "Duplicate of a mute.", csrf_m.clone()).await,
        StatusCode::SEE_OTHER
    );
    // Asking for the status it already has changes nothing and logs nothing.
    assert_eq!(
        review("dismissed", "again", csrf_m.clone()).await,
        StatusCode::SEE_OTHER
    );

    let log: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT actor_email, action, target, detail FROM admin_log
         WHERE action LIKE '%player report' ORDER BY id",
    )
    .fetch_all(&h.pool)
    .await
    .unwrap();
    let target = format!("report {first} on Bravo");
    assert_eq!(
        log,
        [
            (
                "mod@example.test".to_owned(),
                "action player report".to_owned(),
                target.clone(),
                "Voice muted 7 days.".to_owned()
            ),
            (
                "mod@example.test".to_owned(),
                "reopen player report".to_owned(),
                target.clone(),
                String::new()
            ),
            (
                "mod@example.test".to_owned(),
                "dismiss player report".to_owned(),
                target,
                "Duplicate of a mute.".to_owned()
            ),
        ]
    );
    let (_, dismissed) = h
        .get("/admin/player-reports?status=dismissed", Some(&moderator))
        .await;
    assert!(dismissed.contains("Team-killed") && !dismissed.contains("Shot through walls."));

    // Reopening is refused while the reporter has a newer open report on the same player.
    let (s, _) = h
        .report(&alice, "Bravo", "griefing", "Again today.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(
        review("open", "", csrf_m.clone()).await,
        StatusCode::CONFLICT
    );
    test_db.close().await;
}

#[sqlx::test]
async fn reports_about_staff_go_above_them(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool, 5, None).await;
    let alice = h.named("Alice").await;
    h.grant("mod@example.test", "moderator").await;
    h.grant("mod2@example.test", "moderator").await;
    let moderator = h.person("mod", Some("Moddy")).await;
    let peer = h.person("mod2", Some("Peer")).await;
    let owner = h.person("owner", Some("Boss")).await;
    let (s, _) = h
        .report(&alice, "Moddy", "harassment", "Abusive.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let id = h.report_ids().await[0];
    let path = format!("/admin/player-reports/{id}/review");

    for staff in [&moderator, &peer] {
        let (_, page) = h
            .get(&format!("/admin/player-reports/{id}"), Some(staff))
            .await;
        assert!(page.contains("ranked at or above you"));
        let csrf = h.csrf(staff).await;
        let r = h
            .post(
                &path,
                staff,
                &[("csrf", &csrf), ("action", "dismissed"), ("note", "no")],
            )
            .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    let csrf = h.csrf(&owner).await;
    let r = h
        .post(
            &path,
            &owner,
            &[("csrf", &csrf), ("action", "actioned"), ("note", "Warned.")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    test_db.close().await;
}

#[sqlx::test]
async fn deleted_accounts_leave_anonymous_reports(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let h = harness(pool, 5, None).await;
    let alice = h.named("Alice").await;
    let bravo = h.named("Bravo").await;
    h.grant("mod@example.test", "moderator").await;
    let moderator = h.person("mod", Some("Moddy")).await;
    let (s, _) = h
        .report(&alice, "Bravo", "cheating", "Speed hack.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let id = h.report_ids().await[0];

    // The reporter leaves: the report stays, without them.
    let csrf = h.csrf(&alice).await;
    let r = h
        .post(
            "/account/delete",
            &alice,
            &[("csrf", &csrf), ("confirm", "Alice")],
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    // The reported player leaves: the report stays on the character, whose name stays reserved.
    assert!(db::delete_account(&h.pool, bravo.id).await.unwrap());
    let row: (Option<i32>, Option<i32>, String) = sqlx::query_as(
        "SELECT reporter_account, target_account, target_name FROM player_reports WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(row, (None, None, "Bravo".into()));
    let (_, page) = h
        .get(&format!("/admin/player-reports/{id}"), Some(&moderator))
        .await;
    assert!(page.contains("deleted account") && page.contains("Speed hack."));
    assert!(page.contains("no portal account holds this name"));
    // The reserved name can still be reported.
    let (s, _) = h
        .report(&moderator, "Bravo", "other", "Old name.", "")
        .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    test_db.close().await;
}
