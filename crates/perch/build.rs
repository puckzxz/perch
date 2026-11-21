//! Embeds the app icon into the executable.
//!
//! Windows takes a program's icon from its resources, not from anything the
//! process does at runtime, and gpui has no window-icon API on Windows (the
//! `icon` in its window options is X11's alone) — so this is the only place it
//! can come from. It is also where Explorer, the taskbar and Alt-Tab get it.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/perch.ico");

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/perch.ico");

        if let Err(e) = resource.compile() {
            // Not fatal. An icon-less build still runs, and failing a build
            // over decoration would be worse than the missing decoration.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
