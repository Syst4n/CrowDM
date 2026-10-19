//! Owner-controlled announcements are independent of GitHub commits/releases.
use crate::state::AppState;
use crowdm_engine::{Checksum, ChecksumAlgo, NewJob};
use serde::{Deserialize, Serialize};
use tauri::State;
const FEED: &str = "https://raw.githubusercontent.com/Syst4n/CrowDM/main/updates/windows-x64.json";
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub whats_new: Vec<String>,
    pub installer_url: String,
    pub sha256: String,
    pub extension_version: String,
}
#[derive(Deserialize)]
struct Feed {
    schema: u32,
    release: Option<Release>,
}
fn version(s: &str) -> Option<(u32, u32, u32)> {
    let mut p = s.split('.');
    let v = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    p.next().is_none().then_some(v)
}
fn valid(r: &Release) -> bool {
    version(&r.version).is_some()
        && version(&r.extension_version).is_some()
        && r.sha256.len() == 64
        && r.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        && r.installer_url
            == format!("https://github.com/Syst4n/CrowDM/releases/download/v{}/CrowDM_{}_x64-setup.exe", r.version, r.version)
        && r.whats_new.len() <= 40
        && r.whats_new.iter().all(|s| s.len() <= 1000)
}
async fn json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let mut r = crowdm_engine::http::build_client(None)
        .map_err(|_| "Could not create an update connection.")?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("Cache-Control", "no-cache")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Try again later.")?;
    if !r.status().is_success() {
        return Err(format!("GitHub returned HTTP {}. Try again later.", r.status().as_u16()));
    }
    let mut bytes = vec![];
    while let Some(chunk) = r.chunk().await.map_err(|_| "Could not read GitHub's response.")? {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("GitHub response exceeded the size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "GitHub returned an invalid response.".into())
}
#[tauri::command]
pub async fn check_update() -> Result<Option<Release>, String> {
    let f: Feed = json(FEED).await?;
    if f.schema != 1 {
        return Err("This update feed needs a newer CrowDM version.".into());
    }
    match f.release {
        Some(r) if !valid(&r) => Err("The update announcement failed validation.".into()),
        Some(r) if version(&r.version) > version(env!("CARGO_PKG_VERSION")) => Ok(Some(r)),
        _ => Ok(None),
    }
}
#[tauri::command]
pub async fn download_update(state: State<'_, AppState>) -> Result<uuid::Uuid, String> {
    let r = check_update().await?.ok_or("No announced update is available.")?;
    if let Some(j) = state.engine.jobs().iter().find(|j| {
        j.job.url == r.installer_url
            && j.job.checksum.as_ref().is_some_and(|c| c.algo == ChecksumAlgo::Sha256 && c.expected.eq_ignore_ascii_case(&r.sha256))
            && (j.job.state != crowdm_engine::JobState::Completed || j.job.target_path().is_file())
    }) {
        if matches!(j.job.state, crowdm_engine::JobState::Paused | crowdm_engine::JobState::Failed | crowdm_engine::JobState::Queued) {
            state.engine.start(j.job.id).map_err(|_| "Could not resume the installer download.")?;
        }
        return Ok(j.job.id);
    }
    state
        .engine
        .add(NewJob {
            url: r.installer_url,
            file_name: Some(format!("CrowDM_{}_x64-setup.exe", r.version)),
            dir: Some(state.engine.settings().download_root.join("Updates")),
            checksum: Some(Checksum { algo: ChecksumAlgo::Sha256, expected: r.sha256.to_ascii_lowercase() }),
            start: true,
            ..Default::default()
        })
        .map_err(|_| "Could not add the installer download.".into())
}
#[tauri::command]
pub async fn install_update(state: State<'_, AppState>, id: uuid::Uuid) -> Result<(), String> {
    let r = check_update().await?.ok_or("This update is no longer announced. Check for updates again.")?;
    let j = state.engine.job(id).ok_or("The installer download was removed.")?;
    if j.job.state != crowdm_engine::JobState::Completed || j.job.url != r.installer_url {
        return Err("Finish the announced installer download before installing.".into());
    }
    let path = j.job.target_path();
    let verify = path.clone();
    let digest = tauri::async_runtime::spawn_blocking(move || crowdm_engine::checksum::file_digest(&verify, ChecksumAlgo::Sha256))
        .await
        .map_err(|_| "Could not verify the installer.")?
        .map_err(|_| "The downloaded installer is missing or unreadable.")?;
    if !digest.eq_ignore_ascii_case(&r.sha256) {
        return Err("The installer no longer matches its checksum. Download a fresh copy.".into());
    }
    state.engine.pause_all();
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        while state.engine.jobs().iter().any(|j| j.job.state == crowdm_engine::JobState::Downloading) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "Downloads are still stopping. Try installing again after they pause.")?;
    #[cfg(windows)]
    crate::platform::open_path(&path)?;
    Ok(())
}
#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    download_count: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Count {
    version: String,
    downloads: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    total: u64,
    versions: Vec<Count>,
}
#[tauri::command]
pub async fn download_counts() -> Result<Counts, String> {
    let mut versions = vec![];
    let mut total = 0;
    for page in 1..=10 {
        let rows: Vec<GithubRelease> =
            json(&format!("https://api.github.com/repos/Syst4n/CrowDM/releases?per_page=100&page={page}")).await?;
        let done = rows.len() < 100;
        for r in rows.into_iter().filter(|r| !r.draft) {
            let count = r
                .assets
                .iter()
                .filter(|a| a.name.ends_with("_x64-setup.exe") || a.name.ends_with("_x64-portable.zip"))
                .map(|a| a.download_count)
                .sum::<u64>();
            total += count;
            versions.push(Count { version: r.tag_name, downloads: count });
        }
        if done {
            break;
        }
    }
    Ok(Counts { total, versions })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn announcements_validate_version_asset_and_digest() {
        let mut r = Release {
            version: "1.0.0".into(),
            extension_version: "1.0.0".into(),
            whats_new: vec!["New downloads".into()],
            installer_url: "https://github.com/Syst4n/CrowDM/releases/download/v1.0.0/CrowDM_1.0.0_x64-setup.exe".into(),
            sha256: "ab".repeat(32),
        };
        assert!(valid(&r));
        assert!(version("1.10.0") > version("1.9.9"));
        assert!(version("1.0.0.exe").is_none());
        r.installer_url = "https://example.org/malware.exe".into();
        assert!(!valid(&r));
    }
}
