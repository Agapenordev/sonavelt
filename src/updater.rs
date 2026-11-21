//! Self-update from GitHub releases: find a newer tag, download its Linux binary, swap it in
//! place of the running one, restart. Blocking calls; run them on a worker thread.

use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "Agapenordev/sonavelt";
/// Release asset holding the bare executable.
pub const ASSET: &str = "sonavelt-x86_64-linux";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// The latest release if it is newer than this build, `None` when already up to date.
pub fn check() -> Result<Option<Release>, String> {
    let resp = ureq::get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", "sonavelt-updater")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10))
        .call();
    let json: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Ok(None), // no releases yet
        Err(e) => return Err(format!("Couldn't reach GitHub: {e}")),
    };
    let version = json["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    if !newer(&version, VERSION) {
        return Ok(None);
    }
    let url = json["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"] == ASSET))
        .and_then(|x| x["browser_download_url"].as_str())
        .ok_or("The new release has no Linux build yet")?;
    Ok(Some(Release { version, url: url.to_string() }))
}

/// Download the release binary and replace `exe` with it. The running process keeps its copy;
/// the new one takes effect on restart.
pub fn install(r: &Release, exe: &Path) -> Result<(), String> {
    let resp = ureq::get(&r.url).set("User-Agent", "sonavelt-updater").call().map_err(|e| format!("Download failed: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader().take(200 << 20).read_to_end(&mut bytes).map_err(|e| format!("Download failed: {e}"))?;
    // never swap in something that isn't a Linux executable (an error page, a truncated file)
    if bytes.len() < 1 << 20 || !bytes.starts_with(b"\x7fELF") {
        return Err("The download wasn't a valid Sonavelt build".into());
    }
    let tmp = exe.with_extension("new");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Couldn't write the update: {e}"))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    // rename is atomic: the old file stays intact until the new one is complete
    std::fs::rename(&tmp, exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Couldn't replace the app (is its folder writable?): {e}")
    })
}

pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("sonavelt"))
}

/// Numeric dotted-version comparison: is `a` newer than `b`?
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (a, b) = (parse(a), parse(b));
    !a.is_empty() && a.iter().any(|&x| x > 0) && a > b
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn version_ordering() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("0.10.0", "0.9.3"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
        assert!(!newer("", "0.1.0"));
    }
}
