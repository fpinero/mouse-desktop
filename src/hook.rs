//! The global low level mouse hook.
//!
//! `SetWindowsHookEx(WH_MOUSE_LL, ...)` needs no privileges, which is the whole reason
//! this utility can exist on a machine without an Administrator account.
//!
//! The callback runs on the thread that installed the hook, in between its
//! `GetMessage` calls, and Windows silently unhooks callbacks that take longer than
//! `LowLevelHooksTimeout` (300 ms by default). So the callback does arithmetic and posts
//! a message, nothing else: the actual input injection happens back in the message loop.

use std::cell::RefCell;
use std::ptr;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK,
    LLMHF_INJECTED, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1,
};

use crate::desktop::INJECTION_SIGNATURE;
use crate::gesture::{Action, Button, Gesture, GestureConfig, RawEvent};
use crate::{now_ms, WM_ACTION};

thread_local! {
    /// Owned by the hook thread alone, so no locking is needed inside the callback.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    gesture: Gesture,
    /// Thread the callback posts actions back to. Always the thread that installed the
    /// hook, but stored rather than queried on every event.
    thread_id: u32,
    /// Process events that other applications injected. Only ever set by the end to end
    /// tests, which have to synthesise a gesture. Events this process injected are
    /// still ignored, otherwise a replayed click would be read as a new gesture.
    allow_injected: bool,
    enabled: bool,
}

/// A hook that unregisters itself when dropped.
pub struct MouseHook {
    handle: HHOOK,
}

impl MouseHook {
    /// Install the hook on the calling thread. That thread has to keep pumping messages
    /// for the callback to be invoked.
    pub fn install(config: GestureConfig, allow_injected: bool) -> Result<Self, String> {
        STATE.with(|state| {
            *state.borrow_mut() = Some(State {
                gesture: Gesture::new(config),
                thread_id: unsafe { GetCurrentThreadId() },
                allow_injected,
                enabled: true,
            });
        });

        // A WH_MOUSE_LL hook is not injected into other processes, so the module handle
        // is ignored and may be null.
        let handle = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), ptr::null_mut(), 0) };

        if handle.is_null() {
            return Err("SetWindowsHookExW failed for WH_MOUSE_LL".to_owned());
        }

        Ok(Self { handle })
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        unsafe { UnhookWindowsHookEx(self.handle) };
        STATE.with(|state| *state.borrow_mut() = None);
    }
}

/// Turn gesture recognition on or off without unhooking, so the tray menu can toggle it.
pub fn set_enabled(enabled: bool) {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.enabled = enabled;
            state.gesture.reset();
        }
    });
}

/// Apply a reloaded configuration file.
pub fn set_config(config: GestureConfig) {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            state.gesture.set_config(config);
        }
    });
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
    let swallow = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return false;
        };
        handle(state, wparam as u32, info)
    });

    if swallow {
        // A non-zero return value hides the event from every other application.
        return 1;
    }

    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// Decide what to do with one hook event. Split out of `hook_proc` so it holds no unsafe
/// code and stays readable.
fn handle(state: &mut State, message: u32, info: &MSLLHOOKSTRUCT) -> bool {
    // Never react to input this process injected, or the replayed click would be read as
    // the start of a new gesture.
    if info.dwExtraInfo == INJECTION_SIGNATURE {
        return false;
    }
    if !state.allow_injected && info.flags & LLMHF_INJECTED != 0 {
        return false;
    }
    if !state.enabled {
        return false;
    }

    let Some(event) = to_raw_event(message, info) else {
        return false;
    };

    let verdict = state.gesture.on_event(event, now_ms());

    if let Some(action) = verdict.action() {
        post(state.thread_id, action);
    }

    verdict.swallows()
}

/// Translate a hook message into the platform independent event the state machine takes.
fn to_raw_event(message: u32, info: &MSLLHOOKSTRUCT) -> Option<RawEvent> {
    let (x, y) = (info.pt.x, info.pt.y);

    // For the two side buttons the pressed one is in the high word of `mouseData`.
    let x_button = || {
        if (info.mouseData >> 16) as u16 == XBUTTON1 {
            Button::X1
        } else {
            Button::X2
        }
    };

    let event = match message {
        WM_MOUSEMOVE => RawEvent::Move { x, y },
        WM_MBUTTONDOWN => RawEvent::Down {
            button: Button::Middle,
            x,
            y,
        },
        WM_MBUTTONUP => RawEvent::Up {
            button: Button::Middle,
            x,
            y,
        },
        WM_RBUTTONDOWN => RawEvent::Down {
            button: Button::Right,
            x,
            y,
        },
        WM_RBUTTONUP => RawEvent::Up {
            button: Button::Right,
            x,
            y,
        },
        WM_XBUTTONDOWN => RawEvent::Down {
            button: x_button(),
            x,
            y,
        },
        WM_XBUTTONUP => RawEvent::Up {
            button: x_button(),
            x,
            y,
        },
        // Wheel scrolling and the left button are not triggers and never will be, so
        // they are left entirely alone.
        _ => return None,
    };

    Some(event)
}

/// Hand the action to the message loop. `PostThreadMessageW` returns immediately, which
/// is what keeps the callback inside its time budget.
fn post(thread_id: u32, action: Action) {
    let code = match action {
        Action::SwitchLeft => 0,
        Action::SwitchRight => 1,
        Action::ReplayClick(button) => 2 + button_code(button),
    };

    unsafe { PostThreadMessageW(thread_id, WM_ACTION, code, 0) };
}

fn button_code(button: Button) -> usize {
    match button {
        Button::Middle => 0,
        Button::Right => 1,
        Button::X1 => 2,
        Button::X2 => 3,
    }
}

/// Rebuild the action encoded by [`post`]. Lives here so the encoding stays in one file.
pub fn decode_action(code: usize) -> Option<Action> {
    match code {
        0 => Some(Action::SwitchLeft),
        1 => Some(Action::SwitchRight),
        2 => Some(Action::ReplayClick(Button::Middle)),
        3 => Some(Action::ReplayClick(Button::Right)),
        4 => Some(Action::ReplayClick(Button::X1)),
        5 => Some(Action::ReplayClick(Button::X2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_survives_a_round_trip_through_the_message_queue() {
        let actions = [
            Action::SwitchLeft,
            Action::SwitchRight,
            Action::ReplayClick(Button::Middle),
            Action::ReplayClick(Button::Right),
            Action::ReplayClick(Button::X1),
            Action::ReplayClick(Button::X2),
        ];

        for action in actions {
            let code = match action {
                Action::SwitchLeft => 0,
                Action::SwitchRight => 1,
                Action::ReplayClick(button) => 2 + button_code(button),
            };
            assert_eq!(decode_action(code), Some(action));
        }
    }

    #[test]
    fn an_unknown_code_decodes_to_nothing() {
        assert_eq!(decode_action(99), None);
    }
}
