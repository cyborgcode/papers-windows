//! Updates for Papers.
//!
//! Every release publishes `latest.json` next to its installer and portable zip:
//!
//! ```json
//! { "version": "1.2.0", "published": "2026-11-19", "notes": "…",
//!   "notes_url": "https://github.com/…/releases/tag/v1.2.0",
//!   "installer": { "name": "Papers-1.2.0-windows-x64-setup.exe", "url": "…", "size": 123, "sha256": "…" },
//!   "portable":  { "name": "Papers-1.2.0-windows-x64-portable.zip", "url": "…", "size": 456, "sha256": "…" } }
//! ```
//!
//! GitHub serves the newest release's copy at `releases/latest/download/latest.json`, so a
//! check is one small HTTPS request and needs no API token. Packages are downloaded over
//! HTTPS, their size and SHA-256 are checked against the manifest, and only then are they
//! installed: the setup program runs silently, or the portable zip is unpacked over the
//! portable folder with a rollback if anything fails.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const REPOSITORY: &str = "cyborgcode/papers-windows";
pub const RELEASES_URL: &str = "https://github.com/cyborgcode/papers-windows/releases";
/// Marker file that identifies a portable installation (next to `papers.exe`).
pub const PORTABLE_MARKER: &str = "portable.txt";
/// Name of the updater executable shipped next to `papers.exe`.
pub const UPDATER_EXE: &str = if cfg!(windows) { "papers-updater.exe" } else { "papers-updater" };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error(match e {
            ureq::Error::StatusCode(404) => "No release information was found (HTTP 404).".into(),
            ureq::Error::StatusCode(code) => format!("The server answered with HTTP {code}."),
            other => format!("Could not reach the update server: {other}"),
        })
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, Error> {
    Err(Error(message.into()))
}

/// A release package listed in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub published: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub notes_url: String,
    pub installer: Option<Asset>,
    pub portable: Option<Asset>,
}

impl Manifest {
    pub fn asset_for(&self, kind: &InstallKind) -> Option<&Asset> {
        match kind {
            InstallKind::Installer { .. } => self.installer.as_ref(),
            InstallKind::Portable { .. } => self.portable.as_ref(),
            InstallKind::Unmanaged => None,
        }
    }
}

/// A `major.minor.patch` version; a pre-release suffix (`-beta.1`) sorts before the release.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub numbers: [u64; 3],
    /// `true` for plain releases, so they sort after pre-releases of the same numbers.
    pub release: bool,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match s.split_once(['-', '+']) {
            Some((core, rest)) => (core, !rest.is_empty() && s.as_bytes()[core.len()] == b'-'),
            None => (s, false),
        };
        let mut numbers = [0u64; 3];
        let mut parts = core.split('.');
        for slot in numbers.iter_mut() {
            match parts.next() {
                Some(p) => *slot = p.parse().ok()?,
                None => break,
            }
        }
        if parts.next().is_some() || core.is_empty() {
            return None;
        }
        Some(Version { numbers, release: !pre })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c] = self.numbers;
        write!(f, "{a}.{b}.{c}")
    }
}

/// Whether `candidate` is newer than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The manifest URL, overridable with `PAPERS_UPDATE_MANIFEST` for testing.
pub fn manifest_url() -> String {
    std::env::var("PAPERS_UPDATE_MANIFEST").unwrap_or_else(|_| format!("{RELEASES_URL}/latest/download/latest.json"))
}

fn agent() -> ureq::Agent {
    #[allow(unused_mut)]
    let mut config = ureq::Agent::config_builder()
        .user_agent(concat!("Papers/", env!("CARGO_PKG_VERSION"), " (update check)"))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_per_read(Some(Duration::from_secs(60)))
        .max_redirects(10);
    #[cfg(windows)]
    {
        // Trust what Windows trusts, so corporate TLS inspection proxies keep working.
        use ureq::tls::{RootCerts, TlsConfig};
        config = config.tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build());
    }
    config.build().into()
}

fn check_url(url: &str) -> Result<(), Error> {
    let allow_http = std::env::var_os("PAPERS_UPDATE_ALLOW_HTTP").is_some();
    if url.starts_with("https://") || (allow_http && url.starts_with("http://")) {
        Ok(())
    } else {
        err(format!("Refusing to download from a non-HTTPS address: {url}"))
    }
}

/// Download and parse a manifest.
pub fn fetch_manifest(url: &str) -> Result<Manifest, Error> {
    check_url(url)?;
    let mut response = agent().get(url).call()?;
    let text = response.body_mut().with_config().limit(1024 * 1024).read_to_string()?;
    serde_json::from_str(&text).map_err(|e| Error(format!("The release information is not valid: {e}")))
}

/// The newer release, if there is one.
pub fn check(current_version: &str) -> Result<Option<Manifest>, Error> {
    let manifest = fetch_manifest(&manifest_url())?;
    if Version::parse(&manifest.version).is_none() {
        return err(format!("The release information has an invalid version: {:?}", manifest.version));
    }
    Ok(is_newer(&manifest.version, current_version).then_some(manifest))
}

/// How the running copy of Papers was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by the setup program (an Inno Setup uninstaller sits next to the executable).
    Installer { dir: PathBuf },
    /// Unzipped portable copy (has a `portable.txt` marker).
    Portable { dir: PathBuf },
    /// A development build or a package manager's copy: updates are not applied in place.
    Unmanaged,
}

impl InstallKind {
    pub fn detect(exe: &Path) -> InstallKind {
        let Some(dir) = exe.parent() else { return InstallKind::Unmanaged };
        if dir.join(PORTABLE_MARKER).is_file() {
            InstallKind::Portable { dir: dir.to_path_buf() }
        } else if dir.join("unins000.exe").is_file() {
            InstallKind::Installer { dir: dir.to_path_buf() }
        } else {
            InstallKind::Unmanaged
        }
    }

    pub fn current() -> InstallKind {
        std::env::current_exe().map(|exe| Self::detect(&exe)).unwrap_or(InstallKind::Unmanaged)
    }

    pub fn id(&self) -> &'static str {
        match self {
            InstallKind::Installer { .. } => "installer",
            InstallKind::Portable { .. } => "portable",
            InstallKind::Unmanaged => "unmanaged",
        }
    }
}

/// Where downloads and the result of the last update are kept.
pub fn state_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Papers").join("updates")
}

/// A file name that cannot escape the download directory.
fn safe_file_name(name: &str) -> Result<String, Error> {
    let clean: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '\0')).collect();
    if clean.is_empty() || clean.starts_with('.') {
        return err(format!("Invalid package name: {name:?}"));
    }
    Ok(clean)
}

/// SHA-256 of a file, as lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Check a downloaded package against the manifest.
pub fn verify(path: &Path, asset: &Asset) -> Result<(), Error> {
    let size = fs::metadata(path)?.len();
    if size != asset.size {
        return err(format!("The download has the wrong size ({size} bytes instead of {}).", asset.size));
    }
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(asset.sha256.trim()) {
        return err("The download is damaged: its checksum does not match the release.");
    }
    Ok(())
}

/// Download progress: bytes so far and the total.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64);

/// Download a package into `dir`, verifying its size and checksum. Reuses an earlier
/// complete download of the same file.
pub fn download(asset: &Asset, dir: &Path, progress: Progress, cancel: &AtomicBool) -> Result<PathBuf, Error> {
    check_url(&asset.url)?;
    fs::create_dir_all(dir)?;
    let name = safe_file_name(&asset.name)?;
    let target = dir.join(&name);
    if target.is_file() && verify(&target, asset).is_ok() {
        progress(asset.size, asset.size);
        return Ok(target);
    }
    let partial = dir.join(format!("{name}.part"));
    let result = (|| {
        let mut response = agent().get(&asset.url).call()?;
        let mut reader = response.body_mut().with_config().limit(asset.size + 1).reader();
        let mut file = File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        let mut last_report = Instant::now() - Duration::from_secs(1);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return err("Cancelled.");
            }
            let n = reader.read(&mut buf).map_err(|e| Error(format!("The download was interrupted: {e}")))?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > asset.size {
                return err("The download is larger than expected.");
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            if last_report.elapsed() > Duration::from_millis(100) {
                progress(done, asset.size);
                last_report = Instant::now();
            }
        }
        file.sync_all()?;
        drop(file);
        progress(done, asset.size);
        if done != asset.size {
            return err(format!("The download is incomplete ({done} of {} bytes).", asset.size));
        }
        if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(asset.sha256.trim()) {
            return err("The download is damaged: its checksum does not match the release.");
        }
        let _ = fs::remove_file(&target);
        fs::rename(&partial, &target)?;
        Ok(target.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Run the setup program silently and wait for it. The setup program updates the existing
/// installation in place (same folder, same per-user or per-machine mode) and asks for
/// administrator rights itself when the installation needs them.
pub fn apply_installer(installer: &Path) -> Result<(), Error> {
    let status = std::process::Command::new(installer)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CLOSEAPPLICATIONS", "/NOCANCEL"])
        .status()
        .map_err(|e| Error(format!("Could not start the setup program: {e}")))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(2) => err("The update was cancelled (administrator permission was not given)."),
        Some(5) => err("The setup program could not install the update. Close Papers and try again."),
        Some(code) => err(format!("The setup program failed (exit code {code}).")),
        None => err("The setup program was stopped."),
    }
}

/// Paths inside a zip, with a shared top-level folder (`Papers/…`) removed.
fn zip_entries(archive: &mut zip::ZipArchive<File>) -> Result<Vec<(usize, PathBuf)>, Error> {
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| Error(format!("The update package is damaged: {e}")))?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            return err(format!("The update package contains an unsafe path: {}", entry.name()));
        };
        entries.push((i, path));
    }
    if entries.is_empty() {
        return err("The update package is empty.");
    }
    let first = |p: &Path| p.components().next().map(|c| c.as_os_str().to_owned());
    let top = first(&entries[0].1);
    let shared = entries.len() > 1 || entries[0].1.components().count() > 1;
    if shared && top.is_some() && entries.iter().all(|(_, p)| p.components().count() > 1 && first(p) == top) {
        for (_, p) in entries.iter_mut() {
            *p = p.components().skip(1).collect();
        }
    }
    for (_, p) in &entries {
        if p.components().any(|c| !matches!(c, Component::Normal(_))) {
            return err(format!("The update package contains an unsafe path: {}", p.display()));
        }
    }
    Ok(entries)
}

const OLD_SUFFIX: &str = ".papers-old";
const STAGING: &str = ".papers-update";

/// Unpack a portable zip over `dir`. Files are first extracted to a staging folder inside
/// `dir`, then swapped in; if any swap fails, everything already replaced is put back.
pub fn apply_portable(zip_path: &Path, dir: &Path) -> Result<(), Error> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| Error(format!("The update package is damaged: {e}")))?;
    let entries = zip_entries(&mut archive)?;
    if !entries.iter().any(|(_, p)| p.file_name().is_some_and(|n| n == PORTABLE_MARKER)) {
        return err("The package is not a portable build of Papers.");
    }
    let staging = dir.join(STAGING);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    for (index, rel) in &entries {
        let mut entry = archive.by_index(*index).map_err(|e| Error(format!("The update package is damaged: {e}")))?;
        let out = staging.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = File::create(&out)?;
        std::io::copy(&mut entry, &mut f).map_err(|e| Error(format!("Could not unpack the update: {e}")))?;
    }

    let mut swapped: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = (|| -> Result<(), Error> {
        for (_, rel) in &entries {
            let new = staging.join(rel);
            let dest = dir.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let backup = if dest.exists() {
                let mut name = dest.file_name().unwrap_or_default().to_owned();
                name.push(OLD_SUFFIX);
                let backup = dest.with_file_name(name);
                let _ = fs::remove_file(&backup);
                fs::rename(&dest, &backup).map_err(|e| Error(format!("Could not replace {}: {e}", dest.display())))?;
                Some(backup)
            } else {
                None
            };
            swapped.push((dest.clone(), backup));
            fs::rename(&new, &dest).map_err(|e| Error(format!("Could not install {}: {e}", dest.display())))?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            for (_, backup) in &swapped {
                if let Some(b) = backup {
                    let _ = fs::remove_file(b);
                }
            }
            let _ = fs::remove_dir_all(&staging);
            Ok(())
        }
        Err(e) => {
            for (dest, backup) in swapped.iter().rev() {
                let _ = fs::remove_file(dest);
                if let Some(b) = backup {
                    let _ = fs::rename(b, dest);
                }
            }
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Remove leftovers of an earlier portable update (files that were still locked then).
pub fn cleanup(dir: &Path) {
    let _ = fs::remove_dir_all(dir.join(STAGING));
    fn walk(dir: &Path, depth: usize) {
        let Ok(read) = fs::read_dir(dir) else { return };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 4 {
                walk(&path, depth + 1);
            } else if path.to_string_lossy().ends_with(OLD_SUFFIX) {
                let _ = fs::remove_file(&path);
            }
        }
    }
    walk(dir, 0);
}

/// Wait until the process with `pid` has exited. Returns false on timeout.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return true; // already gone
            }
            let r = WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128 - 1) as u32);
            CloseHandle(handle);
            r == WAIT_OBJECT_0
        }
    }
    #[cfg(unix)]
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            // Signal 0 only checks that the process exists.
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

/// The result of the last update, left for the next start of Papers to report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub ok: bool,
    pub version: String,
    pub message: String,
}

const OUTCOME_FILE: &str = "last-update.json";

pub fn write_outcome(outcome: &Outcome) {
    let dir = state_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(outcome) {
        let _ = fs::write(dir.join(OUTCOME_FILE), json);
    }
}

/// Read and forget the result of the last update.
pub fn take_outcome() -> Option<Outcome> {
    let path = state_dir().join(OUTCOME_FILE);
    let text = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    serde_json::from_str(&text).ok()
}

/// Everything the updater needs to finish an update after Papers has closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlan {
    pub package: PathBuf,
    pub asset: Asset,
    pub version: String,
    pub kind: InstallKind,
    pub wait_pid: Option<u32>,
    pub relaunch: Option<PathBuf>,
    pub open: Vec<PathBuf>,
}

impl ApplyPlan {
    pub fn to_args(&self) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = vec!["apply".into()];
        let mut push = |k: &str, v: std::ffi::OsString| {
            args.push(k.into());
            args.push(v);
        };
        push("--package", self.package.clone().into());
        push("--name", self.asset.name.clone().into());
        push("--size", self.asset.size.to_string().into());
        push("--sha256", self.asset.sha256.clone().into());
        push("--version", self.version.clone().into());
        match &self.kind {
            InstallKind::Installer { dir } => {
                push("--kind", "installer".into());
                push("--dir", dir.clone().into());
            }
            InstallKind::Portable { dir } => {
                push("--kind", "portable".into());
                push("--dir", dir.clone().into());
            }
            InstallKind::Unmanaged => push("--kind", "unmanaged".into()),
        }
        if let Some(pid) = self.wait_pid {
            push("--wait-pid", pid.to_string().into());
        }
        if let Some(exe) = &self.relaunch {
            push("--relaunch", exe.clone().into());
        }
        for f in &self.open {
            push("--open", f.clone().into());
        }
        args
    }

    pub fn from_args(args: &[std::ffi::OsString]) -> Result<ApplyPlan, Error> {
        let mut package = None;
        let mut asset = Asset { name: String::new(), url: String::new(), size: 0, sha256: String::new() };
        let mut version = String::new();
        let (mut kind, mut dir) = (String::new(), None);
        let (mut wait_pid, mut relaunch, mut open) = (None, None, Vec::new());
        let mut it = args.iter();
        while let Some(key) = it.next() {
            let key = key.to_string_lossy();
            let value = it.next().ok_or_else(|| Error(format!("Missing value for {key}")))?;
            let text = || value.to_string_lossy().into_owned();
            match key.as_ref() {
                "--package" => package = Some(PathBuf::from(value)),
                "--name" => asset.name = text(),
                "--size" => asset.size = text().parse().map_err(|_| Error("Invalid --size".into()))?,
                "--sha256" => asset.sha256 = text(),
                "--version" => version = text(),
                "--kind" => kind = text(),
                "--dir" => dir = Some(PathBuf::from(value)),
                "--wait-pid" => wait_pid = Some(text().parse().map_err(|_| Error("Invalid --wait-pid".into()))?),
                "--relaunch" => relaunch = Some(PathBuf::from(value)),
                "--open" => open.push(PathBuf::from(value)),
                other => return err(format!("Unknown option {other}")),
            }
        }
        let kind = match (kind.as_str(), dir) {
            ("installer", Some(dir)) => InstallKind::Installer { dir },
            ("portable", Some(dir)) => InstallKind::Portable { dir },
            _ => return err("--kind installer|portable and --dir are required"),
        };
        let package = package.ok_or_else(|| Error("--package is required".into()))?;
        if asset.sha256.len() != 64 || asset.size == 0 {
            return err("--sha256 and --size are required");
        }
        asset.name = package.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(ApplyPlan { package, asset, version, kind, wait_pid, relaunch, open })
    }
}

/// Verify and install a downloaded package, then start Papers again. Used by the updater.
pub fn apply(plan: &ApplyPlan) -> Result<(), Error> {
    if let Some(pid) = plan.wait_pid
        && !wait_for_exit(pid, Duration::from_secs(90))
    {
        return err("Papers is still running. Close it and try the update again.");
    }
    verify(&plan.package, &plan.asset)?;
    match &plan.kind {
        InstallKind::Installer { .. } => apply_installer(&plan.package)?,
        InstallKind::Portable { dir } => apply_portable(&plan.package, dir)?,
        InstallKind::Unmanaged => return err("This copy of Papers cannot be updated in place."),
    }
    let _ = fs::remove_file(&plan.package);
    Ok(())
}

/// Start the updater to finish `plan` after this process exits. The updater is copied to
/// the state folder first, so a portable update can replace the original.
pub fn spawn_updater(updater: &Path, plan: &ApplyPlan) -> Result<(), Error> {
    if !updater.is_file() {
        return err(format!("The updater is missing ({}).", updater.display()));
    }
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let copy = dir.join(UPDATER_EXE);
    fs::copy(updater, &copy).map_err(|e| Error(format!("Could not prepare the updater: {e}")))?;
    let mut command = std::process::Command::new(&copy);
    command.args(plan.to_args()).current_dir(&dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|e| Error(format!("Could not start the updater: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("2.0", "1.99.99"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("1.0.0.0", "0.1"));
        assert_eq!(Version::parse("v3.2.1").unwrap().to_string(), "3.2.1");
    }

    #[test]
    fn plan_round_trips() {
        let plan = ApplyPlan {
            package: PathBuf::from("/tmp/x/Papers-1.2.0-windows-x64-portable.zip"),
            asset: Asset {
                name: "Papers-1.2.0-windows-x64-portable.zip".into(),
                url: String::new(),
                size: 42,
                sha256: "a".repeat(64),
            },
            version: "1.2.0".into(),
            kind: InstallKind::Portable { dir: PathBuf::from("/opt/Papers") },
            wait_pid: Some(77),
            relaunch: Some(PathBuf::from("/opt/Papers/papers")),
            open: vec![PathBuf::from("/home/a b/c.pdf")],
        };
        let args = plan.to_args();
        assert_eq!(args[0], "apply");
        assert_eq!(ApplyPlan::from_args(&args[1..]).unwrap(), plan);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("papers-update-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn portable_update_replaces_files() {
        let root = temp_dir("portable");
        let install = root.join("Papers");
        fs::create_dir_all(install.join("licenses")).unwrap();
        fs::write(install.join("papers.exe"), b"old exe").unwrap();
        fs::write(install.join(PORTABLE_MARKER), b"portable").unwrap();
        fs::write(install.join("keep-me.txt"), b"user file").unwrap();
        let zip = root.join("update.zip");
        make_zip(&zip, &[
            ("Papers/papers.exe", b"new exe"),
            ("Papers/portable.txt", b"portable"),
            ("Papers/pdfium.dll", b"dll"),
            ("Papers/licenses/LICENSE.txt", b"gpl"),
        ]);
        let asset = Asset {
            name: "update.zip".into(),
            url: String::new(),
            size: fs::metadata(&zip).unwrap().len(),
            sha256: sha256_file(&zip).unwrap(),
        };
        let plan = ApplyPlan {
            package: zip.clone(),
            asset,
            version: "9.9.9".into(),
            kind: InstallKind::Portable { dir: install.clone() },
            wait_pid: None,
            relaunch: None,
            open: vec![],
        };
        apply(&plan).unwrap();
        assert_eq!(fs::read(install.join("papers.exe")).unwrap(), b"new exe");
        assert_eq!(fs::read(install.join("pdfium.dll")).unwrap(), b"dll");
        assert_eq!(fs::read(install.join("licenses/LICENSE.txt")).unwrap(), b"gpl");
        assert_eq!(fs::read(install.join("keep-me.txt")).unwrap(), b"user file");
        assert!(!install.join(STAGING).exists());
        assert!(!install.join(format!("papers.exe{OLD_SUFFIX}")).exists());
        assert!(!zip.exists(), "the package is removed after a successful update");
        assert_eq!(InstallKind::detect(&install.join("papers.exe")), InstallKind::Portable { dir: install.clone() });
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn damaged_or_wrong_packages_are_rejected() {
        let root = temp_dir("reject");
        let install = root.join("Papers");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("papers.exe"), b"old exe").unwrap();
        let zip = root.join("update.zip");
        make_zip(&zip, &[("Papers/papers.exe", b"new exe"), ("Papers/portable.txt", b"p")]);
        let good = Asset {
            name: "update.zip".into(),
            url: String::new(),
            size: fs::metadata(&zip).unwrap().len(),
            sha256: sha256_file(&zip).unwrap(),
        };
        let mut bad = good.clone();
        bad.sha256 = "0".repeat(64);
        assert!(verify(&zip, &bad).is_err());
        assert!(verify(&zip, &good).is_ok());

        // Not a portable build: no marker file.
        let other = root.join("other.zip");
        make_zip(&other, &[("Papers/papers.exe", b"evil")]);
        assert!(apply_portable(&other, &install).is_err());
        // Path traversal.
        let evil = root.join("evil.zip");
        make_zip(&evil, &[("../../escape.txt", b"x"), ("portable.txt", b"p")]);
        assert!(apply_portable(&evil, &install).is_err());
        assert_eq!(fs::read(install.join("papers.exe")).unwrap(), b"old exe");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_parses() {
        let json = r#"{"version":"1.2.0","published":"2026-11-19","notes":"n","notes_url":"u",
            "installer":{"name":"a.exe","url":"https://x/a.exe","size":1,"sha256":"00"},
            "portable":null}"#;
        let m: Manifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.asset_for(&InstallKind::Installer { dir: PathBuf::new() }).unwrap().name, "a.exe");
        assert!(m.asset_for(&InstallKind::Portable { dir: PathBuf::new() }).is_none());
    }

    #[test]
    fn unsafe_names_rejected() {
        assert!(safe_file_name("../a/b.zip").is_err());
        assert!(safe_file_name("").is_err());
        assert_eq!(safe_file_name("C:\\x\\Papers.zip").unwrap(), "CxPapers.zip");
        assert_eq!(safe_file_name("Papers-1.0.0-setup.exe").unwrap(), "Papers-1.0.0-setup.exe");
    }
}
