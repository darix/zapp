use serde::Deserialize;
use zapp_core::firmware::{self, Firmware};

#[derive(Debug, thiserror::Error)]
pub enum OryxError {
    #[error("Not a valid Oryx URL")]
    InvalidUrl,
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Firmware error: {0}")]
    Firmware(#[from] zapp_core::ZappError),
}

#[derive(Deserialize)]
struct LatestResponse {
    latest: String,
}

/// Parse an Oryx URL into (layout_id, Option<revision_id>).
///
/// Accepted forms:
///   /voyager/layouts/:layoutId
///   /voyager/layouts/:layoutId/latest
///   /voyager/layouts/:layoutId/latest/0
///   /voyager/layouts/:layoutId/:revisionId
pub fn parse_url(url: &str) -> Result<(&str, Option<&str>), OryxError> {
    let path = url
        .strip_prefix("https://configure.zsa.io/")
        .ok_or(OryxError::InvalidUrl)?;

    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();

    if segments.len() < 3 || segments[1] != "layouts" {
        return Err(OryxError::InvalidUrl);
    }

    let layout_id = segments[2];

    let revision_id = match segments.get(3) {
        None => None,
        Some(&"latest") => None,
        Some(rev) => Some(*rev),
    };

    Ok((layout_id, revision_id))
}

/// Resolve a layout to a specific revision ID.
/// If `revision_id` is already provided, returns it directly.
/// Otherwise fetches the latest revision from Oryx.
pub fn resolve_revision(layout_id: &str, revision_id: Option<&str>) -> Result<String, OryxError> {
    if let Some(rev) = revision_id {
        return Ok(rev.to_string());
    }

    let url = format!("https://oryx.zsa.io/firmware/latest/{layout_id}");
    let resp: LatestResponse = reqwest::blocking::get(&url)?.error_for_status()?.json()?;

    Ok(resp.latest)
}

/// Fetch the latest revision ID for a layout.
pub fn fetch_latest_revision(layout_id: &str) -> Result<String, OryxError> {
    resolve_revision(layout_id, None)
}

/// Download firmware for a given revision.
pub fn download_firmware(revision_id: &str, alt: bool) -> Result<Firmware, OryxError> {
    let mut url = format!("https://oryx.zsa.io/firmware/{revision_id}");
    if alt {
        url.push_str("?alt=true");
    }

    let fw_bytes = reqwest::blocking::get(&url)?.error_for_status()?.bytes()?;

    Ok(firmware::load_firmware_from_bytes(&fw_bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_bare_layout() {
        let (layout, rev) = parse_url("https://configure.zsa.io/voyager/layouts/abcde").unwrap();
        assert_eq!(layout, "abcde");
        assert_eq!(rev, None);
    }

    #[test]
    fn test_parse_url_latest() {
        let (layout, rev) =
            parse_url("https://configure.zsa.io/voyager/layouts/abcde/latest").unwrap();
        assert_eq!(layout, "abcde");
        assert_eq!(rev, None);
    }

    #[test]
    fn test_parse_url_latest_with_zero() {
        let (layout, rev) =
            parse_url("https://configure.zsa.io/voyager/layouts/abcde/latest/0").unwrap();
        assert_eq!(layout, "abcde");
        assert_eq!(rev, None);
    }

    #[test]
    fn test_parse_url_specific_revision() {
        let (layout, rev) =
            parse_url("https://configure.zsa.io/moonlander/layouts/AbCdE/abc123").unwrap();
        assert_eq!(layout, "AbCdE");
        assert_eq!(rev, Some("abc123"));
    }

    #[test]
    fn test_parse_url_trailing_slash() {
        let (layout, rev) =
            parse_url("https://configure.zsa.io/voyager/layouts/abcde/").unwrap();
        assert_eq!(layout, "abcde");
        assert_eq!(rev, None);
    }

    #[test]
    fn test_parse_url_invalid_prefix() {
        assert!(parse_url("https://example.com/voyager/layouts/abcde").is_err());
    }

    #[test]
    fn test_parse_url_missing_layouts_segment() {
        assert!(parse_url("https://configure.zsa.io/voyager/abcde").is_err());
    }
}
