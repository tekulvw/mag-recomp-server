//! Operator commands, run against the database (`kubectl exec … -- mag-portal admin …`).

use anyhow::bail;
use clap::Subcommand;
use sqlx::PgPool;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::db::{self, AssignOutcome, ReserveOutcome, ServiceCreateOutcome};
use crate::report_store::{ReportStore, StoreArgs};
use crate::{reports, staff};

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Show the account that holds a player name.
    Show { name: String },
    /// List accounts signed in with an email address.
    FindEmail { email: String },
    /// Stop an account from getting game tickets.
    Ban {
        name: String,
        #[arg(long)]
        reason: String,
    },
    /// Lift a ban.
    Unban { name: String },
    /// Reserve names nobody may claim, such as names that already have characters.
    Reserve {
        names: Vec<String>,
        #[arg(long, default_value = "existing player")]
        reason: String,
    },
    /// Release a reserved name so anyone may claim it.
    Release { name: String },
    /// Give a reserved name to an account that has no name yet.
    Assign {
        name: String,
        #[arg(long)]
        account_id: i32,
    },
    /// List reserved names.
    Reserved,
    /// Service accounts: player identities for automation, with tokens minted here.
    #[command(subcommand)]
    ServiceAccount(ServiceCommand),
    /// Launcher performance and error reports.
    #[command(subcommand)]
    Reports(ReportsCommand),
}

#[derive(Debug, Subcommand)]
pub enum ReportsCommand {
    /// Recent reports, newest first.
    List {
        /// Only this player's reports.
        #[arg(long)]
        account: Option<String>,
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(reports::TRIGGERS))]
        trigger: Option<String>,
        #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(reports::PLATFORMS))]
        platform: Option<String>,
        /// Only reports received at or after this RFC 3339 time.
        #[arg(long, conflicts_with = "days", value_parser = parse_rfc3339)]
        since: Option<OffsetDateTime>,
        /// Only reports from the last N days.
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=3650))]
        days: Option<i64>,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(i64).range(1..=10_000))]
        limit: i64,
    },
    /// A report's metadata and its client, session and metrics JSON.
    Show { report_id: String },
    /// Write one of a report's logs, decompressed, to stdout.
    Log { report_id: String, name: String },
    /// Reports per UTC day, trigger and platform.
    Stats {
        #[arg(long, default_value_t = 14, value_parser = clap::value_parser!(i64).range(1..=3650))]
        days: i64,
    },
    /// Delete a report and its logs.
    Delete { report_id: String },
}

fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|e| format!("not an RFC 3339 time: {e}"))
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Create a service account holding a free or reserved name (taking over the reservation).
    Create { name: String },
    /// Mint a launcher token for a service account and print it. It is shown only this once.
    MintToken {
        name: String,
        #[arg(long)]
        label: String,
    },
    /// List service accounts and their launcher tokens.
    List,
    /// Revoke one of a service account's launcher tokens.
    RevokeToken { name: String, device_id: i32 },
}

fn describe(a: &db::Account) -> String {
    format!(
        "account {} name={} email={} kind={} created={} {}",
        a.id,
        a.player_name.as_deref().unwrap_or("-"),
        a.email.as_deref().unwrap_or("-"),
        a.kind,
        a.created_at.date(),
        match (&a.banned_at, &a.ban_reason) {
            (Some(at), reason) =>
                format!("BANNED {} ({})", at.date(), reason.as_deref().unwrap_or("")),
            _ => String::new(),
        }
    )
    .trim_end()
    .to_owned()
}

pub async fn run(pool: &PgPool, store: &StoreArgs, command: AdminCommand) -> anyhow::Result<()> {
    match command {
        AdminCommand::Show { name } => match db::account_by_name(pool, &name).await? {
            Some(a) => println!("{}", describe(&a)),
            None => bail!("no account holds {name}"),
        },
        AdminCommand::FindEmail { email } => {
            let found = db::accounts_by_email(pool, &email).await?;
            if found.is_empty() {
                bail!("no account has {email}");
            }
            for a in found {
                println!("{}", describe(&a));
            }
        }
        AdminCommand::Ban { name, reason } => {
            if !db::set_ban(pool, &name, Some(&reason)).await? {
                bail!("no account holds {name}");
            }
            println!("banned {name}");
        }
        AdminCommand::Unban { name } => {
            if !db::set_ban(pool, &name, None).await? {
                bail!("no account holds {name}");
            }
            println!("unbanned {name}");
        }
        AdminCommand::Reserve { names, reason } => {
            let mut failed = false;
            for name in names {
                let outcome = db::reserve_name(pool, &name, &reason).await?;
                failed |= matches!(outcome, ReserveOutcome::Invalid | ReserveOutcome::InUse);
                println!("{name}: {outcome:?}");
            }
            if failed {
                bail!("some names were not reserved");
            }
        }
        AdminCommand::Release { name } => {
            if !db::release_name(pool, &name).await? {
                bail!("{name} is not reserved");
            }
            println!("released {name}");
        }
        AdminCommand::Assign { name, account_id } => {
            match db::assign_reserved_name(pool, &name, account_id).await? {
                AssignOutcome::Assigned => println!("assigned {name} to account {account_id}"),
                other => bail!("{name} not assigned: {other:?}"),
            }
        }
        AdminCommand::Reserved => {
            for r in db::reserved_names(pool).await? {
                println!("{}\t{}\t{}", r.name, r.created_at.date(), r.reason);
            }
        }
        AdminCommand::ServiceAccount(command) => {
            let mut out = std::io::stdout();
            run_service(pool, command, &mut out).await?;
        }
        AdminCommand::Reports(command) => {
            // Only reading and deleting logs needs the store.
            let store = match command {
                ReportsCommand::Log { .. } | ReportsCommand::Delete { .. } => Some(store.open()?),
                _ => None,
            };
            let mut out = std::io::stdout().lock();
            run_reports(pool, store.as_deref(), command, &mut out).await?;
        }
    }
    Ok(())
}

fn describe_service(a: &db::Account) -> String {
    format!(
        "account {} ({})",
        a.id,
        a.player_name.as_deref().unwrap_or("no name")
    )
}

async fn service_account(pool: &PgPool, name: &str) -> anyhow::Result<db::Account> {
    match db::service_account_by_name(pool, name).await? {
        Some(a) => Ok(a),
        None => bail!("no service account holds {name}"),
    }
}

/// Runs a service-account command, writing what it reports to `out`. Changes go to the
/// audit log under the command-line operator.
pub async fn run_service(
    pool: &PgPool,
    command: ServiceCommand,
    out: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    match command {
        ServiceCommand::Create { name } => match db::create_service_account(pool, &name).await? {
            ServiceCreateOutcome::Created {
                account_id,
                name,
                released,
            } => {
                let detail = if released {
                    "took over the reserved name"
                } else {
                    ""
                };
                staff::log_operator(
                    pool,
                    "create service account",
                    &format!("account {account_id} ({name})"),
                    detail,
                )
                .await?;
                writeln!(out, "created service account {account_id} ({name})")?;
            }
            ServiceCreateOutcome::Invalid => bail!("{name} is not a valid player name"),
            ServiceCreateOutcome::Taken => bail!("an account already holds {name}"),
        },
        ServiceCommand::MintToken { name, label } => {
            let Some(label) = db::service_token_label(&label) else {
                bail!("the label must be 1 to 64 printable ASCII characters");
            };
            let account = service_account(pool, &name).await?;
            let Some(minted) = db::mint_service_token(pool, account.id, &label).await? else {
                bail!("no service account holds {name}");
            };
            staff::log_operator(
                pool,
                "mint service token",
                &describe_service(&account),
                &format!("device {}: {label}", minted.device_id),
            )
            .await?;
            // The token alone on stdout, so scripts can capture it; it is never shown again.
            writeln!(out, "{}", minted.token)?;
            eprintln!(
                "minted token {} for {name}; store it now, it is not shown again",
                minted.device_id
            );
        }
        ServiceCommand::List => {
            for a in db::service_accounts(pool).await? {
                let banned = if a.is_banned() { "\tBANNED" } else { "" };
                writeln!(
                    out,
                    "{}\taccount {}{banned}",
                    a.player_name.as_deref().unwrap_or("-"),
                    a.id
                )?;
                for d in db::devices(pool, a.id).await? {
                    let used = d
                        .last_used_at
                        .map_or_else(|| "never".to_owned(), |t| t.date().to_string());
                    writeln!(
                        out,
                        "  device {}\t{}\tminted {}\tlast used {used}",
                        d.id,
                        d.label,
                        d.created_at.date()
                    )?;
                }
            }
        }
        ServiceCommand::RevokeToken { name, device_id } => {
            let account = service_account(pool, &name).await?;
            if !db::revoke_device(pool, account.id, device_id).await? {
                bail!("{name} has no token {device_id}");
            }
            staff::log_operator(
                pool,
                "revoke service token",
                &describe_service(&account),
                &format!("device {device_id}"),
            )
            .await?;
            writeln!(out, "revoked token {device_id} of {name}")?;
        }
    }
    Ok(())
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

fn needs_store(store: Option<&dyn ReportStore>) -> anyhow::Result<&dyn ReportStore> {
    store.ok_or_else(|| {
        anyhow::anyhow!("set PORTAL_REPORT_STORE and its settings to reach report logs")
    })
}

/// The JSON a report stored, indented for reading.
fn pretty(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| json.to_owned())
}

async fn report(pool: &PgPool, id: &str) -> anyhow::Result<reports::ReportDetail> {
    match reports::detail(pool, id).await? {
        Some(r) => Ok(r),
        None => bail!("no report {id}"),
    }
}

/// Runs a reports command, writing what it reports to `out`. `store` is needed by `log` and
/// `delete`; reading a log and deleting a report go to the audit log.
pub async fn run_reports(
    pool: &PgPool,
    store: Option<&dyn ReportStore>,
    command: ReportsCommand,
    out: &mut impl std::io::Write,
) -> anyhow::Result<()> {
    match command {
        ReportsCommand::List {
            account,
            trigger,
            platform,
            since,
            days,
            limit,
        } => {
            let since =
                since.or_else(|| days.map(|d| OffsetDateTime::now_utc() - time::Duration::days(d)));
            let filter = reports::Filter {
                account,
                trigger,
                platform,
                since,
            };
            for r in reports::list(pool, &filter, limit).await? {
                writeln!(
                    out,
                    "{}\t{}\t{} ({})\t{}\t{}\t{}\t{} logs, {} bytes",
                    r.id,
                    rfc3339(r.received_at),
                    r.player_name.as_deref().unwrap_or("-"),
                    r.account_id,
                    r.trigger,
                    r.platform,
                    r.client_version,
                    r.log_names.len(),
                    r.log_bytes
                )?;
            }
        }
        ReportsCommand::Show { report_id } => {
            let d = report(pool, &report_id).await?;
            let r = &d.row;
            writeln!(out, "report {}", r.id)?;
            writeln!(out, "received {}", rfc3339(r.received_at))?;
            writeln!(
                out,
                "account {} ({}, {})",
                r.account_id,
                r.player_name.as_deref().unwrap_or("no name"),
                r.account_kind
            )?;
            writeln!(out, "trigger {}", r.trigger)?;
            writeln!(out, "platform {} version {}", r.platform, r.client_version)?;
            writeln!(
                out,
                "logs {} ({} bytes compressed)",
                if r.log_names.is_empty() {
                    "none".to_owned()
                } else {
                    r.log_names.join(", ")
                },
                r.log_bytes
            )?;
            writeln!(out, "client {}", pretty(&d.client))?;
            for (label, json) in [("session", &d.session), ("metrics", &d.metrics)] {
                match json {
                    Some(j) => writeln!(out, "{label} {}", pretty(j))?,
                    None => writeln!(out, "{label} none")?,
                }
            }
        }
        ReportsCommand::Log { report_id, name } => {
            let store = needs_store(store)?;
            let d = report(pool, &report_id).await?;
            let Some(gz) = reports::log(store, &d.row, &name).await? else {
                bail!("report {report_id} has no log {name}");
            };
            let text = reports::decompress(&gz, reports::MAX_DECOMPRESSED)?;
            staff::log_operator(
                pool,
                "read report log",
                &format!("report {}", d.row.id),
                &name,
            )
            .await?;
            out.write_all(&text)?;
        }
        ReportsCommand::Stats { days } => {
            writeln!(out, "day\ttrigger\tplatform\tcount")?;
            for c in reports::stats(pool, days).await? {
                writeln!(out, "{}\t{}\t{}\t{}", c.day, c.trigger, c.platform, c.count)?;
            }
        }
        ReportsCommand::Delete { report_id } => {
            let store = needs_store(store)?;
            let d = report(pool, &report_id).await?;
            if !reports::delete(pool, store, &d.row.id).await? {
                bail!("no report {report_id}");
            }
            staff::log_operator(
                pool,
                "delete report",
                &format!("report {}", d.row.id),
                &format!(
                    "account {} ({})",
                    d.row.account_id,
                    d.row.player_name.as_deref().unwrap_or("no name")
                ),
            )
            .await?;
            writeln!(out, "deleted report {}", d.row.id)?;
        }
    }
    Ok(())
}
