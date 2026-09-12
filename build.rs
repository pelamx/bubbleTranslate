//! Links the Windows icon and version information into the executable.
//!
//! Nothing to do on macOS, where the bundle carries both, or on Linux, where
//! the desktop file points at an SVG. On Windows the picture has to be inside
//! the `.exe`: it is what Explorer draws, what the taskbar button inherits,
//! and — as resource number one — what the notification area icon is loaded
//! from at runtime.
//!
//! The icon is checked in; `windows\make-icon.ps1` is what draws it.

fn main() {
    #[cfg(target_os = "windows")]
    windows_icon();
}

#[cfg(target_os = "windows")]
fn windows_icon() {
    const ICON: &str = "windows/bubbleTranslate.ico";

    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build.rs");

    let mut resource = winresource::WindowsResource::new();
    // Number one by name, because `shell::app_icon` asks for it by that number
    // rather than by a symbol it has no header to read.
    resource.set_icon_with_id(ICON, "1");
    resource.set("ProductName", "bubbleTranslate");
    resource.set("FileDescription", "bubbleTranslate — a bubble translator");
    resource.set("LegalCopyright", "by pelamx");

    // A warning rather than a failure. A build without the resource compiler
    // produces a working translator with a generic icon, and that is a far
    // better outcome than a build that cannot be made at all.
    if let Err(err) = resource.compile() {
        println!("cargo:warning=could not link the icon into the binary: {err}");
    }
}
