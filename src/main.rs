//! mouse-desktop: switch Windows virtual desktops with a mouse gesture.
//!
//! Hold the wheel button and drag horizontally. Needs no Administrator privileges, no
//! runtime and no driver. See `README.md`.

// Release builds have no console, so nothing flashes on screen when Windows starts the
// utility at sign-in. Debug builds keep one, which is what the end to end test reads.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod config;
mod desktop;
mod gesture;
mod hook;
mod tray;

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MessageBoxW, PostQuitMessage, TranslateMessage, MB_ICONERROR,
    MB_ICONINFORMATION, MB_OK, MSG, SW_SHOWNORMAL, WM_APP,
};

use config::Config;
use desktop::Direction;
use gesture::Action;
use hook::MouseHook;
use tray::Tray;

/// Posted by the hook callback to ask the message loop to carry out an action.
pub const WM_ACTION: u32 = WM_APP + 1;

static VERBOSE: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether gesture recognition is currently switched on. Read by the tray menu.
pub fn gestures_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        show_message("mouse-desktop", &usage(), MB_ICONINFORMATION);
        return;
    }

    VERBOSE.store(
        args.iter().any(|a| a == "--verbose" || a == "-v"),
        Ordering::Relaxed,
    );
    let allow_injected = args.iter().any(|a| a == "--allow-injected");

    if already_running() {
        show_message(
            "mouse-desktop",
            "mouse-desktop is already running. Look for its icon in the notification area.",
            MB_ICONINFORMATION,
        );
        return;
    }

    let config_path = config::path();
    let (config, warnings) = config::load_or_create(&config_path);
    ENABLED.store(config.enabled, Ordering::Relaxed);

    let tray = match Tray::new() {
        Ok(tray) => tray,
        Err(error) => {
            show_message("mouse-desktop", &error, MB_ICONERROR);
            return;
        }
    };

    // Dropping the hook unregisters it, so it has to outlive the message loop.
    let _hook = match MouseHook::install(config.gesture, allow_injected) {
        Ok(hook) => hook,
        Err(error) => {
            show_message("mouse-desktop", &error, MB_ICONERROR);
            return;
        }
    };
    hook::set_enabled(config.enabled);
    tray.set_tooltip(&tooltip(&config));

    log(&format!(
        "started, configuration from {}",
        config_path.display()
    ));
    report(&tray, &warnings);

    run_message_loop(&tray, &config_path);
}

fn usage() -> String {
    format!(
        "mouse-desktop {}\n\n\
         Switch Windows virtual desktops with a mouse gesture.\n\
         Hold the wheel button and drag left or right.\n\n\
         Options:\n\
         \x20 -v, --verbose      Write a log next to the configuration file\n\
         \x20 --allow-injected   Also react to synthetic mouse input, for end to end tests\n\
         \x20 -h, --help         Show this message\n\n\
         Configuration file:\n\x20 {}",
        env!("CARGO_PKG_VERSION"),
        config::path().display()
    )
}

/// Refuse to start a second copy: two hooks would each swallow the trigger button and
/// switch desktops twice.
fn already_running() -> bool {
    // "Local\" scopes the name to this sign-in session, so different users on the same
    // machine do not block each other. The end to end test opens this same name to check
    // no copy is already running, so the two have to stay in step.
    let name = wide(r"Local\mouse-desktop");

    // The handle is deliberately leaked: it must stay open for the lifetime of the
    // process, and the process exiting is what releases it.
    let status = unsafe {
        CreateMutexW(ptr::null(), 1, name.as_ptr());
        GetLastError()
    };

    status == ERROR_ALREADY_EXISTS
}

/// Pump messages on the thread that owns the hook. The hook callback is only invoked
/// while this thread is inside `GetMessageW`.
fn run_message_loop(tray: &Tray, config_path: &Path) {
    let mut msg: MSG = unsafe { std::mem::zeroed() };

    // GetMessageW returns 0 on WM_QUIT and -1 on error, so anything else keeps going.
    while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
        // Actions and menu choices arrive as thread messages, which have no window to be
        // dispatched to.
        if msg.hwnd.is_null() {
            match msg.message {
                WM_ACTION => {
                    if let Some(action) = hook::decode_action(msg.wParam) {
                        perform(action);
                    }
                    continue;
                }
                tray::WM_MENU_COMMAND => {
                    handle_menu(tray, config_path, msg.wParam);
                    continue;
                }
                _ => {}
            }
        }

        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn perform(action: Action) {
    log(&format!("{action:?}"));

    match action {
        Action::SwitchLeft => desktop::switch(Direction::Left),
        Action::SwitchRight => desktop::switch(Direction::Right),
        Action::ReplayClick(button) => desktop::replay_click(button),
    }
}

fn handle_menu(tray: &Tray, config_path: &Path, command: usize) {
    match command {
        tray::MENU_TOGGLE_ENABLED => {
            let enabled = !gestures_enabled();
            ENABLED.store(enabled, Ordering::Relaxed);
            hook::set_enabled(enabled);
            tray.set_tooltip(if enabled {
                "mouse-desktop: gesture enabled"
            } else {
                "mouse-desktop: gesture disabled"
            });
            log(if enabled { "enabled" } else { "disabled" });
        }

        tray::MENU_OPEN_CONFIG => open_in_default_editor(config_path),

        tray::MENU_RELOAD_CONFIG => {
            let (config, warnings) = config::load_or_create(config_path);
            hook::set_config(config.gesture);
            ENABLED.store(config.enabled, Ordering::Relaxed);
            hook::set_enabled(config.enabled);
            tray.set_tooltip(&tooltip(&config));
            log("configuration reloaded");
            report(tray, &warnings);
        }

        tray::MENU_TOGGLE_AUTOSTART => {
            let wanted = !autostart::is_enabled();
            if let Err(error) = autostart::set(wanted) {
                tray.warn("mouse-desktop", &error);
                log(&error);
            }
        }

        tray::MENU_ABOUT => show_message("mouse-desktop", &usage(), MB_ICONINFORMATION),

        tray::MENU_EXIT => {
            log("exiting");
            unsafe { PostQuitMessage(0) };
        }

        tray::MENU_RESTORE_ICON => {
            log("Explorer restarted, putting the tray icon back");
            tray.restore();
        }

        _ => {}
    }
}

fn tooltip(config: &Config) -> String {
    let button = match config.gesture.trigger {
        gesture::Button::Middle => "wheel button",
        gesture::Button::Right => "right button",
        gesture::Button::X1 => "side button 1",
        gesture::Button::X2 => "side button 2",
    };

    if config.enabled {
        format!("mouse-desktop: hold the {button} and drag")
    } else {
        "mouse-desktop: gesture disabled".to_owned()
    }
}

/// Surface configuration problems, which a utility with no window would otherwise hide.
fn report(tray: &Tray, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }

    for warning in warnings {
        log(warning);
    }
    tray.warn("mouse-desktop configuration", &warnings.join("\n"));
}

fn open_in_default_editor(path: &Path) {
    let verb = wide("open");
    let target = wide(&path.to_string_lossy());

    unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
}

fn show_message(title: &str, body: &str, icon: u32) {
    let title = wide(title);
    let body = wide(body);
    unsafe { MessageBoxW(ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

/// A millisecond counter that never goes backwards and never wraps.
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// Diagnostic output, silent unless `--verbose` was passed.
///
/// Release builds have no console, so the same lines also go to a file next to the
/// configuration. Never called from the hook callback, which must not touch the disk.
pub fn log(message: &str) {
    if !VERBOSE.load(Ordering::Relaxed) {
        return;
    }

    eprintln!("[mouse-desktop] {message}");

    if let Some(path) = log_path() {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "[{}] {message}", now_ms());
        }
    }
}

fn log_path() -> Option<PathBuf> {
    config::path()
        .parent()
        .map(|dir| dir.join("mouse-desktop.log"))
}

/// A null terminated UTF-16 string, the form every wide Win32 entry point expects.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
