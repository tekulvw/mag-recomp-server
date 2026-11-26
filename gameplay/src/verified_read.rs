//! Startup reads of resources whose content a recorded SHA-256 pins.
//!
//! Each attempt reads the whole file into memory once; the decoder parses
//! and hashes those same bytes, never a second read. When the bytes do not
//! verify (they fail to decode, or a recorded digest disagrees with them),
//! the file is read again after a short pause, up to `RETRIES` more times,
//! since an identical bundle and binary have verified on the next start.
//! Every failed attempt is logged with the path, both digests and the byte
//! counts. If no attempt verifies, the error names the file and both
//! digests. Errors in bytes that did verify are final: reading them again
//! gives the same answer.
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

/// Extra reads after the first when the bytes do not verify.
pub(crate) const RETRIES: u32 = 2;
/// Pause before retry `n` (1-based): `n` times this.
const BACKOFF: Duration = Duration::from_millis(200);

/// Why one attempt's bytes were not accepted.
pub(crate) enum Rejection<E> {
    /// A recorded digest disagrees with the bytes it covers. `code` is the
    /// stable error code; `bytes` is how many bytes the digest covers.
    Digest {
        code: &'static str,
        expected: String,
        actual: String,
        bytes: usize,
    },
    /// The bytes did not decode; a damaged read looks like this too.
    Decode(String),
    /// The bytes verified but are not acceptable; retrying cannot help.
    Final(E),
}

/// A read that did not produce a verified value.
#[derive(Debug)]
pub(crate) enum Failure<E> {
    /// The file could not be opened or read, or exceeds its byte limit.
    Io(io::Error),
    /// No attempt verified; the message names the file and both digests.
    Unverified(String),
    /// Verified bytes were rejected (see `Rejection::Final`).
    Final(E),
}
impl<E: fmt::Display> fmt::Display for Failure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Unverified(message) => f.write_str(message),
            Self::Final(error) => error.fmt(f),
        }
    }
}
impl<E: fmt::Display> From<Failure<E>> for io::Error {
    fn from(failure: Failure<E>) -> Self {
        match failure {
            Failure::Io(error) => error,
            other => io::Error::other(other.to_string()),
        }
    }
}

/// Read `path` (at most `limit` bytes) and decode it, reading again when the
/// bytes do not verify. `resource` names the kind of file in the log.
pub(crate) fn read<T, E>(
    path: &Path,
    limit: u64,
    resource: &'static str,
    decode: impl FnMut(&[u8]) -> Result<T, Rejection<E>>,
) -> Result<T, Failure<E>> {
    read_with(
        path,
        resource,
        || {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(limit + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                return Err(io::Error::other(format!(
                    "{resource}_size_limit: {} exceeds {limit} bytes",
                    path.display()
                )));
            }
            Ok(bytes)
        },
        BACKOFF,
        decode,
    )
}

/// `read` with the file read and the pause supplied, so tests can stage a
/// damaged read without touching the disk or waiting.
pub(crate) fn read_with<T, E>(
    path: &Path,
    resource: &'static str,
    mut load: impl FnMut() -> io::Result<Vec<u8>>,
    backoff: Duration,
    mut decode: impl FnMut(&[u8]) -> Result<T, Rejection<E>>,
) -> Result<T, Failure<E>> {
    let attempts = RETRIES + 1;
    let mut attempt = 1;
    loop {
        let bytes = load().map_err(Failure::Io)?;
        let file_bytes = bytes.len();
        let rejection = match decode(&bytes) {
            Ok(value) => {
                if attempt > 1 {
                    eprintln!(
                        "{}",
                        serde_json::json!({"event": "resource_read_recovered",
                            "resource": resource, "path": path.display().to_string(),
                            "attempt": attempt, "attempts": attempts,
                            "file_bytes": file_bytes})
                    );
                }
                return Ok(value);
            }
            Err(Rejection::Final(error)) => return Err(Failure::Final(error)),
            Err(rejection) => rejection,
        };
        // The digest of everything read this attempt, so two attempts can be
        // told apart even when the decoder never reached its own digest.
        let file_sha256 = sha256(&bytes);
        drop(bytes);
        let last = attempt == attempts;
        let (code, detail, record) = match &rejection {
            Rejection::Digest {
                code,
                expected,
                actual,
                bytes,
            } => (
                *code,
                format!("expected sha256 {expected}, read sha256 {actual} over {bytes} bytes"),
                serde_json::json!({"reason": code, "expected_sha256": expected,
                    "actual_sha256": actual, "digest_bytes": bytes}),
            ),
            Rejection::Decode(error) => (
                "undecodable",
                format!("read sha256 {file_sha256} does not decode: {error}"),
                serde_json::json!({"reason": "undecodable", "error": error}),
            ),
            Rejection::Final(_) => unreachable!("returned above"),
        };
        let mut event = serde_json::json!({"event": "resource_read_unverified",
            "resource": resource, "path": path.display().to_string(),
            "attempt": attempt, "attempts": attempts, "file_bytes": file_bytes,
            "file_sha256": file_sha256, "final": last});
        if let (Some(event), serde_json::Value::Object(record)) = (event.as_object_mut(), record) {
            event.extend(record);
        }
        eprintln!("{event}");
        if last {
            let code = if code == "undecodable" {
                format!("{resource}_undecodable")
            } else {
                code.to_string()
            };
            return Err(Failure::Unverified(format!(
                "{code}: {}: {detail} ({file_bytes} file bytes, sha256 {file_sha256}); \
                 {attempts} reads, none verified",
                path.display()
            )));
        }
        std::thread::sleep(backoff * attempt);
        attempt += 1;
    }
}

/// The lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    // SYNTHETIC: every input is generated here.
    use super::*;
    use std::path::PathBuf;

    fn digest_check(bytes: &[u8]) -> Result<usize, Rejection<&'static str>> {
        let expected = sha256(b"good");
        let actual = sha256(bytes);
        if actual != expected {
            return Err(Rejection::Digest {
                code: "synthetic_hash_mismatch",
                expected,
                actual,
                bytes: bytes.len(),
            });
        }
        Ok(bytes.len())
    }

    #[test]
    fn a_damaged_read_then_a_good_one_succeeds() {
        let mut reads = vec![b"good".to_vec(), b"gooe".to_vec()];
        let mut count = 0;
        let value = read_with(
            &PathBuf::from("synthetic/resource.json"),
            "synthetic",
            || {
                count += 1;
                Ok(reads.pop().unwrap())
            },
            Duration::ZERO,
            digest_check,
        )
        .unwrap();
        assert_eq!((value, count), (4, 2));
    }

    #[test]
    fn a_persistent_mismatch_fails_naming_the_file_and_both_digests() {
        let mut count = 0;
        let error = read_with(
            &PathBuf::from("synthetic/resource.json"),
            "synthetic",
            || {
                count += 1;
                Ok(b"bad!".to_vec())
            },
            Duration::ZERO,
            digest_check,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(count, 1 + RETRIES);
        assert!(error.starts_with("synthetic_hash_mismatch: synthetic/resource.json: "));
        assert!(error.contains(&format!("expected sha256 {}", sha256(b"good"))));
        assert!(error.contains(&format!("read sha256 {}", sha256(b"bad!"))));
        assert!(error.contains("over 4 bytes"));
    }

    #[test]
    fn undecodable_bytes_are_retried_and_reported() {
        let mut reads = vec![b"good".to_vec(), b"{".to_vec()];
        let decode = |bytes: &[u8]| -> Result<usize, Rejection<&'static str>> {
            if bytes == b"{" {
                return Err(Rejection::Decode("EOF while parsing".into()));
            }
            digest_check(bytes)
        };
        let path = PathBuf::from("synthetic/resource.json");
        let ok = read_with(
            &path,
            "synthetic",
            || Ok(reads.pop().unwrap()),
            Duration::ZERO,
            decode,
        );
        assert_eq!(ok.unwrap(), 4);
        let error = read_with(
            &path,
            "synthetic",
            || Ok(b"{".to_vec()),
            Duration::ZERO,
            decode,
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("synthetic_undecodable: synthetic/resource.json: "));
        assert!(error.contains(&sha256(b"{")));
    }

    #[test]
    fn verified_bytes_that_are_rejected_and_io_errors_are_not_retried() {
        let mut count = 0;
        let path = PathBuf::from("synthetic/resource.json");
        let failure = read_with(
            &path,
            "synthetic",
            || {
                count += 1;
                Ok(b"good".to_vec())
            },
            Duration::ZERO,
            |_| Err::<(), _>(Rejection::Final("synthetic_schema_invalid")),
        )
        .unwrap_err();
        assert!(matches!(
            failure,
            Failure::Final("synthetic_schema_invalid")
        ));
        assert_eq!(count, 1);
        let failure = read_with(
            &path,
            "synthetic",
            || {
                count += 1;
                Err(io::Error::new(io::ErrorKind::NotFound, "synthetic missing"))
            },
            Duration::ZERO,
            digest_check,
        )
        .unwrap_err();
        assert!(matches!(failure, Failure::Io(_)));
        assert_eq!(count, 2);
    }

    #[test]
    fn the_file_reader_enforces_its_byte_limit() {
        let dir = std::env::temp_dir().join(format!("mag-verified-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("resource.json");
        std::fs::write(&path, b"good").unwrap();
        assert_eq!(read(&path, 4, "synthetic", digest_check).unwrap(), 4);
        let error = read(&path, 3, "synthetic", digest_check).unwrap_err();
        assert!(
            matches!(&error, Failure::Io(e) if e.to_string().starts_with("synthetic_size_limit"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
