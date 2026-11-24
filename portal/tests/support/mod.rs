//! Database pools for the integration tests.
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
