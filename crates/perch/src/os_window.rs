//! What Perch asks of a window that gpui does not ask for it: the platform's
//! handle to it, and keeping a pop-out above every other app.
//!
//! gpui 0.2.2 never makes a window topmost on Windows. `WindowKind::PopUp`
//! is a tool window that is neither topmost nor resizable, and `Floating`
//! behaves like `Normal` there (gpui's windows/window.rs:402-404). So a
//! pop-out is a `Normal` window, and this module puts it on top through the
//! platform, the way `instance` brings the main window forward — through the
//! handle `raw-window-handle` gives, since gpui's own `Window::window_handle`
//! is its id for the window rather than the platform's.
//!
//! Elsewhere every function here is a documented no-op: the pop-out is not
//! offered off Windows (`root::pop_out::offered`), so nothing calls them for
//! real there.

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    /// A Win32 window handle, as the user32 calls below take one.
    pub(crate) type Handle = *mut c_void;

    /// `GWL_STYLE`: the window's style bits, for `Get`/`SetWindowLongPtrW`.
    const GWL_STYLE: i32 = -16;
    /// The maximise box, which gpui adds to every resizable window
    /// (windows/window.rs:407-409). Without it a double-click on a caption
    /// does not fill the screen.
    const WS_MAXIMIZEBOX: isize = 0x0001_0000;
    /// `HWND_TOPMOST`, the place in the z-order above every window that is
    /// not topmost itself.
    const HWND_TOPMOST: Handle = -1isize as Handle;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOACTIVATE: u32 = 0x0010;
    /// Tells the window its frame changed, so a style change takes effect.
    const SWP_FRAMECHANGED: u32 = 0x0020;

    #[link(name = "user32")]
    extern "system" {
        fn GetWindowLongPtrW(window: Handle, index: i32) -> isize;
        fn SetWindowLongPtrW(window: Handle, index: i32, value: isize) -> isize;
        fn SetWindowPos(
            window: Handle,
            insert_after: Handle,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    /// The platform's handle to `window`, or `None` if it has none to give.
    pub(crate) fn hwnd(window: &gpui::Window) -> Option<Handle> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        // Through the trait: gpui's own `Window::window_handle` is its id for
        // the window, not the platform's.
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return None;
        };
        Some(win32.hwnd.get() as Handle)
    }

    /// Put `window` above every window that is not topmost itself, and take
    /// away its maximise box.
    ///
    /// The box goes because the whole picture of a pop-out is its caption
    /// (`controls::drag_layer`), and Windows maximises a caption that is
    /// double-clicked when the window has one. Taken off after creation,
    /// which is why the frame is told it changed.
    pub(crate) fn keep_on_top(window: &gpui::Window) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        // SAFETY: the window's own handle, alive while `window` is borrowed;
        // the calls change its style and its place in the z-order and
        // nothing else, without moving, sizing or activating it.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !WS_MAXIMIZEBOX);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

#[cfg(windows)]
pub(crate) use windows::{hwnd, keep_on_top};

/// Off Windows, nothing: the pop-out is not offered there until a Mac has
/// shown what gpui's `PopUp` panel does with the pointer and with focus.
#[cfg(not(windows))]
pub(crate) fn keep_on_top(_window: &gpui::Window) {}
