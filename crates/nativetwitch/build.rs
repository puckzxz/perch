//! Embeds the app icon into the executable.
//!
//! Windows takes a program's icon from its resources, not from anything the
//! process does at runtime, and gpui 0.2.2 has no window-icon API — so this is
//! the only place it can come from. It is also where Explorer, the taskbar and
//! Alt-Tab get it.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/nativetwitch.ico");

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/nativetwitch.ico");

        if let Err(e) = resource.compile() {
            // Not fatal. An icon-less build still runs, and failing a build
            // over decoration would be worse than the missing decoration.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
