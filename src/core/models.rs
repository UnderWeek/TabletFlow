use crate::display::DisplayInfo;

#[derive(Clone, Debug)]
pub enum BackendCommand {
    Detect,
    StartDaemon,
    Shutdown,
    TabletChanged { generation: u64 },
    DriverDisconnected { generation: u64, reason: String },
    ApplyArea(AreaRequest),
}

#[derive(Clone, Debug)]
pub struct AreaRequest {
    pub tablet_name: String,
    pub width: String,
    pub height: String,
    pub x: String,
    pub y: String,
    pub rotation: String,
    pub frequency: String,
    pub display: Option<DisplayInfo>,
}

#[derive(Default, Debug)]
pub struct BackendSnapshot {
    pub state: &'static str,
    pub device_name: String,
    pub preview_width: f32,
    pub preview_height: f32,
    pub tablet_width: f32,
    pub tablet_height: f32,
    pub area_width: String,
    pub area_height: String,
    pub area_x: String,
    pub area_y: String,
    pub area_rotation: String,
    pub area_frequency: String,
    pub monitor_index: i32,
    pub pen_data_available: bool,
}
