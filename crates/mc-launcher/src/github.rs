//! What the launcher asks GitHub: signing in with the OAuth device flow, the newest
//! release, and its files. The repository is private, so every call carries the token the
//! player signed in with.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

pub const REPO: &str = "TheAaronLawrence/minecraft-cardboard";

/// The client ID of the GitHub OAuth App the launcher signs in through, which must have
/// Device Flow enabled. `CARDBOARD_GITHUB_CLIENT_ID`, at build time or when the launcher
/// runs, overrides it.
const CLIENT_ID: &str = "Ov23liktvY2wBSVkt1hI";

pub fn client_id() -> Option<String> {
    std::env::var("CARDBOARD_GITHUB_CLIENT_ID")
        .ok()
        .or(option_env!("CARDBOARD_GITHUB_CLIENT_ID").map(str::to_owned))
        .or(Some(CLIENT_ID.to_owned()))
        .filter(|id| !id.trim().is_empty())
}

/// What GitHub said when the token didn't work, so the launcher can ask to sign in again.
#[derive(Debug)]
pub struct Unauthorized;

impl std::fmt::Display for Unauthorized {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("GitHub didn't accept your sign-in; sign in again")
    }
}

impl std::error::Error for Unauthorized {}

/// A code for the player to enter at `uri`, while the launcher waits for them.
pub struct DeviceCode {
    pub user_code: String,
    pub uri: String,
    device_code: String,
    interval: u64,
    expires_in: u64,
}

/// POSTs a form to github.com's OAuth endpoints, which answer in JSON when asked to.
fn post_form(url: &str, form: &[(&str, &str)]) -> Result<Value> {
    let mut response = ureq::post(url)
        .header("Accept", "application/json")
        .send_form(form.iter().copied())
        .with_context(|| format!("POST {url}"))?;
    Ok(response.body_mut().read_json()?)
}

/// Step 1 of the device flow: a code for the player to enter.
pub fn request_code(client_id: &str) -> Result<DeviceCode> {
    let json = post_form(
        "https://github.com/login/device/code",
        &[("client_id", client_id), ("scope", "repo")],
    )?;
    if let Some(error) = json["error"].as_str() {
        bail!(
            "GitHub sign-in failed: {}",
            json["error_description"].as_str().unwrap_or(error)
        );
    }
    let text = |key: &str| -> Result<String> {
        Ok(json[key]
            .as_str()
            .with_context(|| format!("GitHub's device code had no {key}"))?
            .to_owned())
    };
    Ok(DeviceCode {
        user_code: text("user_code")?,
        uri: text("verification_uri")?,
        device_code: text("device_code")?,
        interval: json["interval"].as_u64().unwrap_or(5),
        expires_in: json["expires_in"].as_u64().unwrap_or(900),
    })
}

/// Step 2: asks every `interval` seconds until the player has entered the code, and
/// returns the token.
pub fn wait_for_token(client_id: &str, code: &DeviceCode) -> Result<String> {
    let mut interval = code.interval;
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in);
    loop {
        std::thread::sleep(Duration::from_secs(interval));
        if std::time::Instant::now() > deadline {
            bail!("The sign-in code expired; sign in again");
        }
        let json = post_form(
            "https://github.com/login/oauth/access_token",
            &[
                ("client_id", client_id),
                ("device_code", &code.device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ],
        )?;
        if let Some(token) = json["access_token"].as_str() {
            return Ok(token.to_owned());
        }
        match json["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval = json["interval"].as_u64().unwrap_or(interval + 5),
            Some("expired_token") => bail!("The sign-in code expired; sign in again"),
            Some("access_denied") => bail!("Sign-in was cancelled on GitHub"),
            Some(error) => bail!(
                "GitHub sign-in failed: {}",
                json["error_description"].as_str().unwrap_or(error)
            ),
            None => bail!("GitHub sent no token"),
        }
    }
}

/// A GET to the GitHub API with the player's token. A 401 is `Unauthorized`.
fn api(token: &str, url: &str, accept: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let response = ureq::get(url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", accept)
        .header("User-Agent", "minecraft-cardboard-launcher")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("GET {url}"))?;
    match response.status().as_u16() {
        200 => Ok(response),
        401 => Err(Unauthorized.into()),
        // A private repository is a 404 to an account that can't see it.
        404 => bail!("Your GitHub account can't see {REPO}; ask its owner for access"),
        status => bail!("GitHub answered {status} to {url}"),
    }
}

/// The signed-in account's login.
pub fn login(token: &str) -> Result<String> {
    let json: Value = api(token, "https://api.github.com/user", "application/json")?
        .body_mut()
        .read_json()?;
    Ok(json["login"].as_str().unwrap_or_default().to_owned())
}

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

/// The newest release that isn't a draft, previews included.
pub fn latest_release(token: &str) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
    let mut response = api(token, &url, "application/vnd.github+json")?;
    let releases: Vec<Value> = response.body_mut().read_json()?;
    let latest = releases
        .iter()
        .find(|r| r["draft"] != true)
        .with_context(|| format!("{REPO} has no releases yet"))?;
    Ok(Release {
        tag: latest["tag_name"].as_str().unwrap_or_default().to_owned(),
        assets: latest["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| Asset {
                name: a["name"].as_str().unwrap_or_default().to_owned(),
                url: a["url"].as_str().unwrap_or_default().to_owned(),
                size: a["size"].as_u64().unwrap_or(0),
            })
            .collect(),
    })
}

/// A release file's bytes, calling `progress` with how many have arrived. GitHub redirects
/// to its file storage, and the token stays behind (ureq drops it on redirects).
pub fn download(token: &str, asset: &Asset, progress: impl FnMut(u64)) -> Result<Vec<u8>> {
    let mut response = api(token, &asset.url, "application/octet-stream")?;
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
