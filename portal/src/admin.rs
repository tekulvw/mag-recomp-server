//! Operator commands, run against the database (`kubectl exec … -- mag-portal admin …`).

use anyhow::bail;
use clap::Subcommand;
use sqlx::PgPool;

use crate::db::{self, AssignOutcome, ReserveOutcome};

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
}

fn describe(a: &db::Account) -> String {
    format!(
        "account {} name={} email={} created={} {}",
        a.id,
        a.player_name.as_deref().unwrap_or("-"),
        a.email.as_deref().unwrap_or("-"),
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
    }
    Ok(())
}
