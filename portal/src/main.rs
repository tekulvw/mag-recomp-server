use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Args, Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;

use mag_portal::config::{self, Settings, TicketIssuer};
use mag_portal::oidc::{self, OidcClient, ProviderConfig};
use mag_portal::{admin, db, web};

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
        #[command(subcommand)]
        command: admin::AdminCommand,
    },
}

#[derive(Args)]
struct DatabaseArgs {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
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
    #[arg(long, env = "PORTAL_TICKET_LIFETIME_SECONDS", default_value_t = 600)]
    ticket_lifetime_seconds: u64,
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
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database.database_url)
        .await
        .context("connecting to the database")
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    let mut settings = Settings::new(&args.public_url)?;
    settings.operator_contact = args.operator_contact.filter(|c| !c.trim().is_empty());
    settings.client_ip_header = args
        .client_ip_header
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty());
    let tickets = TicketIssuer::new(
        config::load_signing_key(&args.ticket_key_file)?,
        args.ticket_key_id,
        &args.ticket_audience,
        args.ticket_lifetime_seconds,
    )?;
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
    let state = Arc::new(web::AppState {
        pool,
        oidc,
        tickets,
        settings,
        limits: web::Limits::default(),
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
        Command::Admin { database, command } => {
            let pool = connect(&database).await?;
            admin::run(&pool, command).await
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
