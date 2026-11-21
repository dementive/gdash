//! Embeds the app icon and name into gdash.exe (shown by Start search, Explorer, the taskbar
//! and Task Manager).

fn main() {
    println!("cargo:rerun-if-changed=assets/gdash.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/gdash.ico");
        res.set("ProductName", "gdash");
        res.set("FileDescription", "gdash");
        res.compile().expect("embedding the icon needs the Windows SDK's rc.exe");
    }
}
