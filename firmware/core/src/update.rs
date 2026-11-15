//! Update notification: read a release manifest (ESP Web Tools format) and compare versions. Updates are
//! offered, never installed automatically.

use crate::json::{parse, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Version without a leading `v`.
    pub version: String,
    /// Absolute URL of the firmware image.
    pub url: String,
}

fn semver(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let t = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    it.next().is_none().then_some(t)
}

/// True when `latest` is a plain `x.y.z` release above `current` (pre-releases are ignored).
pub fn newer(current: &str, latest: &str) -> bool {
    match (semver(current), semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Version and image URL from a manifest fetched from `manifest_url` (relative part paths are resolved
/// against it). `None` for anything malformed.
pub fn parse_manifest(text: &str, manifest_url: &str) -> Option<UpdateInfo> {
    let v = parse(text).ok()?;
    let version = v.get("version")?.as_str()?.trim_start_matches('v').to_string();
    semver(&version)?;
    let Some(Value::Arr(builds)) = v.get("builds") else { return None };
    let Some(Value::Arr(parts)) = builds.first()?.get("parts") else { return None };
    let path = parts.first()?.get("path")?.as_str()?;
    let url = if path.starts_with("https://") || path.starts_with("http://") {
        path.to_string()
    } else {
        let base = manifest_url.rsplit_once('/')?.0;
        format!("{base}/{path}")
    };
    Some(UpdateInfo { version, url })
}
