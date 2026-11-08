//! Parsing and validation for the GitHub release metadata used by the updater.

use serde_json::Value;

const REPOSITORY: &str = "shentharo/ArcaneVR";
const ASSET_NAME: &str = "ArcaneVR.exe";
const MAX_ASSET_SIZE: u64 = 200 * 1024 * 1024;
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub executable_url: String,
    pub sha256: String,
    pub notes_url: String,
    pub size: u64,
}

/// Parse GitHub's latest-release response, returning a release only when it is
/// newer than `current`. Invalid or unverifiable metadata is an error.
pub fn parse_release(value: &Value, current: &str) -> Result<Option<Release>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "release metadata must be a JSON object".to_string())?;

    let tag = required_string(object.get("tag_name"), "tag_name")?;
    let version = parse_version(tag).ok_or_else(|| {
        "release tag must be a three-part numeric version, optionally prefixed with v".to_string()
    })?;

    let draft = required_bool(object.get("draft"), "draft")?;
    let prerelease = required_bool(object.get("prerelease"), "prerelease")?;
    if draft || prerelease {
        return Ok(None);
    }

    let current_version = parse_version(current).ok_or_else(|| {
        "current version must be a three-part numeric version, optionally prefixed with v"
            .to_string()
    })?;
    if version <= current_version {
        return Ok(None);
    }

    let assets = object
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| "release assets must be an array".to_string())?;
    let mut matching = assets
        .iter()
        .filter(|asset| asset.get("name").and_then(Value::as_str) == Some(ASSET_NAME));
    let asset = matching
        .next()
        .ok_or_else(|| format!("release is missing required asset {ASSET_NAME}"))?;
    if matching.next().is_some() {
        return Err(format!("release contains more than one {ASSET_NAME} asset"));
    }
    let asset = asset
        .as_object()
        .ok_or_else(|| format!("{ASSET_NAME} asset metadata must be an object"))?;

    let size = asset
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{ASSET_NAME} asset size must be a nonnegative integer"))?;
    if size == 0 || size > MAX_ASSET_SIZE {
        return Err(format!(
            "{ASSET_NAME} asset size must be between 1 byte and 200 MiB"
        ));
    }

    let expected_url =
        format!("https://github.com/{REPOSITORY}/releases/download/{tag}/{ASSET_NAME}");
    let actual_url = required_string(asset.get("browser_download_url"), "browser_download_url")?;
    if actual_url != expected_url {
        return Err(format!(
            "{ASSET_NAME} download URL does not match the canonical GitHub release URL"
        ));
    }

    let digest = required_string(asset.get("digest"), "digest")?;
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("{ASSET_NAME} asset digest must use sha256"))?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{ASSET_NAME} asset digest must contain exactly 64 hexadecimal characters"
        ));
    }

    Ok(Some(Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_string(),
        executable_url: expected_url,
        sha256: hex.to_ascii_lowercase(),
        notes_url: format!("https://github.com/{REPOSITORY}/releases/tag/{tag}"),
        size,
    }))
}

fn required_string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("release metadata field {field} must be a nonempty string"))
}

fn required_bool(value: Option<&Value>, field: &str) -> Result<bool, String> {
    value
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("release metadata field {field} must be a boolean"))
}

fn parse_version(input: &str) -> Option<(u64, u64, u64)> {
    let numeric = input.strip_prefix('v').unwrap_or(input);
    let mut parts = numeric.split('.');
    let major = parse_component(parts.next()?)?;
    let minor = parse_component(parts.next()?)?;
    let patch = parse_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_component(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(tag: &str) -> Value {
        json!({
            "tag_name": tag,
            "draft": false,
            "prerelease": false,
            "assets": [{
                "name": "ArcaneVR.exe",
                "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/{tag}/ArcaneVR.exe"),
                "digest": format!("sha256:{}", "a".repeat(64)),
                "size": 1234
            }]
        })
    }

    #[test]
    fn accepts_newer_release_and_constructs_notes_url() {
        let release = parse_release(&manifest("v1.1.1"), "1.1.0")
            .unwrap()
            .unwrap();
        assert_eq!(release.version, "1.1.1");
        assert_eq!(
            release.notes_url,
            format!("https://github.com/{REPOSITORY}/releases/tag/v1.1.1")
        );
        assert_eq!(release.size, 1234);
        assert_eq!(release.sha256, "a".repeat(64));
    }

    #[test]
    fn ignores_older_and_equal_releases() {
        assert_eq!(parse_release(&manifest("1.0.9"), "1.1.0").unwrap(), None);
        assert_eq!(parse_release(&manifest("v1.1.0"), "1.1.0").unwrap(), None);
    }

    #[test]
    fn ignores_drafts_and_prereleases() {
        let mut draft = manifest("1.2.0");
        draft["draft"] = json!(true);
        assert_eq!(parse_release(&draft, "1.1.0").unwrap(), None);
        let mut prerelease = manifest("1.2.0");
        prerelease["prerelease"] = json!(true);
        assert_eq!(parse_release(&prerelease, "1.1.0").unwrap(), None);
    }

    #[test]
    fn rejects_missing_digest_and_untrusted_url() {
        let mut missing_digest = manifest("1.2.0");
        missing_digest["assets"][0]
            .as_object_mut()
            .unwrap()
            .remove("digest");
        assert!(parse_release(&missing_digest, "1.1.0")
            .unwrap_err()
            .contains("digest"));

        let mut malicious = manifest("1.2.0");
        malicious["assets"][0]["browser_download_url"] = json!("https://evil.example/ArcaneVR.exe");
        assert!(parse_release(&malicious, "1.1.0")
            .unwrap_err()
            .contains("URL"));
    }

    #[test]
    fn rejects_malicious_tags_and_bad_versions() {
        for tag in ["../1.2.0", "1.2.0?x=y", "1.2.0-rc.1", "1.2", "01.2.3"] {
            assert!(
                parse_release(&manifest(tag), "1.1.0").is_err(),
                "accepted {tag}"
            );
        }
        assert!(parse_release(&manifest("1.2.0"), "1.1.0-beta").is_err());
    }

    #[test]
    fn rejects_zero_or_oversized_asset() {
        let mut zero = manifest("1.2.0");
        zero["assets"][0]["size"] = json!(0);
        assert!(parse_release(&zero, "1.1.0").is_err());
        let mut huge = manifest("1.2.0");
        huge["assets"][0]["size"] = json!(MAX_ASSET_SIZE + 1);
        assert!(parse_release(&huge, "1.1.0").is_err());
    }
}
