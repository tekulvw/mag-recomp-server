//! Database pools for the integration tests, and a check of the times a page shows.
//!
//! `#[sqlx::test]` creates, migrates and (when the test passes) drops one database per test.
//! The pool it would hand a test borrows its connection permits from a single 20-connection
//! pool shared by the whole test binary, keeps every permit it has borrowed until the test's
//! runtime shuts down, and gives up after the default 30 s acquire timeout. With up to five
//! permits held per test and one test thread per CPU, tests queue behind each other for those
//! permits; on a loaded machine the queue outlasts the timeout and a random test fails with
//! `failed to connect test pool: PoolTimedOut`.
//!
//! Tests therefore take the connect options instead (`#[sqlx::test]` still manages the
//! database) and open a pool of their own through [`pool`]. A test first waits for one of
//! [`SLOTS`] slots, outside any timeout, so the number of PostgreSQL connections stays bounded
//! however many test threads run, and its own acquires never wait on other tests.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::{Semaphore, SemaphorePermit};

/// Tests holding a pool at the same time. With [`MAX_CONNECTIONS`] each, plus the 20
/// connections `#[sqlx::test]` uses to create and drop databases, this stays well under
/// PostgreSQL's default `max_connections` (100).
pub const SLOTS: usize = 8;
/// Connections one test may hold at once (the portal itself runs with 10).
const MAX_CONNECTIONS: u32 = 5;
/// How long one acquire may take: generous, because CI shares its runner with large builds.
/// Only this test's own connections count, so a timeout still points at a real stall.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(60);

static TEST_SLOTS: Semaphore = Semaphore::const_new(SLOTS);

/// A test's claim on its database. Call [`TestDb::close`] at the end of the test.
pub struct TestDb {
    pool: PgPool,
    slot: Option<SemaphorePermit<'static>>,
}

/// Waits for a slot, then connects a pool to the test's database.
pub async fn pool(connect: PgConnectOptions) -> (PgPool, TestDb) {
    let slot = TEST_SLOTS.acquire().await.expect("never closed");
    let pool = PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(connect)
        .await
        .expect("failed to connect test pool");
    (
        pool.clone(),
        TestDb {
            pool,
            slot: Some(slot),
        },
    )
}

impl TestDb {
    /// Closes every connection, so `#[sqlx::test]` can drop the database, and frees the slot.
    pub async fn close(mut self) {
        tokio::time::timeout(ACQUIRE_TIMEOUT, self.pool.close())
            .await
            .expect("the test still holds a database connection");
        self.slot = None;
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // A test that ends without closing would leave its database behind.
        if self.slot.is_some() && !std::thread::panicking() {
            panic!("end the test with `test_db.close().await`");
        }
    }
}

/// Checks the times on a page and returns how many `<time>` elements it has. Every `<time>`
/// carries a machine-readable `datetime` in UTC: an instant (`2026-11-28T05:25:47Z`), which the
/// time script shows in the viewer's zone and whose fallback text is labelled UTC, or a calendar
/// day (`2026-11-28`). Outside them, no date and time shows without a zone (`Z` or ` UTC`).
#[allow(dead_code)]
pub fn page_times(html: &str) -> usize {
    let mut found = 0;
    let mut text = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<time") {
        text.push_str(&rest[..start]);
        let tag_end = start + rest[start..].find('>').expect("an unclosed <time> tag");
        let tag = &rest[start..tag_end];
        let close = tag_end
            + rest[tag_end..]
                .find("</time>")
                .expect("a <time> without </time>");
        let inner = &rest[tag_end + 1..close];
        let datetime = tag
            .split_once("datetime=\"")
            .and_then(|(_, v)| v.split_once('"'))
            .map(|(v, _)| v)
            .unwrap_or_else(|| panic!("a <time> without datetime: {tag}"));
        match datetime.len() {
            20 => {
                assert!(
                    is_date_time(datetime.as_bytes()) && datetime.ends_with('Z'),
                    "not an ISO UTC instant: {tag}"
                );
                assert!(
                    tag.contains("class=\"local-time\"") && inner.ends_with(" UTC"),
                    "an instant must be local-time and labelled UTC: {tag}>{inner}"
                );
            }
            10 => assert!(is_date(datetime.as_bytes()), "not a day: {tag}"),
            _ => panic!("not an ISO UTC datetime: {tag}"),
        }
        found += 1;
        rest = &rest[close + "</time>".len()..];
    }
    text.push_str(rest);
    // What a reader sees outside the <time> elements: no tags (their attributes are data).
    let mut visible = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                visible.push(' ');
            }
            _ if !in_tag => visible.push(c),
            _ => {}
        }
    }
    let b = visible.as_bytes();
    for i in 0..b.len() {
        if b.len() - i >= 16 && is_date_time(&b[i..i + 16]) {
            let mut end = i + 16;
            if b.get(end) == Some(&b':') && b.len() >= end + 3 {
                end += 3;
            }
            if b.get(end) == Some(&b'.') {
                end += 1;
                while b.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
            let labelled = b.get(end) == Some(&b'Z') || b[end..].starts_with(b" UTC");
            assert!(
                labelled,
                "a date and time without a zone: {:?}",
                &visible[i.saturating_sub(40)..(end + 20).min(visible.len())]
            );
        }
    }
    found
}

fn is_date(b: &[u8]) -> bool {
    b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// `2026-11-28T05:25` or `2026-11-28 05:25` at the start of `b` (and, for 20 bytes, `:47Z`).
fn is_date_time(b: &[u8]) -> bool {
    let base = b.len() >= 16
        && is_date(b)
        && (b[10] == b'T' || b[10] == b' ')
        && b[11..13].iter().all(u8::is_ascii_digit)
        && b[13] == b':'
        && b[14..16].iter().all(u8::is_ascii_digit);
    base && (b.len() != 20 || (b[16] == b':' && b[17..19].iter().all(u8::is_ascii_digit)))
}
