fn main() {
    println!("cargo:rerun-if-changed=ui/app.slint");
    println!("cargo:rerun-if-changed=ui/tray.svg");
    slint_build::compile("ui/app.slint").expect("Unable to compile Slint UI");
}
