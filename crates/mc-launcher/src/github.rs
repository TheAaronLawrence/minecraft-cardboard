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
    let latest = newest(&releases).context("The game has no releases yet")?;
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

/// The most recently published of `releases` that isn't a draft. GitHub doesn't list them
/// newest first: it put preview.9 before preview.10. Its timestamps are UTC in one ISO 8601
/// form, so they sort as text.
fn newest(releases: &[Value]) -> Option<&Value> {
    releases
        .iter()
        .filter(|r| r["draft"] != true)
        .max_by_key(|r| r["published_at"].as_str().unwrap_or_default())
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

#[cfg(test)]
mod tests {
    /// The newest release by when it was published, not by where GitHub lists it or by its
    /// tag sorted as text, and never a draft.
    #[test]
    fn newest_release_is_the_last_published() {
        let releases = serde_json::json!([
            {"tag_name": "v0.0.1-preview.9", "draft": false, "published_at": "2026-11-20T07:03:33Z"},
            {"tag_name": "v0.0.1-preview.11", "draft": true, "published_at": null},
            {"tag_name": "v0.0.1-preview.10", "draft": false, "published_at": "2026-11-20T07:26:50Z"},
            {"tag_name": "v0.0.1-preview.8", "draft": false, "published_at": "2026-11-19T19:12:03Z"},
        ]);
        let newest = super::newest(releases.as_array().unwrap()).unwrap();
        assert_eq!(newest["tag_name"], "v0.0.1-preview.10");
    }
}
