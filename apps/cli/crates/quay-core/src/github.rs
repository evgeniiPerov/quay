use crate::error::{QuayError, Result};
use crate::fetcher::{RegistryFetcher, SkillFileFetcher};
use crate::registry::Registry;

const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";

/// HTTPS-based fetcher that targets `raw.githubusercontent.com`.
/// Hub URLs must look like `https://github.com/{owner}/{repo}.git` or `git@github.com:{owner}/{repo}.git`.
pub struct GithubRawFetcher {
    pub branch: String,
    client: reqwest::blocking::Client,
}

impl GithubRawFetcher {
    pub fn new(branch: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            client: reqwest::blocking::Client::builder()
                .user_agent(concat!("quay/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn parse_owner_repo(hub_url: &str) -> Result<(String, String)> {
        const HTTPS_PREFIX: &str = "https://github.com/";
        const SSH_PREFIX: &str = "git@github.com:";
        let trimmed = if let Some(rest) = hub_url.strip_prefix(HTTPS_PREFIX) {
            rest
        } else if let Some(rest) = hub_url.strip_prefix(SSH_PREFIX) {
            rest
        } else {
            return Err(QuayError::InvalidConfig {
				path: hub_url.into(),
				reason: "only github.com URLs are supported in this version (gitlab/azure/bitbucket are tracked for a future release)".into(),
			});
        };
        let trimmed = trimmed.trim_end_matches(".git");
        let mut parts = trimmed.splitn(2, '/');
        let owner = parts.next().ok_or_else(|| QuayError::InvalidConfig {
            path: hub_url.into(),
            reason: "url does not include owner".into(),
        })?;
        let repo = parts.next().ok_or_else(|| QuayError::InvalidConfig {
            path: hub_url.into(),
            reason: "url does not include repo".into(),
        })?;
        if owner.is_empty() || repo.is_empty() {
            return Err(QuayError::InvalidConfig {
                path: hub_url.into(),
                reason: "owner or repo is empty".into(),
            });
        }
        Ok((owner.to_string(), repo.to_string()))
    }

    fn fetch_bytes(
        &self,
        base: &str,
        owner: &str,
        repo: &str,
        git_ref: &str,
        path: &str,
    ) -> Result<Vec<u8>> {
        // Cache-bust the CDN: `raw.githubusercontent.com` caches up to ~5
        // minutes per (owner, repo, ref, path), and a fresh `registry.json`
        // pushed seconds ago can otherwise return stale bytes. Appending a
        // unique query string forces a fresh fetch without hurting clean
        // first-time loads.
        let cb = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let url = format!(
            "{}/{}/{}/{}/{}?_cb={}",
            base, owner, repo, git_ref, path, cb
        );
        let resp = self
            .client
            .get(&url)
            .header("Cache-Control", "no-cache")
            .send()
            .map_err(QuayError::Network)?
            .error_for_status()
            .map_err(QuayError::Network)?;
        Ok(resp.bytes().map_err(QuayError::Network)?.to_vec())
    }
}

impl RegistryFetcher for GithubRawFetcher {
    fn fetch(&self, hub_url: &str) -> Result<Registry> {
        let (owner, repo) = Self::parse_owner_repo(hub_url)?;
        let bytes = self.fetch_bytes(
            GITHUB_RAW_BASE,
            &owner,
            &repo,
            &self.branch,
            "registry.json",
        )?;
        let text = String::from_utf8(bytes).map_err(|e| QuayError::InvalidRegistry {
            reason: format!("registry.json is not valid UTF-8: {}", e),
        })?;
        Registry::parse(&text)
    }

    fn fetch_at(&self, hub_url: &str, git_ref: &str) -> Result<Registry> {
        let (owner, repo) = Self::parse_owner_repo(hub_url)?;
        let bytes =
            self.fetch_bytes(GITHUB_RAW_BASE, &owner, &repo, git_ref, "registry.json")?;
        let text = String::from_utf8(bytes).map_err(|e| QuayError::InvalidRegistry {
            reason: format!("registry.json is not valid UTF-8: {}", e),
        })?;
        Registry::parse(&text)
    }
}

impl SkillFileFetcher for GithubRawFetcher {
    fn fetch_file(&self, hub_url: &str, path: &str) -> Result<Vec<u8>> {
        let (owner, repo) = Self::parse_owner_repo(hub_url)?;
        self.fetch_bytes(GITHUB_RAW_BASE, &owner, &repo, &self.branch, path)
    }

    fn fetch_file_at(&self, hub_url: &str, path: &str, git_ref: &str) -> Result<Vec<u8>> {
        let (owner, repo) = Self::parse_owner_repo(hub_url)?;
        self.fetch_bytes(GITHUB_RAW_BASE, &owner, &repo, git_ref, path)
    }
}

/// Test-only variant that hits a custom base URL (e.g., a wiremock instance).
#[cfg(debug_assertions)]
pub struct GithubRawFetcherWithBase {
    pub branch: String,
    pub base_url: String,
    client: reqwest::blocking::Client,
}

#[cfg(debug_assertions)]
impl GithubRawFetcherWithBase {
    pub fn new(branch: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            base_url: base_url.into(),
            client: reqwest::blocking::Client::builder()
                .user_agent("quay-test")
                .build()
                .expect("reqwest client"),
        }
    }

    fn fetch_bytes(&self, owner: &str, repo: &str, git_ref: &str, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}/{}/{}/{}/{}", self.base_url, owner, repo, git_ref, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(QuayError::Network)?
            .error_for_status()
            .map_err(QuayError::Network)?;
        Ok(resp.bytes().map_err(QuayError::Network)?.to_vec())
    }
}

#[cfg(debug_assertions)]
impl RegistryFetcher for GithubRawFetcherWithBase {
    fn fetch(&self, hub_url: &str) -> Result<Registry> {
        let (owner, repo) = GithubRawFetcher::parse_owner_repo(hub_url)?;
        let bytes = self.fetch_bytes(&owner, &repo, &self.branch, "registry.json")?;
        let text = String::from_utf8(bytes).map_err(|e| QuayError::InvalidRegistry {
            reason: format!("registry.json is not valid UTF-8: {}", e),
        })?;
        Registry::parse(&text)
    }

    fn fetch_at(&self, hub_url: &str, git_ref: &str) -> Result<Registry> {
        let (owner, repo) = GithubRawFetcher::parse_owner_repo(hub_url)?;
        let bytes = self.fetch_bytes(&owner, &repo, git_ref, "registry.json")?;
        let text = String::from_utf8(bytes).map_err(|e| QuayError::InvalidRegistry {
            reason: format!("registry.json is not valid UTF-8: {}", e),
        })?;
        Registry::parse(&text)
    }
}

#[cfg(debug_assertions)]
impl SkillFileFetcher for GithubRawFetcherWithBase {
    fn fetch_file(&self, hub_url: &str, path: &str) -> Result<Vec<u8>> {
        let (owner, repo) = GithubRawFetcher::parse_owner_repo(hub_url)?;
        self.fetch_bytes(&owner, &repo, &self.branch, path)
    }

    fn fetch_file_at(&self, hub_url: &str, path: &str, git_ref: &str) -> Result<Vec<u8>> {
        let (owner, repo) = GithubRawFetcher::parse_owner_repo(hub_url)?;
        self.fetch_bytes(&owner, &repo, git_ref, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url() {
        let (o, r) = GithubRawFetcher::parse_owner_repo("https://github.com/foo/bar.git").unwrap();
        assert_eq!(o, "foo");
        assert_eq!(r, "bar");
    }

    #[test]
    fn parses_ssh_url() {
        let (o, r) = GithubRawFetcher::parse_owner_repo("git@github.com:foo/bar.git").unwrap();
        assert_eq!(o, "foo");
        assert_eq!(r, "bar");
    }

    #[test]
    fn rejects_url_without_repo() {
        let err = GithubRawFetcher::parse_owner_repo("https://github.com/foo").unwrap_err();
        assert!(format!("{}", err).contains("repo"));
    }

    #[test]
    fn rejects_non_github_url() {
        let err = GithubRawFetcher::parse_owner_repo("https://gitlab.com/foo/bar.git").unwrap_err();
        assert!(format!("{}", err).contains("only github.com URLs"));
    }
}
