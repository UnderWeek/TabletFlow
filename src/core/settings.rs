use crate::platform::Platform;
use std::fs;
use std::io;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: String,
    pub accent: String,
    pub custom_colors: bool,
    pub custom_background_hue: f32,
    pub custom_background_saturation: f32,
    pub custom_background_value: f32,
    pub custom_accent_hue: f32,
    pub custom_accent_saturation: f32,
    pub custom_accent_value: f32,
    pub compact_ui: bool,
    pub reduce_animations: bool,
    pub start_with_system: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub check_updates: bool,
    pub pause_hidden: bool,
    pub disable_unfocused_animations: bool,
    pub polling_interval: String,
    pub low_power_mode: bool,
    pub show_diagnostics: bool,
}

impl Settings {
    pub fn defaults_for(platform: &dyn Platform) -> Self {
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
            close_to_tray: platform.default_close_to_tray(),
            check_updates: true,
            pause_hidden: true,
            disable_unfocused_animations: false,
            polling_interval: "Auto".into(),
            low_power_mode: false,
            show_diagnostics: false,
        }
    }

    pub fn load(platform: &dyn Platform) -> Self {
        let defaults = Self::defaults_for(platform);
        let primary = platform.settings_path();
        let contents = primary
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .or_else(|| {
                platform
                    .legacy_settings_paths()
                    .into_iter()
                    .filter(|path| Some(path) != primary.as_ref())
                    .find_map(|path| fs::read_to_string(path).ok())
            });
        contents
            .as_deref()
            .map(|contents| Self::parse(contents, defaults.clone()))
            .unwrap_or(defaults)
    }

    pub fn save(&self, platform: &dyn Platform) -> io::Result<()> {
        let Some(path) = platform.settings_path() else {
            return Ok(());
        };
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, self.serialize())?;
        platform.replace_file(&temporary, &path)
    }

    fn parse(contents: &str, mut settings: Self) -> Self {
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
                "custom_colors" => {
                    settings.custom_colors = parse_bool(value, settings.custom_colors)
                }
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
                "close_to_tray" => {
                    settings.close_to_tray = parse_bool(value, settings.close_to_tray)
                }
                "check_updates" => {
                    settings.check_updates = parse_bool(value, settings.check_updates)
                }
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

    fn serialize(&self) -> String {
        format!(
            "theme={}\naccent={}\ncustom_colors={}\ncustom_background_hue={:.2}\ncustom_background_saturation={:.4}\ncustom_background_value={:.4}\ncustom_accent_hue={:.2}\ncustom_accent_saturation={:.4}\ncustom_accent_value={:.4}\ncompact_ui={}\nreduce_animations={}\nstart_with_system={}\nstart_minimized={}\nclose_to_tray={}\ncheck_updates={}\npause_hidden={}\ndisable_unfocused_animations={}\npolling_interval={}\nlow_power_mode={}\nshow_diagnostics={}\n",
            self.theme,
            self.accent,
            self.custom_colors,
            self.custom_background_hue,
            self.custom_background_saturation,
            self.custom_background_value,
            self.custom_accent_hue,
            self.custom_accent_saturation,
            self.custom_accent_value,
            self.compact_ui,
            self.reduce_animations,
            self.start_with_system,
            self.start_minimized,
            self.close_to_tray,
            self.check_updates,
            self.pause_hidden,
            self.disable_unfocused_animations,
            self.polling_interval,
            self.low_power_mode,
            self.show_diagnostics,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Platform, Transport};
    use std::path::PathBuf;

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn name(&self) -> &'static str {
            "test"
        }
        fn default_close_to_tray(&self) -> bool {
            true
        }
        fn settings_path(&self) -> Option<PathBuf> {
            None
        }
        fn acquire_instance_guard(&self) -> Option<Box<dyn Send>> {
            None
        }
        fn connect_transport(&self) -> io::Result<Box<dyn Transport>> {
            Err(io::ErrorKind::Unsupported.into())
        }
        fn ipc_available(&self) -> bool {
            false
        }
        fn owned_daemon_running(&self) -> bool {
            false
        }
        fn start_daemon(&self) -> io::Result<()> {
            Ok(())
        }
        fn stop_daemon(&self) {}
        fn configure_autostart(&self, _enabled: bool, _start_minimized: bool) -> io::Result<()> {
            Ok(())
        }
        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
        fn run_driver_self_test(&self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn platform_default_controls_close_to_tray() {
        assert!(Settings::defaults_for(&TestPlatform).close_to_tray);
    }

    #[test]
    fn parser_clamps_and_rejects_invalid_values() {
        let defaults = Settings::defaults_for(&TestPlatform);
        let parsed = Settings::parse(
            "theme=Dark\ncustom_background_hue=999\ncustom_accent_value=nan\npolling_interval=High\n",
            defaults,
        );
        assert_eq!(parsed.theme, "Dark");
        assert_eq!(parsed.custom_background_hue, 360.0);
        assert_eq!(parsed.custom_accent_value, 0.86);
        assert_eq!(parsed.polling_interval, "High");
    }
}
