//! "New version available" check. Reads the public Releases page of the GitHub repository this
//! copy was built from (set at build time by the release workflow). Sends no personal data and
//! downloads nothing; the user decides whether to open the download page. Can be turned off in Settings.
use anyhow::{bail, Result};
use serde::Serialize;

/// "owner/repo", injected by .github/workflows/release.yml. Unset in local builds → no checks.
pub const REPO: Option<&str> = option_env!("OMN_UPDATE_REPO");

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Update { pub version: String, pub current: String, pub url: String, pub notes: String }

/// "v1.2.0" / "1.2" → (1, 2, 0)
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map(|p| p.parse::<u64>());
    Some((it.next()?.ok()?, it.next().and_then(|x| x.ok()).unwrap_or(0), it.next().and_then(|x| x.ok()).unwrap_or(0)))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(a), Some(b)) if a > b)
}

pub async fn check(current: &str) -> Result<Option<Update>> {
    let Some(repo) = REPO.filter(|r| !r.is_empty()) else { return Ok(None) };
    let r = reqwest::Client::new().get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("User-Agent", "OpenMeeting-Notes").header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10)).send().await?;
    if r.status().as_u16() == 404 { return Ok(None); }
    if !r.status().is_success() { bail!("Couldn't check for updates ({}).", r.status()); }
    let v: serde_json::Value = r.json().await?;
    let tag = v["tag_name"].as_str().unwrap_or_default();
    if !is_newer(tag, current) { return Ok(None); }
    Ok(Some(Update { version: tag.trim_start_matches('v').to_string(), current: current.into(),
                     url: v["html_url"].as_str().unwrap_or_default().into(), notes: v["body"].as_str().unwrap_or_default().chars().take(1500).collect() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compare() {
        assert!(is_newer("v1.2.0", "1.1.0"));
        assert!(is_newer("1.10", "1.9.3"));
        assert!(!is_newer("v1.1.0", "1.1.0"));
        assert!(!is_newer("build-12", "1.1.0"));
    }
}
