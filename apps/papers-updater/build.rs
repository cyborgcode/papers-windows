//! Embed the icon and version information into the Windows executable.
const DESCRIPTION: &str = "Papers updater";

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/papers.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/papers.ico")
        .set("FileDescription", DESCRIPTION)
        .set("ProductName", "Papers")
        .set("OriginalFilename", concat!(env!("CARGO_PKG_NAME"), ".exe"))
        .set("LegalCopyright", "GPL-2.0-or-later");
    if let Err(e) = res.compile() {
        // Resources are cosmetic; never fail the build over them.
        println!("cargo:warning=could not embed Windows resources: {e}");
    }
}
