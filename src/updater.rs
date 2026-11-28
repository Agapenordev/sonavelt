//! Self-update from GitHub releases: find a newer tag, download the build for this system, swap
//! it in place of the running one, restart. Blocking calls; run them on a worker thread.

use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "Agapenordev/sonavelt";
/// Release asset holding the bare executable for this system.
#[cfg(windows)]
pub const ASSET: &str = "sonavelt-x86_64-windows.exe";
#[cfg(not(windows))]
pub const ASSET: &str = "sonavelt-x86_64-linux";
/// How a valid executable for this system starts.
const MAGIC: &[u8] = if cfg!(windows) { b"MZ" } else { b"\x7fELF" };
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
        .ok_or("The new release has no build for your system yet")?;
    Ok(Some(Release { version, url: url.to_string() }))
}

/// Download the release binary and replace `exe` with it. The running process keeps its copy;
/// the new one takes effect on restart.
pub fn install(r: &Release, exe: &Path) -> Result<(), String> {
    let resp = ureq::get(&r.url).set("User-Agent", "sonavelt-updater").call().map_err(|e| format!("Download failed: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader().take(200 << 20).read_to_end(&mut bytes).map_err(|e| format!("Download failed: {e}"))?;
    // never swap in something that isn't an executable for this system (an error page, a truncated file)
    if bytes.len() < 1 << 20 || !bytes.starts_with(MAGIC) {
        return Err("The download wasn't a valid Sonavelt build".into());
    }
    let tmp = exe.with_extension("new");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("Couldn't write the update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    replace(&tmp, exe)
}

/// Swap the downloaded file in. A rename is atomic: the old file stays intact until the new one is complete.
#[cfg(not(windows))]
fn replace(tmp: &Path, exe: &Path) -> Result<(), String> {
    std::fs::rename(tmp, exe).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        format!("Couldn't replace the app (is its folder writable?): {e}")
    })
}

/// Windows won't replace a running .exe, but it will rename it: move it aside, then swap in.
#[cfg(windows)]
fn replace(tmp: &Path, exe: &Path) -> Result<(), String> {
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old); // left over from the previous update
    std::fs::rename(exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(tmp);
        format!("Couldn't replace the app (is its folder writable?): {e}")
    })?;
    std::fs::rename(tmp, exe).map_err(|e| {
        let _ = std::fs::rename(&old, exe); // put the working copy back
        let _ = std::fs::remove_file(tmp);
        format!("Couldn't replace the app: {e}")
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

    // Talks to GitHub; run with `cargo test -- --ignored updater`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn updater_live_release() {
        assert!(super::check().unwrap().is_none(), "this build should be the latest release");
        // install the published binary over a scratch file, through the real download and checks
        let url = format!("https://github.com/{}/releases/latest/download/{}", super::REPO, super::ASSET);
        let dir = std::env::temp_dir().join(format!("sonavelt-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("sonavelt");
        std::fs::write(&exe, b"old").unwrap();
        super::install(&super::Release { version: "test".into(), url }, &exe).unwrap();
        let new = std::fs::read(&exe).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(new.starts_with(b"\x7fELF") && new.len() > 1 << 20);
    }

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
