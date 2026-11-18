// Embeds the application icon and version information into spacescout.exe so
// Explorer, Start menu shortcuts and Settings > Apps show the SpaceScout icon.
fn main() {
    println!("cargo:rerun-if-changed=assets/spacescout.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/spacescout.ico");
        res.set("FileDescription", "SpaceScout disk space analyzer");
        res.set("ProductName", "SpaceScout");
        res.compile().expect("failed to embed Windows resources");
    }
}
