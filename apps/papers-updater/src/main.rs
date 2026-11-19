//! `papers-updater`: keeps Papers up to date.
//!
//! ```text
//! papers-updater check            Show the installed and the latest version.
//! papers-updater update [--yes]   Download, verify and install the latest version for the
//!                                 installation this updater belongs to.
//! papers-updater apply …          Used by Papers itself to finish an update after it closes.
//! ```

use papers_update::{ApplyPlan, InstallKind, Outcome};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_EXE: &str = if cfg!(windows) { "papers.exe" } else { "papers" };

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let command = args.first().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
    let result = match command.as_str() {
        "check" => check(),
        "update" => update(args.iter().any(|a| a == "--yes" || a == "-y")),
        "apply" => return apply(&args[1..]),
        "--version" | "-V" => {
            println!("papers-updater {VERSION}");
            Ok(())
        }
        _ => {
            eprintln!("usage: papers-updater check | update [--yes] | --version");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

fn install_kind() -> InstallKind {
    install_dir().map(|d| InstallKind::detect(&d.join(APP_EXE))).unwrap_or(InstallKind::Unmanaged)
}

fn check() -> Result<(), papers_update::Error> {
    println!("Installed: Papers {VERSION} ({})", install_kind().id());
    match papers_update::check(VERSION)? {
        Some(m) => {
            println!("Available: Papers {} (published {})", m.version, m.published);
            if !m.notes_url.is_empty() {
                println!("Release notes: {}", m.notes_url);
            }
        }
        None => println!("Papers is up to date."),
    }
    Ok(())
}

fn update(yes: bool) -> Result<(), papers_update::Error> {
    let kind = install_kind();
    let Some(manifest) = papers_update::check(VERSION)? else {
        println!("Papers {VERSION} is up to date.");
        return Ok(());
    };
    let Some(asset) = manifest.asset_for(&kind).cloned() else {
        return Err(papers_update::Error(format!(
            "Papers {} is available, but this copy cannot be updated in place. Download it from {}",
            manifest.version,
            papers_update::RELEASES_URL
        )));
    };
    if !yes {
        print!("Update Papers {VERSION} to {}? [y/N] ", manifest.version);
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("Not updated.");
            return Ok(());
        }
    }
    let cancel = AtomicBool::new(false);
    let mut last = u64::MAX;
    let package = papers_update::download(&asset, &papers_update::state_dir(), &mut |done, total| {
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        if pct != last {
            last = pct;
            print!("\rDownloading {}: {pct:3}%", asset.name);
            let _ = std::io::stdout().flush();
        }
    }, &cancel)?;
    println!();
    let plan = ApplyPlan {
        package,
        asset,
        version: manifest.version.clone(),
        kind,
        wait_pid: None,
        relaunch: None,
        open: vec![],
    };
    papers_update::apply(&plan)?;
    println!("Papers was updated to {}.", manifest.version);
    Ok(())
}

/// Finish an update started from Papers: wait for it to close, install, start it again.
fn apply(args: &[OsString]) -> ExitCode {
    let plan = match ApplyPlan::from_args(args) {
        Ok(plan) => plan,
        Err(e) => {
            show_error(&format!("The update could not start: {e}"));
            return ExitCode::from(2);
        }
    };
    let result = papers_update::apply(&plan);
    let outcome = match &result {
        Ok(()) => Outcome { ok: true, version: plan.version.clone(), message: String::new() },
        Err(e) => Outcome { ok: false, version: plan.version.clone(), message: e.to_string() },
    };
    papers_update::write_outcome(&outcome);
    if let Err(e) = &result {
        show_error(&format!("Papers could not be updated to {}.\n\n{e}", plan.version));
    }
    // Start Papers again either way, so the person is not left without their documents.
    if let Some(exe) = &plan.relaunch {
        let mut command = std::process::Command::new(exe);
        command.args(&plan.open);
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        let _ = command.spawn();
    }
    if result.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn show_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (text, title) = (wide(message), wide("Papers update"));
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
        }
    }
}
