//! Public-name changes must never select a different character owner.
use mag_portal::db::{self, ClaimOutcome};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
mod support;

#[sqlx::test(migrations = false)]
async fn migration_backfills_people_and_services_without_moving_devices(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    for migration in db::MIGRATOR.iter().filter(|m| m.version < 9) {
        sqlx::raw_sql(migration.sql.clone())
            .execute(&pool)
            .await
            .unwrap();
    }
    let (person,): (i32,) = sqlx::query_as("INSERT INTO accounts(provider,subject,player_name) VALUES('google','before','Before') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO accounts(provider,subject,player_name,kind) VALUES('service','before-service','BeforeService','service')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO accounts(provider,subject) VALUES('google','unnamed')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices(account_id,token_hash,label) VALUES($1,'\\x0102','retained')")
        .bind(person)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO reserved_names(name_key,name,reason) VALUES('legacy','Legacy','imported or deleted owner')").execute(&pool).await.unwrap();
    sqlx::raw_sql(
        db::MIGRATOR
            .iter()
            .find(|m| m.version == 9)
            .unwrap()
            .sql
            .clone(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let mapped: Vec<(i32, String)> = sqlx::query_as("SELECT id,game_account_name FROM accounts WHERE player_name IS NOT NULL AND banned_at IS NULL ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0], (person, "before".to_owned()));
    assert_eq!(mapped[1].1, "beforeservice");
    assert_eq!(
        db::devices(&pool, person).await.unwrap()[0].label,
        "retained"
    );
    assert!(
        !db::release_name(&pool, "Legacy").await.unwrap(),
        "legacy reservations remain explicit recovery only"
    );
    test_db.close().await;
}

async fn named(pool: &sqlx::PgPool, name: &str) -> i32 {
    let a = db::upsert_identity(pool, "google", name, None)
        .await
        .unwrap();
    assert_eq!(
        db::claim_name(pool, a.id, name).await.unwrap(),
        ClaimOutcome::Claimed
    );
    a.id
}

#[sqlx::test]
async fn renamed_identity_retains_owner_and_old_names(_: PgPoolOptions, connect: PgConnectOptions) {
    use db::RenameOutcome::*;
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    let b = named(&pool, "Bravo").await;
    assert_eq!(
        db::rename_name(&pool, a, "Alpha", "NewAlpha")
            .await
            .unwrap(),
        Changed
    );
    let current = db::account(&pool, a).await.unwrap().unwrap();
    assert_eq!(current.player_name.as_deref(), Some("NewAlpha"));
    assert_eq!(current.game_account_name.as_deref(), Some("alpha"));
    assert_eq!(
        db::rename_name(&pool, a, "NewAlpha", "NewAlpha")
            .await
            .unwrap(),
        Unchanged
    );
    assert_eq!(
        db::rename_name(&pool, a, "Alpha", "NewAlpha")
            .await
            .unwrap(),
        Unchanged,
        "retrying the successful form is idempotent"
    );
    assert!(matches!(
        db::rename_name(&pool, a, "NewAlpha", "newalpha")
            .await
            .unwrap(),
        Cooldown(_)
    ));
    assert_eq!(
        db::rename_name(&pool, a, "Alpha", "Third").await.unwrap(),
        Stale
    );
    assert_eq!(
        db::rename_name(&pool, b, "Bravo", "Alpha").await.unwrap(),
        Reserved
    );
    assert!(!db::release_name(&pool, "Alpha").await.unwrap());
    assert_eq!(
        db::create_service_account(&pool, "Alpha").await.unwrap(),
        db::ServiceCreateOutcome::Taken
    );
    let unnamed = db::upsert_identity(&pool, "google", "unnamed", None)
        .await
        .unwrap();
    assert_eq!(
        db::claim_name(&pool, unnamed.id, "Alpha").await.unwrap(),
        ClaimOutcome::Reserved
    );
    assert_eq!(
        db::assign_reserved_name(&pool, "Alpha", unnamed.id)
            .await
            .unwrap(),
        db::AssignOutcome::Owned
    );
    sqlx::query("UPDATE accounts SET name_changed_at=now()-interval '24 hours' WHERE id=$1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        db::rename_name(&pool, a, "NewAlpha", "Alpha")
            .await
            .unwrap(),
        Changed
    );
    assert_eq!(
        db::rename_name(&pool, b, "Bravo", "NewAlpha")
            .await
            .unwrap(),
        Reserved
    );
    db::delete_account(&pool, a).await.unwrap();
    assert!(
        !db::release_name(&pool, "NewAlpha").await.unwrap(),
        "deleted owner history is recoverable only by explicit assignment"
    );
    assert_eq!(
        db::claim_name(&pool, unnamed.id, "NewAlpha").await.unwrap(),
        ClaimOutcome::Reserved
    );
    assert_eq!(
        db::assign_reserved_name(&pool, "NewAlpha", unnamed.id)
            .await
            .unwrap(),
        db::AssignOutcome::Assigned
    );
    assert_eq!(
        db::account(&pool, unnamed.id)
            .await
            .unwrap()
            .unwrap()
            .game_account_name
            .as_deref(),
        Some("alpha")
    );
    // Explicit recovery must move *all* historical reservations to the recovered account.
    assert!(!db::release_name(&pool, "Alpha").await.unwrap());
    test_db.close().await;
}

#[sqlx::test]
async fn cooldown_applies_to_case_changes_and_ends_at_24_hours(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    assert_eq!(
        db::rename_name(&pool, a, "Alpha", "ALPHA").await.unwrap(),
        db::RenameOutcome::Changed
    );
    assert!(matches!(
        db::rename_name(&pool, a, "ALPHA", "Other").await.unwrap(),
        db::RenameOutcome::Cooldown(_)
    ));
    sqlx::query(
        "UPDATE accounts SET name_changed_at=now()-interval '23 hours 59 minutes' WHERE id=$1",
    )
    .bind(a)
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        db::rename_name(&pool, a, "ALPHA", "Other").await.unwrap(),
        db::RenameOutcome::Cooldown(_)
    ));
    sqlx::query("UPDATE accounts SET name_changed_at=now()-interval '24 hours' WHERE id=$1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        db::rename_name(&pool, a, "ALPHA", "Other").await.unwrap(),
        db::RenameOutcome::Changed
    );
    test_db.close().await;
}

#[sqlx::test]
async fn competing_renames_have_one_winner(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    let b = named(&pool, "Bravo").await;
    let (left, right) = tokio::join!(
        db::rename_name(&pool, a, "Alpha", "Target"),
        db::rename_name(&pool, b, "Bravo", "Target")
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == db::RenameOutcome::Changed)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == db::RenameOutcome::Taken)
            .count(),
        1
    );
    test_db.close().await;
}

#[sqlx::test]
async fn invalid_banned_service_and_missing_accounts_cannot_rename(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    for name in ["ab", "9bad", "Bad Name", "abcdefghijklmnop", "näme"] {
        assert_eq!(
            db::rename_name(&pool, a, "Alpha", name).await.unwrap(),
            db::RenameOutcome::Invalid
        );
    }
    db::set_ban(&pool, "Alpha", Some("synthetic ban"))
        .await
        .unwrap();
    assert_eq!(
        db::rename_name(&pool, a, "Alpha", "Other").await.unwrap(),
        db::RenameOutcome::Unavailable
    );
    let db::ServiceCreateOutcome::Created { account_id, .. } =
        db::create_service_account(&pool, "Service").await.unwrap()
    else {
        panic!("service must exist")
    };
    assert_eq!(
        db::rename_name(&pool, account_id, "Service", "Other")
            .await
            .unwrap(),
        db::RenameOutcome::Unavailable
    );
    assert_eq!(
        db::rename_name(&pool, i32::MAX, "Missing", "Other")
            .await
            .unwrap(),
        db::RenameOutcome::Unavailable
    );
    assert_eq!(
        db::account(&pool, a)
            .await
            .unwrap()
            .unwrap()
            .player_name
            .as_deref(),
        Some("Alpha")
    );
    test_db.close().await;
}

#[sqlx::test]
async fn report_labels_change_without_changing_the_reported_owner(
    _: PgPoolOptions,
    connect: PgConnectOptions,
) {
    use mag_portal::player_reports::{self, NewReport, Reason, Resolved, SubmitOutcome};
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    let reporter = named(&pool, "Reporter").await;
    db::rename_name(&pool, a, "Alpha", "NewAlpha")
        .await
        .unwrap();
    let Resolved::Found(target) = player_reports::resolve(&pool, None, "NewAlpha")
        .await
        .unwrap()
    else {
        panic!("renamed account must resolve")
    };
    assert_eq!(
        (target.key.as_str(), target.name.as_str(), target.account),
        ("alpha", "NewAlpha", Some(a))
    );
    let SubmitOutcome::Filed(id) = player_reports::submit(
        &pool,
        &NewReport {
            reporter,
            target: &target,
            reason: Reason::Other,
            description: "synthetic",
            occurred_at: None,
        },
        10,
    )
    .await
    .unwrap() else {
        panic!("report must file")
    };
    let Resolved::Found(former) = player_reports::resolve(&pool, None, "Alpha").await.unwrap()
    else {
        panic!("former name must resolve")
    };
    assert_eq!((former.key.as_str(), former.account), ("alpha", Some(a)));
    assert_eq!(
        player_reports::submit(
            &pool,
            &NewReport {
                reporter,
                target: &former,
                reason: Reason::Other,
                description: "same owner",
                occurred_at: None
            },
            10
        )
        .await
        .unwrap(),
        SubmitOutcome::Duplicate
    );
    sqlx::query("UPDATE accounts SET name_changed_at=now()-interval '24 hours' WHERE id=$1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    db::rename_name(&pool, a, "NewAlpha", "ThirdName")
        .await
        .unwrap();
    let saved = player_reports::report(&pool, id).await.unwrap().unwrap();
    assert_eq!(
        (saved.target_key.as_str(), saved.target_name.as_str()),
        ("alpha", "NewAlpha")
    );
    assert_eq!(
        player_reports::suggest(&pool, None, "Thi", Some("alpha"))
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    test_db.close().await;
}

#[sqlx::test]
async fn rename_preserves_social_and_device_identity(_: PgPoolOptions, connect: PgConnectOptions) {
    use mag_portal::social;
    let (pool, test_db) = support::pool(connect).await;
    let a = named(&pool, "Alpha").await;
    let b = named(&pool, "Bravo").await;
    social::request(&pool, a, b).await.unwrap();
    social::accept(&pool, b, a).await.unwrap();
    social::set_presence(&pool, a, b"synthetic-presence")
        .await
        .unwrap();
    let token = "mgd_synthetic_launcher";
    sqlx::query(
        "INSERT INTO devices(account_id,token_hash,label) VALUES($1,$2,'synthetic launcher')",
    )
    .bind(a)
    .bind(mag_portal::tokens::sha256(token.as_bytes()).as_slice())
    .execute(&pool)
    .await
    .unwrap();
    db::rename_name(&pool, a, "Alpha", "NewAlpha")
        .await
        .unwrap();
    let account = db::device_account(&pool, token).await.unwrap().unwrap();
    assert_eq!(
        (
            account.id,
            account.game_account_name.as_deref(),
            account.player_name.as_deref()
        ),
        (a, Some("alpha"), Some("NewAlpha"))
    );
    assert_eq!(
        social::find(&pool, "NewAlpha").await.unwrap().unwrap().id,
        a
    );
    let list = social::overview(&pool, b).await.unwrap();
    assert_eq!(list.friends.len(), 1);
    assert_eq!(list.friends[0].name, "NewAlpha");
    test_db.close().await;
}

#[sqlx::test]
async fn first_claim_keeps_an_immutable_game_owner(_: PgPoolOptions, connect: PgConnectOptions) {
    let (pool, test_db) = support::pool(connect).await;
    let account = db::upsert_identity(&pool, "google", "owner", None)
        .await
        .unwrap();
    assert_eq!(
        db::claim_name(&pool, account.id, "Alpha").await.unwrap(),
        ClaimOutcome::Claimed
    );
    let (owner,): (String,) = sqlx::query_as("SELECT game_account_name FROM accounts WHERE id=$1")
        .bind(account.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, "alpha");
    assert!(
        sqlx::query("UPDATE accounts SET game_account_name='someone_else' WHERE id=$1")
            .bind(account.id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE accounts SET game_account_name=NULL WHERE id=$1")
            .bind(account.id)
            .execute(&pool)
            .await
            .is_err()
    );
    test_db.close().await;
}
