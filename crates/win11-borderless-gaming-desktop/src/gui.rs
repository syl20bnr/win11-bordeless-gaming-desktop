use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write as _,
    os::windows::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Layout, Margin,
    RichText, Stroke, StrokeKind, TextStyle, Vec2, WidgetInfo, WidgetType,
};
use raw_window_handle::RawWindowHandle;
use serde::{Deserialize, Serialize};
use tray_icon::{
    Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows::{
    Win32::{
        Foundation::HWND,
        Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
        UI::WindowsAndMessaging::{HMENU, SetMenuDefaultItem},
    },
    core::PCWSTR,
};

use crate::sound::{self, SoundCue};
use crate::{
    app::{self, ToggleOptions},
    audio::{self, OutputDevice, OutputSnapshot},
    behavior,
    display::{self, ChangeOutcome, DisplayError, Resolution},
};

const APP_ID: &str = "win11-borderless-gaming-desktop";
const APP_TITLE: &str = "Borderless Gaming Desktop";
const APP_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const STORAGE_KEY: &str = "gui-settings";
const CURRENT_SETTINGS_VERSION: u8 = 2;
const CURRENT_MODE_STATE_VERSION: u8 = 2;
const MODE_STATE_FILE_NAME: &str = "mode-state.ron";
const MODE_STATE_TEMP_FILE_NAME: &str = "mode-state.tmp";
const EFRAME_WINDOW_STORAGE_KEY: &str = "window";
const EFRAME_MEMORY_STORAGE_KEY: &str = "egui";
const GAMING_COUNTDOWN_SECONDS: f64 = 3.0;
const MODE_BUTTON_CLICK_ANIMATION_SECONDS: f64 = 0.16;
const TRAY_BLINK_HALF_PERIOD_SECONDS: f64 = 0.5;
const FIXED_SIZE_SETTLE_PASSES: u8 = 20;
const WORDMARK_TEXTURE_HEIGHT: u32 = 96;
const WORDMARK_DISPLAY_HEIGHT: f32 = 30.0;
const WORDMARK_TEXTURE_OPTIONS: egui::TextureOptions =
    egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
const HEADER_HEIGHT: f32 = 50.0;
const HEADER_CLOSE_CLEARANCE: f32 = 24.0;
const WINDOW_WIDTH: f32 = 520.0;
const MAIN_BOTTOM_INSET: i8 = 12;
// Measured from the full rendered app so the application-behavior card keeps
// a compact bottom inset in the fixed window.
const WINDOW_HEIGHT: f32 = 782.0;
const TRANSPARENCY_CONTROL_HEIGHT: f32 = 66.0;
const TRAY_OPEN_MENU_POSITION: u32 = 0;
const WINDOW_SIZE: [f32; 2] = [WINDOW_WIDTH, WINDOW_HEIGHT];

const BACKGROUND: Color32 = Color32::from_rgb(12, 15, 22);
const CARD: Color32 = Color32::from_rgb(22, 27, 38);
const CARD_BORDER: Color32 = Color32::from_rgb(45, 54, 72);
const TEXT_MUTED: Color32 = Color32::from_rgb(151, 161, 180);
const ACCENT: Color32 = Color32::from_rgb(112, 98, 255);
const ACCENT_HOVER: Color32 = Color32::from_rgb(130, 117, 255);
// Keep these status colors in sync with the tray derivatives in `xtask`.
const GAMING_GREEN: Color32 = Color32::from_rgb(74, 222, 128);
const ACTIVATING_ORANGE: Color32 = Color32::from_rgb(249, 151, 71);
const LED_OFF: Color32 = Color32::from_rgb(148, 156, 181);

pub fn run() -> eframe::Result {
    let mut viewport = fixed_viewport(egui::ViewportBuilder::default().with_app_id(APP_ID));
    let start_minimized =
        std::env::args_os().any(|argument| argument == behavior::START_MINIMIZED_ARGUMENT);

    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        centered: true,
        // App settings are persisted explicitly. Native geometry never is.
        persist_window: false,
        // Eframe loads legacy geometry even when `persist_window` is false. This
        // late hook runs after that load and overwrites every mutable size flag.
        window_builder: Some(Box::new(fixed_viewport)),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        native_options,
        Box::new(move |creation_context| {
            Ok(Box::new(GuiApp::new(creation_context, start_minimized)?))
        }),
    )
}

fn fixed_viewport(mut viewport: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let fixed_size = Vec2::from(WINDOW_SIZE);

    // Eframe centers using the restored size before invoking `window_builder`.
    // Correct that position by the size delta while replacing stale geometry.
    if let (Some(position), Some(restored_size)) = (viewport.position, viewport.inner_size) {
        viewport.position = Some(position + (restored_size - fixed_size) * 0.5);
    }

    viewport
        .with_inner_size(fixed_size)
        .with_min_inner_size(fixed_size)
        .with_max_inner_size(fixed_size)
        .with_clamp_size_to_monitor_size(false)
        .with_fullscreen(false)
        .with_maximized(false)
        .with_decorations(false)
        .with_resizable(false)
        .with_maximize_button(false)
}

fn window_icon() -> Option<egui::IconData> {
    let image = decode_png(include_bytes!("../assets/runtime/app.png")).ok()?;
    let (width, height) = image.dimensions();

    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
struct FeatureChoices {
    change_resolution: bool,
    change_output_device: bool,
    taskbar_auto_hide: bool,
    desktop_icons: bool,
    desktop_background: bool,
    minimize_all_windows: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for FeatureChoices {
    fn default() -> Self {
        Self {
            change_resolution: true,
            change_output_device: false,
            taskbar_auto_hide: true,
            desktop_icons: true,
            desktop_background: true,
            minimize_all_windows: true,
        }
    }
}

impl FeatureChoices {
    fn toggle_options(&self) -> ToggleOptions {
        ToggleOptions {
            taskbar_auto_hide: self.taskbar_auto_hide,
            desktop_icons: self.desktop_icons,
            desktop_background: self.desktop_background,
            minimize_all_windows: self.minimize_all_windows,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum ResolutionProfile {
    Desktop,
    Gaming,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PendingResolution {
    profile: ResolutionProfile,
    target: Resolution,
}

impl PendingResolution {
    const fn desktop(target: Resolution) -> Self {
        Self {
            profile: ResolutionProfile::Desktop,
            target,
        }
    }

    const fn gaming(target: Resolution) -> Self {
        Self {
            profile: ResolutionProfile::Gaming,
            target,
        }
    }

    const fn for_mode(gaming_mode_active: bool, target: Resolution) -> Self {
        if gaming_mode_active {
            Self::gaming(target)
        } else {
            Self::desktop(target)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
struct Settings {
    #[serde(default = "legacy_settings_version")]
    settings_version: u8,
    gaming_mode_active: bool,
    features: FeatureChoices,
    active_mode_features: Option<FeatureChoices>,
    taskbar_auto_hide_before_activation: Option<bool>,
    desktop_resolution: Option<Resolution>,
    gaming_resolution: Option<Resolution>,
    gaming_output_device_id: Option<String>,
    output_before_activation: Option<OutputSnapshot>,
    pending_resolution: Option<PendingResolution>,
    #[serde(default, skip_serializing, rename = "pending_resolution_target")]
    legacy_pending_resolution_target: Option<Resolution>,
    sounds_enabled: bool,
    #[serde(skip)]
    startup_at_login: bool,
    startup_minimized: bool,
    window_transparency: u8,
    tray_close_notice_acknowledged: bool,
}

const fn legacy_settings_version() -> u8 {
    0
}

#[allow(clippy::derivable_impls)]
impl Default for Settings {
    fn default() -> Self {
        Self {
            settings_version: CURRENT_SETTINGS_VERSION,
            gaming_mode_active: false,
            features: FeatureChoices::default(),
            active_mode_features: None,
            taskbar_auto_hide_before_activation: None,
            desktop_resolution: None,
            gaming_resolution: None,
            gaming_output_device_id: None,
            output_before_activation: None,
            pending_resolution: None,
            legacy_pending_resolution_target: None,
            sounds_enabled: true,
            startup_at_login: false,
            startup_minimized: false,
            window_transparency: 0,
            tray_close_notice_acknowledged: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
struct DurableModeState {
    version: u8,
    active: bool,
    active_mode_features: Option<FeatureChoices>,
    taskbar_auto_hide_before_activation: Option<bool>,
    output_before_activation: Option<OutputSnapshot>,
    pending_resolution: Option<PendingResolution>,
    #[serde(default, skip_serializing, rename = "pending_resolution_target")]
    legacy_pending_resolution_target: Option<Resolution>,
}

impl Default for DurableModeState {
    fn default() -> Self {
        Self {
            version: CURRENT_MODE_STATE_VERSION,
            active: false,
            active_mode_features: None,
            taskbar_auto_hide_before_activation: None,
            output_before_activation: None,
            pending_resolution: None,
            legacy_pending_resolution_target: None,
        }
    }
}

impl DurableModeState {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            version: CURRENT_MODE_STATE_VERSION,
            active: settings.gaming_mode_active,
            active_mode_features: settings.active_mode_features.clone(),
            taskbar_auto_hide_before_activation: settings.taskbar_auto_hide_before_activation,
            output_before_activation: settings.output_before_activation.clone(),
            pending_resolution: settings.pending_resolution,
            legacy_pending_resolution_target: None,
        }
    }

    fn apply_to_settings(&self, settings: &mut Settings) {
        settings.gaming_mode_active = self.active;
        settings.active_mode_features = self.active_mode_features.clone();
        settings.taskbar_auto_hide_before_activation = self.taskbar_auto_hide_before_activation;
        settings.output_before_activation = self.output_before_activation.clone();
        settings.pending_resolution = self.pending_resolution;
        settings.legacy_pending_resolution_target = None;
    }
}

fn migrate_durable_mode_state(state: &mut DurableModeState) -> Result<bool, String> {
    if state.version == 1 {
        state.pending_resolution = state.pending_resolution.or_else(|| {
            state
                .legacy_pending_resolution_target
                .map(|target| PendingResolution::for_mode(state.active, target))
        });
        state.legacy_pending_resolution_target = None;
        state.version = CURRENT_MODE_STATE_VERSION;
        Ok(true)
    } else if state.version == CURRENT_MODE_STATE_VERSION {
        Ok(false)
    } else {
        Err(format!(
            "The saved mode-state version {} is unsupported.",
            state.version
        ))
    }
}

fn mode_state_paths() -> Result<(PathBuf, PathBuf), String> {
    let directory = eframe::storage_dir(APP_ID)
        .ok_or_else(|| "Windows did not provide an application-data directory.".to_owned())?;
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create the mode-state directory {}: {error}",
            directory.display()
        )
    })?;
    Ok((
        directory.join(MODE_STATE_FILE_NAME),
        directory.join(MODE_STATE_TEMP_FILE_NAME),
    ))
}

fn load_durable_mode_state() -> Result<Option<(DurableModeState, bool)>, String> {
    let (path, _) = mode_state_paths()?;
    load_durable_mode_state_at(&path)
}

fn load_durable_mode_state_at(path: &Path) -> Result<Option<(DurableModeState, bool)>, String> {
    let serialized = match fs::read_to_string(path) {
        Ok(serialized) => serialized,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Could not read the saved mode state {}: {error}",
                path.display()
            ));
        }
    };
    let mut state: DurableModeState = ron::from_str(&serialized).map_err(|error| {
        format!(
            "Could not decode the saved mode state {}: {error}",
            path.display()
        )
    })?;
    let needs_persist = migrate_durable_mode_state(&mut state)?;
    Ok(Some((state, needs_persist)))
}

fn save_durable_mode_state(state: &DurableModeState) -> Result<(), String> {
    let (path, temporary_path) = mode_state_paths()?;
    save_durable_mode_state_at(&path, &temporary_path, state)
}

fn save_durable_mode_state_at(
    path: &Path,
    temporary_path: &Path,
    state: &DurableModeState,
) -> Result<(), String> {
    let serialized = ron::to_string(state)
        .map_err(|error| format!("Could not encode the mode recovery state: {error}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(temporary_path)
        .map_err(|error| {
            format!(
                "Could not open the temporary mode-state file {}: {error}",
                temporary_path.display()
            )
        })?;
    file.write_all(serialized.as_bytes()).map_err(|error| {
        format!(
            "Could not write the temporary mode-state file {}: {error}",
            temporary_path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "Could not synchronize the temporary mode-state file {}: {error}",
            temporary_path.display()
        )
    })?;
    drop(file);

    let temporary_wide = wide_path(temporary_path);
    let destination_wide = wide_path(path);
    unsafe {
        MoveFileExW(
            PCWSTR(temporary_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        format!(
            "Could not commit the mode-state file {}: {error}",
            path.display()
        )
    })
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

fn migrate_settings(settings: &mut Settings, legacy_taskbar_auto_hide: bool) {
    if settings.settings_version >= CURRENT_SETTINGS_VERSION {
        return;
    }

    if settings.settings_version == 0 {
        // Before the app owned its mode state, taskbar auto-hide was the source
        // of truth. Consult it exactly once while migrating so an in-progress
        // Gaming session remains recoverable after upgrading.
        settings.gaming_mode_active = legacy_taskbar_auto_hide;
        if settings.gaming_mode_active {
            if settings.active_mode_features.is_none() {
                settings.active_mode_features = Some(settings.features.clone());
            }
            if settings.features.taskbar_auto_hide {
                // The legacy app always restored auto-hide to off.
                settings.taskbar_auto_hide_before_activation = Some(false);
            }
        } else {
            settings.active_mode_features = None;
            settings.taskbar_auto_hide_before_activation = None;
        }
    }

    if settings.settings_version < 2 {
        settings.pending_resolution = settings.pending_resolution.or_else(|| {
            settings
                .legacy_pending_resolution_target
                .map(|target| PendingResolution::for_mode(settings.gaming_mode_active, target))
        });
        settings.legacy_pending_resolution_target = None;
    }
    settings.settings_version = CURRENT_SETTINGS_VERSION;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayCommand {
    Open,
    ToggleMode,
    ActivateModeWithoutResolution,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModeVisualState {
    Desktop,
    EnterPressed,
    Countdown(u8),
    Activating,
    Gaming,
    RestorePressed,
    Restoring,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ModeTransitionState {
    EnterPressed(f64),
    Countdown(f64),
    ApplyingGaming(f64),
    RestorePressed(f64),
    ApplyingDesktop(f64),
}

impl ModeTransitionState {
    fn started_at(self) -> f64 {
        match self {
            Self::EnterPressed(started_at)
            | Self::Countdown(started_at)
            | Self::ApplyingGaming(started_at)
            | Self::RestorePressed(started_at)
            | Self::ApplyingDesktop(started_at) => started_at,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayIconState {
    Desktop,
    ActivatingBright,
    ActivatingDim,
    Gaming,
}

struct TrayMenuItems {
    mode: MenuItem,
    mode_without_resolution: MenuItem,
}

#[derive(Clone, Copy)]
enum AnimatedButtonTone {
    Primary,
    Secondary,
}

#[derive(Default)]
struct DisplayPicker {
    resolutions: Vec<Resolution>,
    available_resolutions: Vec<Resolution>,
    native: Option<Resolution>,
}

#[derive(Default)]
struct AudioPicker {
    devices: Vec<OutputDevice>,
}

impl AudioPicker {
    fn load(&mut self) -> Result<(), String> {
        self.devices = audio::output_devices()?;
        Ok(())
    }
}

struct ModeWordmark {
    texture: egui::TextureHandle,
    aspect_ratio: f32,
}

struct ModeWordmarks {
    desktop: ModeWordmark,
    gaming: ModeWordmark,
}

impl ModeWordmarks {
    fn load(context: &egui::Context) -> Result<Self, image::ImageError> {
        Ok(Self {
            desktop: load_mode_wordmark(
                context,
                "desktop-mode-wordmark",
                include_bytes!("../assets/runtime/desktop-mode-wordmark.png"),
            )?,
            gaming: load_mode_wordmark(
                context,
                "gaming-mode-wordmark",
                include_bytes!("../assets/runtime/gaming-mode-wordmark.png"),
            )?,
        })
    }

    fn for_state(&self, state: ModeVisualState) -> &ModeWordmark {
        if mode_uses_desktop_wordmark(state) {
            &self.desktop
        } else {
            &self.gaming
        }
    }
}

fn mode_uses_desktop_wordmark(state: ModeVisualState) -> bool {
    matches!(
        state,
        ModeVisualState::Desktop
            | ModeVisualState::EnterPressed
            | ModeVisualState::Countdown(_)
            | ModeVisualState::Restoring
    )
}

fn load_mode_wordmark(
    context: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Result<ModeWordmark, image::ImageError> {
    let image = decode_png(bytes)?;
    debug_assert_eq!(image.height(), WORDMARK_TEXTURE_HEIGHT);
    let size = [image.width() as usize, image.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    let texture = context.load_texture(name, color_image, WORDMARK_TEXTURE_OPTIONS);

    Ok(ModeWordmark {
        texture,
        aspect_ratio: size[0] as f32 / size[1] as f32,
    })
}

fn decode_png(bytes: &[u8]) -> Result<image::RgbaImage, image::ImageError> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map(image::DynamicImage::into_rgba8)
}

impl DisplayPicker {
    fn load(&mut self, settings: &mut Settings) -> Result<(), DisplayError> {
        self.resolutions.clear();
        self.available_resolutions.clear();
        self.native = None;

        let mut resolutions = display::primary_resolutions()?;
        let current = display::primary_resolution()?;
        let native = display::primary_native_resolution().ok();

        resolutions.retain(|resolution| resolution_is_allowed(*resolution, native));

        for resolution in [Some(current), native].into_iter().flatten() {
            if resolution_is_allowed(resolution, native) && !resolutions.contains(&resolution) {
                resolutions.push(resolution);
            }
        }
        display::sort_resolutions_descending(&mut resolutions);
        resolutions.dedup();

        let default = if resolution_is_allowed(current, native) {
            current
        } else {
            native
                .or_else(|| resolutions.first().copied())
                .ok_or(DisplayError::NoResolutions)?
        };

        self.available_resolutions = resolutions.clone();
        self.native = native;
        initialize_profile(&mut settings.desktop_resolution, default);
        initialize_profile(&mut settings.gaming_resolution, default);
        // Desktop resolution is a hidden first-start restore point. Keep it
        // intact across temporary monitor, dock, and Remote Desktop changes.
        replace_disallowed_profile(&mut settings.gaming_resolution, default, native);

        // Preserve a Gaming profile that is temporarily unavailable (for
        // example under Remote Desktop or while another monitor is connected).
        // It remains visible but disabled instead of being overwritten on save.
        if let Some(resolution) = settings.gaming_resolution
            && resolution_is_allowed(resolution, native)
            && !resolutions.contains(&resolution)
        {
            resolutions.push(resolution);
        }
        display::sort_resolutions_descending(&mut resolutions);
        resolutions.dedup();
        self.resolutions = resolutions;
        Ok(())
    }
}

fn resolution_is_allowed(resolution: Resolution, native: Option<Resolution>) -> bool {
    native.is_none_or(|native| resolution.height <= native.height)
}

fn replace_disallowed_profile(
    profile: &mut Option<Resolution>,
    default: Resolution,
    native: Option<Resolution>,
) {
    if profile.is_some_and(|resolution| !resolution_is_allowed(resolution, native)) {
        *profile = Some(default);
    }
}

fn initialize_profile(profile: &mut Option<Resolution>, default: Resolution) {
    if profile.is_none() {
        *profile = Some(default);
    }
}

fn cancel_pending_gaming_resolution(pending: &mut Option<PendingResolution>) -> bool {
    if pending.is_some_and(|pending| pending.profile == ResolutionProfile::Gaming) {
        *pending = None;
        true
    } else {
        false
    }
}

fn pending_resolution_for_mode(
    gaming_mode_active: bool,
    change_resolution: bool,
    target: Option<Resolution>,
) -> Option<PendingResolution> {
    if change_resolution {
        target.map(|target| PendingResolution::for_mode(gaming_mode_active, target))
    } else {
        None
    }
}

fn activation_choices(features: &FeatureChoices, change_resolution: bool) -> FeatureChoices {
    let mut choices = features.clone();
    choices.change_resolution &= change_resolution;
    choices
}

struct GuiApp {
    settings: Settings,
    native_window: Option<HWND>,
    window_transparency_reapply_pending: bool,
    display: DisplayPicker,
    audio: AudioPicker,
    mode_wordmarks: ModeWordmarks,
    errors: Vec<String>,
    pending_resolution: Option<(PendingResolution, String)>,
    tray_commands: Receiver<TrayCommand>,
    tray_icon: TrayIcon,
    tray_icon_state: TrayIconState,
    tray_mode_item: MenuItem,
    tray_mode_without_resolution_item: MenuItem,
    tray_menu_state: ModeVisualState,
    mode_transition: Option<ModeTransitionState>,
    activation_changes_resolution: bool,
    last_countdown_digit: Option<u8>,
    sound_player: sound::SoundPlayer,
    close_notice_open: bool,
    fixed_size_settle_passes: u8,
    restore_actions_failed: bool,
    quitting: bool,
}

impl GuiApp {
    fn new(
        creation_context: &eframe::CreationContext<'_>,
        start_minimized: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        configure_context(&creation_context.egui_ctx);
        let mode_wordmarks = ModeWordmarks::load(&creation_context.egui_ctx)?;

        let mut settings: Settings = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default();
        let mut errors = Vec::new();
        if settings.settings_version < CURRENT_SETTINGS_VERSION {
            migrate_settings(&mut settings, app::taskbar_auto_hide_enabled());
        }
        match load_durable_mode_state() {
            Ok(Some((mode_state, needs_persist))) => {
                mode_state.apply_to_settings(&mut settings);
                if needs_persist && let Err(error) = save_durable_mode_state(&mode_state) {
                    errors.push(format!(
                        "Persistent mode state could not be upgraded: {error}"
                    ));
                }
            }
            Ok(None) => {
                if let Err(error) =
                    save_durable_mode_state(&DurableModeState::from_settings(&settings))
                {
                    errors.push(format!("Persistent mode state is unavailable: {error}"));
                }
            }
            Err(error) => errors.push(format!("Persistent mode state is unavailable: {error}")),
        }
        settings.window_transparency = settings
            .window_transparency
            .min(behavior::MAX_TRANSPARENCY_PERCENT);

        match behavior::startup_at_login_enabled() {
            Ok(enabled) => settings.startup_at_login = enabled,
            Err(error) => errors.push(error),
        }

        let native_window = native_window_handle(creation_context);
        // eframe creates the root HWND hidden and invokes the app creator before
        // its first visible paint. Prime the restored alpha here to avoid an
        // opaque flash, then reapply it after eframe makes the window visible.
        match native_window {
            Some(hwnd) => {
                if let Err(error) =
                    behavior::set_window_transparency(hwnd, settings.window_transparency)
                {
                    errors.push(error);
                }
            }
            None => errors.push("Could not access the native window for transparency.".to_owned()),
        }

        let gaming_mode = settings.gaming_mode_active;
        let pending_resolution = settings.pending_resolution.map(|pending| {
            let target = pending.target;
            (
                pending,
                format!("The saved resolution change to {target} still needs to be retried."),
            )
        });

        let mut display = DisplayPicker::default();
        if let Err(error) = display.load(&mut settings) {
            errors.push(format!("Display modes are unavailable: {error}"));
        }
        let mut audio = AudioPicker::default();
        if let Err(error) = audio.load() {
            errors.push(error);
        }

        let mode_state = if gaming_mode {
            ModeVisualState::Gaming
        } else {
            ModeVisualState::Desktop
        };
        let tray_icon_state = if gaming_mode {
            TrayIconState::Gaming
        } else {
            TrayIconState::Desktop
        };
        let (tray_icon, tray_menu_items, tray_commands) =
            create_tray(&creation_context.egui_ctx, tray_icon_state, mode_state)?;

        if start_minimized {
            hide_to_tray(&creation_context.egui_ctx);
        }

        Ok(Self {
            settings,
            native_window,
            // A tray-hidden startup is repaired when the tray opens it. Leaving
            // this pending while hidden would schedule needless repaint loops.
            window_transparency_reapply_pending: !start_minimized,
            display,
            audio,
            mode_wordmarks,
            errors,
            pending_resolution,
            tray_commands,
            tray_icon,
            tray_icon_state,
            tray_mode_item: tray_menu_items.mode,
            tray_mode_without_resolution_item: tray_menu_items.mode_without_resolution,
            tray_menu_state: mode_state,
            mode_transition: None,
            activation_changes_resolution: true,
            last_countdown_digit: None,
            sound_player: sound::SoundPlayer::default(),
            close_notice_open: false,
            fixed_size_settle_passes: 0,
            restore_actions_failed: false,
            quitting: false,
        })
    }

    fn reapply_window_transparency_after_show(
        &mut self,
        context: &egui::Context,
        frame: &eframe::Frame,
    ) {
        if !self.window_transparency_reapply_pending {
            return;
        }

        self.native_window = native_window_handle(frame).or(self.native_window);
        let Some(hwnd) = self.native_window else {
            self.window_transparency_reapply_pending = false;
            push_unique_error(
                &mut self.errors,
                "Could not access the native window for transparency.".to_owned(),
            );
            return;
        };

        if !behavior::window_is_visible(hwnd) {
            context.request_repaint_after(Duration::from_millis(16));
            return;
        }

        self.window_transparency_reapply_pending = false;
        match behavior::set_window_transparency(hwnd, self.settings.window_transparency) {
            Ok(()) => remove_errors_containing(&mut self.errors, "transparency"),
            Err(error) => push_unique_error(&mut self.errors, error),
        }
    }

    fn toggle_mode(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        self.errors.clear();
        self.pending_resolution = None;
        let was_enabled = self.settings.gaming_mode_active;
        let choices = if was_enabled {
            self.settings
                .active_mode_features
                .clone()
                .unwrap_or_else(|| self.settings.features.clone())
        } else {
            let choices =
                activation_choices(&self.settings.features, self.activation_changes_resolution);
            self.activation_changes_resolution = true;
            let previous_pending_resolution = self.settings.pending_resolution;
            let previous_output_snapshot = self.settings.output_before_activation.clone();
            if choices.change_output_device {
                let Some(_) = self.settings.gaming_output_device_id else {
                    self.errors
                        .push("Choose a Gaming Mode output device first.".to_owned());
                    self.begin_fixed_size_settle(context);
                    return;
                };
                match audio::default_output_snapshot() {
                    Ok(snapshot) => self.settings.output_before_activation = Some(snapshot),
                    Err(error) => {
                        self.errors.push(error);
                        self.begin_fixed_size_settle(context);
                        return;
                    }
                }
            } else {
                self.settings.output_before_activation = None;
            }
            self.settings.gaming_mode_active = true;
            self.settings.active_mode_features = Some(choices.clone());
            self.settings.taskbar_auto_hide_before_activation = choices
                .taskbar_auto_hide
                .then(app::taskbar_auto_hide_enabled);
            self.settings.pending_resolution = pending_resolution_for_mode(
                true,
                choices.change_resolution,
                self.settings.gaming_resolution,
            );

            // Commit a synchronous recovery record before changing Windows. A
            // crash at any later point can then be repaired or retried.
            if let Err(error) = self.persist_mode_state(frame) {
                self.settings.gaming_mode_active = false;
                self.settings.active_mode_features = None;
                self.settings.taskbar_auto_hide_before_activation = None;
                self.settings.pending_resolution = previous_pending_resolution;
                self.settings.output_before_activation = previous_output_snapshot;
                self.errors.push(format!(
                    "Gaming Mode was not activated because its recovery state could not be saved: {error}"
                ));
                self.begin_fixed_size_settle(context);
                return;
            }
            choices
        };

        let report = if was_enabled {
            app::restore(
                choices.toggle_options(),
                self.settings.taskbar_auto_hide_before_activation,
            )
        } else {
            app::activate(choices.toggle_options())
        };

        let mut errors = report.errors;
        let audio_result = if choices.change_output_device {
            if was_enabled {
                self.settings
                    .output_before_activation
                    .as_ref()
                    .ok_or_else(|| {
                        "Audio output: The original output device is unavailable.".to_owned()
                    })
                    .and_then(audio::restore_output)
            } else {
                self.settings
                    .gaming_output_device_id
                    .as_deref()
                    .ok_or_else(|| "Audio output: No Gaming Mode device was selected.".to_owned())
                    .and_then(audio::select_output)
            }
        } else {
            Ok(())
        };
        if let Err(error) = audio_result {
            errors.push(error);
        }
        self.restore_actions_failed = was_enabled && !errors.is_empty();
        if self.restore_actions_failed {
            self.settings.active_mode_features = Some(choices.clone());
            errors.push(
                "Uncheck a failing action if Windows cannot restore it, then retry.".to_owned(),
            );
            if let Some(pending) = self.settings.pending_resolution {
                let target = pending.target;
                self.pending_resolution = Some((
                    pending,
                    format!("The saved resolution change to {target} still needs to be retried."),
                ));
            }
        } else {
            if was_enabled {
                let original_taskbar_auto_hide = self.settings.taskbar_auto_hide_before_activation;
                let previous_pending_resolution = self.settings.pending_resolution;
                let original_output_snapshot = self.settings.output_before_activation.clone();
                self.settings.gaming_mode_active = false;
                self.settings.active_mode_features = None;
                self.settings.taskbar_auto_hide_before_activation = None;
                self.settings.pending_resolution = pending_resolution_for_mode(
                    false,
                    choices.change_resolution,
                    self.settings.desktop_resolution,
                );
                self.settings.output_before_activation = None;

                // Windows actions are restored, but the app remains logically
                // active until the Desktop state is durably committed.
                if let Err(error) = self.persist_mode_state(frame) {
                    self.settings.gaming_mode_active = true;
                    self.settings.active_mode_features = Some(choices.clone());
                    self.settings.taskbar_auto_hide_before_activation = original_taskbar_auto_hide;
                    self.settings.pending_resolution = previous_pending_resolution;
                    self.settings.output_before_activation = original_output_snapshot;
                    self.restore_actions_failed = true;
                    errors.push(format!(
                        "Windows was restored, but Desktop Mode could not be saved. Retry the restore: {error}"
                    ));
                }
            }

            if self.settings.gaming_mode_active != was_enabled {
                self.apply_pending_resolution();
            }
        }

        if self.settings.sounds_enabled
            && let Some(cue) = sound::transition_cue(was_enabled, self.settings.gaming_mode_active)
        {
            self.sound_player.play(cue);
        }

        self.errors = errors;
        if let Err(error) = self.persist_mode_state(frame) {
            push_unique_error(
                &mut self.errors,
                format!("Could not update the persistent mode state: {error}"),
            );
        }
        // ChangeDisplaySettingsW can synchronously alter the viewport placement.
        // Reassert the immutable dimensions now and again after Windows settles.
        self.begin_fixed_size_settle(context);
    }

    fn persist_mode_state(&self, frame: &mut eframe::Frame) -> Result<(), String> {
        save_durable_mode_state(&DurableModeState::from_settings(&self.settings))?;
        if let Some(storage) = frame.storage_mut() {
            store_settings(storage, &self.settings);
            storage.flush();
        }
        Ok(())
    }

    fn apply_pending_resolution(&mut self) {
        let Some(pending) = self.settings.pending_resolution else {
            return;
        };
        let target = pending.target;
        match self.apply_profile_resolution(target) {
            Ok(()) => self.settings.pending_resolution = None,
            Err(error) => self.pending_resolution = Some((pending, error)),
        }
    }

    fn begin_fixed_size_settle(&mut self, context: &egui::Context) {
        self.fixed_size_settle_passes = FIXED_SIZE_SETTLE_PASSES;
        request_fixed_window_size(context);
        context.request_repaint();
    }

    fn visual_state(&self, gaming_mode: bool, now: f64) -> ModeVisualState {
        match (gaming_mode, self.mode_transition) {
            (false, Some(ModeTransitionState::EnterPressed(_))) => ModeVisualState::EnterPressed,
            (false, Some(ModeTransitionState::Countdown(started_at))) => {
                ModeVisualState::Countdown(countdown_digit(started_at, now))
            }
            (false, Some(ModeTransitionState::ApplyingGaming(_))) => ModeVisualState::Activating,
            (true, Some(ModeTransitionState::RestorePressed(_))) => ModeVisualState::RestorePressed,
            (true, Some(ModeTransitionState::ApplyingDesktop(_))) => ModeVisualState::Restoring,
            (false, _) => ModeVisualState::Desktop,
            (true, _) => ModeVisualState::Gaming,
        }
    }

    fn request_mode_toggle(&mut self, context: &egui::Context, now: f64) {
        self.request_mode_toggle_with_resolution(context, now, true);
    }

    fn request_mode_toggle_with_resolution(
        &mut self,
        context: &egui::Context,
        now: f64,
        change_resolution: bool,
    ) {
        if self.mode_transition.is_some() {
            return;
        }

        self.errors.clear();
        self.pending_resolution = None;
        let gaming_mode = self.settings.gaming_mode_active;
        self.activation_changes_resolution = gaming_mode || change_resolution;
        self.mode_transition = Some(if gaming_mode {
            ModeTransitionState::RestorePressed(now)
        } else {
            ModeTransitionState::EnterPressed(now)
        });
        self.last_countdown_digit = None;
        if self.settings.sounds_enabled {
            if gaming_mode {
                self.sound_player.warm_up();
            } else if self.sound_player.prepare_countdown() {
                // Digit 3 is already queued directly after the silent
                // click-animation lead-in on the persistent output handle.
                self.last_countdown_digit = Some(3);
            }
        }
        context.request_repaint();
    }

    fn clear_mode_transition(&mut self) {
        self.mode_transition = None;
        self.last_countdown_digit = None;
    }

    fn sync_countdown_sound(&mut self, digit: u8) {
        if mark_countdown_digit(&mut self.last_countdown_digit, digit)
            && self.settings.sounds_enabled
        {
            self.sound_player.play(SoundCue::Countdown);
        }
    }

    fn request_close_to_tray(&mut self, context: &egui::Context) {
        if self.settings.tray_close_notice_acknowledged {
            hide_to_tray(context);
        } else {
            self.close_notice_open = true;
            context.request_repaint();
        }
    }

    fn desired_tray_icon_state(&self, state: ModeVisualState, now: f64) -> TrayIconState {
        match state {
            ModeVisualState::Desktop | ModeVisualState::EnterPressed => TrayIconState::Desktop,
            ModeVisualState::Gaming | ModeVisualState::RestorePressed => TrayIconState::Gaming,
            ModeVisualState::Countdown(_)
            | ModeVisualState::Activating
            | ModeVisualState::Restoring => activating_tray_icon_state(
                self.mode_transition
                    .map_or(0.0, |transition| now - transition.started_at()),
            ),
        }
    }

    fn sync_tray_icon(&mut self, state: TrayIconState) {
        if state == self.tray_icon_state {
            return;
        }

        let result = tray_status_icon(state)
            .and_then(|icon| self.tray_icon.set_icon(Some(icon)).map_err(Into::into));
        match result {
            Ok(()) => self.tray_icon_state = state,
            Err(error) => {
                let message = format!("Could not update the system-tray status icon: {error}");
                if !self.errors.contains(&message) {
                    self.errors.push(message);
                }
            }
        }
    }

    fn sync_tray_menu(&mut self, state: ModeVisualState) {
        if state == self.tray_menu_state {
            return;
        }

        let (label, enabled) = tray_mode_menu_presentation(state);
        self.tray_mode_item.set_text(label);
        self.tray_mode_item.set_enabled(enabled);
        self.tray_mode_without_resolution_item
            .set_enabled(matches!(state, ModeVisualState::Desktop));
        self.tray_menu_state = state;
    }

    fn apply_profile_resolution(&self, target: Resolution) -> Result<(), String> {
        let current = display::primary_resolution()
            .map_err(|error| format!("Could not read the current resolution: {error}"))?;
        if current == target {
            return Ok(());
        }

        match display::set_primary_resolution(target) {
            Ok(ChangeOutcome::Applied) => {
                let applied = display::primary_resolution()
                    .map_err(|error| format!("Could not verify {target}: {error}"))?;
                if applied == target {
                    Ok(())
                } else {
                    Err(format!(
                        "Windows kept the primary monitor at {applied} instead of {target}."
                    ))
                }
            }
            Ok(ChangeOutcome::RestartRequired) => Err(format!(
                "Windows accepted {target}, but a restart is required before it can be used."
            )),
            Err(error) => Err(format!("Could not apply {target}: {error}")),
        }
    }

    fn show_header(&self, ui: &mut egui::Ui, state: ModeVisualState) {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), HEADER_HEIGHT),
            egui::Sense::hover(),
        );
        let wordmark = self.mode_wordmarks.for_state(state);
        let wordmark_size = Vec2::new(
            wordmark.aspect_ratio * WORDMARK_DISPLAY_HEIGHT,
            WORDMARK_DISPLAY_HEIGHT,
        );
        let status_width = 20.0 + 8.0 + wordmark_size.x;
        let status_right = rect.right() - HEADER_CLOSE_CLEARANCE;
        let status_rect = egui::Rect::from_min_size(
            egui::pos2(
                status_right - status_width,
                rect.center().y - wordmark_size.y * 0.5,
            ),
            Vec2::new(status_width, wordmark_size.y),
        );
        let title_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(status_rect.left() - 12.0, rect.bottom()),
        );

        let mut title_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(title_rect)
                .layout(Layout::top_down(Align::LEFT)),
        );
        title_ui.spacing_mut().item_spacing.y = 3.0;
        title_ui.label(
            RichText::new("Borderless Gaming")
                .size(24.0)
                .strong()
                .color(Color32::WHITE),
        );
        title_ui.label(
            RichText::new("The perfect gaming XP in one click")
                .size(13.0)
                .color(TEXT_MUTED),
        );

        let mut status_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(status_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        status_ui.spacing_mut().item_spacing.x = 8.0;
        mode_led(&mut status_ui, state);
        status_ui
            .add(
                egui::Image::new(&wordmark.texture)
                    .fit_to_exact_size(wordmark_size)
                    .alt_text(match state {
                        ModeVisualState::Desktop
                        | ModeVisualState::EnterPressed
                        | ModeVisualState::Countdown(_) => "Desktop mode",
                        ModeVisualState::Activating => "Gaming mode activating",
                        ModeVisualState::Gaming | ModeVisualState::RestorePressed => "Gaming mode",
                        ModeVisualState::Restoring => "Desktop mode restoring",
                    }),
            )
            .on_hover_text(mode_status_description(state));
    }

    fn show_mode_button(&mut self, ui: &mut egui::Ui, state: ModeVisualState) {
        let (label, spinner) = mode_button_presentation(state);

        let enabled = mode_button_enabled(state);
        let width = ui.available_width();
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                animated_text_button(
                    ui,
                    &label,
                    Vec2::new(width, 48.0),
                    AnimatedButtonTone::Primary,
                    spinner,
                )
            })
            .inner;

        if response.clicked() {
            self.request_mode_toggle(ui.ctx(), ui.input(|input| input.time));
        }
    }

    fn show_gaming_options(
        &mut self,
        ui: &mut egui::Ui,
        gaming_mode: bool,
        frame: &mut eframe::Frame,
    ) {
        let previous_gaming_resolution = self.settings.gaming_resolution;
        let previous_output_device = self.settings.gaming_output_device_id.clone();
        let resolutions = self.display.resolutions.clone();
        let available_resolutions = self.display.available_resolutions.clone();
        let native_resolution = self.display.native;

        card(ui, |ui| {
            ui.add_enabled_ui(!gaming_mode || self.restore_actions_failed, |ui| {
                accent_checkbox(
                    ui,
                    &mut self.settings.features.taskbar_auto_hide,
                    "Auto-hide the taskbar",
                );
                accent_checkbox(
                    ui,
                    &mut self.settings.features.desktop_icons,
                    "Hide desktop icons",
                );
                accent_checkbox(
                    ui,
                    &mut self.settings.features.desktop_background,
                    "Use a solid black desktop background",
                );
                accent_checkbox(
                    ui,
                    &mut self.settings.features.minimize_all_windows,
                    "Minimize open windows",
                );
            });

            if gaming_mode && self.restore_actions_failed {
                ui.add_space(3.0);
                ui.label(
                    RichText::new("Adjust the failing action, then retry desktop restore.")
                        .size(12.0)
                        .color(TEXT_MUTED),
                );
            }

            ui.add_space(4.0);
            ui.add_enabled_ui(!gaming_mode || self.restore_actions_failed, |ui| {
                accent_checkbox(
                    ui,
                    &mut self.settings.features.change_resolution,
                    "Change resolution:",
                );
            });
            ui.add_space(3.0);
            gaming_resolution_combo_box(
                ui,
                &resolutions,
                &available_resolutions,
                native_resolution,
                &mut self.settings.gaming_resolution,
            );

            ui.add_space(7.0);
            ui.add_enabled_ui(!gaming_mode || self.restore_actions_failed, |ui| {
                accent_checkbox(
                    ui,
                    &mut self.settings.features.change_output_device,
                    "Change audio output:",
                );
            });
            ui.add_space(3.0);
            gaming_output_combo_box(
                ui,
                &self.audio.devices,
                &mut self.settings.gaming_output_device_id,
            );

            if gaming_mode && self.restore_actions_failed {
                self.settings.active_mode_features = Some(self.settings.features.clone());
            }
        });

        if self.settings.gaming_resolution != previous_gaming_resolution {
            self.errors.clear();
            if cancel_pending_gaming_resolution(&mut self.settings.pending_resolution) {
                self.pending_resolution = None;
            }
            if let Err(error) = self.persist_mode_state(frame) {
                self.errors.push(format!(
                    "Could not save the updated Gaming resolution: {error}"
                ));
            }
        }
        if self.settings.gaming_output_device_id != previous_output_device {
            self.errors.clear();
            if let Err(error) = self.persist_mode_state(frame) {
                self.errors.push(format!(
                    "Could not save the updated Gaming audio output: {error}"
                ));
            }
        }
    }

    fn show_application_behavior(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("Application behavior").size(15.0).strong());
            ui.add_space(4.0);

            let previous_startup = self.settings.startup_at_login;
            let startup_response =
                accent_checkbox(ui, &mut self.settings.startup_at_login, "Start at login");
            if startup_response.changed() {
                let result = behavior::set_startup_at_login(
                    self.settings.startup_at_login,
                    self.settings.startup_minimized,
                );
                if let Err(error) = result {
                    self.settings.startup_at_login = previous_startup;
                    push_unique_error(&mut self.errors, error);
                } else {
                    remove_errors_containing(&mut self.errors, "Windows login setting");
                }
            }

            let previous_minimized = self.settings.startup_minimized;
            let minimized_response = accent_checkbox(
                ui,
                &mut self.settings.startup_minimized,
                "Start minimized into the system tray",
            );
            if minimized_response.changed() && self.settings.startup_at_login {
                if let Err(error) =
                    behavior::set_startup_at_login(true, self.settings.startup_minimized)
                {
                    self.settings.startup_minimized = previous_minimized;
                    push_unique_error(&mut self.errors, error);
                } else {
                    remove_errors_containing(&mut self.errors, "Windows login setting");
                }
            }

            accent_checkbox(ui, &mut self.settings.sounds_enabled, "Enable sounds");

            let previous_transparency = self.settings.window_transparency;
            let transparency_response =
                window_transparency_slider(ui, &mut self.settings.window_transparency);
            if transparency_response.changed() {
                let result = self.native_window.ok_or_else(|| {
                    "Could not access the native window for transparency.".to_owned()
                });
                let result = result.and_then(|hwnd| {
                    behavior::set_window_transparency(hwnd, self.settings.window_transparency)
                });

                if let Err(error) = result {
                    self.settings.window_transparency = previous_transparency;
                    push_unique_error(&mut self.errors, error);
                } else {
                    remove_errors_containing(&mut self.errors, "transparency");
                }
            }
        });
    }

    fn show_errors(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.errors.is_empty() {
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new("!")
                        .strong()
                        .color(Color32::from_rgb(248, 113, 113)),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(self.errors.join("\n"))
                            .size(12.5)
                            .color(Color32::from_rgb(248, 170, 170)),
                    )
                    .wrap(),
                );
            });
        }

        if let Some((pending, error)) = self.pending_resolution.clone() {
            let target = pending.target;
            if !self.errors.is_empty() {
                ui.add_space(5.0);
            }
            ui.horizontal_top(|ui| {
                ui.label(
                    RichText::new("!")
                        .strong()
                        .color(Color32::from_rgb(248, 113, 113)),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(error)
                            .size(12.5)
                            .color(Color32::from_rgb(248, 170, 170)),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);
            if animated_text_button(
                ui,
                "Retry resolution",
                Vec2::new(138.0, 30.0),
                AnimatedButtonTone::Secondary,
                false,
            )
            .clicked()
            {
                match self.apply_profile_resolution(target) {
                    Ok(()) => {
                        self.settings.pending_resolution = None;
                        self.pending_resolution = None;
                        if let Err(error) = self.persist_mode_state(frame) {
                            self.settings.pending_resolution = Some(pending);
                            self.pending_resolution = Some((
                                pending,
                                format!(
                                    "The resolution was applied, but its completion could not be saved: {error}"
                                ),
                            ));
                        }
                    }
                    Err(error) => {
                        self.settings.pending_resolution = Some(pending);
                        self.pending_resolution = Some((pending, error));
                    }
                }
                self.begin_fixed_size_settle(ui.ctx());
            }
        }
    }

    fn show_close_notice(&mut self, context: &egui::Context) {
        if !self.close_notice_open {
            return;
        }

        let accepted = egui::Modal::new(egui::Id::new("tray-close-notice"))
            .backdrop_color(Color32::from_black_alpha(196))
            .frame(
                Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.72)))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(326.0);
                ui.vertical_centered(|ui| {
                    let (icon_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        icon_rect.center(),
                        21.0,
                        Color32::from_rgba_unmultiplied(112, 98, 255, 34),
                    );
                    ui.painter()
                        .circle_stroke(icon_rect.center(), 15.0, Stroke::new(1.5, ACCENT));
                    ui.painter().text(
                        icon_rect.center(),
                        Align2::CENTER_CENTER,
                        "i",
                        FontId::new(20.0, FontFamily::Proportional),
                        Color32::WHITE,
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Minimized to the tray")
                            .size(19.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "The application won't close, it will minimize to the system tray.",
                            )
                            .size(13.5)
                            .color(TEXT_MUTED),
                        )
                        .wrap(),
                    );
                    ui.add_space(18.0);
                    let width = ui.available_width();
                    animated_text_button(
                        ui,
                        "OK",
                        Vec2::new(width, 40.0),
                        AnimatedButtonTone::Primary,
                        false,
                    )
                    .clicked()
                })
                .inner
            })
            .inner;

        if accepted {
            self.settings.tray_close_notice_acknowledged = true;
            self.close_notice_open = false;
            hide_to_tray(context);
        }
    }
}

fn app_fonts() -> egui::FontDefinitions {
    const FONT_NAME: &str = "Ubuntu-Light";

    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(
            epaint_default_fonts::UBUNTU_LIGHT,
        )),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .get_mut(&family)
            .expect("egui defines every built-in font family")
            .push(FONT_NAME.to_owned());
    }
    fonts
}

fn configure_context(context: &egui::Context) {
    // No egui interaction/layout state is persistent. In particular, never
    // restore a scroll offset captured from an iconic 160x64 viewport.
    context.memory_mut(|memory| *memory = egui::Memory::default());

    // `set_fonts` queues the definitions in egui memory for the next pass, so
    // it must happen after the memory reset above.
    context.set_fonts(app_fonts());
    configure_style(context);
}

impl eframe::App for GuiApp {
    fn logic(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        self.reapply_window_transparency_after_show(context, frame);

        let now = context.input(|input| input.time);

        while let Ok(command) = self.tray_commands.try_recv() {
            match command {
                TrayCommand::Open => {
                    // winit rewrites the extended window style while applying
                    // Visible(true), which removes WS_EX_LAYERED. Repair it on
                    // the following visible frame just as we do at startup.
                    self.window_transparency_reapply_pending = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    request_fixed_window_size(context);
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.begin_fixed_size_settle(context);
                }
                TrayCommand::ToggleMode => self.request_mode_toggle(context, now),
                TrayCommand::ActivateModeWithoutResolution => {
                    self.request_mode_toggle_with_resolution(context, now, false);
                }
                TrayCommand::Quit => {
                    self.quitting = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        let mut gaming_mode = self.settings.gaming_mode_active;

        if let Some(transition) = self.mode_transition {
            match (gaming_mode, transition) {
                (false, ModeTransitionState::EnterPressed(clicked_at))
                    if mode_button_click_animation_finished(clicked_at, now) =>
                {
                    self.mode_transition = Some(ModeTransitionState::Countdown(now));
                    self.sync_countdown_sound(3);
                    context.request_repaint();
                }
                (false, ModeTransitionState::EnterPressed(_)) => {
                    context.request_repaint_after(Duration::from_millis(16));
                }
                (false, ModeTransitionState::Countdown(started_at))
                    if countdown_finished(started_at, now) =>
                {
                    // Render the final "Activating..." state before running the
                    // synchronous Windows actions on the following frame.
                    self.mode_transition = Some(ModeTransitionState::ApplyingGaming(started_at));
                    context.request_repaint();
                }
                (false, ModeTransitionState::Countdown(_started_at)) => {
                    self.sync_countdown_sound(countdown_digit(_started_at, now));
                    context.request_repaint_after(Duration::from_millis(16));
                }
                (false, ModeTransitionState::ApplyingGaming(_)) => {
                    self.clear_mode_transition();
                    self.toggle_mode(context, frame);
                    gaming_mode = self.settings.gaming_mode_active;
                }
                (true, ModeTransitionState::RestorePressed(clicked_at))
                    if mode_button_click_animation_finished(clicked_at, now) =>
                {
                    // As with activation, render "Restoring..." before the
                    // synchronous restore work begins on the following frame.
                    self.mode_transition = Some(ModeTransitionState::ApplyingDesktop(clicked_at));
                    context.request_repaint();
                }
                (true, ModeTransitionState::RestorePressed(_)) => {
                    context.request_repaint_after(Duration::from_millis(16));
                }
                (true, ModeTransitionState::ApplyingDesktop(_)) => {
                    self.clear_mode_transition();
                    self.toggle_mode(context, frame);
                    gaming_mode = self.settings.gaming_mode_active;
                }
                // A stale transition no longer matches the persisted mode.
                _ => self.clear_mode_transition(),
            }
        }

        let state = self.visual_state(gaming_mode, now);
        let tray_icon_state = self.desired_tray_icon_state(state, now);
        self.sync_tray_icon(tray_icon_state);
        self.sync_tray_menu(state);

        if self.fixed_size_settle_passes > 0 {
            request_fixed_window_size(context);
            self.fixed_size_settle_passes -= 1;
            context.request_repaint_after(Duration::from_millis(16));
        }

        let size_changed_while_visible = context.input(|input| {
            let viewport = input.viewport();
            viewport.minimized != Some(true)
                && viewport
                    .inner_rect
                    .is_some_and(|rect| !is_fixed_window_size(rect.size()))
        });
        if size_changed_while_visible {
            request_fixed_window_size(context);
        }

        let close_requested = context.input(|input| input.viewport().close_requested());

        if close_requested && !self.quitting {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close_to_tray(context);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Register this before the controls so they keep interaction priority
        // over the full-window drag surface.
        let drag_response = ui.interact(
            ui.max_rect(),
            ui.make_persistent_id("window-drag-background"),
            egui::Sense::click_and_drag(),
        );
        if drag_response.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let gaming_mode = self.settings.gaming_mode_active;
        let state = self.visual_state(gaming_mode, ui.input(|input| input.time));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BACKGROUND).inner_margin(Margin {
                left: 24,
                right: 24,
                top: 32,
                // The compact window needs this space for the final card's
                // bottom padding and stroke, otherwise the scroll clip cuts it off.
                bottom: MAIN_BOTTOM_INSET,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("main-controls")
                    .max_height(ui.available_height())
                    .auto_shrink([false, false])
                    // A minimized native window can briefly report a zero-sized
                    // viewport and make egui animate a stale scrollbar on restore.
                    // Keep scrolling available for small screens without showing
                    // that transient outer-page bar.
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.show_header(ui, state);
                        ui.add_space(8.0);
                        self.show_mode_button(ui, state);
                        if !self.errors.is_empty() || self.pending_resolution.is_some() {
                            ui.add_space(9.0);
                            self.show_errors(ui, frame);
                        }
                        ui.add_space(9.0);
                        self.show_gaming_options(ui, gaming_mode, frame);
                        ui.add_space(9.0);
                        self.show_application_behavior(ui);
                    });
            });

        if window_close_button(ui.ctx()) {
            self.request_close_to_tray(ui.ctx());
        }
        window_version_label(ui.ctx());

        self.show_close_notice(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        store_settings(storage, &self.settings);
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(5)
    }
}

fn store_settings(storage: &mut dyn eframe::Storage, settings: &Settings) {
    // Migrate away from eframe's native geometry/UI persistence. These keys
    // may contain the 160x64 iconic size and its associated scroll state.
    storage.remove_string(EFRAME_WINDOW_STORAGE_KEY);
    storage.remove_string(EFRAME_MEMORY_STORAGE_KEY);
    eframe::set_value(storage, STORAGE_KEY, settings);
}

fn native_window_handle(window: &impl raw_window_handle::HasWindowHandle) -> Option<HWND> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

fn push_unique_error(errors: &mut Vec<String>, error: String) {
    if !errors.contains(&error) {
        errors.push(error);
    }
}

fn remove_errors_containing(errors: &mut Vec<String>, fragment: &str) {
    errors.retain(|error| !error.contains(fragment));
}

fn request_fixed_window_size(context: &egui::Context) {
    let size = Vec2::from(WINDOW_SIZE);
    context.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
    context.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
    context.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
    context.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
}

fn hide_to_tray(context: &egui::Context) {
    // A tray-hidden window is explicitly restored out of the iconic state so
    // Windows cannot retain the 160x64 minimized geometry. Visibility alone
    // removes it from the taskbar; ordinary native minimization is untouched.
    context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
}

fn is_fixed_window_size(size: Vec2) -> bool {
    const EPSILON: f32 = 0.5;
    let expected = Vec2::from(WINDOW_SIZE);
    (size.x - expected.x).abs() <= EPSILON && (size.y - expected.y).abs() <= EPSILON
}

fn configure_style(context: &egui::Context) {
    context.options_mut(|options| options.zoom_with_keyboard = false);
    context.set_zoom_factor(1.0);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(10, 13, 20);
    visuals.faint_bg_color = Color32::from_rgb(28, 34, 46);
    visuals.selection.bg_fill = ACCENT;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(50, 58, 78);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.bg_fill = ACCENT_HOVER;
    visuals.widgets.open.corner_radius = CornerRadius::same(8);
    context.set_visuals(visuals);

    context.style_mut_of(egui::Theme::Dark, |style| {
        style.animation_time = 0.18;
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.interact_size = Vec2::new(32.0, 28.0);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
    });
}

fn create_tray(
    context: &egui::Context,
    icon_state: TrayIconState,
    mode_state: ModeVisualState,
) -> Result<(TrayIcon, TrayMenuItems, Receiver<TrayCommand>), Box<dyn Error + Send + Sync>> {
    let menu = Menu::new();
    let open_item = MenuItem::new("Open", true, None);
    let (mode_label, mode_enabled) = tray_mode_menu_presentation(mode_state);
    let mode_item = MenuItem::new(mode_label, mode_enabled, None);
    let mode_without_resolution_item = MenuItem::new(
        "Activate without resolution",
        matches!(mode_state, ModeVisualState::Desktop),
        None,
    );
    let open_separator = PredefinedMenuItem::separator();
    let quit_separator = PredefinedMenuItem::separator();
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&open_item)?;
    menu.append(&open_separator)?;
    menu.append(&mode_item)?;
    menu.append(&mode_without_resolution_item)?;
    menu.append(&quit_separator)?;
    menu.append(&quit_item)?;
    unsafe {
        SetMenuDefaultItem(
            HMENU(menu.hpopupmenu() as *mut core::ffi::c_void),
            TRAY_OPEN_MENU_POSITION,
            1,
        )?;
    }

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip(APP_TITLE)
        .with_icon(tray_status_icon(icon_state)?)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(true)
        .build()?;

    let (sender, receiver) = mpsc::channel();

    let open_id = open_item.id().clone();
    let mode_id = mode_item.id().clone();
    let mode_without_resolution_id = mode_without_resolution_item.id().clone();
    let quit_id = quit_item.id().clone();
    let menu_sender = sender.clone();
    let menu_context = context.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = if event.id == open_id {
            Some(TrayCommand::Open)
        } else if event.id == mode_id {
            Some(TrayCommand::ToggleMode)
        } else if event.id == mode_without_resolution_id {
            Some(TrayCommand::ActivateModeWithoutResolution)
        } else if event.id == quit_id {
            Some(TrayCommand::Quit)
        } else {
            None
        };

        if let Some(command) = command {
            let _ = menu_sender.send(command);
            menu_context.request_repaint();
        }
    }));

    let tray_context = context.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if matches!(
            event,
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            }
        ) {
            let _ = sender.send(tray_default_command());
            tray_context.request_repaint();
        }
    }));

    Ok((
        tray_icon,
        TrayMenuItems {
            mode: mode_item,
            mode_without_resolution: mode_without_resolution_item,
        },
        receiver,
    ))
}

fn tray_status_icon(state: TrayIconState) -> Result<Icon, Box<dyn Error + Send + Sync>> {
    let bytes = match state {
        TrayIconState::Desktop => include_bytes!("../assets/runtime/tray-desktop.png").as_slice(),
        TrayIconState::ActivatingBright => {
            include_bytes!("../assets/runtime/tray-activating.png").as_slice()
        }
        TrayIconState::ActivatingDim => {
            include_bytes!("../assets/runtime/tray-activating-dim.png").as_slice()
        }
        TrayIconState::Gaming => include_bytes!("../assets/runtime/tray-gaming.png").as_slice(),
    };
    let image = decode_png(bytes)?;
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}

fn gaming_resolution_combo_box(
    ui: &mut egui::Ui,
    resolutions: &[Resolution],
    available_resolutions: &[Resolution],
    native: Option<Resolution>,
    selection: &mut Option<Resolution>,
) {
    if resolutions.is_empty() {
        Frame::new()
            .fill(Color32::from_rgb(14, 18, 28))
            .stroke(Stroke::new(1.0, CARD_BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("No primary-monitor modes available")
                        .size(12.5)
                        .color(Color32::from_rgb(248, 113, 113)),
                );
            });
        return;
    }

    let selected_text = selection.map_or_else(
        || "Select a resolution".to_owned(),
        |resolution| {
            resolution_picker_label(
                resolution,
                available_resolutions.contains(&resolution),
                native,
            )
        },
    );

    ui.scope(|ui| {
        style_resolution_combo(ui);
        egui::ComboBox::from_id_salt("gaming-resolution-combo")
            .width(ui.available_width())
            .selected_text(
                RichText::new(selected_text)
                    .size(13.5)
                    .color(Color32::WHITE),
            )
            .popup_style(resolution_combo_popup_style())
            .icon(|ui, rect, visuals, is_open| {
                let half_width = rect.width() * 0.32;
                let half_height = rect.height() * 0.18;
                let center = rect.center();
                let stroke = Stroke::new(
                    1.7,
                    if is_open {
                        ACCENT_HOVER
                    } else {
                        visuals.fg_stroke.color
                    },
                );
                let points = if is_open {
                    [
                        center + Vec2::new(-half_width, half_height),
                        center + Vec2::new(0.0, -half_height),
                        center + Vec2::new(half_width, half_height),
                    ]
                } else {
                    [
                        center + Vec2::new(-half_width, -half_height),
                        center + Vec2::new(0.0, half_height),
                        center + Vec2::new(half_width, -half_height),
                    ]
                };
                ui.painter().line_segment([points[0], points[1]], stroke);
                ui.painter().line_segment([points[1], points[2]], stroke);
            })
            .show_ui(ui, |ui| {
                for resolution in resolutions.iter().copied() {
                    let available = available_resolutions.contains(&resolution);
                    let label = resolution_picker_label(resolution, available, native);
                    ui.add_enabled_ui(available, |ui| {
                        ui.selectable_value(selection, Some(resolution), label);
                    });
                }
            });
    });
}

fn gaming_output_combo_box(
    ui: &mut egui::Ui,
    devices: &[OutputDevice],
    selection: &mut Option<String>,
) {
    let selected_text = selection.as_ref().map_or_else(
        || "Select an output device".to_owned(),
        |selected| {
            devices
                .iter()
                .find(|device| &device.id == selected)
                .map(|device| device.name.clone())
                .unwrap_or_else(|| "Selected device (unavailable)".to_owned())
        },
    );

    ui.scope(|ui| {
        style_resolution_combo(ui);
        egui::ComboBox::from_id_salt("gaming-output-combo")
            .width(ui.available_width())
            .selected_text(
                RichText::new(selected_text)
                    .size(13.5)
                    .color(Color32::WHITE),
            )
            .popup_style(resolution_combo_popup_style())
            .show_ui(ui, |ui| {
                for device in devices {
                    ui.selectable_value(selection, Some(device.id.clone()), &device.name);
                }
            });
    });
}

fn style_resolution_combo(ui: &mut egui::Ui) {
    ui.spacing_mut().button_padding = Vec2::new(12.0, 8.0);
    let widgets = &mut ui.visuals_mut().widgets;

    widgets.inactive.weak_bg_fill = Color32::from_rgb(14, 18, 28);
    widgets.inactive.bg_stroke = Stroke::new(1.0, CARD_BORDER);
    widgets.inactive.corner_radius = CornerRadius::same(8);
    widgets.inactive.fg_stroke = Stroke::new(1.2, TEXT_MUTED);

    widgets.hovered.weak_bg_fill = Color32::from_rgb(29, 33, 50);
    widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.hovered.corner_radius = CornerRadius::same(8);
    widgets.hovered.fg_stroke = Stroke::new(1.4, Color32::WHITE);

    widgets.active.weak_bg_fill = Color32::from_rgb(39, 34, 79);
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_HOVER);
    widgets.active.corner_radius = CornerRadius::same(8);
    widgets.active.fg_stroke = Stroke::new(1.4, Color32::WHITE);

    widgets.open = widgets.active;
}

fn resolution_combo_popup_style() -> egui::style::StyleModifier {
    egui::style::StyleModifier::new(|style| {
        style.spacing.item_spacing.y = 3.0;
        style.visuals.window_fill = Color32::from_rgb(14, 18, 28);
        style.visuals.window_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.72));
        style.visuals.menu_corner_radius = CornerRadius::same(8);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(39, 34, 79);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(48, 41, 99);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(6);
        style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.72);
        style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    })
}

fn resolution_picker_label(
    resolution: Resolution,
    available: bool,
    native: Option<Resolution>,
) -> String {
    if native == Some(resolution) {
        format!("{resolution} (native)")
    } else if !available {
        format!("{resolution} (unavailable)")
    } else {
        resolution.to_string()
    }
}

fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, CARD_BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(15))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
}

fn accent_checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), egui::Sense::click());

    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }

    let selected = *checked;
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), selected, label)
    });

    if ui.is_rect_visible(rect) {
        let context = ui.ctx();
        let checked_t = context.animate_bool_with_time(response.id.with("checked"), selected, 0.18);
        let hover_t =
            context.animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.15);
        let press_t = context.animate_bool_with_time(
            response.id.with("press"),
            response.is_pointer_button_down_on(),
            0.10,
        );
        let enabled_alpha = if ui.is_enabled() {
            1.0
        } else {
            ui.visuals().disabled_alpha()
        };
        let painter = ui.painter().with_clip_rect(rect);

        if hover_t > 0.0 {
            painter.rect_filled(
                rect,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(112, 98, 255, (18.0 * hover_t) as u8),
            );
        }

        let icon_center = egui::pos2(rect.left() + 10.0, rect.center().y);
        let icon_rect =
            egui::Rect::from_center_size(icon_center, Vec2::splat(18.0)).shrink(press_t * 0.8);
        let unchecked = Color32::from_rgb(35, 42, 56);
        let fill = mix_color(unchecked, ACCENT, checked_t).gamma_multiply(enabled_alpha);
        let border = mix_color(CARD_BORDER, ACCENT_HOVER, checked_t).gamma_multiply(enabled_alpha);
        painter.rect(
            icon_rect,
            CornerRadius::same(5),
            fill,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );

        if checked_t > 0.0 {
            let p0 = egui::pos2(icon_rect.left() + 4.0, icon_rect.center().y);
            let p1 = egui::pos2(icon_rect.left() + 7.5, icon_rect.bottom() - 4.5);
            let p2 = egui::pos2(icon_rect.right() - 3.5, icon_rect.top() + 4.0);
            let points = if checked_t < 0.48 {
                vec![p0, p0.lerp(p1, checked_t / 0.48)]
            } else {
                vec![p0, p1, p1.lerp(p2, (checked_t - 0.48) / 0.52)]
            };
            painter.add(egui::Shape::line(
                points,
                Stroke::new(2.0, Color32::WHITE.gamma_multiply(enabled_alpha)),
            ));
        }

        painter.text(
            egui::pos2(rect.left() + 30.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::new(14.0, FontFamily::Proportional),
            Color32::WHITE.gamma_multiply(enabled_alpha),
        );
    }

    response
}

fn window_transparency_slider(ui: &mut egui::Ui, transparency: &mut u8) -> egui::Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(width, TRANSPARENCY_CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let slider_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 28.0),
        rect.right_bottom(),
    );
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 52.0, rect.top()),
        Vec2::new(52.0, 24.0),
    );
    let label_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(badge_rect.left() - 8.0, badge_rect.bottom()),
    );
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let label_response = label_ui.label("Window transparency");
    let mut slider_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(slider_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );

    slider_ui.spacing_mut().slider_width = slider_rect.width();
    slider_ui.spacing_mut().slider_rail_height = 12.0;
    slider_ui.spacing_mut().interact_size.y = slider_rect.height();
    {
        let visuals = slider_ui.visuals_mut();
        visuals.selection.bg_fill = ACCENT;
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 41, 56);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.5, ACCENT.gamma_multiply(0.72));
        visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
        visuals.widgets.hovered.bg_fill = ACCENT_HOVER;
        visuals.widgets.hovered.fg_stroke = Stroke::new(2.0, Color32::WHITE);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
        visuals.widgets.active.bg_fill = ACCENT;
        visuals.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
        visuals.widgets.active.corner_radius = CornerRadius::same(8);
    }

    let response = slider_ui
        .add(
            egui::Slider::new(transparency, 0..=behavior::MAX_TRANSPARENCY_PERCENT)
                .show_value(false)
                .trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle),
        )
        .labelled_by(label_response.id);

    let hover_t = ui.ctx().animate_bool_with_time(
        response.id.with("header-hover"),
        response.hovered() || response.has_focus(),
        0.16,
    );
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(
        badge_rect,
        CornerRadius::same(12),
        color_with_alpha(ACCENT, (24.0 + 22.0 * hover_t) as u8),
        Stroke::new(
            1.0,
            color_with_alpha(ACCENT_HOVER, (80.0 + 64.0 * hover_t) as u8),
        ),
        StrokeKind::Inside,
    );
    painter.text(
        badge_rect.center(),
        Align2::CENTER_CENTER,
        format!("{transparency}%"),
        FontId::new(12.0, FontFamily::Proportional),
        mix_color(TEXT_MUTED, Color32::WHITE, 0.45 + hover_t * 0.55),
    );

    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("0% is opaque; the safety limit is 80% transparency")
}

fn mode_status_description(state: ModeVisualState) -> String {
    match state {
        ModeVisualState::Desktop | ModeVisualState::EnterPressed => {
            "Desktop mode is active".to_owned()
        }
        ModeVisualState::Countdown(countdown) => {
            format!("Gaming mode activates in {countdown}")
        }
        ModeVisualState::Activating => "Gaming mode is activating".to_owned(),
        ModeVisualState::Gaming | ModeVisualState::RestorePressed => {
            "Gaming mode is active".to_owned()
        }
        ModeVisualState::Restoring => "Desktop mode is restoring".to_owned(),
    }
}

fn mode_led(ui: &mut egui::Ui, state: ModeVisualState) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
    let center = rect.center();
    let painter = ui.painter();

    let powered = !matches!(
        state,
        ModeVisualState::Desktop | ModeVisualState::EnterPressed
    );
    let gaming = matches!(
        state,
        ModeVisualState::Gaming | ModeVisualState::RestorePressed
    );
    let power_t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("power"), powered, 0.30);
    let green_t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("green"), gaming, 0.24);
    let active_color = mode_led_active_color(state, green_t);
    let color = mix_color(LED_OFF, active_color, power_t);
    let pulse = if matches!(
        state,
        ModeVisualState::Countdown(_) | ModeVisualState::Activating | ModeVisualState::Restoring
    ) {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
        let time = ui.input(|input| input.time);
        0.5 + 0.5 * (time * std::f64::consts::TAU * 1.35).sin() as f32
    } else {
        0.35
    };

    painter.circle_filled(
        center,
        7.0 + power_t * (1.0 + pulse),
        color_with_alpha(color, (18.0 + 36.0 * power_t * pulse) as u8),
    );
    painter.circle_filled(
        center,
        5.0 + 0.7 * power_t * pulse,
        color_with_alpha(color, (58.0 + 42.0 * power_t) as u8),
    );
    painter.circle_filled(center, 3.5 + 0.7 * power_t, color);
    if power_t > 0.05 {
        painter.circle_filled(
            center - Vec2::new(1.1, 1.1),
            1.1,
            mix_color(color, Color32::WHITE, 0.68).gamma_multiply(power_t),
        );
    }

    response.on_hover_text(mode_status_description(state));
}

fn mode_led_active_color(state: ModeVisualState, green_t: f32) -> Color32 {
    if matches!(state, ModeVisualState::Restoring) {
        ACTIVATING_ORANGE
    } else {
        mix_color(ACTIVATING_ORANGE, GAMING_GREEN, green_t)
    }
}

fn animated_text_button(
    ui: &mut egui::Ui,
    label: &str,
    size: Vec2,
    tone: AnimatedButtonTone,
    spinner: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));

    if ui.is_rect_visible(rect) {
        let context = ui.ctx();
        let hover_t =
            context.animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.16);
        let press_t = context.animate_bool_with_time(
            response.id.with("press"),
            response.is_pointer_button_down_on(),
            0.10,
        );
        let visual_rect = rect.shrink(press_t * 1.35);
        let (base, hover, stroke, font_size) = match tone {
            AnimatedButtonTone::Primary => {
                (ACCENT, ACCENT_HOVER, ACCENT.gamma_multiply(1.25), 16.0)
            }
            AnimatedButtonTone::Secondary => (
                Color32::from_rgb(31, 38, 52),
                Color32::from_rgb(47, 55, 74),
                CARD_BORDER,
                13.0,
            ),
        };
        let mut fill = mix_color(base, hover, hover_t);
        if !ui.is_enabled() {
            fill = fill.gamma_multiply(0.82);
        }
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect(
            visual_rect,
            CornerRadius::same(if matches!(tone, AnimatedButtonTone::Primary) {
                10
            } else {
                8
            }),
            fill,
            Stroke::new(1.0, stroke.gamma_multiply(0.82 + 0.18 * hover_t)),
            StrokeKind::Inside,
        );

        let text_color = if ui.is_enabled() {
            Color32::WHITE
        } else {
            Color32::WHITE.gamma_multiply(0.82)
        };
        let font = FontId::new(font_size, FontFamily::Proportional);
        if spinner {
            let galley = painter.layout_no_wrap(label.to_owned(), font.clone(), text_color);
            let spinner_size = 18.0;
            let gap = 9.0;
            let total_width = spinner_size + gap + galley.size().x;
            let left = visual_rect.center().x - total_width * 0.5;
            let spinner_rect = egui::Rect::from_min_size(
                egui::pos2(left, visual_rect.center().y - spinner_size * 0.5),
                Vec2::splat(spinner_size),
            );
            egui::Spinner::new()
                .size(spinner_size)
                .color(text_color)
                .paint_at(ui, spinner_rect);
            painter.galley(
                egui::pos2(
                    left + spinner_size + gap,
                    visual_rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                text_color,
            );
        } else {
            painter.text(
                visual_rect.center(),
                Align2::CENTER_CENTER,
                label,
                font,
                text_color,
            );
        }
    }

    response
}

fn animated_close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), "Close"));

    if ui.is_rect_visible(rect) {
        let hover_t =
            ui.ctx()
                .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.16);
        let press_t = ui.ctx().animate_bool_with_time(
            response.id.with("press"),
            response.is_pointer_button_down_on(),
            0.10,
        );
        let visual_rect = rect.shrink(press_t * 1.1);
        ui.painter().rect(
            visual_rect,
            CornerRadius::same(8),
            Color32::from_rgba_unmultiplied(112, 98, 255, (18.0 + 54.0 * hover_t) as u8),
            Stroke::new(
                1.0,
                color_with_alpha(ACCENT_HOVER, (68.0 + 92.0 * hover_t) as u8),
            ),
            StrokeKind::Inside,
        );
        let half = 4.2 - press_t * 0.45;
        let color = mix_color(TEXT_MUTED, Color32::WHITE, hover_t);
        ui.painter().line_segment(
            [
                visual_rect.center() - Vec2::splat(half),
                visual_rect.center() + Vec2::splat(half),
            ],
            Stroke::new(1.7, color),
        );
        ui.painter().line_segment(
            [
                visual_rect.center() + Vec2::new(-half, half),
                visual_rect.center() + Vec2::new(half, -half),
            ],
            Stroke::new(1.7, color),
        );
    }

    response.on_hover_text("Minimize to the system tray")
}

fn window_close_button(context: &egui::Context) -> bool {
    egui::Area::new(egui::Id::new("window-close-button"))
        .anchor(Align2::RIGHT_TOP, Vec2::new(-8.0, 8.0))
        .order(egui::Order::Foreground)
        .movable(false)
        .show(context, animated_close_button)
        .inner
        .clicked()
}

fn window_version_label(context: &egui::Context) {
    egui::Area::new(egui::Id::new("window-version"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-8.0, -4.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .movable(false)
        .show(context, |ui| {
            ui.label(
                RichText::new(APP_VERSION)
                    .size(10.5)
                    .color(color_with_alpha(TEXT_MUTED, 150)),
            );
        });
}

fn countdown_digit(started_at: f64, now: f64) -> u8 {
    (3_i32 - (now - started_at).max(0.0).floor() as i32).clamp(1, 3) as u8
}

fn countdown_finished(started_at: f64, now: f64) -> bool {
    now - started_at >= GAMING_COUNTDOWN_SECONDS
}

fn mode_button_click_animation_finished(clicked_at: f64, now: f64) -> bool {
    now - clicked_at >= MODE_BUTTON_CLICK_ANIMATION_SECONDS
}

fn mark_countdown_digit(last_digit: &mut Option<u8>, digit: u8) -> bool {
    if *last_digit == Some(digit) {
        return false;
    }

    *last_digit = Some(digit);
    true
}

fn activating_tray_icon_state(elapsed: f64) -> TrayIconState {
    let phase = (elapsed.max(0.0) / TRAY_BLINK_HALF_PERIOD_SECONDS).floor() as u64;
    if phase.is_multiple_of(2) {
        TrayIconState::ActivatingBright
    } else {
        TrayIconState::ActivatingDim
    }
}

fn tray_mode_menu_presentation(state: ModeVisualState) -> (String, bool) {
    match state {
        ModeVisualState::Desktop => ("Activate Gaming Mode".to_owned(), true),
        ModeVisualState::EnterPressed => ("Activate Gaming Mode".to_owned(), false),
        ModeVisualState::Countdown(countdown) => (format!("Activating in {countdown}…"), false),
        ModeVisualState::Activating => ("Activating…".to_owned(), false),
        ModeVisualState::Gaming => ("Restore Desktop Mode".to_owned(), true),
        ModeVisualState::RestorePressed => ("Restore Desktop Mode".to_owned(), false),
        ModeVisualState::Restoring => ("Restoring…".to_owned(), false),
    }
}

const fn tray_default_command() -> TrayCommand {
    TrayCommand::Open
}

fn mode_button_presentation(state: ModeVisualState) -> (String, bool) {
    match state {
        ModeVisualState::Desktop | ModeVisualState::EnterPressed => {
            ("Activate gaming mode".to_owned(), false)
        }
        ModeVisualState::Countdown(countdown) => (format!("Activating in {countdown}"), true),
        ModeVisualState::Activating => ("Activating...".to_owned(), false),
        ModeVisualState::Gaming | ModeVisualState::RestorePressed => {
            ("Restore desktop mode".to_owned(), false)
        }
        ModeVisualState::Restoring => ("Restoring...".to_owned(), false),
    }
}

fn mode_button_enabled(state: ModeVisualState) -> bool {
    matches!(state, ModeVisualState::Desktop | ModeVisualState::Gaming)
}

fn mix_color(left: Color32, right: Color32, amount: f32) -> Color32 {
    egui::lerp(
        egui::Rgba::from(left)..=egui::Rgba::from(right),
        amount.clamp(0.0, 1.0),
    )
    .into()
}

fn color_with_alpha(color: Color32, alpha: u8) -> Color32 {
    let [red, green, blue, _] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(red, green, blue, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage as _;

    #[derive(Default)]
    struct MemoryStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn missing_desktop_profile_captures_the_current_resolution_on_first_start() {
        let mut profile = None;

        initialize_profile(&mut profile, Resolution::new(1920, 1080));

        assert_eq!(profile, Some(Resolution::new(1920, 1080)));
    }

    #[test]
    fn captured_desktop_profile_is_not_overwritten_on_later_starts() {
        let current = Resolution::new(1920, 1080);
        let saved = Resolution::new(2560, 1440);
        let mut profile = Some(saved);

        initialize_profile(&mut profile, current);

        assert_eq!(profile, Some(saved));
    }

    #[test]
    fn resolutions_taller_than_native_are_rejected() {
        let native = Some(Resolution::new(2560, 1440));

        assert!(resolution_is_allowed(Resolution::new(3840, 1440), native));
        assert!(resolution_is_allowed(Resolution::new(1920, 1080), native));
        assert!(!resolution_is_allowed(Resolution::new(2560, 1600), native));
    }

    #[test]
    fn disallowed_saved_gaming_profile_is_replaced_by_the_allowed_default() {
        let native = Some(Resolution::new(2560, 1440));
        let default = Resolution::new(1920, 1080);
        let mut profile = Some(Resolution::new(2560, 1600));

        replace_disallowed_profile(&mut profile, default, native);

        assert_eq!(profile, Some(default));
    }

    #[test]
    fn resolution_picker_labels_native_and_unavailable_modes() {
        let native = Resolution::new(5120, 1440);

        assert_eq!(
            resolution_picker_label(native, true, Some(native)),
            "5120 × 1440 (native)"
        );
        assert_eq!(
            resolution_picker_label(Resolution::new(3440, 1440), true, Some(native)),
            "3440 × 1440"
        );
        assert_eq!(
            resolution_picker_label(Resolution::new(2560, 1080), false, Some(native)),
            "2560 × 1080 (unavailable)"
        );
    }

    #[test]
    fn gaming_resolution_combo_uses_the_full_control_width() {
        let context = egui::Context::default();
        configure_context(&context);
        let resolutions = [Resolution::new(5120, 1440), Resolution::new(3440, 1440)];
        let mut selection = Some(resolutions[0]);
        let mut control_height = 0.0;

        let _output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(440.0);
            let top = ui.cursor().top();
            gaming_resolution_combo_box(
                ui,
                &resolutions,
                &resolutions,
                Some(resolutions[0]),
                &mut selection,
            );
            control_height = ui.cursor().top() - top;
        });

        assert_eq!(control_height, 40.0);
    }

    #[test]
    fn gaming_countdown_displays_three_two_one_then_finishes() {
        let started_at = 10.0;

        assert_eq!(countdown_digit(started_at, 10.0), 3);
        assert_eq!(countdown_digit(started_at, 10.999), 3);
        assert_eq!(countdown_digit(started_at, 11.0), 2);
        assert_eq!(countdown_digit(started_at, 12.0), 1);
        assert_eq!(countdown_digit(started_at, 12.999), 1);
        assert!(!countdown_finished(started_at, 12.999));
        assert!(countdown_finished(started_at, 13.0));
    }

    #[test]
    fn mode_transition_waits_for_the_button_click_animation() {
        let clicked_at = 10.0;

        assert!(!mode_button_click_animation_finished(clicked_at, 10.159));
        assert!(mode_button_click_animation_finished(clicked_at, 10.160));
        assert_eq!(
            MODE_BUTTON_CLICK_ANIMATION_SECONDS,
            crate::sound::WARM_UP_MILLISECONDS as f64 / 1_000.0
        );
    }

    #[test]
    fn countdown_sound_plays_once_per_visible_digit() {
        let mut last_digit = None;

        assert!(mark_countdown_digit(&mut last_digit, 3));
        assert!(!mark_countdown_digit(&mut last_digit, 3));
        assert!(mark_countdown_digit(&mut last_digit, 2));
        assert!(!mark_countdown_digit(&mut last_digit, 2));
        assert!(mark_countdown_digit(&mut last_digit, 1));
        assert!(!mark_countdown_digit(&mut last_digit, 1));

        last_digit = None;
        assert!(mark_countdown_digit(&mut last_digit, 3));
    }

    #[test]
    fn sounds_are_enabled_by_default() {
        assert!(Settings::default().sounds_enabled);
    }

    #[test]
    fn taskbar_auto_hide_is_enabled_by_default() {
        assert!(Settings::default().features.taskbar_auto_hide);
    }

    #[test]
    fn resolution_changes_are_enabled_by_default() {
        assert!(Settings::default().features.change_resolution);
    }

    #[test]
    fn audio_output_changes_are_opt_in() {
        assert!(!Settings::default().features.change_output_device);
        assert!(Settings::default().gaming_output_device_id.is_none());
    }

    #[test]
    fn activation_without_resolution_preserves_every_other_configured_action() {
        let configured = FeatureChoices::default();

        let choices = activation_choices(&configured, false);

        assert!(!choices.change_resolution);
        assert_eq!(
            choices.change_output_device,
            configured.change_output_device
        );
        assert_eq!(choices.taskbar_auto_hide, configured.taskbar_auto_hide);
        assert_eq!(choices.desktop_icons, configured.desktop_icons);
        assert_eq!(choices.desktop_background, configured.desktop_background);
        assert_eq!(
            choices.minimize_all_windows,
            configured.minimize_all_windows
        );
    }

    #[test]
    fn unchecked_resolution_change_does_not_create_a_pending_transition() {
        let gaming = Resolution::new(3440, 1440);

        assert_eq!(pending_resolution_for_mode(true, false, Some(gaming)), None);
        assert_eq!(
            pending_resolution_for_mode(false, false, Some(gaming)),
            None
        );
    }

    #[test]
    fn checked_resolution_change_targets_the_requested_mode() {
        let gaming = Resolution::new(3440, 1440);
        let desktop = Resolution::new(5120, 1440);

        assert_eq!(
            pending_resolution_for_mode(true, true, Some(gaming)),
            Some(PendingResolution::gaming(gaming))
        );
        assert_eq!(
            pending_resolution_for_mode(false, true, Some(desktop)),
            Some(PendingResolution::desktop(desktop))
        );
    }

    #[test]
    fn legacy_gaming_mode_migrates_to_persisted_state() {
        let mut settings = Settings {
            settings_version: 0,
            active_mode_features: None,
            ..Settings::default()
        };

        migrate_settings(&mut settings, true);

        assert_eq!(settings.settings_version, CURRENT_SETTINGS_VERSION);
        assert!(settings.gaming_mode_active);
        assert!(settings.active_mode_features.is_some());
        assert_eq!(settings.taskbar_auto_hide_before_activation, Some(false));
    }

    #[test]
    fn legacy_taskbar_state_wins_over_a_stale_action_snapshot() {
        let mut settings = Settings {
            settings_version: 0,
            active_mode_features: Some(FeatureChoices::default()),
            taskbar_auto_hide_before_activation: Some(false),
            ..Settings::default()
        };

        migrate_settings(&mut settings, false);

        assert!(!settings.gaming_mode_active);
        assert!(settings.active_mode_features.is_none());
        assert_eq!(settings.taskbar_auto_hide_before_activation, None);
    }

    #[test]
    fn settings_without_a_version_are_detected_as_legacy() {
        let mut storage = MemoryStorage::default();
        storage.set_string(STORAGE_KEY, "()".to_owned());

        let settings: Settings = eframe::get_value(&storage, STORAGE_KEY).unwrap();

        assert_eq!(settings.settings_version, 0);
        assert!(settings.features.taskbar_auto_hide);
    }

    #[test]
    fn saved_feature_choices_without_resolution_checkbox_keep_legacy_behavior() {
        let settings: Settings = ron::from_str(
            "(settings_version:2,features:(taskbar_auto_hide:false,desktop_icons:false,desktop_background:false,minimize_all_windows:false))",
        )
        .unwrap();

        assert!(settings.features.change_resolution);
    }

    #[test]
    fn legacy_pending_resolution_is_typed_from_the_persisted_mode() {
        let mut settings: Settings = ron::from_str(
            "(settings_version:1,gaming_mode_active:false,pending_resolution_target:Some((width:3440,height:1440)))",
        )
        .unwrap();

        migrate_settings(&mut settings, true);

        assert_eq!(settings.settings_version, CURRENT_SETTINGS_VERSION);
        assert_eq!(
            settings.pending_resolution,
            Some(PendingResolution::desktop(Resolution::new(3440, 1440)))
        );
        assert!(settings.legacy_pending_resolution_target.is_none());
    }

    #[test]
    fn legacy_durable_pending_resolution_migrates_and_updates_its_version() {
        let mut state: DurableModeState = ron::from_str(
            "(version:1,active:true,pending_resolution_target:Some((width:2560,height:1440)))",
        )
        .unwrap();

        assert!(migrate_durable_mode_state(&mut state).unwrap());
        assert_eq!(state.version, CURRENT_MODE_STATE_VERSION);
        assert_eq!(
            state.pending_resolution,
            Some(PendingResolution::gaming(Resolution::new(2560, 1440)))
        );
        assert!(state.legacy_pending_resolution_target.is_none());
    }

    #[test]
    fn migrated_recovery_state_remains_available_when_its_rewrite_would_fail() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "borderless-gaming-mode-migration-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(MODE_STATE_FILE_NAME);
        let blocked_temporary_path = directory.join(MODE_STATE_TEMP_FILE_NAME);
        fs::write(
            &path,
            "(version:1,active:true,pending_resolution_target:Some((width:2560,height:1440)))",
        )
        .unwrap();
        fs::create_dir(&blocked_temporary_path).unwrap();

        let (state, needs_persist) = load_durable_mode_state_at(&path).unwrap().unwrap();

        assert!(needs_persist);
        assert!(state.active);
        assert_eq!(
            state.pending_resolution,
            Some(PendingResolution::gaming(Resolution::new(2560, 1440)))
        );
        assert!(save_durable_mode_state_at(&path, &blocked_temporary_path, &state).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn changing_gaming_resolution_preserves_a_desktop_retry() {
        let pending = PendingResolution::desktop(Resolution::new(5120, 1440));
        let mut saved = Some(pending);

        assert!(!cancel_pending_gaming_resolution(&mut saved));
        assert_eq!(saved, Some(pending));
    }

    #[test]
    fn changing_gaming_resolution_cancels_only_a_gaming_retry() {
        let mut saved = Some(PendingResolution::gaming(Resolution::new(3440, 1440)));

        assert!(cancel_pending_gaming_resolution(&mut saved));
        assert!(saved.is_none());
    }

    #[test]
    fn active_mode_and_taskbar_snapshot_round_trip_through_storage() {
        let mut storage = MemoryStorage::default();
        let settings = Settings {
            features: FeatureChoices {
                change_resolution: false,
                ..FeatureChoices::default()
            },
            gaming_mode_active: true,
            active_mode_features: Some(FeatureChoices::default()),
            taskbar_auto_hide_before_activation: Some(true),
            gaming_resolution: Some(Resolution::new(3440, 1440)),
            gaming_output_device_id: Some("gaming-speakers".to_owned()),
            output_before_activation: Some(OutputSnapshot {
                console: "desktop-console".to_owned(),
                multimedia: "desktop-media".to_owned(),
                communications: "desktop-comms".to_owned(),
            }),
            pending_resolution: Some(PendingResolution::gaming(Resolution::new(3440, 1440))),
            ..Settings::default()
        };

        store_settings(&mut storage, &settings);
        let restored: Settings = eframe::get_value(&storage, STORAGE_KEY).unwrap();

        assert!(restored.gaming_mode_active);
        assert!(!restored.features.change_resolution);
        assert_eq!(
            restored.gaming_resolution,
            Some(Resolution::new(3440, 1440))
        );
        assert_eq!(
            restored.gaming_output_device_id.as_deref(),
            Some("gaming-speakers")
        );
        assert_eq!(
            restored
                .output_before_activation
                .as_ref()
                .unwrap()
                .multimedia,
            "desktop-media"
        );
        assert!(restored.active_mode_features.is_some());
        assert_eq!(restored.taskbar_auto_hide_before_activation, Some(true));
        assert_eq!(
            restored.pending_resolution,
            Some(PendingResolution::gaming(Resolution::new(3440, 1440)))
        );
    }

    #[test]
    fn durable_mode_state_round_trips_through_an_atomic_file() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "borderless-gaming-mode-state-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(MODE_STATE_FILE_NAME);
        let temporary_path = directory.join(MODE_STATE_TEMP_FILE_NAME);
        let state = DurableModeState {
            active: true,
            active_mode_features: Some(FeatureChoices::default()),
            taskbar_auto_hide_before_activation: Some(false),
            output_before_activation: Some(OutputSnapshot {
                console: "desktop-console".to_owned(),
                multimedia: "desktop-media".to_owned(),
                communications: "desktop-comms".to_owned(),
            }),
            pending_resolution: Some(PendingResolution::gaming(Resolution::new(2560, 1440))),
            ..DurableModeState::default()
        };

        save_durable_mode_state_at(&path, &temporary_path, &state).unwrap();
        let restored: DurableModeState =
            ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

        assert!(restored.active);
        assert!(restored.active_mode_features.is_some());
        assert_eq!(restored.taskbar_auto_hide_before_activation, Some(false));
        assert_eq!(
            restored.output_before_activation.as_ref().unwrap().console,
            "desktop-console"
        );
        assert_eq!(
            restored.pending_resolution,
            Some(PendingResolution::gaming(Resolution::new(2560, 1440)))
        );

        save_durable_mode_state_at(&path, &temporary_path, &DurableModeState::default()).unwrap();
        let restored: DurableModeState =
            ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!restored.active);
        assert!(restored.active_mode_features.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn current_persisted_mode_ignores_external_taskbar_changes() {
        let mut settings = Settings {
            gaming_mode_active: false,
            ..Settings::default()
        };

        migrate_settings(&mut settings, true);

        assert!(!settings.gaming_mode_active);
        assert!(settings.active_mode_features.is_none());
    }

    #[test]
    fn application_behavior_has_safe_defaults() {
        let settings = Settings::default();

        assert!(!settings.startup_at_login);
        assert!(!settings.startup_minimized);
        assert_eq!(settings.window_transparency, 0);
    }

    #[test]
    fn transparency_slider_uses_the_full_available_width() {
        let context = egui::Context::default();
        configure_context(&context);
        let mut transparency = 40;
        let mut available_width = 0.0;
        let mut slider_size = Vec2::ZERO;

        let _output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(440.0);
            available_width = ui.available_width();
            slider_size = window_transparency_slider(ui, &mut transparency)
                .rect
                .size();
        });

        assert!((slider_size.x - available_width).abs() <= 0.5);
        assert_eq!(slider_size.y, TRANSPARENCY_CONTROL_HEIGHT - 28.0);
    }

    #[test]
    fn tray_mode_menu_tracks_the_current_transition() {
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::Desktop),
            ("Activate Gaming Mode".to_owned(), true)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::EnterPressed),
            ("Activate Gaming Mode".to_owned(), false)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::Countdown(2)),
            ("Activating in 2…".to_owned(), false)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::Activating),
            ("Activating…".to_owned(), false)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::Gaming),
            ("Restore Desktop Mode".to_owned(), true)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::RestorePressed),
            ("Restore Desktop Mode".to_owned(), false)
        );
        assert_eq!(
            tray_mode_menu_presentation(ModeVisualState::Restoring),
            ("Restoring…".to_owned(), false)
        );
    }

    #[test]
    fn tray_default_action_opens_the_window() {
        assert_eq!(TRAY_OPEN_MENU_POSITION, 0);
        assert_eq!(tray_default_command(), TrayCommand::Open);
    }

    #[test]
    fn mode_button_tracks_click_countdown_and_apply_phases() {
        assert_eq!(
            mode_button_presentation(ModeVisualState::EnterPressed),
            ("Activate gaming mode".to_owned(), false)
        );
        assert!(!mode_button_enabled(ModeVisualState::EnterPressed));
        assert_eq!(
            mode_button_presentation(ModeVisualState::Countdown(1)),
            ("Activating in 1".to_owned(), true)
        );
        assert!(!mode_button_enabled(ModeVisualState::Countdown(1)));
        assert_eq!(
            mode_button_presentation(ModeVisualState::Activating),
            ("Activating...".to_owned(), false)
        );
        assert!(!mode_button_enabled(ModeVisualState::Activating));
        assert_eq!(
            mode_button_presentation(ModeVisualState::RestorePressed),
            ("Restore desktop mode".to_owned(), false)
        );
        assert!(!mode_button_enabled(ModeVisualState::RestorePressed));
        assert_eq!(
            mode_button_presentation(ModeVisualState::Restoring),
            ("Restoring...".to_owned(), false)
        );
        assert!(!mode_button_enabled(ModeVisualState::Restoring));
    }

    #[test]
    fn countdown_keeps_the_desktop_wordmark_until_activation() {
        assert!(mode_uses_desktop_wordmark(ModeVisualState::Desktop));
        assert!(mode_uses_desktop_wordmark(ModeVisualState::Countdown(3)));
        assert!(mode_uses_desktop_wordmark(ModeVisualState::Countdown(1)));
        assert!(!mode_uses_desktop_wordmark(ModeVisualState::Activating));
        assert!(!mode_uses_desktop_wordmark(ModeVisualState::Gaming));
    }

    #[test]
    fn restoring_led_is_immediately_orange() {
        assert_eq!(
            mode_led_active_color(ModeVisualState::Restoring, 1.0),
            ACTIVATING_ORANGE
        );
        assert_eq!(
            mode_led_active_color(ModeVisualState::Gaming, 1.0),
            GAMING_GREEN
        );
    }

    #[test]
    fn activating_tray_led_blinks_twice_per_second() {
        assert_eq!(
            activating_tray_icon_state(0.0),
            TrayIconState::ActivatingBright
        );
        assert_eq!(
            activating_tray_icon_state(0.499),
            TrayIconState::ActivatingBright
        );
        assert_eq!(
            activating_tray_icon_state(0.5),
            TrayIconState::ActivatingDim
        );
        assert_eq!(
            activating_tray_icon_state(0.999),
            TrayIconState::ActivatingDim
        );
        assert_eq!(
            activating_tray_icon_state(1.0),
            TrayIconState::ActivatingBright
        );
    }

    #[test]
    fn late_viewport_hook_overrides_corrupt_saved_geometry() {
        let viewport = egui::ViewportBuilder::default()
            .with_inner_size([160.0, 64.0])
            .with_min_inner_size([64.0, 64.0])
            .with_max_inner_size([160.0, 64.0])
            .with_clamp_size_to_monitor_size(true)
            .with_fullscreen(true)
            .with_maximized(true)
            .with_decorations(true)
            .with_resizable(true)
            .with_maximize_button(true);

        let fixed = fixed_viewport(viewport);
        let size = Some(Vec2::from(WINDOW_SIZE));

        assert_eq!(fixed.inner_size, size);
        assert_eq!(fixed.min_inner_size, size);
        assert_eq!(fixed.max_inner_size, size);
        assert_eq!(fixed.clamp_size_to_monitor_size, Some(false));
        assert_eq!(fixed.fullscreen, Some(false));
        assert_eq!(fixed.maximized, Some(false));
        assert_eq!(fixed.decorations, Some(false));
        assert_eq!(fixed.resizable, Some(false));
        assert_eq!(fixed.maximize_button, Some(false));
    }

    #[test]
    fn tray_hidden_window_is_invisible_and_not_minimized() {
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            hide_to_tray(ui.ctx());
        });
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;

        assert_eq!(
            commands,
            &[
                egui::ViewportCommand::Visible(false),
                egui::ViewportCommand::Minimized(false),
            ]
        );
    }

    #[test]
    fn iconic_window_size_is_never_accepted_as_fixed() {
        assert!(is_fixed_window_size(Vec2::from(WINDOW_SIZE)));
        assert!(!is_fixed_window_size(Vec2::new(160.0, 64.0)));
    }

    #[test]
    fn fixed_window_height_preserves_the_measured_outer_inset() {
        assert_eq!(WINDOW_SIZE, [520.0, 764.0]);
    }

    #[test]
    fn final_card_border_fits_inside_scroll_clip() {
        let context = egui::Context::default();
        configure_context(&context);
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::from(WINDOW_SIZE),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default()
                    .frame(Frame::new().inner_margin(Margin {
                        left: 24,
                        right: 24,
                        top: 32,
                        bottom: MAIN_BOTTOM_INSET,
                    }))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            )
                            .show(ui, |ui| {
                                // The application-behavior card begins here in the normal layout.
                                ui.allocate_exact_size(Vec2::new(1.0, 392.0), egui::Sense::hover());
                                card(ui, |ui| {
                                    ui.label(
                                        RichText::new("Application behavior").size(15.0).strong(),
                                    );
                                    ui.add_space(4.0);
                                    let mut checked = false;
                                    accent_checkbox(ui, &mut checked, "Start at login");
                                    accent_checkbox(
                                        ui,
                                        &mut checked,
                                        "Start minimized into the system tray",
                                    );
                                    accent_checkbox(ui, &mut checked, "Enable sounds");
                                    window_transparency_slider(ui, &mut 0);
                                });
                            });
                    });
            },
        );
        let card_shape = output
            .shapes
            .iter()
            .find(|shape| {
                matches!(
                    &shape.shape, egui::epaint::Shape::Rect(rect) if rect.fill == CARD
                )
            })
            .expect("application behavior card was painted");
        if let egui::epaint::Shape::Rect(rect) = &card_shape.shape {
            assert!(
                rect.rect.bottom() + 1.0 <= card_shape.clip_rect.bottom(),
                "card bottom {} is clipped at {}",
                rect.rect.bottom(),
                card_shape.clip_rect.bottom()
            );
        }
    }

    #[test]
    fn another_styled_checkbox_row_uses_thirty_six_pixels() {
        fn rows_height(row_count: usize) -> f32 {
            let context = egui::Context::default();
            configure_context(&context);
            let mut height = 0.0;

            let _output = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(440.0);
                let top = ui.cursor().top();
                card(ui, |ui| {
                    let mut checked = true;
                    for index in 0..row_count {
                        accent_checkbox(ui, &mut checked, &format!("Option {index}"));
                    }
                });
                height = ui.cursor().top() - top;
            });

            height
        }

        assert_eq!(rows_height(4) - rows_height(3), 36.0);
    }

    #[test]
    fn app_bundles_only_the_proportional_typeface_it_uses() {
        let fonts = app_fonts();

        assert_eq!(fonts.font_data.len(), 1);
        assert!(fonts.font_data.contains_key("Ubuntu-Light"));
        assert_eq!(fonts.families[&FontFamily::Proportional], ["Ubuntu-Light"]);
    }

    #[test]
    fn context_initialization_keeps_the_custom_font_installed() {
        let context = egui::Context::default();
        configure_context(&context);
        let mut label_size = Vec2::ZERO;

        let _output = context.run_ui(egui::RawInput::default(), |ui| {
            label_size = ui.label("Gaming mode actions").rect.size();
        });

        assert!(label_size.x > 50.0);
        assert!(label_size.y > 10.0);
    }

    #[test]
    fn mode_wordmarks_fit_beside_the_header_and_close_button() {
        for bytes in [
            include_bytes!("../assets/runtime/desktop-mode-wordmark.png").as_slice(),
            include_bytes!("../assets/runtime/gaming-mode-wordmark.png").as_slice(),
        ] {
            let image = decode_png(bytes).unwrap();
            let aspect_ratio = image.width() as f32 / image.height() as f32;

            assert_eq!(image.height(), WORDMARK_TEXTURE_HEIGHT);
            assert!(aspect_ratio * WORDMARK_DISPLAY_HEIGHT <= 205.0);
        }
    }

    #[test]
    fn mode_wordmarks_use_linear_mipmap_antialiasing() {
        assert_eq!(
            WORDMARK_TEXTURE_OPTIONS.magnification,
            egui::TextureFilter::Linear
        );
        assert_eq!(
            WORDMARK_TEXTURE_OPTIONS.minification,
            egui::TextureFilter::Linear
        );
        assert_eq!(
            WORDMARK_TEXTURE_OPTIONS.mipmap_mode,
            Some(egui::TextureFilter::Linear)
        );
    }
}
