#[cfg(target_os = "windows")]
fn main() {
    use std::path::Path;

    let enable_autohide_icon = "assets/taskbar-enable-autohide.ico";
    let disable_autohide_icon = "assets/taskbar-disable-autohide.ico";
    let legacy_icon = "assets/taskbar-autohide.ico";

    println!("cargo:rerun-if-changed={enable_autohide_icon}");
    println!("cargo:rerun-if-changed={disable_autohide_icon}");
    println!("cargo:rerun-if-changed={legacy_icon}");

    let icon_path = if Path::new(enable_autohide_icon).exists() {
        Some(enable_autohide_icon)
    } else if Path::new(disable_autohide_icon).exists() {
        Some(disable_autohide_icon)
    } else if Path::new(legacy_icon).exists() {
        Some(legacy_icon)
    } else {
        None
    };

    if let Some(icon_path) = icon_path {
        let mut res = winres::WindowsResource::new();
        res.set_icon(icon_path);
        res.compile().expect("failed to compile Windows resources");
    } else {
        println!(
            "cargo:warning=No Windows executable icon found. Add assets/taskbar-enable-autohide.ico or assets/taskbar-disable-autohide.ico (or legacy assets/taskbar-autohide.ico)."
        );
    }

    if !Path::new(enable_autohide_icon).exists() {
        println!("cargo:warning=Icon missing: {enable_autohide_icon}.");
    }

    if !Path::new(disable_autohide_icon).exists() {
        println!("cargo:warning=Icon missing: {disable_autohide_icon}.");
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
