//! What the launcher asks GitHub: the newest release of the game, and its files. The
//! repository is public, so it asks without signing in.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::Read;

const RELEASES_URL: &str =
    "https://api.github.com/repos/TheAaronLawrence/minecraft-cardboard/releases?per_page=20";

pub struct Release {
    pub tag: String,
    pub assets: Vec<Asset>,
}

pub struct Asset {
    pub name: String,
    url: String,
    pub size: u64,
}

impl Release {
    /// The file whose name ends in `suffix`.
    pub fn asset(&self, suffix: &str) -> Result<&Asset> {
        self.assets
            .iter()
            .find(|a| a.name.ends_with(suffix))
            .with_context(|| format!("Release {} has no file ending in {suffix}", self.tag))
    }
}

/// A GET that succeeds only with a 200.
fn get(url: &str, accept: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let response = ureq::get(url)
        .header("Accept", accept)
        .header("User-Agent", "minecraft-cardboard-launcher")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("GET {url}"))?;
    match response.status().as_u16() {
        200 => Ok(response),
        // GitHub allows 60 API calls an hour from one address without signing in.
        403 | 429 => bail!("GitHub is limiting requests from this network; try again later"),
        status => bail!("GitHub answered {status} to {url}"),
    }
}

/// The newest release that isn't a draft, previews included.
pub fn latest_release() -> Result<Release> {
    let releases: Vec<Value> = get(RELEASES_URL, "application/vnd.github+json")?
        .body_mut()
        .read_json()?;
    let latest = releases
        .iter()
        .find(|r| r["draft"] != true)
        .context("The game has no releases yet")?;
    Ok(Release {
        tag: latest["tag_name"].as_str().unwrap_or_default().to_owned(),
        assets: latest["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| Asset {
                name: a["name"].as_str().unwrap_or_default().to_owned(),
                url: a["browser_download_url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                size: a["size"].as_u64().unwrap_or(0),
            })
            .collect(),
    })
}

/// A release file's bytes, calling `progress` with how many have arrived.
pub fn download(asset: &Asset, progress: impl FnMut(u64)) -> Result<Vec<u8>> {
    let mut response = get(&asset.url, "application/octet-stream")?;
    read_body(&mut response, &asset.name, progress)
}

/// A response's body, calling `progress` with how many bytes have arrived.
pub fn read_body(
    response: &mut ureq::http::Response<ureq::Body>,
    name: &str,
    mut progress: impl FnMut(u64),
) -> Result<Vec<u8>> {
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("downloading {name}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64);
    }
    Ok(bytes)
}
