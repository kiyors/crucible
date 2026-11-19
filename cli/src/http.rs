//! One HTTP path for everything the CLI downloads.
//!
//! GitHub's release CDN and static.rust-lang.org time out now and then, and the
//! updater runs from a scheduled job, so a transient failure must not fail the
//! run. The backoff is linear because the upstreams are rate-limited rather
//! than overloaded.

use std::io::Read;
use std::time::Duration;

use anyhow::Result;

const ATTEMPTS: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(60);

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

    /// GET `url` as bytes, retrying three times with a linear backoff (2s,
    /// then 4s).
    pub fn get(&self, url: &str) -> Result<Vec<u8>> {
        let mut last = String::new();
        for attempt in 1..=ATTEMPTS {
            match self.try_get(url) {
                Ok(body) => return Ok(body),
                Err(err) => {
                    last = err.to_string();
                    if attempt == ATTEMPTS {
                        break;
                    }
                    println!("  retrying {url} after: {last}");
                    std::thread::sleep(Duration::from_secs(2 * u64::from(attempt)));
                }
            }
        }
        anyhow::bail!("GET {url} failed: {last}")
    }

    fn try_get(&self, url: &str) -> Result<Vec<u8>> {
        let mut request = self.agent.get(url);
        if url.starts_with("https://api.github.com/") {
            if let Some(token) = &self.token {
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
        }
        let response = request.call()?;
        let mut body = Vec::new();
        response.into_reader().read_to_end(&mut body)?;
        Ok(body)
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}
