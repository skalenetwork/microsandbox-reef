use anyhow::{Context, Result, bail};
use semver::Version;
use sha2::{Digest as _, Sha256};
use std::io::{ErrorKind, IsTerminal, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

const REPO: &str = "https://github.com/skalenetwork/reef";
const TTL: u64 = 24 * 60 * 60;
const WAIT: Duration = Duration::from_secs(2);

pub struct Notice {
    path: PathBuf,
    known: Option<Version>,
    refresh: Option<JoinHandle<Option<Version>>>,
}

impl Notice {
    pub fn start(dir: &Path) -> Option<Self> {
        if muted() {
            return None;
        }
        let path = dir.join("update.check");
        let check = load(&path);
        let fresh = check
            .as_ref()
            .is_some_and(|&(_, at)| now().saturating_sub(at) < TTL);
        Some(Self {
            path,
            known: check.map(|(latest, _)| latest),
            refresh: (!fresh).then(|| tokio::spawn(async { latest().await.ok() })),
        })
    }

    pub async fn finish(self) {
        let fetched = match self.refresh {
            Some(task) => tokio::time::timeout(WAIT, task)
                .await
                .ok()
                .and_then(Result::ok)
                .flatten(),
            None => None,
        };
        if let Some(latest) = &fetched {
            save(&self.path, latest);
        }
        if let Some(latest) = fetched.or(self.known)
            && latest > current()
        {
            eprintln!("reef {latest} available — run: reef update");
        }
    }
}

pub async fn run() -> Result<()> {
    let current = current();
    let latest = latest().await?;
    if latest <= current {
        println!("reef {current} is the latest release");
        return Ok(());
    }
    let asset = format!("reef-{}.tar.gz", target()?);
    let base = format!("{REPO}/releases/download/v{latest}");
    let archive = get(&format!("{base}/{asset}")).await?;
    let checksums = get(&format!("{base}/checksums.sha256")).await?;
    verify(&archive, &checksums, &asset)?;
    replace(&unpack(&archive)?)?;
    println!("reef {latest} installed");
    Ok(())
}

fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

fn muted() -> bool {
    !std::io::stderr().is_terminal()
        || std::env::var_os("CI").is_some()
        || std::env::var_os("REEF_NO_UPDATE_CHECK").is_some()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load(path: &Path) -> Option<(Version, u64)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (latest, at) = raw.trim().split_once(' ')?;
    Some((Version::parse(latest).ok()?, at.parse().ok()?))
}

fn save(path: &Path, latest: &Version) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, format!("{latest} {}", now()));
    }
}

async fn latest() -> Result<Version> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(WAIT)
        .build()?
        .head(format!("{REPO}/releases/latest"))
        .send()
        .await?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .context("no redirect to the latest release")?
        .to_str()?;
    let (_, tag) = location
        .rsplit_once("/tag/v")
        .context("unexpected latest release url")?;
    Ok(Version::parse(tag)?)
}

async fn get(url: &str) -> Result<Vec<u8>> {
    let response = reqwest::get(url)
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("cannot download {url}"))?;
    Ok(response.bytes().await?.to_vec())
}

fn target() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        (os, arch) => bail!("no reef release for {os} {arch}"),
    })
}

fn verify(archive: &[u8], checksums: &[u8], asset: &str) -> Result<()> {
    let expected = str::from_utf8(checksums)?
        .lines()
        .find_map(|line| line.strip_suffix(asset)?.split_whitespace().next())
        .with_context(|| format!("no checksum published for {asset}"))?;
    let actual = format!("{:x}", Sha256::digest(archive));
    if actual != expected {
        bail!("checksum mismatch for {asset}");
    }
    Ok(())
}

fn unpack(archive: &[u8]) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_os_str() == "reef" {
            let mut binary = Vec::new();
            entry.read_to_end(&mut binary)?;
            return Ok(binary);
        }
    }
    bail!("release archive holds no reef binary")
}

fn replace(binary: &[u8]) -> Result<()> {
    let path = std::env::current_exe()?;
    let staged = path.with_file_name(".reef.update");
    let install = || {
        std::fs::write(&staged, binary)?;
        std::fs::set_permissions(&staged, PermissionsExt::from_mode(0o755))?;
        std::fs::rename(&staged, &path)
    };
    if let Err(err) = install() {
        let _ = std::fs::remove_file(&staged);
        if err.kind() == ErrorKind::PermissionDenied {
            bail!("cannot replace {}: re-run with sudo", path.display());
        }
        return Err(err).with_context(|| format!("cannot replace {}", path.display()));
    }
    Ok(())
}
