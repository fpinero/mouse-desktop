//! Synthetic input: virtual desktop switching and click replay.
//!
//! Desktop switching is done by synthesising the documented `Ctrl+Win+Left` and
//! `Ctrl+Win+Right` shortcuts. The undocumented `IVirtualDesktopManagerInternal` COM
//! interface would switch desktops directly, but Microsoft changes its GUIDs with nearly
//! every major Windows update, so callers break silently several times a year. See
//! `CLAUDE.md`.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    VIRTUAL_KEY, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RIGHT,
    VK_RMENU, VK_RSHIFT, VK_RWIN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{XBUTTON1, XBUTTON2};

use crate::gesture::Button;

/// Tag written into `dwExtraInfo` of every event this process injects, so the hook can
/// recognise its own input even when injected events are not being filtered out.
pub const INJECTION_SIGNATURE: usize = 0x4D44_5F31; // "MD_1"

/// Which neighbouring desktop to move to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
}

/// Modifiers that would change the meaning of the shortcut if the user happens to be
/// holding them, so they are released for the duration of the injection and restored
/// afterwards.
const INTERFERING_MODIFIERS: [VIRTUAL_KEY; 4] = [VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU];

/// Move to the neighbouring virtual desktop.
///
/// Windows does not wrap around: on the first or last desktop this is a no-op, which is
/// the same behaviour as pressing the shortcut by hand.
pub fn switch(direction: Direction) {
    let arrow = match direction {
        Direction::Left => VK_LEFT,
        Direction::Right => VK_RIGHT,
    };

    // Pressing a modifier the user is already physically holding, and then releasing it,
    // would leave applications believing the key came up. Only inject what is missing.
    let ctrl_held = is_down(VK_LCONTROL) || is_down(VK_RCONTROL);
    let win_held = is_down(VK_LWIN) || is_down(VK_RWIN);
    let interfering: Vec<VIRTUAL_KEY> = INTERFERING_MODIFIERS
        .into_iter()
        .filter(|vk| is_down(*vk))
        .collect();

    let mut inputs = Vec::with_capacity(12);

    for vk in &interfering {
        inputs.push(key(*vk, KEYEVENTF_KEYUP));
    }
    if !ctrl_held {
        inputs.push(key(VK_LCONTROL, 0));
    }
    if !win_held {
        inputs.push(key(VK_LWIN, 0));
    }

    // Arrow keys are extended keys; without the flag the shortcut may not register.
    inputs.push(key(arrow, KEYEVENTF_EXTENDEDKEY));
    inputs.push(key(arrow, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP));

    // The Windows key is released after another key has been pressed, so the Start menu
    // does not open.
    if !win_held {
        inputs.push(key(VK_LWIN, KEYEVENTF_KEYUP));
    }
    if !ctrl_held {
        inputs.push(key(VK_LCONTROL, KEYEVENTF_KEYUP));
    }
    for vk in &interfering {
        inputs.push(key(*vk, 0));
    }

    send(&inputs);
}

/// Re-inject a real click at the current cursor position.
///
/// Called when the trigger button was pressed and released without a gesture, so the
/// application under the cursor gets the click the utility swallowed.
pub fn replay_click(button: Button) {
    let (down, up, data) = match button {
        Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        Button::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1 as u32),
        Button::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2 as u32),
    };

    send(&[mouse(down, data), mouse(up, data)]);
}

/// Whether a virtual key is currently held down.
fn is_down(vk: VIRTUAL_KEY) -> bool {
    // The high order bit of the return value is set while the key is down.
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

fn key(vk: VIRTUAL_KEY, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTION_SIGNATURE,
            },
        },
    }
}

fn mouse(flags: u32, mouse_data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTION_SIGNATURE,
            },
        },
    }
}

/// Submit the whole sequence in a single call, so no real input can be interleaved with
/// it.
fn send(inputs: &[INPUT]) {
    let size = std::mem::size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size) };

    if sent as usize != inputs.len() {
        // The usual cause is UIPI: the foreground window belongs to a process running at
        // a higher integrity level, so a non-elevated process may not inject into it.
        // There is no fix without Administrator privileges, which this utility refuses to
        // require, so report and carry on.
        crate::log(&format!(
            "input injection blocked: {sent} of {} events accepted, \
             probably an elevated foreground window",
            inputs.len()
        ));
    }
}
