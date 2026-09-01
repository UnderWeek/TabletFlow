//! Platform display enumeration and OpenTabletDriver coordinate mapping.

#[cfg(target_os = "windows")]
use std::ffi::c_void;
#[cfg(target_os = "linux")]
use std::process::Command;

#[derive(Clone, Debug)]
pub(super) struct DisplayInfo {
    pub(super) index: i32,
    pub(super) label: String,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) detected: bool,
    pub(super) primary: bool,
}

impl DisplayInfo {
    fn fallback() -> Self {
        Self {
            index: 0,
            label: "Primary display · 1920 × 1080".into(),
            width: 1920.0,
            height: 1080.0,
            x: 960.0,
            y: 540.0,
            detected: false,
            primary: true,
        }
    }
}

fn display_label(index: usize, width: f32, height: f32) -> String {
    format!("Display {} · {:.0} × {:.0}", index + 1, width, height)
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MacPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MacSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct MacRect {
    origin: MacPoint,
    size: MacSize,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: u32) -> MacRect;
    fn CGMainDisplayID() -> u32;
}

#[cfg(target_os = "macos")]
fn enumerate_platform_displays() -> Vec<DisplayInfo> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let result = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if result != 0 || count == 0 {
        return vec![DisplayInfo::fallback()];
    }

    let primary_id = unsafe { CGMainDisplayID() };
    let bounds = ids[..count.min(ids.len() as u32) as usize]
        .iter()
        .map(|id| (*id, unsafe { CGDisplayBounds(*id) }))
        .collect::<Vec<_>>();
    let min_x = bounds
        .iter()
        .map(|(_, rect)| rect.origin.x)
        .fold(f64::INFINITY, f64::min);
    let min_y = bounds
        .iter()
        .map(|(_, rect)| rect.origin.y)
        .fold(f64::INFINITY, f64::min);

    let displays = bounds
        .into_iter()
        .enumerate()
        .filter_map(|(index, (id, rect))| {
            let width = rect.size.width as f32;
            let height = rect.size.height as f32;
            (width > 0.0 && height > 0.0).then(|| DisplayInfo {
                index: index as i32,
                label: if id == primary_id {
                    format!("Primary display · {:.0} × {:.0}", width, height)
                } else {
                    display_label(index, width, height)
                },
                width,
                height,
                x: (rect.origin.x - min_x) as f32 + width / 2.0,
                y: (rect.origin.y - min_y) as f32 + height / 2.0,
                detected: true,
                primary: id == primary_id,
            })
        })
        .collect::<Vec<_>>();
    if displays.is_empty() {
        vec![DisplayInfo::fallback()]
    } else {
        displays
    }
}

#[cfg(target_os = "linux")]
fn parse_xrandr(stdout: &str) -> Vec<DisplayInfo> {
    let mut raw = Vec::new();
    for line in stdout.lines() {
        if !line.contains(" connected") {
            continue;
        }
        let Some(geometry) = line.split_whitespace().find(|part| {
            let Some(size_end) = part.find(['+', '-']) else {
                return false;
            };
            part[..size_end].contains('x') && part[size_end + 1..].find(['+', '-']).is_some()
        }) else {
            continue;
        };
        let Some(size_end) = geometry.find(['+', '-']) else {
            continue;
        };
        let Some(separator) = geometry[size_end + 1..]
            .find(['+', '-'])
            .map(|offset| size_end + 1 + offset)
        else {
            continue;
        };
        let Some((width, height)) = geometry[..size_end]
            .split_once('x')
            .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
        else {
            continue;
        };
        let Ok(x) = geometry[size_end..separator].parse::<f32>() else {
            continue;
        };
        let Ok(y) = geometry[separator..].parse::<f32>() else {
            continue;
        };
        raw.push((
            width,
            height,
            x,
            y,
            line.split_whitespace().any(|part| part == "primary"),
        ));
    }

    if raw.is_empty() {
        return Vec::new();
    }
    let min_x = raw
        .iter()
        .map(|display| display.2)
        .fold(f32::INFINITY, f32::min);
    let min_y = raw
        .iter()
        .map(|display| display.3)
        .fold(f32::INFINITY, f32::min);
    raw.into_iter()
        .enumerate()
        .map(|(index, (width, height, x, y, primary))| DisplayInfo {
            index: index as i32,
            label: if primary {
                format!("Primary display · {:.0} × {:.0}", width, height)
            } else {
                display_label(index, width, height)
            },
            width,
            height,
            x: x - min_x + width / 2.0,
            y: y - min_y + height / 2.0,
            detected: true,
            primary,
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn enumerate_platform_displays() -> Vec<DisplayInfo> {
    let displays = Command::new("xrandr")
        .arg("--query")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_xrandr(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    if displays.is_empty() {
        vec![DisplayInfo::fallback()]
    } else {
        displays
    }
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WindowsRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WindowsMonitorInfo {
    size: u32,
    monitor: WindowsRect,
    work: WindowsRect,
    flags: u32,
}

#[cfg(target_os = "windows")]
struct WindowsDisplayBounds {
    rect: WindowsRect,
    primary: bool,
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn EnumDisplayMonitors(
        device_context: *mut c_void,
        clip: *const WindowsRect,
        callback: Option<unsafe extern "system" fn(isize, isize, *mut WindowsRect, isize) -> i32>,
        data: isize,
    ) -> i32;
    fn GetMonitorInfoW(monitor: isize, info: *mut WindowsMonitorInfo) -> i32;
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn collect_windows_display(
    monitor: isize,
    _: isize,
    _: *mut WindowsRect,
    data: isize,
) -> i32 {
    let mut info = WindowsMonitorInfo {
        size: std::mem::size_of::<WindowsMonitorInfo>() as u32,
        monitor: WindowsRect::default(),
        work: WindowsRect::default(),
        flags: 0,
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return 1;
    }
    if info.monitor.right > info.monitor.left && info.monitor.bottom > info.monitor.top {
        let displays = unsafe { &mut *(data as *mut Vec<WindowsDisplayBounds>) };
        displays.push(WindowsDisplayBounds {
            rect: info.monitor,
            primary: info.flags & 1 != 0,
        });
    }
    1
}

#[cfg(target_os = "windows")]
fn enumerate_platform_displays() -> Vec<DisplayInfo> {
    let mut bounds = Vec::new();
    let result = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_windows_display),
            (&mut bounds as *mut Vec<WindowsDisplayBounds>) as isize,
        )
    };
    if result == 0 || bounds.is_empty() {
        return vec![DisplayInfo::fallback()];
    }
    let min_x = bounds
        .iter()
        .map(|display| display.rect.left)
        .min()
        .unwrap_or(0);
    let min_y = bounds
        .iter()
        .map(|display| display.rect.top)
        .min()
        .unwrap_or(0);
    // OpenTabletDriver's WindowsDisplay exposes its virtual-screen Position as
    // the primary monitor's normalized offset. Reproduce the same transform so
    // values written into AbsoluteModeSettings.Display match OTD exactly.
    let primary_offset = bounds
        .iter()
        .find(|display| display.primary)
        .map(|display| (display.rect.left - min_x, display.rect.top - min_y))
        .unwrap_or((0, 0));
    bounds
        .into_iter()
        .enumerate()
        .map(|(index, display)| {
            let width = (display.rect.right - display.rect.left) as f32;
            let height = (display.rect.bottom - display.rect.top) as f32;
            DisplayInfo {
                index: index as i32,
                label: if display.primary {
                    format!("Primary display · {:.0} × {:.0}", width, height)
                } else {
                    display_label(index, width, height)
                },
                width,
                height,
                x: (display.rect.left - min_x + primary_offset.0) as f32 + width / 2.0,
                y: (display.rect.top - min_y + primary_offset.1) as f32 + height / 2.0,
                detected: true,
                primary: display.primary,
            }
        })
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn enumerate_platform_displays() -> Vec<DisplayInfo> {
    vec![DisplayInfo::fallback()]
}

pub(super) fn enumerate_displays() -> Vec<DisplayInfo> {
    enumerate_platform_displays()
}

pub(super) fn selected_display_index(
    displays: &[DisplayInfo],
    width: f32,
    height: f32,
    x: f32,
    y: f32,
) -> i32 {
    displays
        .iter()
        .find(|display| {
            (display.width - width).abs() < 2.0
                && (display.height - height).abs() < 2.0
                && (display.x - x).abs() < 3.0
                && (display.y - y).abs() < 3.0
        })
        .map(|display| display.index)
        .unwrap_or(-1)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn parses_negative_xrandr_offsets() {
        let input = "DP-1 connected primary 2560x1440+0+0\nHDMI-1 connected 1920x1080-1920+180\n";
        let displays = parse_xrandr(input);
        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].x, 3200.0);
        assert_eq!(displays[1].x, 960.0);
    }
}
