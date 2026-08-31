use std::fs;
use std::io;
use std::path::PathBuf;

fn quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub fn configure(enabled: bool, start_minimized: bool) -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let Some(config_home) = config_home else {
        return Ok(());
    };
    let path = config_home.join("autostart/TabletFlow.desktop");
    if !enabled {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent)?;
    let executable = quote(&executable.to_string_lossy());
    let minimized = if start_minimized {
        " --start-minimized"
    } else {
        ""
    };
    fs::write(path, format!("[Desktop Entry]\nType=Application\nName=TabletFlow\nExec={executable}{minimized}\nX-GNOME-Autostart-enabled=true\n"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn executable_paths_are_quoted() {
        assert_eq!(
            super::quote(r#"/tmp/Tablet Flow/$test\"#),
            r#"\"/tmp/Tablet Flow/\$test\\\""#
        );
    }
}
