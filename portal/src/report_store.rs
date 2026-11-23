//! Where report logs live: an S3-compatible bucket in production, or a local folder for tests
//! and development. Logs never go into PostgreSQL.
//!
//! Every log has a key `<yyyy-mm-dd>/<report id>/<name>.gz` (the UTC date the report arrived),
//! built only from values the API has validated, so a key can never leave its store. The S3
//! store prefixes keys with `reports/`.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use reqwest::{StatusCode, Url};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use sqlx::PgPool;
use time::OffsetDateTime;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// The key of one log of a report.
pub fn log_key(received_at: OffsetDateTime, report_id: &str, name: &str) -> String {
    format!("{}/{report_id}/{name}.gz", day(received_at))
}

/// The UTC date of `t` as `yyyy-mm-dd`.
pub fn day(t: OffsetDateTime) -> String {
    let d = t.to_offset(time::UtcOffset::UTC).date();
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

/// Keys are `<date>/<id>/<name>.gz` from validated parts; anything else is refused outright.
fn safe_key(key: &str) -> anyhow::Result<()> {
    let ok = !key.is_empty()
        && key.len() <= 200
        && key.split('/').count() == 3
        && key.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        });
    if !ok {
        bail!("refusing report store key {key:?}");
    }
    Ok(())
}

/// A place to keep report logs. Writes are durable once `put` returns.
pub trait ReportStore: Send + Sync {
    /// A description for startup logs; never includes credentials.
    fn describe(&self) -> String;
    /// Fails, with a message fit for the operator, when the store cannot be used.
    fn check(&self) -> BoxFut<'_, ()>;
    fn put<'a>(&'a self, key: &'a str, data: Vec<u8>) -> BoxFut<'a, ()>;
    /// `None` when there is no such log.
    fn get<'a>(&'a self, key: &'a str) -> BoxFut<'a, Option<Vec<u8>>>;
    /// Removes a log; a missing one is not an error.
    fn delete<'a>(&'a self, key: &'a str) -> BoxFut<'a, ()>;
    /// Removes logs no report row refers to (failed writes, deleted accounts) and anything
    /// dated before `cutoff`, returning how many report folders went. Stores that cannot list
    /// their contents return 0 and rely on their own expiry (the S3 bucket's lifecycle rule).
    fn sweep_orphans<'a>(&'a self, pool: &'a PgPool, cutoff: OffsetDateTime) -> BoxFut<'a, u64> {
        let _ = (pool, cutoff);
        Box::pin(async { Ok(0) })
    }
}

// ---- Folder store ----

/// Logs as files under a private folder: `<dir>/<key>`, folders 0700 and files 0600, each
/// written to a temporary file, flushed and renamed into place.
pub struct DirStore {
    root: PathBuf,
}

/// Leftovers younger than this may belong to a report being written, so sweeps keep them.
const SWEEP_GRACE: Duration = Duration::from_secs(3600);

impl DirStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> anyhow::Result<PathBuf> {
        safe_key(key)?;
        Ok(self.root.join(key))
    }
}

fn private_dirs(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().expect("keys have a folder");
    private_dirs(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.tmp-{}",
        crate::tokens::b64url(&crate::tokens::random_bytes::<6>())
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        // The rename is durable once the folder is.
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn is_day(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn older_than(meta: &std::fs::Metadata, age: Duration) -> bool {
    meta.modified()
        .ok()
        .and_then(|m| m.elapsed().ok())
        .is_some_and(|e| e >= age)
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .context("report store task")?
}

impl ReportStore for DirStore {
    fn describe(&self) -> String {
        format!("folder {}", self.root.display())
    }

    fn check(&self) -> BoxFut<'_, ()> {
        let root = self.root.clone();
        Box::pin(blocking(move || {
            let meta = std::fs::metadata(&root).with_context(|| {
                format!(
                    "the report folder {} (PORTAL_REPORT_DIR) is missing",
                    root.display()
                )
            })?;
            if !meta.is_dir() {
                bail!("the report folder {} is not a folder", root.display());
            }
            let probe = root.join(format!(
                ".write-check-{}",
                crate::tokens::b64url(&crate::tokens::random_bytes::<6>())
            ));
            std::fs::write(&probe, b"ok")
                .and_then(|()| std::fs::remove_file(&probe))
                .with_context(|| format!("the report folder {} is not writable", root.display()))?;
            Ok(())
        }))
    }

    fn put<'a>(&'a self, key: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let path = self.path(key)?;
            blocking(move || {
                write_atomically(&path, &data)
                    .with_context(|| format!("writing {}", path.display()))
            })
            .await
        })
    }

    fn get<'a>(&'a self, key: &'a str) -> BoxFut<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let path = self.path(key)?;
            match tokio::fs::read(&path).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
            }
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let path = self.path(key)?;
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
            }
            // Drop the report's folder, then the day's, once they are empty.
            let report_dir = path.parent().expect("keys have a folder");
            if tokio::fs::remove_dir(report_dir).await.is_ok()
                && let Some(day_dir) = report_dir.parent()
            {
                let _ = tokio::fs::remove_dir(day_dir).await;
            }
            Ok(())
        })
    }

    fn sweep_orphans<'a>(&'a self, pool: &'a PgPool, cutoff: OffsetDateTime) -> BoxFut<'a, u64> {
        Box::pin(async move {
            let root = self.root.clone();
            let cutoff_day = day(cutoff);
            // Days wholly before the cutoff go entirely; other days' report folders are
            // candidates, kept if a row still refers to them.
            let (mut removed, candidates) = blocking(move || {
                let mut removed = 0u64;
                let mut candidates: Vec<(PathBuf, String)> = Vec::new();
                for entry in std::fs::read_dir(&root)? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let meta = entry.metadata()?;
                    if name.starts_with(".write-check-") && older_than(&meta, SWEEP_GRACE) {
                        let _ = std::fs::remove_file(entry.path());
                        continue;
                    }
                    if !is_day(&name) || !meta.is_dir() {
                        continue;
                    }
                    if name < cutoff_day {
                        removed += std::fs::read_dir(entry.path())?.count() as u64;
                        std::fs::remove_dir_all(entry.path())?;
                        continue;
                    }
                    for report in std::fs::read_dir(entry.path())? {
                        let report = report?;
                        let id = report.file_name().to_string_lossy().into_owned();
                        if crate::reports::valid_report_id(&id).is_some()
                            && older_than(&report.metadata()?, SWEEP_GRACE)
                        {
                            candidates.push((report.path(), id));
                        }
                    }
                }
                Ok((removed, candidates))
            })
            .await?;
            if candidates.is_empty() {
                return Ok(removed);
            }
            let ids: Vec<String> = candidates.iter().map(|(_, id)| id.clone()).collect();
            let known: HashSet<String> = sqlx::query_as::<_, (String,)>(
                "SELECT id::text FROM reports WHERE id = ANY($1::uuid[])",
            )
            .bind(&ids)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(id,)| id)
            .collect();
            let orphans: Vec<PathBuf> = candidates
                .into_iter()
                .filter(|(_, id)| !known.contains(id))
                .map(|(path, _)| path)
                .collect();
            removed += orphans.len() as u64;
            blocking(move || {
                for path in orphans {
                    std::fs::remove_dir_all(&path)
                        .with_context(|| format!("removing {}", path.display()))?;
                    if let Some(day_dir) = path.parent() {
                        let _ = std::fs::remove_dir(day_dir);
                    }
                }
                Ok(())
            })
            .await?;
            Ok(removed)
        })
    }
}

// ---- S3 store ----

/// Settings of an S3-compatible bucket (path-style addressing).
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

/// Logs as objects `reports/<key>` in a private bucket, through presigned (SigV4 query
/// signature) requests.
pub struct S3Store {
    bucket: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
}

const S3_PREFIX: &str = "reports/";
/// How long each presigned request stays valid.
const SIGNATURE_TTL: Duration = Duration::from_secs(300);

impl S3Store {
    pub fn new(config: S3Config) -> anyhow::Result<Self> {
        let endpoint: Url = config
            .endpoint
            .parse()
            .context("PORTAL_REPORT_S3_ENDPOINT must be an http(s) URL")?;
        let bucket = Bucket::new(endpoint, UrlStyle::Path, config.bucket, config.region).context(
            "PORTAL_REPORT_S3_ENDPOINT and PORTAL_REPORT_S3_BUCKET do not make a bucket URL",
        )?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            bucket,
            credentials: Credentials::new(config.access_key, config.secret_key),
            http,
        })
    }

    fn object(key: &str) -> anyhow::Result<String> {
        safe_key(key)?;
        Ok(format!("{S3_PREFIX}{key}"))
    }
}

/// An error for a failed S3 request that names the status but never the signed URL.
fn s3_error(what: &str, status: StatusCode) -> anyhow::Error {
    anyhow::anyhow!("report store: {what} failed with HTTP {status}")
}

/// Strips the presigned URL (its query holds the signature) from transport errors.
fn s3_send_error(what: &str, e: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("report store: {what} failed: {}", e.without_url())
}

impl ReportStore for S3Store {
    fn describe(&self) -> String {
        format!("S3 bucket {}", self.bucket.base_url())
    }

    fn check(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let url = self
                .bucket
                .head_bucket(Some(&self.credentials))
                .sign(SIGNATURE_TTL);
            let response =
                self.http.head(url).send().await.map_err(|e| {
                    s3_send_error(&format!("reaching {}", self.bucket.base_url()), e)
                })?;
            match response.status() {
                s if s.is_success() => Ok(()),
                StatusCode::NOT_FOUND => bail!(
                    "the report bucket {} does not exist (PORTAL_REPORT_S3_BUCKET)",
                    self.bucket.base_url()
                ),
                StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => bail!(
                    "the report bucket {} refused the configured keys (HTTP {})",
                    self.bucket.base_url(),
                    response.status()
                ),
                s => Err(s3_error("checking the bucket", s)),
            }
        })
    }

    fn put<'a>(&'a self, key: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let object = Self::object(key)?;
            let url = self
                .bucket
                .put_object(Some(&self.credentials), &object)
                .sign(SIGNATURE_TTL);
            let response = self
                .http
                .put(url)
                .header(reqwest::header::CONTENT_TYPE, "application/gzip")
                .body(data)
                .send()
                .await
                .map_err(|e| s3_send_error("upload", e))?;
            if !response.status().is_success() {
                return Err(s3_error("upload", response.status()));
            }
            Ok(())
        })
    }

    fn get<'a>(&'a self, key: &'a str) -> BoxFut<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            let object = Self::object(key)?;
            let url = self
                .bucket
                .get_object(Some(&self.credentials), &object)
                .sign(SIGNATURE_TTL);
            let response = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|e| s3_send_error("download", e))?;
            match response.status() {
                StatusCode::NOT_FOUND => Ok(None),
                s if s.is_success() => Ok(Some(
                    response
                        .bytes()
                        .await
                        .map_err(|e| s3_send_error("download", e))?
                        .to_vec(),
                )),
                s => Err(s3_error("download", s)),
            }
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let object = Self::object(key)?;
            let url = self
                .bucket
                .delete_object(Some(&self.credentials), &object)
                .sign(SIGNATURE_TTL);
            let response = self
                .http
                .delete(url)
                .send()
                .await
                .map_err(|e| s3_send_error("delete", e))?;
            let status = response.status();
            if status.is_success() || status == StatusCode::NOT_FOUND {
                Ok(())
            } else {
                Err(s3_error("delete", status))
            }
        })
    }
}

// ---- Configuration ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StoreKind {
    /// An S3-compatible bucket (production).
    S3,
    /// A local folder (tests and development).
    Dir,
}

/// Where report logs live; shared by `serve` and `admin reports`.
#[derive(Debug, Clone, clap::Args)]
pub struct StoreArgs {
    #[arg(long, env = "PORTAL_REPORT_STORE", value_enum)]
    pub report_store: Option<StoreKind>,
    /// Folder for `dir`; must exist and be writable.
    #[arg(long, env = "PORTAL_REPORT_DIR")]
    pub report_dir: Option<PathBuf>,
    /// S3 endpoint origin for `s3`, such as https://files.example.test.
    #[arg(long, env = "PORTAL_REPORT_S3_ENDPOINT")]
    pub report_s3_endpoint: Option<String>,
    #[arg(long, env = "PORTAL_REPORT_S3_BUCKET")]
    pub report_s3_bucket: Option<String>,
    #[arg(long, env = "PORTAL_REPORT_S3_REGION", default_value = "us-east-1")]
    pub report_s3_region: String,
    /// File holding the S3 access key ID.
    #[arg(long, env = "PORTAL_REPORT_S3_ACCESS_KEY_FILE")]
    pub report_s3_access_key_file: Option<PathBuf>,
    /// File holding the S3 secret access key.
    #[arg(long, env = "PORTAL_REPORT_S3_SECRET_KEY_FILE")]
    pub report_s3_secret_key_file: Option<PathBuf>,
}

impl StoreArgs {
    /// The configured store (not yet checked: see `ReportStore::check`).
    pub fn open(&self) -> anyhow::Result<Arc<dyn ReportStore>> {
        let Some(kind) = self.report_store else {
            bail!("set PORTAL_REPORT_STORE to s3 or dir (or PORTAL_REPORTS=off)");
        };
        Ok(match kind {
            StoreKind::Dir => {
                let Some(dir) = &self.report_dir else {
                    bail!("PORTAL_REPORT_STORE=dir needs PORTAL_REPORT_DIR");
                };
                Arc::new(DirStore::new(dir))
            }
            StoreKind::S3 => {
                let need = |value: &Option<String>, name: &str| match value {
                    Some(v) if !v.trim().is_empty() => Ok(v.trim().to_owned()),
                    _ => Err(anyhow::anyhow!("PORTAL_REPORT_STORE=s3 needs {name}")),
                };
                let secret = |path: &Option<PathBuf>, name: &str| match path {
                    Some(p) => crate::config::read_secret_file(p)
                        .with_context(|| format!("reading {name}")),
                    None => Err(anyhow::anyhow!("PORTAL_REPORT_STORE=s3 needs {name}")),
                };
                Arc::new(S3Store::new(S3Config {
                    endpoint: need(&self.report_s3_endpoint, "PORTAL_REPORT_S3_ENDPOINT")?,
                    bucket: need(&self.report_s3_bucket, "PORTAL_REPORT_S3_BUCKET")?,
                    region: self.report_s3_region.trim().to_owned(),
                    access_key: secret(
                        &self.report_s3_access_key_file,
                        "PORTAL_REPORT_S3_ACCESS_KEY_FILE",
                    )?,
                    secret_key: secret(
                        &self.report_s3_secret_key_file,
                        "PORTAL_REPORT_S3_SECRET_KEY_FILE",
                    )?,
                })?)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let t = time::macros::datetime!(2026-11-03 23:59:59 -2);
        assert_eq!(
            log_key(t, "0b9c4f5e-1d2a-4b3c-8d4e-5f6a7b8c9d0e", "launcher.log"),
            "2026-11-04/0b9c4f5e-1d2a-4b3c-8d4e-5f6a7b8c9d0e/launcher.log.gz"
        );
        assert!(safe_key("2026-11-04/abc/launcher.log.gz").is_ok());
        for bad in [
            "", "../x/y", "a/../b", "a/b", "a/b/c/d", "/a/b", "a/b/.c", "a/B/c", "a//c",
        ] {
            assert!(safe_key(bad).is_err(), "{bad}");
        }
        assert!(is_day("2026-11-04") && !is_day("2026-1-04") && !is_day("abcd-ef-gh"));
    }

    #[tokio::test]
    async fn folder_round_trip() {
        let root = std::env::temp_dir().join(format!(
            "portal-store-{}",
            crate::tokens::b64url(&crate::tokens::random_bytes::<9>())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = DirStore::new(&root);
        store.check().await.unwrap();
        let key = "2026-11-04/0b9c4f5e-1d2a-4b3c-8d4e-5f6a7b8c9d0e/a.gz";
        store.put(key, b"\x1f\x8bdata".to_vec()).await.unwrap();
        assert_eq!(store.get(key).await.unwrap().unwrap(), b"\x1f\x8bdata");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(root.join(key)), 0o600);
            assert_eq!(mode(root.join(key).parent().unwrap().to_owned()), 0o700);
        }
        // No temporary files stay behind.
        let dir = root.join(key).parent().unwrap().to_owned();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        store.delete(key).await.unwrap();
        store.delete(key).await.unwrap();
        assert!(store.get(key).await.unwrap().is_none());
        assert!(!root.join("2026-11-04").exists(), "empty folders go too");
        assert!(store.put("../escape/x.gz", vec![]).await.is_err());
        assert!(
            DirStore::new(root.join("missing"))
                .check()
                .await
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
