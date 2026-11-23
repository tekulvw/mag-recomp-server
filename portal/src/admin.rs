//! Operator commands, run against the database (`kubectl exec … -- mag-portal admin …`).

use anyhow::bail;
use clap::Subcommand;
use sqlx::PgPool;

use crate::db::{self, AssignOutcome, ReserveOutcome, ServiceCreateOutcome};
use crate::staff;

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

pub async fn run(pool: &PgPool, command: AdminCommand) -> anyhow::Result<()> {
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
