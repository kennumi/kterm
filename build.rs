// Embeds the app icon (resource id 1, also used for the window icon) and version info into kterm.exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/kterm.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/kterm.ico").set("FileDescription", "KTerm").set("ProductName", "KTerm");
    res.compile().expect("embedding icon resource");
}
