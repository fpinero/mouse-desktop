//! The notification area icon and its menu.
//!
//! The icon needs a window to send its callback messages to, so a hidden one is created
//! and never shown. It is a normal window rather than a message-only window on purpose:
//! message-only windows do not receive the `TaskbarCreated` broadcast, and without that
//! the icon would disappear for good the first time Explorer restarts.
//!
//! The icon bitmap is drawn in code instead of being embedded as a resource file, so the
//! repository carries no binary assets and the build needs no extra crate.

use std::ptr;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, GetCursorPos, PostQuitMessage, PostThreadMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu, HICON, MF_CHECKED, MF_SEPARATOR,
    MF_STRING, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_DESTROY,
    WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

use crate::autostart;

/// Sent by the notification icon to the hidden window. The mouse event is in `lparam`.
const WM_TRAYICON: u32 = WM_APP + 2;

/// Posted to the message loop when the user picks a menu entry.
pub const WM_MENU_COMMAND: u32 = WM_APP + 3;

pub const MENU_TOGGLE_ENABLED: usize = 1;
pub const MENU_OPEN_CONFIG: usize = 2;
pub const MENU_RELOAD_CONFIG: usize = 3;
pub const MENU_TOGGLE_AUTOSTART: usize = 4;
pub const MENU_ABOUT: usize = 5;
pub const MENU_EXIT: usize = 6;

const CLASS_NAME: &str = "MouseDesktopTray";
const ICON_ID: u32 = 1;

/// The notification icon, removed from the tray when dropped.
pub struct Tray {
    window: HWND,
    icon: HICON,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let window = create_hidden_window()?;
        let icon = create_icon();

        let tray = Self { window, icon };
        if !tray.send(NIM_ADD, tray.base_data()) {
            return Err("Shell_NotifyIconW could not add the tray icon".to_owned());
        }

        Ok(tray)
    }

    /// Put the icon back after Explorer restarts, which otherwise loses it silently.
    pub fn restore(&self) {
        self.send(NIM_ADD, self.base_data());
    }

    pub fn set_tooltip(&self, text: &str) {
        let mut data = self.base_data();
        copy_into(&mut data.szTip, text);
        self.send(NIM_MODIFY, data);
    }

    /// Show a balloon, used to surface configuration problems that would otherwise be
    /// invisible in a windowless utility.
    pub fn warn(&self, title: &str, body: &str) {
        let mut data = self.base_data();
        data.uFlags |= NIF_INFO;
        data.dwInfoFlags = NIIF_WARNING;
        copy_into(&mut data.szInfoTitle, title);
        copy_into(&mut data.szInfo, body);
        self.send(NIM_MODIFY, data);
    }

    fn base_data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.window;
        data.uID = ICON_ID;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAYICON;
        data.hIcon = self.icon;
        copy_into(&mut data.szTip, "mouse-desktop");
        data
    }

    fn send(&self, message: u32, data: NOTIFYICONDATAW) -> bool {
        unsafe { Shell_NotifyIconW(message, &data) != 0 }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.window;
        data.uID = ICON_ID;

        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
            DestroyIcon(self.icon);
            DestroyWindow(self.window);
        }
    }
}

/// Identifier of the broadcast Explorer sends after it restarts.
fn taskbar_created_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
}

fn create_hidden_window() -> Result<HWND, String> {
    let class_name = wide(CLASS_NAME);
    let instance = unsafe { GetModuleHandleW(ptr::null()) };

    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    unsafe { RegisterClassW(&class) };

    // Created without WS_VISIBLE and never shown: it exists only to receive messages.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };

    if window.is_null() {
        Err("could not create the hidden tray window".to_owned())
    } else {
        Ok(window)
    }
}

unsafe extern "system" fn wnd_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == taskbar_created_message() {
        // The message loop owns the Tray, so ask it to put the icon back rather than
        // reaching for it from here.
        post_command(MENU_RESTORE_ICON);
        return 0;
    }

    match message {
        WM_TRAYICON => {
            let event = lparam as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP || event == WM_CONTEXTMENU {
                show_menu(window);
            }
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Internal command, outside the range the menu itself can produce.
pub const MENU_RESTORE_ICON: usize = 100;

fn post_command(command: usize) {
    unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_MENU_COMMAND, command, 0) };
}

fn show_menu(window: HWND) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }

    let enabled = crate::gestures_enabled();
    add_item(menu, MENU_TOGGLE_ENABLED, "Gesture enabled", enabled);
    add_separator(menu);
    add_item(menu, MENU_OPEN_CONFIG, "Open configuration file", false);
    add_item(menu, MENU_RELOAD_CONFIG, "Reload configuration", false);
    add_separator(menu);
    add_item(
        menu,
        MENU_TOGGLE_AUTOSTART,
        "Start with Windows",
        autostart::is_enabled(),
    );
    add_separator(menu);
    add_item(menu, MENU_ABOUT, "About mouse-desktop", false);
    add_item(menu, MENU_EXIT, "Exit", false);

    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) };

    // Without this the menu stays open when the user clicks somewhere else, a documented
    // quirk of tray menus.
    unsafe { SetForegroundWindow(window) };

    let choice = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };

    if choice > 0 {
        post_command(choice as usize);
    }
}

fn add_item(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    id: usize,
    label: &str,
    checked: bool,
) {
    let text = wide(label);
    let flags = MF_STRING | if checked { MF_CHECKED } else { 0 };
    unsafe { AppendMenuW(menu, flags, id, text.as_ptr().cast()) };
}

fn add_separator(menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU) {
    unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) };
}

/// Draw the tray icon: two arrows pointing away from each other, which is what the
/// gesture does to the desktops.
///
/// `CreateIcon` takes bottom-up rows, a 32 bit BGRA colour plane and a 1 bit mask where a
/// set bit means transparent. Both the mask and the alpha channel are filled in, because
/// older shells honour only the mask while current ones honour only the alpha.
fn create_icon() -> HICON {
    const SIZE: usize = 32;

    let mut colour = [0u8; SIZE * SIZE * 4];
    let mut mask = [0xFFu8; SIZE * SIZE / 8];

    for y in 0..SIZE {
        for x in 0..SIZE {
            if !inside_glyph(x, y, SIZE) {
                continue;
            }

            let row = SIZE - 1 - y;
            let pixel = row * SIZE + x;

            colour[pixel * 4] = 0xFF; // blue
            colour[pixel * 4 + 1] = 0x9B; // green
            colour[pixel * 4 + 2] = 0x2E; // red
            colour[pixel * 4 + 3] = 0xFF; // opaque

            mask[pixel / 8] &= !(0x80 >> (pixel % 8));
        }
    }

    unsafe {
        CreateIcon(
            ptr::null_mut(),
            SIZE as i32,
            SIZE as i32,
            1,
            32,
            mask.as_ptr(),
            colour.as_ptr(),
        )
    }
}

/// Two solid triangles, one pointing left and one pointing right.
///
/// Worked out in fractions of the icon rather than in pixels, so the shape keeps its
/// proportions whatever size the shell asks for, and so it fills nearly the whole canvas.
/// A glyph that leaves wide margins ends up looking smaller than its neighbours once the
/// notification area scales it down.
fn inside_glyph(x: usize, y: usize, size: usize) -> bool {
    /// Where the tips sit, as a fraction of the width in from each edge.
    const TIP: f32 = 0.02;
    /// Where the wide ends sit, which also sets the gap down the middle.
    const BASE: f32 = 0.46;
    /// Half the height of a wide end, as a fraction of the icon.
    const REACH: f32 = 0.46;

    // Sample pixel centres, which keeps the shape exactly symmetric.
    let horizontal = (x as f32 + 0.5) / size as f32;
    let vertical = (y as f32 + 0.5) / size as f32;
    let distance_from_middle = (vertical - 0.5).abs();

    let slope = |run: f32| run / (BASE - TIP) * REACH;

    let left =
        (TIP..=BASE).contains(&horizontal) && distance_from_middle <= slope(horizontal - TIP);
    let right = ((1.0 - BASE)..=(1.0 - TIP)).contains(&horizontal)
        && distance_from_middle <= slope((1.0 - TIP) - horizontal);

    left || right
}

/// A null terminated UTF-16 string, the form every wide Win32 entry point expects.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy text into one of the fixed size buffers in `NOTIFYICONDATAW`, truncating rather
/// than overflowing and always leaving room for the terminator.
fn copy_into(buffer: &mut [u16], text: &str) {
    let limit = buffer.len() - 1;
    let encoded: Vec<u16> = text.encode_utf16().take(limit).collect();

    buffer[..encoded.len()].copy_from_slice(&encoded);
    buffer[encoded.len()] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape at 32 pixels, which every coordinate below is read off:
    ///
    /// ```text
    /// ..............#..#..............
    /// ...........####..####...........
    /// ......#########..#########......
    /// .##############..##############.
    /// ......#########..#########......
    /// ...........####..####...........
    /// ..............#..#..............
    /// ```
    #[test]
    fn the_glyph_has_two_arrows_facing_outwards() {
        // Tips one pixel in from each edge, on the middle row.
        assert!(inside_glyph(1, 15, 32), "left arrow tip");
        assert!(inside_glyph(30, 15, 32), "right arrow tip");

        // The tall inner edges, which is where each triangle is widest.
        assert!(
            inside_glyph(14, 2, 32),
            "top of the left arrow's inner edge"
        );
        assert!(
            inside_glyph(17, 29, 32),
            "bottom of the right arrow's inner edge"
        );

        assert!(!inside_glyph(15, 15, 32), "gap between the arrows");
        assert!(!inside_glyph(16, 15, 32), "gap between the arrows");
        assert!(!inside_glyph(0, 0, 32), "corner stays empty");
        assert!(!inside_glyph(1, 0, 32), "above the left arrow tip");
    }

    #[test]
    fn the_glyph_is_symmetric_at_every_size_the_shell_may_ask_for() {
        for size in [16, 20, 24, 32, 48, 64] {
            for y in 0..size {
                for x in 0..size {
                    assert_eq!(
                        inside_glyph(x, y, size),
                        inside_glyph(size - 1 - x, y, size),
                        "size {size}, pixel ({x}, {y}) breaks left to right symmetry"
                    );
                    assert_eq!(
                        inside_glyph(x, y, size),
                        inside_glyph(x, size - 1 - y, size),
                        "size {size}, pixel ({x}, {y}) breaks top to bottom symmetry"
                    );
                }
            }
        }
    }

    #[test]
    fn the_glyph_fills_enough_of_the_canvas_to_read_at_tray_size() {
        // The first version drew a small shape in the middle of the icon, which came out
        // visibly punier than its neighbours once the shell scaled it down.
        for size in [16, 32, 64] {
            let filled = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|(x, y)| inside_glyph(*x, *y, size))
                .count();
            let share = filled as f32 / (size * size) as f32;

            assert!(
                share > 0.30,
                "at size {size} the glyph covers only {:.0}% of the icon",
                share * 100.0
            );
        }
    }

    #[test]
    fn the_arrows_reach_almost_to_both_edges() {
        for size in [16, 32, 64] {
            let lit: Vec<usize> = (0..size)
                .filter(|x| (0..size).any(|y| inside_glyph(*x, y, size)))
                .collect();

            assert!(
                lit.first().is_some_and(|first| *first <= 2),
                "size {size}: left margin is {:?} pixels wide",
                lit.first()
            );
            assert!(
                lit.last().is_some_and(|last| *last >= size - 3),
                "size {size}: right margin is too wide, last lit column is {:?}",
                lit.last()
            );
        }
    }

    #[test]
    fn text_longer_than_the_buffer_is_truncated_and_terminated() {
        let mut buffer = [0xFFFFu16; 4];
        copy_into(&mut buffer, "abcdefgh");
        assert_eq!(buffer, [b'a' as u16, b'b' as u16, b'c' as u16, 0]);
    }

    #[test]
    fn short_text_is_copied_whole() {
        let mut buffer = [0xFFFFu16; 8];
        copy_into(&mut buffer, "hi");
        assert_eq!(&buffer[..3], &[b'h' as u16, b'i' as u16, 0]);
    }

    #[test]
    fn menu_identifiers_are_distinct() {
        let ids = [
            MENU_TOGGLE_ENABLED,
            MENU_OPEN_CONFIG,
            MENU_RELOAD_CONFIG,
            MENU_TOGGLE_AUTOSTART,
            MENU_ABOUT,
            MENU_EXIT,
            MENU_RESTORE_ICON,
        ];
        for (index, id) in ids.iter().enumerate() {
            assert!(!ids[index + 1..].contains(id), "identifier {id} is reused");
            assert_ne!(*id, 0, "zero means the menu was dismissed");
        }
    }
}
