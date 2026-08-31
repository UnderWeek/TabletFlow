//! Persistent application settings.

#[cfg(target_os = "windows")]
use super::windows_runtime;
use super::{AppTheme, MainWindow};
use slint::ComponentHandle;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(super) struct Settings {
    pub(super) theme: String,
    pub(super) accent: String,
    pub(super) custom_colors: bool,
    pub(super) custom_background_hue: f32,
    pub(super) custom_background_saturation: f32,
    pub(super) custom_background_value: f32,
    pub(super) custom_accent_hue: f32,
    pub(super) custom_accent_saturation: f32,
    pub(super) custom_accent_value: f32,
    pub(super) compact_ui: bool,
    pub(super) reduce_animations: bool,
    pub(super) start_with_system: bool,
    pub(super) start_minimized: bool,
    pub(super) close_to_tray: bool,
    pub(super) check_updates: bool,
    pub(super) pause_hidden: bool,
    pub(super) disable_unfocused_animations: bool,
    pub(super) polling_interval: String,
    pub(super) low_power_mode: bool,
    pub(super) show_diagnostics: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "System".into(),
            accent: "Blue".into(),
            custom_colors: false,
            custom_background_hue: 212.0,
            custom_background_saturation: 0.10,
            custom_background_value: 0.96,
            custom_accent_hue: 212.0,
            custom_accent_saturation: 0.42,
            custom_accent_value: 0.86,
            compact_ui: false,
            reduce_animations: false,
            start_with_system: false,
            start_minimized: false,
            close_to_tray: cfg!(target_os = "windows"),
            check_updates: true,
            pause_hidden: true,
            disable_unfocused_animations: false,
            polling_interval: "Auto".into(),
            low_power_mode: false,
            show_diagnostics: false,
        }
    }
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value {
        "true" => true,
        "false" => false,
        _ => fallback,
    }
}

fn parse_float(value: &str, fallback: f32, minimum: f32, maximum: f32) -> f32 {
    value
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(minimum, maximum))
        .unwrap_or(fallback)
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn settings_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return home_directory()
            .map(|home| home.join("Library/Application Support/TabletFlow/settings.conf"));
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(home_directory)
            .map(|root| root.join("TabletFlow/settings.conf"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_directory().map(|home| home.join(".config")))
            .map(|root| root.join("tabletflow/settings.conf"));
    }
    #[allow(unreachable_code)]
    home_directory().map(|home| home.join(".tabletflow/settings.conf"))
}

fn legacy_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_directory() {
        paths.push(home.join(".tabletflow/settings.conf"));
    }
    #[cfg(target_os = "linux")]
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(config_home).join("TabletFlow/settings.conf"));
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = home_directory() {
        paths.push(home.join(".config/TabletFlow/settings.conf"));
    }
    paths
}

pub(super) fn load_settings() -> Settings {
    let mut settings = Settings::default();
    let primary = settings_path();
    let contents = primary
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .or_else(|| {
            legacy_paths()
                .into_iter()
                .filter(|path| Some(path) != primary.as_ref())
                .find_map(|path| fs::read_to_string(path).ok())
        });
    let Some(contents) = contents else {
        return settings;
    };

    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "theme" if matches!(value, "System" | "Light" | "Dark") => {
                settings.theme = value.into()
            }
            "accent" if matches!(value, "Blue" | "Amber" | "Mint") => {
                settings.accent = value.into()
            }
            "custom_colors" => settings.custom_colors = parse_bool(value, settings.custom_colors),
            "custom_background_hue" => {
                settings.custom_background_hue =
                    parse_float(value, settings.custom_background_hue, 0.0, 360.0)
            }
            "custom_background_saturation" => {
                settings.custom_background_saturation =
                    parse_float(value, settings.custom_background_saturation, 0.0, 1.0)
            }
            "custom_background_value" => {
                settings.custom_background_value =
                    parse_float(value, settings.custom_background_value, 0.0, 1.0)
            }
            "custom_accent_hue" => {
                settings.custom_accent_hue =
                    parse_float(value, settings.custom_accent_hue, 0.0, 360.0)
            }
            "custom_accent_saturation" => {
                settings.custom_accent_saturation =
                    parse_float(value, settings.custom_accent_saturation, 0.0, 1.0)
            }
            "custom_accent_value" => {
                settings.custom_accent_value =
                    parse_float(value, settings.custom_accent_value, 0.0, 1.0)
            }
            "compact_ui" => settings.compact_ui = parse_bool(value, settings.compact_ui),
            "reduce_animations" => {
                settings.reduce_animations = parse_bool(value, settings.reduce_animations)
            }
            "start_with_system" => {
                settings.start_with_system = parse_bool(value, settings.start_with_system)
            }
            "start_minimized" => {
                settings.start_minimized = parse_bool(value, settings.start_minimized)
            }
            "close_to_tray" => settings.close_to_tray = parse_bool(value, settings.close_to_tray),
            "check_updates" => settings.check_updates = parse_bool(value, settings.check_updates),
            "pause_hidden" => settings.pause_hidden = parse_bool(value, settings.pause_hidden),
            "disable_unfocused_animations" => {
                settings.disable_unfocused_animations =
                    parse_bool(value, settings.disable_unfocused_animations)
            }
            "polling_interval" if matches!(value, "Auto" | "Low" | "High") => {
                settings.polling_interval = value.into()
            }
            "low_power_mode" => {
                settings.low_power_mode = parse_bool(value, settings.low_power_mode)
            }
            "show_diagnostics" => {
                settings.show_diagnostics = parse_bool(value, settings.show_diagnostics)
            }
            _ => {}
        }
    }
    settings
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    windows_runtime::replace_file(source, destination)
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

pub(super) fn save_settings(ui: &MainWindow) -> io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let contents = format!(
        "theme={}\naccent={}\ncustom_colors={}\ncustom_background_hue={:.2}\ncustom_background_saturation={:.4}\ncustom_background_value={:.4}\ncustom_accent_hue={:.2}\ncustom_accent_saturation={:.4}\ncustom_accent_value={:.4}\ncompact_ui={}\nreduce_animations={}\nstart_with_system={}\nstart_minimized={}\nclose_to_tray={}\ncheck_updates={}\npause_hidden={}\ndisable_unfocused_animations={}\npolling_interval={}\nlow_power_mode={}\nshow_diagnostics={}\n",
        ui.get_theme(), ui.get_accent(), ui.get_custom_colors(),
        ui.get_custom_background_hue(), ui.get_custom_background_saturation(), ui.get_custom_background_value(),
        ui.get_custom_accent_hue(), ui.get_custom_accent_saturation(), ui.get_custom_accent_value(),
        ui.get_compact_ui(), ui.get_reduce_animations(), ui.get_start_with_system(), ui.get_start_minimized(),
        ui.get_close_to_tray(), ui.get_check_updates(), ui.get_pause_hidden(), ui.get_disable_unfocused_animations(),
        ui.get_polling_interval(), ui.get_low_power_mode(), ui.get_show_diagnostics(),
    );
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    replace_file(&temporary, &path)
}

pub(super) fn apply_settings(ui: &MainWindow, settings: &Settings) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsers_reject_invalid_values() {
        assert!(parse_bool("true", false));
        assert!(!parse_bool("invalid", false));
        assert_eq!(parse_float("999", 10.0, 0.0, 360.0), 360.0);
        assert_eq!(parse_float("nan", 10.0, 0.0, 360.0), 10.0);
    }
}
