//! End to end test: run the real executable, synthesise a gesture, and check that
//! Windows actually changed virtual desktop.
//!
//! Marked `#[ignore]` because it needs an interactive desktop session and it takes over
//! the mouse pointer for a couple of seconds. Run it with:
//!
//! ```text
//! cargo test -- --ignored --nocapture
//! ```
//!
//! The test creates two scratch virtual desktops, measures the gesture between them, and
//! closes both afterwards, so it does not care how many desktops the machine already had
//! and leaves it as it was found.
//!
//! Desktop membership is read through `IVirtualDesktopManager`, which unlike
//! `IVirtualDesktopManagerInternal` is documented and stable. windows-sys ships the
//! class id but no COM interface definitions, so the three method vtable below is written
//! out by hand from `shobjidl_core.h`.

#![cfg(windows)]

use std::ffi::c_void;
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};

/// windows-sys models the Win32 `BOOL` as a plain `i32`.
type Bool = i32;
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::System::Threading::{OpenMutexW, MUTEX_MODIFY_STATE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEINPUT, VIRTUAL_KEY, VK_D, VK_F4, VK_LCONTROL, VK_LEFT, VK_LWIN,
};
use windows_sys::Win32::UI::Shell::VirtualDesktopManager;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
    PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, TranslateMessage, LWA_ALPHA, MSG,
    PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
};

/// `IVirtualDesktopManager`, `{A5CD92FF-29BE-454C-8D04-D82879FB3F1B}`.
const IID_IVIRTUAL_DESKTOP_MANAGER: GUID = GUID::from_u128(0xa5cd92ff_29be_454c_8d04_d82879fb3f1b);

#[repr(C)]
struct VirtualDesktopManagerVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    is_window_on_current_virtual_desktop:
        unsafe extern "system" fn(*mut c_void, HWND, *mut Bool) -> HRESULT,
    get_window_desktop_id: unsafe extern "system" fn(*mut c_void, HWND, *mut GUID) -> HRESULT,
    move_window_to_desktop: unsafe extern "system" fn(*mut c_void, HWND, *const GUID) -> HRESULT,
}

/// Minimal owning wrapper over the COM object.
struct DesktopManager(*mut *const VirtualDesktopManagerVtbl);

impl DesktopManager {
    fn new() -> Self {
        let mut raw: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &VirtualDesktopManager,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IVIRTUAL_DESKTOP_MANAGER,
                &mut raw,
            )
        };
        assert!(
            hr >= 0,
            "CoCreateInstance(VirtualDesktopManager) failed: {hr:#x}"
        );
        Self(raw.cast())
    }

    /// Identifier of the desktop the window belongs to, rendered as text because
    /// windows-sys derives neither `Debug` nor `PartialEq` on `GUID`.
    fn desktop_id(&self, window: HWND) -> Result<String, HRESULT> {
        let mut id = GUID::from_u128(0);
        let hr = unsafe { ((**self.0).get_window_desktop_id)(self.0.cast(), window, &mut id) };
        if hr < 0 {
            return Err(hr);
        }
        Ok(format!(
            "{:08x}-{:04x}-{:04x}-{}",
            id.data1,
            id.data2,
            id.data3,
            id.data4
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ))
    }

    /// Whether the given top level window lives on the desktop the user is looking at.
    fn is_on_current_desktop(&self, window: HWND) -> bool {
        let mut on_current: Bool = 0;
        let hr = unsafe {
            ((**self.0).is_window_on_current_virtual_desktop)(
                self.0.cast(),
                window,
                &mut on_current,
            )
        };
        assert!(hr >= 0, "IsWindowOnCurrentVirtualDesktop failed: {hr:#x}");
        on_current != 0
    }
}

impl Drop for DesktopManager {
    fn drop(&mut self) {
        unsafe { ((**self.0).release)(self.0.cast()) };
    }
}

/// Kills the utility when the test ends, however it ends.
struct RunningUtility(Child);

impl Drop for RunningUtility {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "takes over the mouse pointer and switches virtual desktops"]
fn the_wheel_drag_gesture_changes_virtual_desktop() {
    let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    assert!(hr >= 0, "CoInitializeEx failed: {hr:#x}");

    // A copy already running would win the single instance guard, and the copy this test
    // starts would put up a dialog instead of installing its hook. Caught here so the
    // failure says what to do, rather than looking like a broken gesture.
    assert!(
        !another_copy_is_running(),
        "mouse-desktop is already running. Stop it first, with Exit in its tray menu \
         or with 'Stop-Process -Name mouse-desktop', then run the test again."
    );

    let manager = DesktopManager::new();

    // The utility ignores synthetic input by default, so that a click it replays is not
    // read back as a new gesture. The test needs that filter lifted.
    let utility = RunningUtility(
        Command::new(env!("CARGO_BIN_EXE_mouse-desktop"))
            .arg("--allow-injected")
            .arg("--verbose")
            .stdout(Stdio::null())
            .spawn()
            .expect("failed to start mouse-desktop"),
    );
    // Give the child time to install its hook.
    sleep(Duration::from_millis(1500));

    // Marks the desktop the user was on when the test started, so the machine can be put
    // back exactly as it was found.
    let origin = create_probe_window(&manager);
    pump();
    assert!(
        manager.is_on_current_desktop(origin),
        "the origin window should start on the desktop the user is on"
    );

    // Two scratch desktops rather than one, because Ctrl+Win+D appends the new desktop to
    // the end of the list rather than next to the current one. Only two consecutive
    // creations are guaranteed to leave the second immediately to the right of the first,
    // and adjacency is exactly what a one desktop drag is being measured against.
    press_with_win_and_ctrl(VK_D);
    wait_until(|| !manager.is_on_current_desktop(origin))
        .expect("Ctrl+Win+D should have moved us off the origin desktop");
    let left_desktop = create_probe_window(&manager);
    pump();

    press_with_win_and_ctrl(VK_D);
    wait_until(|| !manager.is_on_current_desktop(left_desktop))
        .expect("the second Ctrl+Win+D should have created another desktop");
    let right_desktop = create_probe_window(&manager);
    pump();

    // The actual subject of the test: the gesture, not the shortcut.
    wheel_drag(Direction::Left);
    let dragged_left = wait_until(|| manager.is_on_current_desktop(left_desktop));

    let dragged_right = if dragged_left.is_ok() {
        wheel_drag(Direction::Right);
        wait_until(|| manager.is_on_current_desktop(right_desktop))
    } else {
        Err("not attempted, dragging left had already failed")
    };

    let cleanup = tidy_up(&manager, origin, left_desktop, right_desktop);

    unsafe { DestroyWindow(origin) };
    drop(utility);
    drop(manager);
    unsafe { CoUninitialize() };

    dragged_left.expect("holding the wheel button and dragging left did not change desktop");
    dragged_right.expect("holding the wheel button and dragging right did not change desktop");
    cleanup.expect("the test could not put the virtual desktops back as it found them");
}

/// Close the two scratch desktops and return to where the user was.
///
/// Every close is guarded by the marker window that proves which desktop is in front of
/// us. Closing a desktop by position instead would move the user's windows somewhere else
/// the moment a gesture landed one desktop off.
fn tidy_up(
    manager: &DesktopManager,
    origin: HWND,
    left_desktop: HWND,
    right_desktop: HWND,
) -> Result<(), &'static str> {
    for (marker, window) in [(right_desktop, right_desktop), (left_desktop, left_desktop)] {
        if !manager.is_on_current_desktop(marker) {
            // Standing somewhere unexpected, so guessing which desktop to close is not
            // safe. Leave them and say so.
            unsafe {
                DestroyWindow(right_desktop);
                DestroyWindow(left_desktop);
            }
            eprintln!("warning: left up to two empty virtual desktops behind, close them by hand");
            return Err("could not identify the scratch desktop");
        }

        unsafe { DestroyWindow(window) };
        press_with_win_and_ctrl(VK_F4);
        sleep(Duration::from_millis(400));
    }

    // Closing the last desktop lands on the new last one, which is at or to the right of
    // where the user started, so walking left always finds the way back.
    for _ in 0..32 {
        if manager.is_on_current_desktop(origin) {
            return Ok(());
        }
        press_with_win_and_ctrl_arrow(VK_LEFT);
    }

    Err("could not find the way back to the starting desktop")
}

/// Diagnostic aid, not an assertion of product behaviour: prints what the documented
/// desktop APIs report before and after a desktop change, which is the only way to tell a
/// broken shortcut apart from a probe window that is being reported wrongly.
#[test]
#[ignore = "diagnostic, prints virtual desktop identifiers"]
fn report_what_the_desktop_apis_see() {
    let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    assert!(hr >= 0, "CoInitializeEx failed: {hr:#x}");
    let manager = DesktopManager::new();

    // The virtual desktop manager only tracks windows it would show in Task View, and
    // reports every other window as being on the current desktop no matter what. Finding
    // a style it does track is what makes the end to end test meaningful.
    let candidates: [(&str, u32, u32); 4] = [
        (
            "layered popup, tool window",
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            WS_POPUP | WS_VISIBLE,
        ),
        ("layered popup", WS_EX_LAYERED, WS_POPUP | WS_VISIBLE),
        (
            "layered overlapped",
            WS_EX_LAYERED,
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        ),
        ("plain overlapped", 0, WS_OVERLAPPEDWINDOW | WS_VISIBLE),
    ];

    for (name, ex_style, style) in candidates {
        let window = create_window(ex_style, style);
        pump();
        match manager.desktop_id(window) {
            Ok(id) => println!("{name}: tracked, desktop {id}"),
            Err(hr) => println!("{name}: NOT tracked, GetWindowDesktopId returned {hr:#x}"),
        }
        unsafe { DestroyWindow(window) };
        pump();
    }

    drop(manager);
    unsafe { CoUninitialize() };
}

/// Whether another copy of the utility holds the single instance mutex.
///
/// The name has to match the one in `src/main.rs`. An integration test cannot see inside
/// the binary it drives, so this is the one piece of knowledge the two share.
fn another_copy_is_running() -> bool {
    let name: Vec<u16> = r"Local\mouse-desktop"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // Any access right answers the question; the handle is only opened to see whether the
    // object exists at all.
    let handle = unsafe { OpenMutexW(MUTEX_MODIFY_STATE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }

    unsafe { CloseHandle(handle) };
    true
}

/// Poll a condition for up to three seconds, pumping messages so the probe window stays
/// responsive while the desktop animation runs.
fn wait_until(mut condition: impl FnMut() -> bool) -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        pump();
        if condition() {
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    Err("timed out")
}

enum Direction {
    Left,
    Right,
}

/// Synthesise the gesture: press the wheel button, drag horizontally, release.
///
/// The drag is made of absolute `SendInput` moves rather than `SetCursorPos`, because
/// `SetCursorPos` warps the pointer without generating the events a low level mouse hook
/// sees. Absolute coordinates also make the distance travelled independent of the
/// machine's pointer speed and acceleration settings.
fn wheel_drag(direction: Direction) {
    let (width, height) = screen_size();
    let start_x = width / 2;
    let y = height / 2;
    let step = match direction {
        Direction::Left => -25,
        Direction::Right => 25,
    };

    move_cursor_to(start_x, y);
    sleep(Duration::from_millis(80));

    send(&[mouse_event(MOUSEEVENTF_MIDDLEDOWN)]);
    sleep(Duration::from_millis(40));

    // Well past the 60 pixel threshold, but quick enough that the repeat cooldown keeps
    // it to a single desktop switch.
    for i in 1..=4 {
        move_cursor_to(start_x + step * i, y);
        sleep(Duration::from_millis(30));
    }

    send(&[mouse_event(MOUSEEVENTF_MIDDLEUP)]);
    sleep(Duration::from_millis(100));
}

/// Move the pointer to a screen pixel using the normalised 0..65535 range `SendInput`
/// expects for absolute motion.
fn move_cursor_to(x: i32, y: i32) {
    let (width, height) = screen_size();
    let normalised_x = (x as i64 * 65535 / (width - 1) as i64) as i32;
    let normalised_y = (y as i64 * 65535 / (height - 1) as i64) as i32;

    let mut input = mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE);
    input.Anonymous.mi.dx = normalised_x;
    input.Anonymous.mi.dy = normalised_y;
    send(&[input]);
}

/// Send a keyboard shortcut of the form `Ctrl+Win+key`, used only for test setup and
/// teardown, never for the behaviour under test.
fn press_with_win_and_ctrl(key: VIRTUAL_KEY) {
    press_shortcut(key, 0);
}

/// The same, for the arrow keys, which are extended keys and need the flag to register.
fn press_with_win_and_ctrl_arrow(key: VIRTUAL_KEY) {
    press_shortcut(key, KEYEVENTF_EXTENDEDKEY);
}

fn press_shortcut(key: VIRTUAL_KEY, key_flags: u32) {
    send(&[
        key_event(VK_LCONTROL, 0),
        key_event(VK_LWIN, 0),
        key_event(key, key_flags),
        key_event(key, key_flags | KEYEVENTF_KEYUP),
        key_event(VK_LWIN, KEYEVENTF_KEYUP),
        key_event(VK_LCONTROL, KEYEVENTF_KEYUP),
    ]);
    sleep(Duration::from_millis(400));
}

fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// A real top level window, needed because `IsWindowOnCurrentVirtualDesktop` has no
/// desktop to report for message-only windows.
///
/// It is fully transparent and covers the middle of the screen, so that if the gesture
/// were not recognised the replayed middle click lands here instead of on the user's
/// applications.
/// A window the virtual desktop manager actually tracks.
///
/// This has to be a visible overlapped window: `WS_POPUP` and tool windows are invisible
/// to the desktop manager, which then answers `IsWindowOnCurrentVirtualDesktop` with
/// "yes" for every desktop and makes the test pass no matter what. Layering it at alpha 8
/// keeps it from getting in the way while leaving it a real window.
fn create_probe_window(manager: &DesktopManager) -> HWND {
    let window = create_window(WS_EX_LAYERED, WS_OVERLAPPEDWINDOW | WS_VISIBLE);

    // Desktop assignment is not immediate, and an unassigned window reports the all zero
    // identifier.
    wait_until(|| {
        manager
            .desktop_id(window)
            .is_ok_and(|id| !id.trim_matches(['0', '-']).is_empty())
    })
    .expect("the virtual desktop manager never assigned the probe window to a desktop");

    window
}

fn create_window(ex_style: u32, style: u32) -> HWND {
    let class_name: Vec<u16> = "MouseDesktopProbe\0".encode_utf16().collect();

    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(probe_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: ptr::null_mut(),
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    // A second run in the same process would fail here, which is fine: the class is
    // already registered and CreateWindowExW below will still find it.
    unsafe { RegisterClassW(&class) };

    let (width, height) = screen_size();
    let size = 400;

    let window = unsafe {
        CreateWindowExW(
            ex_style,
            class_name.as_ptr(),
            class_name.as_ptr(),
            style,
            (width - size) / 2,
            (height - size) / 2,
            size,
            size,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    assert!(!window.is_null(), "failed to create the probe window");

    // Invisible but still part of the desktop, and still able to absorb a stray click.
    unsafe { SetLayeredWindowAttributes(window, 0, 8, LWA_ALPHA) };

    window
}

unsafe extern "system" fn probe_wnd_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn pump() {
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn key_event(vk: VIRTUAL_KEY, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_event(flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    let size = std::mem::size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size) };
    assert_eq!(
        sent as usize,
        inputs.len(),
        "SendInput was blocked, most likely by an elevated foreground window"
    );
}
