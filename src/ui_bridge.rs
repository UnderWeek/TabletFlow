use crate::core::backend::BackendSink;
use crate::core::models::{AreaRequest, BackendCommand, BackendSnapshot};
use crate::core::settings::Settings;
use crate::display::DisplayInfo;
use crate::platform::Platform;
use crate::{AppTheme, MainWindow, TrayIcon};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::sync::mpsc::Sender;

const GITHUB_URL: &str = "https://github.com/UnderWeek/TabletFlow";

pub struct UiSink {
    ui: slint::Weak<MainWindow>,
}

impl UiSink {
    pub fn new(ui: slint::Weak<MainWindow>) -> Self {
        Self { ui }
    }

    fn invoke(&self, update: impl FnOnce(&MainWindow) + Send + 'static) {
        let weak_ui = self.ui.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak_ui.upgrade() {
                update(&ui);
            }
        });
    }
}

impl BackendSink for UiSink {
    fn connection_state(&self, state: &'static str) {
        self.invoke(move |ui| ui.set_backend_state(state.into()));
    }

    fn snapshot(&self, snapshot: BackendSnapshot) {
        self.invoke(move |ui| publish_snapshot(ui, snapshot));
    }

    fn applied_area(&self, request: AreaRequest) {
        self.invoke(move |ui| publish_applied_area(ui, request));
    }
}

pub struct UiSession {
    _tray: TrayIcon,
}

fn publish_snapshot(ui: &MainWindow, snapshot: BackendSnapshot) {
    ui.set_backend_state(snapshot.state.into());
    ui.set_device_name(snapshot.device_name.into());
    ui.set_area_preview_width(snapshot.preview_width);
    ui.set_area_preview_height(snapshot.preview_height);
    ui.set_tablet_bounds_width(snapshot.tablet_width);
    ui.set_tablet_bounds_height(snapshot.tablet_height);
    ui.set_area_width(snapshot.area_width.into());
    ui.set_area_height(snapshot.area_height.into());
    ui.set_area_x(snapshot.area_x.into());
    ui.set_area_y(snapshot.area_y.into());
    ui.set_area_rotation(snapshot.area_rotation.into());
    ui.set_area_frequency(snapshot.area_frequency.into());
    ui.set_monitor_index(snapshot.monitor_index);
    ui.set_pen_data_available(snapshot.pen_data_available);
}

fn publish_applied_area(ui: &MainWindow, request: AreaRequest) {
    ui.set_area_width(request.width.into());
    ui.set_area_height(request.height.into());
    ui.set_area_x(request.x.into());
    ui.set_area_y(request.y.into());
    ui.set_area_rotation(request.rotation.into());
    ui.set_area_frequency(request.frequency.into());
    if let Some(display) = request.display {
        ui.set_monitor_index(display.index);
        ui.set_area_preview_width(display.width / 10.0);
        ui.set_area_preview_height(display.height / 10.0);
    }
}

fn sync_theme(ui: &MainWindow, settings: &Settings) {
    let theme = ui.global::<AppTheme>();
    theme.set_mode(
        match settings.theme.as_str() {
            "Light" => "light",
            "Dark" => "dark",
            _ => "system",
        }
        .into(),
    );
    theme.set_accent(settings.accent.clone().into());
    theme.set_custom_colors(settings.custom_colors);
    theme.set_custom_background_hue(settings.custom_background_hue);
    theme.set_custom_background_saturation(settings.custom_background_saturation);
    theme.set_custom_background_value(settings.custom_background_value);
    theme.set_custom_accent_hue(settings.custom_accent_hue);
    theme.set_custom_accent_saturation(settings.custom_accent_saturation);
    theme.set_custom_accent_value(settings.custom_accent_value);
    theme.set_compact(settings.compact_ui);
    theme.set_reduce_motion(settings.reduce_animations);
}

pub fn apply_settings(ui: &MainWindow, settings: &Settings) {
    ui.set_theme(settings.theme.clone().into());
    ui.set_accent(settings.accent.clone().into());
    ui.set_custom_colors(settings.custom_colors);
    ui.set_custom_background_hue(settings.custom_background_hue);
    ui.set_custom_background_saturation(settings.custom_background_saturation);
    ui.set_custom_background_value(settings.custom_background_value);
    ui.set_custom_accent_hue(settings.custom_accent_hue);
    ui.set_custom_accent_saturation(settings.custom_accent_saturation);
    ui.set_custom_accent_value(settings.custom_accent_value);
    ui.set_compact_ui(settings.compact_ui);
    ui.set_reduce_animations(settings.reduce_animations);
    ui.set_start_with_system(settings.start_with_system);
    ui.set_start_minimized(settings.start_minimized);
    ui.set_close_to_tray(settings.close_to_tray);
    ui.set_check_updates(settings.check_updates);
    ui.set_pause_hidden(settings.pause_hidden);
    ui.set_disable_unfocused_animations(settings.disable_unfocused_animations);
    ui.set_polling_interval(settings.polling_interval.clone().into());
    ui.set_low_power_mode(settings.low_power_mode);
    ui.set_show_diagnostics(settings.show_diagnostics);
    sync_theme(ui, settings);
}

fn read_settings(ui: &MainWindow) -> Settings {
    Settings {
        theme: ui.get_theme().to_string(),
        accent: ui.get_accent().to_string(),
        custom_colors: ui.get_custom_colors(),
        custom_background_hue: ui.get_custom_background_hue(),
        custom_background_saturation: ui.get_custom_background_saturation(),
        custom_background_value: ui.get_custom_background_value(),
        custom_accent_hue: ui.get_custom_accent_hue(),
        custom_accent_saturation: ui.get_custom_accent_saturation(),
        custom_accent_value: ui.get_custom_accent_value(),
        compact_ui: ui.get_compact_ui(),
        reduce_animations: ui.get_reduce_animations(),
        start_with_system: ui.get_start_with_system(),
        start_minimized: ui.get_start_minimized(),
        close_to_tray: ui.get_close_to_tray(),
        check_updates: ui.get_check_updates(),
        pause_hidden: ui.get_pause_hidden(),
        disable_unfocused_animations: ui.get_disable_unfocused_animations(),
        polling_interval: ui.get_polling_interval().to_string(),
        low_power_mode: ui.get_low_power_mode(),
        show_diagnostics: ui.get_show_diagnostics(),
    }
}

fn refresh_permissions(ui: &MainWindow, platform: &dyn Platform) -> bool {
    let (input_monitoring_granted, accessibility_granted) = platform.permissions();
    ui.set_input_monitoring_granted(input_monitoring_granted);
    ui.set_accessibility_granted(accessibility_granted);
    input_monitoring_granted && accessibility_granted
}

pub fn initialize_permissions(ui: &MainWindow, platform: &'static dyn Platform) {
    ui.set_permission_platform(
        if platform.name() == "macOS" {
            "macOS"
        } else {
            "other"
        }
        .into(),
    );
    if !refresh_permissions(ui, platform) && platform.name() == "macOS" {
        platform.request_permissions();
    }
}

pub fn set_monitor_options(ui: &MainWindow, displays: &[DisplayInfo]) {
    let options = ModelRc::new(VecModel::from_iter(
        displays
            .iter()
            .map(|display| SharedString::from(display.label.clone())),
    ));
    ui.set_monitor_options(options);
}

pub fn wire(
    ui: &MainWindow,
    platform: &'static dyn Platform,
    backend_commands: Sender<BackendCommand>,
    displays: Vec<DisplayInfo>,
) -> Result<UiSession, slint::PlatformError> {
    let tray = TrayIcon::new()?;
    tray.set_shown(ui.get_close_to_tray());

    let weak_ui = ui.as_weak();
    tray.on_open(move || {
        if let Some(ui) = weak_ui.upgrade() {
            let _ = ui.show();
        }
    });
    tray.on_quit(move || {
        let _ = slint::quit_event_loop();
    });

    let weak_ui = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak_ui.upgrade() {
            if ui.get_close_to_tray() {
                return CloseRequestResponse::HideWindow;
            }
        }
        let _ = slint::quit_event_loop();
        CloseRequestResponse::HideWindow
    });

    let weak_ui = ui.as_weak();
    ui.on_navigate(move |page| {
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_active_page(page);
        }
    });

    let retry_commands = backend_commands.clone();
    let weak_ui = ui.as_weak();
    ui.on_retry_backend(move || {
        let _ = retry_commands.send(BackendCommand::Detect);
        if let Some(ui) = weak_ui.upgrade() {
            ui.set_backend_state("unknown".into());
        }
    });

    let start_commands = backend_commands.clone();
    ui.on_start_daemon(move || {
        let _ = start_commands.send(BackendCommand::StartDaemon);
    });

    let apply_commands = backend_commands.clone();
    ui.on_apply_area(
        move |tablet, width, height, x, y, rotation, frequency, monitor| {
            let display = displays
                .iter()
                .find(|display| display.index == monitor)
                .cloned();
            let request = AreaRequest {
                tablet_name: tablet.to_string(),
                width: width.to_string(),
                height: height.to_string(),
                x: x.to_string(),
                y: y.to_string(),
                rotation: rotation.to_string(),
                frequency: frequency.to_string(),
                display,
            };
            let _ = apply_commands.send(BackendCommand::ApplyArea(request));
        },
    );

    let refresh_commands = backend_commands.clone();
    ui.on_refresh_area(move || {
        let _ = refresh_commands.send(BackendCommand::RefreshSettings);
    });

    let weak_ui = ui.as_weak();
    let weak_tray = tray.as_weak();
    // Only re-runs `configure_autostart` (which shells out to `launchctl` on
    // macOS / rewrites a `.desktop` file on Linux / touches the registry on
    // Windows) when the two settings it actually depends on changed, instead
    // of on every settings-changed event - including ones from unrelated
    // controls like a color slider drag.
    let last_autostart_settings =
        std::cell::Cell::new((ui.get_start_with_system(), ui.get_start_minimized()));
    ui.on_settings_changed(move || {
        let Some(ui) = weak_ui.upgrade() else {
            return;
        };
        let settings = read_settings(&ui);
        sync_theme(&ui, &settings);
        if let Err(error) = settings.save(platform) {
            platform.log(&format!("failed to save settings: {error}"));
        }
        let autostart_settings = (settings.start_with_system, settings.start_minimized);
        if autostart_settings != last_autostart_settings.get() {
            if let Err(error) =
                platform.configure_autostart(settings.start_with_system, settings.start_minimized)
            {
                platform.log(&format!("failed to configure autostart: {error}"));
            } else {
                last_autostart_settings.set(autostart_settings);
            }
        }
        if let Some(tray) = weak_tray.upgrade() {
            tray.set_shown(settings.close_to_tray);
        }
    });

    ui.on_open_github(move || {
        if let Err(error) = platform.open_url(GITHUB_URL) {
            platform.log(&format!("failed to open project URL: {error}"));
        }
    });

    let weak_ui = ui.as_weak();
    ui.on_check_permissions(move || {
        if let Some(ui) = weak_ui.upgrade() {
            refresh_permissions(&ui, platform);
        }
    });

    let weak_ui = ui.as_weak();
    ui.on_request_permissions(move || {
        platform.request_permissions();
        if let Some(ui) = weak_ui.upgrade() {
            refresh_permissions(&ui, platform);
        }
    });

    Ok(UiSession { _tray: tray })
}
