//! In-app updates from versioned releases (every merge to main that passed CI).
//! Only the AppImage replaces itself; the download must carry
//! an Ed25519 signature from the release key, bound to its tag so an older
//! build cannot be replayed as a newer one.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{anyhow, bail, Context, Result};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};

use crate::fetch::download;

const REPO: &str = "MDHaarith/Linux-Android-AirDrop";
const ASSET: &str = "airbridge-x86_64.AppImage";
const MAX_SIZE: u64 = 512 * 1024 * 1024;
/// Release signing key; the private half lives only in the repo's `UPDATE_SIGNING_KEY` secret.
const PUBLIC_KEY: [u8; 32] = [
    0xaa, 0x73, 0xb8, 0xf3, 0x37, 0xfa, 0xca, 0xc6, 0x1a, 0x52, 0x6a, 0x52, 0xd7, 0x19, 0x49, 0x66,
    0x4f, 0x83, 0x99, 0xd8, 0xeb, 0x77, 0x6a, 0xe6, 0x57, 0x4a, 0xe2, 0x78, 0x0a, 0xf0, 0x49, 0x2f,
];

/// Set once a new AppImage is in place; `main` relaunches it after shutdown.
pub static RESTART: AtomicBool = AtomicBool::new(false);

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The AppImage file we are running from, if any.
pub fn appimage() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

#[derive(Serialize, Clone)]
pub struct Available {
    pub tag: String,
    pub page: String,
    #[serde(skip)]
    app_url: String,
    #[serde(skip)]
    sig_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn version(tag: &str) -> Option<[u64; 3]> {
    let mut parts = tag.strip_prefix('v')?.split('.');
    let number = |part: &str| {
        if part.is_empty()
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        part.parse().ok()
    };
    let parsed = [
        number(parts.next()?)?,
        number(parts.next()?)?,
        number(parts.next()?)?,
    ];
    parts.next().is_none().then_some(parsed)
}

/// The newest stable version that is newer than this one and ships a signed AppImage.
pub async fn check() -> Result<Option<Available>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
    let releases: Vec<Release> = serde_json::from_slice(&download(&url, 4 * 1024 * 1024).await?)
        .context("unexpected answer from GitHub")?;
    Ok(newest(releases, current_version()))
}

fn newest(releases: Vec<Release>, current: &str) -> Option<Available> {
    let current = version(&format!("v{current}"))?;
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| {
            let release_version = version(&r.tag_name).filter(|&v| v > current)?;
            let url = |name: &str| {
                r.assets
                    .iter()
                    .find(|a| a.name == name)
                    .map(|a| a.browser_download_url.clone())
            };
            Some((
                release_version,
                Available {
                    app_url: url(ASSET)?,
                    sig_url: url(&format!("{ASSET}.sig"))?,
                    tag: r.tag_name,
                    page: r.html_url,
                },
            ))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(_, available)| available)
}

/// What the signature covers: the file, then a newline and its tag.
fn verify(data: &[u8], tag: &str, signature: &[u8]) -> Result<()> {
    let mut message = Vec::with_capacity(data.len() + tag.len() + 1);
    message.extend_from_slice(data);
    message.push(b'\n');
    message.extend_from_slice(tag.as_bytes());
    UnparsedPublicKey::new(&ED25519, PUBLIC_KEY)
        .verify(&message, signature)
        .map_err(|_| anyhow!("the update's signature is not valid"))
}

/// Downloads, verifies and swaps in the new AppImage. Takes effect on restart.
pub async fn install(update: &Available) -> Result<()> {
    let target = appimage().ok_or_else(|| anyhow!("only the AppImage can update itself"))?;
    let signature = download(&update.sig_url, 1024).await?;
    let data = download(&update.app_url, MAX_SIZE).await?;
    verify(&data, &update.tag, &signature)?;
    if !data.starts_with(b"\x7fELF") {
        bail!("the update is not a Linux program");
    }
    let dir = target.parent().context("AppImage has no folder")?;
    let temp = dir.join(format!(".{ASSET}.update"));
    let written = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp)?;
        file.write_all(&data)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        file.sync_all()?;
        // The running copy keeps its open inode, so replacing the file is safe.
        std::fs::rename(&temp, &target)
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(err).with_context(|| format!("could not replace {}", target.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with the release key: `printf airbridge-update-selftest` + "\n" + "main-1".
    const SELFTEST_SIG: [u8; 64] = [
        0xb9, 0x13, 0xb4, 0x7a, 0x81, 0x54, 0xfb, 0xe7, 0x30, 0xf6, 0xc9, 0x51, 0xc0, 0x7b, 0x0d,
        0xc2, 0x16, 0x51, 0xfb, 0x6e, 0xed, 0x92, 0xb2, 0x8a, 0x4f, 0xf2, 0x81, 0x4a, 0x34, 0xb4,
        0x51, 0xc0, 0x0d, 0xf9, 0x06, 0x29, 0xf5, 0xeb, 0xbf, 0x0a, 0x31, 0x1a, 0x5f, 0x59, 0xee,
        0xdc, 0xb5, 0x41, 0x3c, 0x06, 0x3a, 0x61, 0xb9, 0xf8, 0x33, 0xa1, 0x38, 0x50, 0xcf, 0x06,
        0x5b, 0xf0, 0x67, 0x08,
    ];

    #[test]
    fn signature_binds_file_and_tag() {
        let data = b"airbridge-update-selftest";
        verify(data, "main-1", &SELFTEST_SIG).unwrap();
        assert!(verify(data, "main-2", &SELFTEST_SIG).is_err());
        assert!(verify(b"airbridge-update-selftesT", "main-1", &SELFTEST_SIG).is_err());
        assert!(verify(data, "main-1", &[0; 64]).is_err());
    }

    #[test]
    fn picks_the_newest_signed_stable_version() {
        let release = |tag: &str, names: &[&str]| Release {
            tag_name: tag.into(),
            html_url: format!("https://example/{tag}"),
            draft: false,
            prerelease: false,
            assets: names
                .iter()
                .map(|n| Asset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example/{tag}/{n}"),
                })
                .collect(),
        };
        let signed = [ASSET, "airbridge-x86_64.AppImage.sig"];
        let releases = || {
            vec![
                release("v9.0.0", &[ASSET]), // unsigned: skipped
                release("main-42", &signed),
                release("v0.1.5", &signed),
                release("v0.1.9", &signed),
                release("v0.1.10", &signed),
                release("v0.2.0-rc.1", &signed),
                release("v0.01.99", &signed),
                Release {
                    draft: true,
                    ..release("v1.0.0", &signed)
                },
                Release {
                    prerelease: true,
                    ..release("v2.0.0", &signed)
                },
            ]
        };
        assert_eq!(newest(releases(), "0.1.5").unwrap().tag, "v0.1.10");
        assert!(newest(releases(), "0.1.10").is_none());
        assert!(newest(releases(), "0.2.0").is_none());
        assert_eq!(
            newest(vec![release("v0.2.0", &signed)], "0.1.99")
                .unwrap()
                .tag,
            "v0.2.0"
        );
    }
}
