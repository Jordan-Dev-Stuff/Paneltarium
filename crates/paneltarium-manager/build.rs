//! Windows: embed the app icon and file description in the .exe, so
//! Explorer, Task Manager and the taskbar show the Paneltarium icon.
//! Never fails the build: without the Windows SDK's resource compiler the
//! exe just keeps the default icon.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/paneltarium.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/paneltarium.ico")
        .set("ProductName", "Paneltarium")
        .set("FileDescription", "Paneltarium profile manager");
    if let Err(e) = res.compile() {
        println!("cargo:warning=couldn't embed the Windows icon ({e}); the exe keeps the default icon");
    }
}
