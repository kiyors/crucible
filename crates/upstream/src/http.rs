//! One HTTP path for everything the CLI downloads.
//!
//! GitHub's release CDN and static.rust-lang.org time out now and then, and the
//! updater runs from a scheduled job, so a transient failure must not fail the
//! run. A permanent one, though, must not be retried: a 404 is an answer, and
//! retrying it only delays the update.

use std::error::Error;
use std::io::Read;
use std::time::Duration;

use anyhow::Result;

const ATTEMPTS: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(60);
/// An upstream that asks us to wait longer than this is better retried by the
/// next scheduled run than by a sleeping job.
const MAX_RETRY_AFTER: u64 = 60;

/// A failed GET, split by what the caller can do about it.
#[derive(Debug)]
pub enum HttpError {
    /// The upstream says the resource does not exist: an archive gap, not a
    /// transient failure.
    NotFound { url: String },
    /// Any other HTTP status, retryable or not according to `should_retry`.
    Status { url: String, code: u16 },
    /// DNS, TLS, connection and body-read failures.
    Transport {
        url: String,
        source: Box<dyn Error + Send + Sync>,
    },
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { url } => write!(f, "GET {url} returned 404"),
            Self::Status { url, code } => write!(f, "GET {url} returned HTTP {code}"),
            Self::Transport { url, source } => write!(f, "GET {url} failed: {source}"),
        }
    }
}

impl Error for HttpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(source.as_ref()),
            Self::NotFound { .. } | Self::Status { .. } => None,
        }
    }
}

#[derive(Clone)]
pub struct Http {
    agent: ureq::Agent,
    token: Option<String>,
}

impl Http {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new().timeout(TIMEOUT).build(),
            // The API allows 60 unauthenticated requests an hour; CI passes its
            // token, a developer usually does not.
            token: std::env::var("GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.is_empty()),
        }
    }

    /// GET `url` as bytes, retrying transport failures, 429s and 5xx.
    ///
    /// `Retry-After` wins over the linear backoff (2s, then 4s), so a
    /// rate-limiting upstream is not hammered.
    pub fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        // A loop returns the last failure from inside the body, so there is no
        // unreachable tail to panic through when the attempts run out.
        let mut attempt = 1;
        loop {
            match self.try_get(url) {
                Ok(body) => return Ok(body),
                Err(failure) => {
                    if attempt == ATTEMPTS || !should_retry(&failure.error) {
                        return Err(failure.error);
                    }
                    let delay = failure
                        .retry_after
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| Duration::from_secs(2 * u64::from(attempt)));
                    println!(
                        "  retrying {url} in {}s after: {}",
                        delay.as_secs(),
                        failure.error
                    );
                    std::thread::sleep(delay);
                    attempt += 1;
                }
            }
        }
    }

    fn try_get(&self, url: &str) -> Result<Vec<u8>, Failure> {
        let mut request = self.agent.get(url);
        if url.starts_with("https://api.github.com/") {
            if let Some(token) = &self.token {
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                return Err(Failure {
                    error: if code == 404 {
                        HttpError::NotFound {
                            url: url.to_string(),
                        }
                    } else {
                        HttpError::Status {
                            url: url.to_string(),
                            code,
                        }
                    },
                    retry_after: retry_after_seconds(response.header("Retry-After")),
                });
            }
            Err(err) => {
                return Err(Failure {
                    error: HttpError::Transport {
                        url: url.to_string(),
                        source: Box::new(err),
                    },
                    retry_after: None,
                });
            }
        };

        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|err| Failure {
                error: HttpError::Transport {
                    url: url.to_string(),
                    source: Box::new(err),
                },
                retry_after: None,
            })?;
        Ok(body)
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

/// A failed attempt and how long the upstream asked us to wait first.
struct Failure {
    error: HttpError,
    retry_after: Option<u64>,
}

/// Permanent answers (404 and the rest of 4xx) are never retried.
fn should_retry(error: &HttpError) -> bool {
    match error {
        HttpError::NotFound { .. } => false,
        HttpError::Status { code, .. } => *code == 429 || (500..=599).contains(code),
        HttpError::Transport { .. } => true,
    }
}

/// `Retry-After` as seconds, capped. The header also allows an HTTP date, which
/// this CLI does not parse; such a value falls back to the linear backoff.
fn retry_after_seconds(value: Option<&str>) -> Option<u64> {
    let seconds: u64 = value?.trim().parse().ok()?;
    Some(seconds.min(MAX_RETRY_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn transport() -> HttpError {
        HttpError::Transport {
            url: "https://example.test/x".to_string(),
            source: Box::new(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
        }
    }

    #[test]
    fn retries_only_transport_429_and_5xx() {
        assert!(should_retry(&transport()));
        assert!(should_retry(&HttpError::Status {
            url: "u".to_string(),
            code: 429,
        }));
        assert!(should_retry(&HttpError::Status {
            url: "u".to_string(),
            code: 500,
        }));
        assert!(should_retry(&HttpError::Status {
            url: "u".to_string(),
            code: 503,
        }));

        assert!(!should_retry(&HttpError::NotFound {
            url: "u".to_string(),
        }));
        for code in [400, 403, 404, 410, 301] {
            assert!(
                !should_retry(&HttpError::Status {
                    url: "u".to_string(),
                    code,
                }),
                "{code} should not be retried"
            );
        }
    }

    #[test]
    fn honours_retry_after_in_seconds_and_caps_it() {
        assert_eq!(retry_after_seconds(Some("30")), Some(30));
        assert_eq!(retry_after_seconds(Some(" 5 ")), Some(5));
        assert_eq!(retry_after_seconds(Some("0")), Some(0));
        assert_eq!(retry_after_seconds(Some("600")), Some(MAX_RETRY_AFTER));
        assert_eq!(retry_after_seconds(None), None);
        // An HTTP date is legal but not something this CLI needs.
        assert_eq!(
            retry_after_seconds(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            None
        );
        assert_eq!(retry_after_seconds(Some("-1")), None);
    }

    #[test]
    fn permanent_errors_carry_the_url() {
        let error = HttpError::NotFound {
            url: "https://static.rust-lang.org/dist/2026-01-01/x.toml".to_string(),
        };
        assert!(error.to_string().contains("404"));
        assert!(error.to_string().contains("2026-01-01"));
        assert!(error.source().is_none());
        assert!(transport().source().is_some());
    }
}
