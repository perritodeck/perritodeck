//! Is there a newer PerritoDeck? Asks GitHub for the latest release. Nothing is
//! downloaded or installed; the app just says so and links to it.

use serde::Serialize;

/// Where release pages live; the app only opens links under this.
pub const RELEASES: &str = "https://github.com/perritodeck/perritodeck/releases/";
const LATEST: &str = "https://api.github.com/repos/perritodeck/perritodeck/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// `Ok(Some(..))` when a newer release exists, `Ok(None)` when this is the latest.
pub fn check(current: &str) -> Result<Option<Update>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(LATEST)
        .header("User-Agent", "PerritoDeck")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Err("No releases are published yet.".into()),
        code => return Err(format!("GitHub answered {code}.")),
    }
    let body: serde_json::Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    let tag = body["tag_name"].as_str().unwrap_or_default();
    let url = body["html_url"].as_str().unwrap_or_default().to_owned();
    Ok(newer(tag, current).then(|| Update {
        version: tag.trim_start_matches('v').to_owned(),
        url,
    }))
}

/// Compare dotted versions (`v1.2.10` > `1.2.9`); anything after `-` is ignored.
pub fn newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1.2.10", "1.2.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
    }
}
