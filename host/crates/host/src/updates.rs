//! Is there a newer PerritoDeck? Asks GitHub for the recent releases. Nothing is
//! downloaded or installed; the app just says so and links to it.
//!
//! It lists releases instead of asking for `/releases/latest`, because that one skips
//! pre-releases, and release.yml marks every 0.x release as one.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Where release pages live; the app only opens links under this.
pub const RELEASES: &str = "https://github.com/perritodeck/perritodeck/releases/";
const API: &str = "https://api.github.com";
/// The most recent releases, newest first; the highest version among them wins.
const LIST: &str = "/repos/perritodeck/perritodeck/releases?per_page=10";
/// GitHub allows 60 unauthenticated requests an hour from one address.
const LIMITED: &str = "GitHub is limiting checks from this network; try again in an hour.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    pub url: String,
}

/// One entry of GitHub's release list, with only what the check needs.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Release {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
}

/// `Ok(Some(..))` when a newer release exists, `Ok(None)` when this is the latest.
pub fn check(current: &str) -> Result<Option<Update>, String> {
    check_at(API, current)
}

fn check_at(api: &str, current: &str) -> Result<Option<Update>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(format!("{api}{LIST}"))
        .header("User-Agent", "PerritoDeck")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
    // Out of requests is a 403 or 429 with none remaining; any 429 is a rate limit.
    let none_left = resp
        .headers()
        .get("x-ratelimit-remaining")
        .is_some_and(|v| v.to_str().is_ok_and(|v| v.trim() == "0"));
    match resp.status().as_u16() {
        200 => {}
        404 => return Err("No releases are published yet.".into()),
        429 => return Err(LIMITED.into()),
        403 if none_left => return Err(LIMITED.into()),
        code => return Err(format!("GitHub answered {code}.")),
    }
    let releases: Vec<Release> = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("Unexpected answer from GitHub: {e}"))?;
    if releases.iter().all(|r| r.draft) {
        return Err("No releases are published yet.".into());
    }
    Ok(pick(&releases, current))
}

/// The highest release that is newer than `current` and meant for it:
/// - drafts never count, nor tags release.yml doesn't make (see [`release_tag`]);
/// - a release GitHub marks as a pre-release (every 0.x, and every `-rc.1` style
///   version) counts while `current` is itself 0.x or a pre-release, so 0.1.0 sees 0.2.0
///   and 1.0.0 doesn't see 1.1.0-beta;
/// - a version with a pre-release part (`0.3.0-rc.1`) counts only when `current` has
///   one too, so someone on 0.1.0 is told about 0.2.0 but not about its release
///   candidates.
fn pick(releases: &[Release], current: &str) -> Option<Update> {
    let (numbers, pre) = parse(current);
    let on_pre_release = pre.is_some();
    let on_zero = numbers.first().copied().unwrap_or(0) == 0;
    releases
        .iter()
        .filter(|r| !r.draft && release_tag(&r.tag_name))
        .filter(|r| {
            if parse(&r.tag_name).1.is_some() {
                on_pre_release
            } else if r.prerelease {
                on_zero || on_pre_release
            } else {
                true
            }
        })
        .filter(|r| newer(&r.tag_name, current))
        .max_by(|a, b| compare(&a.tag_name, &b.tag_name))
        .map(|r| {
            let tag = r.tag_name.trim();
            Update {
                version: tag.trim_start_matches('v').to_owned(),
                url: if r.html_url.is_empty() {
                    format!("{RELEASES}tag/{tag}")
                } else {
                    r.html_url.clone()
                },
            }
        })
}

/// Does `tag` look like one release.yml builds: an optional `v`, then three dotted
/// numbers, then nothing or a `-pre-release` or `+build` part? release.yml runs for
/// `v*.*.*` tags, or by hand for any tag, and strips only a lower-case `v`, so
/// `nightly`, `v1.2` and `V1.2.0` aren't releases.
fn release_tag(tag: &str) -> bool {
    let tag = tag.trim();
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let numbers = version.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = numbers.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Is `candidate` a later version than `current`? See `compare`.
pub fn newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Ordering::Greater
}

/// Order versions the way semver does: the dotted numbers first (`v1.2.10` > `1.2.9`,
/// and `1.0` = `1.0.0`), then a release above its pre-releases
/// (`1.0.0` > `1.0.0-rc.2` > `1.0.0-rc.1` > `1.0.0-beta`). Build metadata (`+...`) is
/// ignored.
fn compare(a: &str, b: &str) -> Ordering {
    let (a_numbers, a_pre) = parse(a);
    let (b_numbers, b_pre) = parse(b);
    for i in 0..a_numbers.len().max(b_numbers.len()) {
        let (x, y) = (
            a_numbers.get(i).copied().unwrap_or(0),
            b_numbers.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x.cmp(&y);
        }
    }
    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => compare_pre_release(x, y),
    }
}

/// A version's dotted numbers and its pre-release part (`rc.1` in `v1.2.0-rc.1`), if
/// any. A part that isn't a number counts as 0.
fn parse(version: &str) -> (Vec<u64>, Option<&str>) {
    let v = version.trim().trim_start_matches('v');
    let v = v.split('+').next().unwrap_or("");
    let (numbers, pre) = match v.split_once('-') {
        Some((numbers, pre)) => (numbers, Some(pre).filter(|p| !p.is_empty())),
        None => (v, None),
    };
    let numbers = numbers.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    (numbers, pre)
}

/// Semver's pre-release order: field by field, numbers compared as numbers and below
/// words, words compared alphabetically; more fields win when the rest are equal.
fn compare_pre_release(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.split('.'), b.split('.'));
    loop {
        let (x, y) = match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => (x, y),
        };
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("1.2.10", "1.2.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("1.0", "1.0.0"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
        assert!(!newer("1.0.0+build.5", "1.0.0"));
    }

    #[test]
    fn a_release_is_above_its_pre_releases() {
        // In order, from semver.org's example.
        let order = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "v1.0.1-rc.1",
        ];
        for pair in order.windows(2) {
            assert!(newer(pair[1], pair[0]), "{} > {}", pair[1], pair[0]);
            assert!(!newer(pair[0], pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("{RELEASES}tag/{tag}"),
            draft: false,
            prerelease,
        }
    }

    fn version(releases: &[Release], current: &str) -> Option<String> {
        pick(releases, current).map(|u| u.version)
    }

    #[test]
    fn a_zero_x_version_sees_the_next_zero_x() {
        // release.yml marks every 0.x as a pre-release.
        let list = [release("v0.2.0", true), release("v0.1.0", true)];
        assert_eq!(
            pick(&list, "0.1.0"),
            Some(Update {
                version: "0.2.0".into(),
                url: format!("{RELEASES}tag/v0.2.0"),
            })
        );
        assert_eq!(version(&list, "0.2.0"), None);
        assert_eq!(
            version(&[release("v1.0.0", false)], "0.2.0").as_deref(),
            Some("1.0.0")
        );
    }

    #[test]
    fn a_stable_version_ignores_pre_releases() {
        let list = [
            release("v1.2.0-rc.1", true),
            release("v1.1.0", false),
            release("v1.0.0", false),
        ];
        assert_eq!(version(&list, "1.0.0").as_deref(), Some("1.1.0"));
        assert_eq!(version(&list, "1.1.0"), None);
    }

    #[test]
    fn release_candidates_are_only_for_people_on_one() {
        let list = [release("v0.3.0-rc.1", true), release("v0.2.0", true)];
        assert_eq!(version(&list, "0.1.0").as_deref(), Some("0.2.0"));
        assert_eq!(version(&list, "0.2.0"), None);
        assert_eq!(version(&list, "0.2.0-rc.2").as_deref(), Some("0.3.0-rc.1"));
        // Someone on a release candidate hears about the release itself.
        let list = [release("v0.3.0", true), release("v0.3.0-rc.2", true)];
        assert_eq!(version(&list, "0.3.0-rc.1").as_deref(), Some("0.3.0"));
        assert_eq!(
            version(&list[1..], "0.3.0-rc.1").as_deref(),
            Some("0.3.0-rc.2")
        );
    }

    #[test]
    fn picks_the_highest_version_and_skips_drafts() {
        let mut draft = release("v0.11.0", true);
        draft.draft = true;
        // Not newest first: a fix for an older line can be published later.
        let list = [
            release("v0.9.1", true),
            draft,
            release("v0.10.0", true),
            release("v0.2.0", true),
            release("", true),
        ];
        assert_eq!(version(&list, "0.1.0").as_deref(), Some("0.10.0"));
    }

    #[test]
    fn skips_tags_release_yml_does_not_make() {
        // Read as versions, all but `nightly` would be newer than 0.1.0 (`V0` reads as 0).
        for tag in [
            "nightly", "V0.2.0", "v1", "v0.2", "2026.10", "v0.2.0.1", "v0.2.x",
        ] {
            assert_eq!(version(&[release(tag, false)], "0.1.0"), None, "{tag}");
        }
        // The shape is what counts: release.yml can be run by hand on a tag without the
        // v, and a build part or an empty pre-release part leaves three numbers.
        for tag in [
            "v0.2.0",
            " v0.2.0 ",
            "v0.2.0+build.5",
            "v0.2.0-",
            "2026.10.14",
        ] {
            assert!(pick(&[release(tag, false)], "0.1.0").is_some(), "{tag}");
        }
    }

    #[test]
    fn links_the_tag_when_github_gives_no_page() {
        let mut r = release("v0.2.0", true);
        r.html_url.clear();
        assert_eq!(
            pick(&[r], "0.1.0").unwrap().url,
            format!("{RELEASES}tag/v0.2.0")
        );
    }

    /// Answer one request on 127.0.0.1 with `status` and `body`; the receiver gets the
    /// request line and headers.
    fn github(status: &str, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
        github_with(status, "", body)
    }

    /// [`github`], adding `headers` (each line ending in `\r\n`) to the answer.
    fn github_with(
        status: &str,
        headers: &str,
        body: &str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            stream.write_all(response.as_bytes()).unwrap();
            let _ = tx.send(head);
        });
        (api, rx)
    }

    #[test]
    fn asks_github_for_the_release_list() {
        let body = r#"[
            {"tag_name": "v0.3.0", "html_url": "https://github.com/perritodeck/perritodeck/releases/tag/v0.3.0", "draft": true, "prerelease": true},
            {"tag_name": "v0.2.0", "html_url": "https://github.com/perritodeck/perritodeck/releases/tag/v0.2.0", "draft": false, "prerelease": true, "assets": []},
            {"tag_name": "v0.1.0", "html_url": "https://github.com/perritodeck/perritodeck/releases/tag/v0.1.0", "draft": false, "prerelease": true}
        ]"#;
        let (api, request) = github("200 OK", body);
        assert_eq!(
            check_at(&api, "0.1.0").unwrap(),
            Some(Update {
                version: "0.2.0".into(),
                url: "https://github.com/perritodeck/perritodeck/releases/tag/v0.2.0".into(),
            })
        );
        let head = request.recv().unwrap();
        assert!(
            head.starts_with(
                "GET /repos/perritodeck/perritodeck/releases?per_page=10 HTTP/1.1\r\n"
            ),
            "{head}"
        );
        let head = head.to_ascii_lowercase();
        assert!(head.contains("user-agent: perritodeck\r\n"), "{head}");
        assert!(
            head.contains("accept: application/vnd.github+json\r\n"),
            "{head}"
        );

        let (api, _) = github("200 OK", body);
        assert_eq!(check_at(&api, "0.2.0").unwrap(), None);
    }

    #[test]
    fn says_why_the_check_failed() {
        let (api, _) = github("200 OK", "[]");
        assert_eq!(
            check_at(&api, "0.1.0").unwrap_err(),
            "No releases are published yet."
        );
        let (api, _) = github("404 Not Found", r#"{"message": "Not Found"}"#);
        assert_eq!(
            check_at(&api, "0.1.0").unwrap_err(),
            "No releases are published yet."
        );
        let (api, _) = github("403 Forbidden", r#"{"message": "rate limit"}"#);
        assert_eq!(check_at(&api, "0.1.0").unwrap_err(), "GitHub answered 403.");
        let (api, _) = github("200 OK", r#"{"tag_name": "v0.2.0"}"#);
        assert!(check_at(&api, "0.1.0")
            .unwrap_err()
            .starts_with("Unexpected answer from GitHub"));
    }

    #[test]
    fn says_when_github_limits_checks() {
        let limited = "GitHub is limiting checks from this network; try again in an hour.";
        let out = r#"{"message": "API rate limit exceeded for 192.0.2.1."}"#;
        let none_left = "X-RateLimit-Limit: 60\r\nX-RateLimit-Remaining: 0\r\n";
        for status in ["403 Forbidden", "429 Too Many Requests"] {
            let (api, _) = github_with(status, none_left, out);
            assert_eq!(check_at(&api, "0.1.0").unwrap_err(), limited, "{status}");
        }
        let (api, _) = github_with("429 Too Many Requests", "Retry-After: 60\r\n", "{}");
        assert_eq!(check_at(&api, "0.1.0").unwrap_err(), limited);
        // A 403 with requests left is something else, such as a blocked network.
        let (api, _) = github_with("403 Forbidden", "X-RateLimit-Remaining: 59\r\n", "{}");
        assert_eq!(check_at(&api, "0.1.0").unwrap_err(), "GitHub answered 403.");
    }
}
