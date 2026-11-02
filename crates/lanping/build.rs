//! Embeds the app icon in the Windows executable, so Explorer shows it for the file.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/lanping.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../../packaging/lanping.ico")
        .compile()
        .expect("the Windows resource compiler should embed the icon");
}
