use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Args, Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

use mag_portal::config::{self, Settings, TicketIssuer};
use mag_portal::game_admin::GameAdmin;
use mag_portal::oidc::{self, OidcClient, ProviderConfig};
use mag_portal::report_store::StoreArgs;
use mag_portal::{admin, db, reports, status, web};

#[derive(Parser)]
#[command(
    name = "mag-portal",
    version,
    about = "Player accounts portal for the MAG replacement server"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the website and launcher API.
    Serve(Box<ServeArgs>),
    /// Apply database migrations and exit.
    Migrate(DatabaseArgs),
    /// Write a new ticket signing key (PKCS#8 PEM) and print its public key.
    Keygen {
        /// Where to write the private key; must not exist yet.
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the public key of a ticket signing key, for game server configuration.
    PublicKey {
        #[arg(long, env = "PORTAL_TICKET_KEY_FILE")]
        key_file: PathBuf,
    },
    /// Operator commands.
    Admin {
        #[command(flatten)]
        database: DatabaseArgs,
        /// The report store, for `reports log` and `reports delete`.
        #[command(flatten)]
        store: Box<StoreArgs>,
        #[command(subcommand)]
        command: admin::AdminCommand,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Switch {
    On,
    Off,
}

#[derive(Args)]
struct DatabaseArgs {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    /// Seconds a request waits for a database connection before it answers 503 busy.
    #[arg(long, env = "PORTAL_DB_ACQUIRE_TIMEOUT_SECONDS", default_value_t = 3,
          value_parser = clap::value_parser!(u64).range(1..=60))]
    db_acquire_timeout_seconds: u64,
}

#[derive(Args)]
struct ServeArgs {
    #[command(flatten)]
    database: DatabaseArgs,
    #[arg(long, env = "PORTAL_LISTEN", default_value = "0.0.0.0:8080")]
    listen: String,
    /// The site's public origin, such as https://magtest.online.
    #[arg(long, env = "PORTAL_PUBLIC_URL")]
    public_url: String,
    /// Header a trusted proxy appends the client address to, such as x-forwarded-for.
    /// Without it, rate limits use the connecting address.
    #[arg(long, env = "PORTAL_CLIENT_IP_HEADER")]
    client_ip_header: Option<String>,
    /// Comma-separated Google emails with the owner role (full admin access, not removable).
    /// Folder holding the fonts.txt web fonts (the image sets it).
    #[arg(long, env = "PORTAL_FONT_DIR")]
    font_dir: Option<PathBuf>,
    #[arg(long, env = "PORTAL_OWNER_EMAILS", default_value = "")]
    owner_emails: String,
    /// The game server's internal admin API origin, such as http://mag-medius-admin:10490.
    /// Set with --game-admin-secret-file, or leave both unset.
    #[arg(long, env = "PORTAL_GAME_ADMIN_URL")]
    game_admin_url: Option<String>,
    /// File holding the game admin API's shared secret (at least 32 bytes).
    #[arg(long, env = "PORTAL_GAME_ADMIN_SECRET_FILE")]
    game_admin_secret_file: Option<PathBuf>,
    /// Seconds between the status page's polls of each game server.
    #[arg(long, env = "PORTAL_STATUS_REFRESH_SECONDS", default_value_t = mag_portal::status::DEFAULT_REFRESH_SECONDS,
          value_parser = clap::value_parser!(u64).range(5..=300))]
    status_refresh_seconds: u64,
    #[arg(long, env = "PORTAL_OPERATOR_CONTACT")]
    operator_contact: Option<String>,
    #[arg(long, env = "PORTAL_GOOGLE_CLIENT_ID")]
    google_client_id: String,
    #[arg(long, env = "PORTAL_GOOGLE_CLIENT_SECRET_FILE")]
    google_client_secret_file: PathBuf,
    #[arg(long, env = "PORTAL_TICKET_KEY_FILE")]
    ticket_key_file: PathBuf,
    #[arg(long, env = "PORTAL_TICKET_KEY_ID", default_value_t = 1)]
    ticket_key_id: u8,
    /// The game server host name tickets are for.
    #[arg(long, env = "PORTAL_TICKET_AUDIENCE")]
    ticket_audience: String,
    /// More game server host names, comma-separated, a launcher may ask tickets for.
    #[arg(long, env = "PORTAL_TICKET_EXTRA_AUDIENCES", default_value = "")]
    ticket_extra_audiences: String,
    #[arg(long, env = "PORTAL_TICKET_LIFETIME_SECONDS", default_value_t = 600)]
    ticket_lifetime_seconds: u64,
    /// Accept launcher reports (`on`) or answer 503 reports_disabled (`off`).
    #[arg(long, env = "PORTAL_REPORTS", value_enum, default_value = "on")]
    reports: Switch,
    /// Days reports (rows and logs) are kept.
    #[arg(long, env = "PORTAL_REPORT_RETENTION_DAYS", default_value_t = 30,
          value_parser = clap::value_parser!(i64).range(1..=3650))]
    report_retention_days: i64,
    #[command(flatten)]
    report_store: StoreArgs,
    /// Player reports one account may file in 24 hours (docs/portal-player-reports.md).
    #[arg(long, env = "PORTAL_PLAYER_REPORTS_PER_DAY", default_value_t = mag_portal::player_reports::DEFAULT_PER_DAY,
          value_parser = clap::value_parser!(i64).range(1..=1000))]
    player_reports_per_day: i64,
    /// Allow name changes only after upgrading every game-ticket audience's owner lookup.
    #[arg(long, env = "PORTAL_PLAYER_NAME_CHANGES", value_parser = clap::builder::BoolishValueParser::new())]
    player_name_changes: bool,
    /// Skip migrations at start (run `mag-portal migrate` separately).
    #[arg(long, env = "PORTAL_SKIP_MIGRATIONS")]
    skip_migrations: bool,
    #[arg(long, env = "PORTAL_OIDC_AUTH_URL", default_value = oidc::GOOGLE_AUTH_URL, hide = true)]
    oidc_auth_url: String,
    #[arg(long, env = "PORTAL_OIDC_TOKEN_URL", default_value = oidc::GOOGLE_TOKEN_URL, hide = true)]
    oidc_token_url: String,
    #[arg(long, env = "PORTAL_OIDC_JWKS_URL", default_value = oidc::GOOGLE_JWKS_URL, hide = true)]
    oidc_jwks_url: String,
}

async fn connect(database: &DatabaseArgs) -> anyhow::Result<sqlx::PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(database.db_acquire_timeout_seconds))
        .connect(&database.database_url)
        .await
        .context("connecting to the database")
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let mut settings = Settings::new(&args.public_url)?;
    settings.operator_contact = args.operator_contact.filter(|c| !c.trim().is_empty());
    settings.owner_emails = mag_portal::staff::parse_owner_emails(&args.owner_emails);
    settings.font_dir = args.font_dir;
    settings.player_reports_per_day = args.player_reports_per_day;
    settings.player_name_changes = args.player_name_changes;
    settings.client_ip_header = args
        .client_ip_header
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty());
    settings.game_admin = GameAdmin::from_config(
        args.game_admin_url.as_deref(),
        args.game_admin_secret_file.as_deref(),
    )?;
    match &settings.game_admin {
        Some(api) => tracing::info!(url = api.base_url(), "game server admin API configured"),
        None => tracing::info!("game server admin API not configured"),
    }
    // PORTAL_STATUS_SERVERS and each server's PORTAL_STATUS_<ID>_* settings (docs/portal-status.md).
    let status_servers = status::servers_from_env(settings.game_admin.as_ref(), |name| {
        std::env::var(name).ok()
    })?;
    for server in &status_servers {
        tracing::info!(
            server = %server.id,
            url = server.admin.base_url(),
            staff_only = server.staff_only,
            "status page server configured"
        );
    }
    let status_service = Arc::new(status::StatusService::new(
        status_servers,
        Duration::from_secs(args.status_refresh_seconds),
    ));
    let tickets = TicketIssuer::new(
        config::load_signing_key(&args.ticket_key_file)?,
        args.ticket_key_id,
        &args.ticket_audience,
        args.ticket_lifetime_seconds,
    )?
    .with_extra_audiences(&config::parse_audiences(&args.ticket_extra_audiences)?)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let oidc = OidcClient::new(
        ProviderConfig {
            provider: "google".into(),
            client_id: args.google_client_id,
            client_secret: config::read_secret_file(&args.google_client_secret_file)?,
            auth_url: args.oidc_auth_url,
            token_url: args.oidc_token_url,
            jwks_url: args.oidc_jwks_url,
            issuers: oidc::GOOGLE_ISSUERS.iter().map(|s| s.to_string()).collect(),
            redirect_uri: format!("{}/auth/google/callback", settings.public_url),
        },
        http,
    );
    let report_service = match args.reports {
        Switch::Off => {
            tracing::info!("launcher reports off");
            None
        }
        Switch::On => {
            let store = args
                .report_store
                .open()
                .context("launcher reports are on (PORTAL_REPORTS)")?;
            store
                .check()
                .await
                .context("launcher reports are on (PORTAL_REPORTS)")?;
            tracing::info!(
                store = store.describe(),
                days = args.report_retention_days,
                "launcher reports on"
            );
            Some(Arc::new(reports::Reports::new(
                store,
                args.report_retention_days,
            )))
        }
    };
    let pool = connect(&args.database).await?;
    if !args.skip_migrations {
        db::MIGRATOR
            .run(&pool)
            .await
            .context("applying migrations")?;
    }
    let cleanup_pool = pool.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(600));
        loop {
            every.tick().await;
            match db::cleanup(&cleanup_pool).await {
                Ok(n) if n > 0 => tracing::info!(removed = n, "expired rows removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "cleanup failed"),
            }
        }
    });
    if let Some(service) = &report_service {
        reports::spawn_purge(pool.clone(), service.clone());
    }
    // Polls the game servers and reloads the banner now and every refresh interval.
    status_service.spawn(pool.clone());
    let state = Arc::new(web::AppState {
        pool,
        oidc,
        tickets,
        settings,
        limits: web::Limits::default(),
        reports: report_service,
        status: status_service,
    });
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .with_context(|| format!("listening on {}", args.listen))?;
    tracing::info!(address = %args.listen, "serving");
    axum::serve(
        listener,
        web::router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Serve(args) => serve(*args).await,
        Command::Migrate(database) => {
            let pool = connect(&database).await?;
            db::MIGRATOR.run(&pool).await?;
            println!("migrations applied");
            Ok(())
        }
        Command::Keygen { out } => {
            let (pem, public) = config::generate_signing_key_pem()?;
            write_private(&out, &pem)?;
            println!("wrote {}", out.display());
            println!("public key (SPKI, base64): {}", STANDARD.encode(public));
            Ok(())
        }
        Command::PublicKey { key_file } => {
            let key = config::load_signing_key(&key_file)?;
            println!("{}", STANDARD.encode(config::public_key_der(&key)));
            Ok(())
        }
        Command::Admin {
            database,
            store,
            command,
        } => {
            let pool = connect(&database).await?;
            admin::run(&pool, &store, command).await
        }
    }
}

fn write_private(path: &std::path::Path, contents: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}
