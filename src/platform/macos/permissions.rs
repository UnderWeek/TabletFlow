use std::process::Command;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: u32) -> u32;
    fn IOHIDRequestAccess(request_type: u32) -> bool;
}

const LISTEN_EVENT: u32 = 1;

pub fn status() -> (bool, bool) {
    (unsafe { IOHIDCheckAccess(LISTEN_EVENT) } == 0, unsafe {
        AXIsProcessTrusted()
    })
}

pub fn request() {
    let (input, accessibility) = status();
    if !input {
        let _ = unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();
    } else if !accessibility {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}
