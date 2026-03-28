#[cfg(target_os = "windows")]
fn main() {
    use std::path::Path;

    let disabled_icon = "assets/taskbar-autohide-disabled.ico";
    let enabled_icon = "assets/taskbar-autohide-enabled.ico";
    let legacy_icon = "assets/taskbar-autohide.ico";

    println!("cargo:rerun-if-changed={disabled_icon}");
    println!("cargo:rerun-if-changed={enabled_icon}");
    println!("cargo:rerun-if-changed={legacy_icon}");

    let icon_path = if Path::new(disabled_icon).exists() {
        Some(disabled_icon)
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
            "cargo:warning=No Windows executable icon found. Add assets/taskbar-autohide-disabled.ico (or legacy assets/taskbar-autohide.ico)."
        );
    }

    if !Path::new(enabled_icon).exists() {
        println!(
            "cargo:warning=Optional icon missing: {enabled_icon}. Add it to keep enabled/disabled icon assets together."
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
