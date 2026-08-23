//! Gesture recognition.
//!
//! This module is deliberately free of Win32 calls, I/O and clock reads: every input the
//! machine needs is handed to it. That keeps the only interesting logic in the project
//! testable without a physical mouse. See `CLAUDE.md` for the rule.

/// A mouse button that can act as the gesture trigger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    /// The wheel button. Present on essentially every mouse, hence the default.
    Middle,
    Right,
    /// First side button, usually "back".
    X1,
    /// Second side button, usually "forward".
    X2,
}

/// A mouse event, normalised by the platform layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawEvent {
    Down { button: Button, x: i32, y: i32 },
    Up { button: Button, x: i32, y: i32 },
    Move { x: i32, y: i32 },
}

/// Something the utility should carry out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Move to the virtual desktop on the left (`Ctrl+Win+Left`).
    SwitchLeft,
    /// Move to the virtual desktop on the right (`Ctrl+Win+Right`).
    SwitchRight,
    /// Re-inject a real click, because the press turned out not to be a gesture.
    ReplayClick(Button),
}

/// What the hook should tell Windows to do with the event that produced this verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Hide the event from the rest of the system.
    Swallow(Option<Action>),
    /// Let the event reach whatever is under the cursor.
    PassThrough(Option<Action>),
}

impl Verdict {
    /// The action to carry out, if any.
    pub fn action(self) -> Option<Action> {
        match self {
            Verdict::Swallow(action) | Verdict::PassThrough(action) => action,
        }
    }

    /// Whether the originating event must be hidden from other applications.
    pub fn swallows(self) -> bool {
        matches!(self, Verdict::Swallow(_))
    }
}

/// Tunables for the state machine. Mirrors the user facing configuration file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureConfig {
    /// Button that has to be held for a drag to count as a gesture.
    pub trigger: Button,
    /// Horizontal distance, in pixels, that fires one desktop switch.
    pub threshold_px: i32,
    /// Minimum delay between two switches, so the desktop animation can keep up.
    pub repeat_cooldown_ms: u64,
    /// Maximum `|dy| / |dx|` still accepted as a horizontal drag.
    pub max_vertical_ratio: f32,
    /// Swap the direction of travel.
    pub invert: bool,
    /// Re-inject a real click when the trigger is released without a gesture.
    pub replay_click_when_no_gesture: bool,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            trigger: Button::Middle,
            threshold_px: 60,
            repeat_cooldown_ms: 250,
            max_vertical_ratio: 1.0,
            invert: false,
            replay_click_when_no_gesture: true,
        }
    }
}

/// State of an in-progress press of the trigger button.
#[derive(Clone, Copy, Debug)]
struct Armed {
    /// Point the current drag distance is measured from. Reset on every switch, which is
    /// what lets one long drag walk across several desktops.
    anchor_x: i32,
    anchor_y: i32,
    /// Whether this press has already produced at least one switch.
    fired: bool,
    last_fire_ms: u64,
}

/// The gesture state machine.
#[derive(Clone, Debug)]
pub struct Gesture {
    config: GestureConfig,
    armed: Option<Armed>,
}

impl Gesture {
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            armed: None,
        }
    }

    /// Replace the configuration and abandon any press in progress.
    pub fn set_config(&mut self, config: GestureConfig) {
        self.config = config;
        self.reset();
    }

    /// Forget any press in progress, for example when the utility is disabled.
    pub fn reset(&mut self) {
        self.armed = None;
    }

    /// Feed one mouse event and find out what to do with it.
    ///
    /// `now_ms` is any monotonically increasing millisecond counter.
    pub fn on_event(&mut self, event: RawEvent, now_ms: u64) -> Verdict {
        match event {
            RawEvent::Down { button, x, y } if button == self.config.trigger => {
                self.armed = Some(Armed {
                    anchor_x: x,
                    anchor_y: y,
                    fired: false,
                    last_fire_ms: 0,
                });
                // The application under the cursor must not see the press, otherwise a
                // drag would also open a context menu or start an autoscroll.
                Verdict::Swallow(None)
            }

            RawEvent::Up { button, .. } if button == self.config.trigger => {
                match self.armed.take() {
                    // A press we swallowed has to be released by us too, and if it never
                    // became a gesture the click is handed back to the application.
                    Some(armed) => {
                        let replay = !armed.fired && self.config.replay_click_when_no_gesture;
                        Verdict::Swallow(replay.then_some(Action::ReplayClick(button)))
                    }
                    // The press happened before we were listening, so its release is not
                    // ours to swallow.
                    None => Verdict::PassThrough(None),
                }
            }

            // Pointer movement always reaches the system: the cursor has to keep moving
            // while the user drags.
            RawEvent::Move { x, y } => Verdict::PassThrough(self.on_move(x, y, now_ms)),

            _ => Verdict::PassThrough(None),
        }
    }

    fn on_move(&mut self, x: i32, y: i32, now_ms: u64) -> Option<Action> {
        let config = self.config;
        let armed = self.armed.as_mut()?;

        let dx = x - armed.anchor_x;
        let dy = y - armed.anchor_y;

        if dx.abs() < config.threshold_px {
            return None;
        }

        // Reject drags that are more vertical than horizontal, so scrolling-like motions
        // with the wheel button held do not jump desktops.
        if (dy.abs() as f32) > dx.abs() as f32 * config.max_vertical_ratio {
            return None;
        }

        // The first switch of a press is immediate; later ones wait for the desktop
        // animation. The anchor is left alone while the cooldown blocks, so the distance
        // the user has already dragged still counts towards the next switch.
        if armed.fired && now_ms.saturating_sub(armed.last_fire_ms) < config.repeat_cooldown_ms {
            return None;
        }

        armed.anchor_x = x;
        armed.anchor_y = y;
        armed.fired = true;
        armed.last_fire_ms = now_ms;

        let rightwards = (dx > 0) != config.invert;
        Some(if rightwards {
            Action::SwitchRight
        } else {
            Action::SwitchLeft
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gesture() -> Gesture {
        Gesture::new(GestureConfig::default())
    }

    fn down(x: i32) -> RawEvent {
        RawEvent::Down {
            button: Button::Middle,
            x,
            y: 0,
        }
    }

    fn up(x: i32) -> RawEvent {
        RawEvent::Up {
            button: Button::Middle,
            x,
            y: 0,
        }
    }

    fn mv(x: i32) -> RawEvent {
        RawEvent::Move { x, y: 0 }
    }

    #[test]
    fn trigger_press_and_release_are_hidden_from_applications() {
        let mut g = gesture();
        assert!(g.on_event(down(100), 0).swallows());
        assert!(g.on_event(up(100), 10).swallows());
    }

    #[test]
    fn dragging_right_past_the_threshold_switches_right() {
        let mut g = gesture();
        g.on_event(down(100), 0);
        assert_eq!(g.on_event(mv(159), 10).action(), None, "below threshold");
        assert_eq!(
            g.on_event(mv(160), 20).action(),
            Some(Action::SwitchRight),
            "exactly at the threshold"
        );
    }

    #[test]
    fn dragging_left_past_the_threshold_switches_left() {
        let mut g = gesture();
        g.on_event(down(100), 0);
        assert_eq!(g.on_event(mv(40), 10).action(), Some(Action::SwitchLeft));
    }

    #[test]
    fn movement_is_never_swallowed_so_the_cursor_keeps_moving() {
        let mut g = gesture();
        g.on_event(down(100), 0);
        assert!(!g.on_event(mv(300), 10).swallows());
    }

    #[test]
    fn a_click_without_a_drag_is_replayed() {
        let mut g = gesture();
        g.on_event(down(100), 0);
        g.on_event(mv(110), 5);
        assert_eq!(
            g.on_event(up(110), 10).action(),
            Some(Action::ReplayClick(Button::Middle))
        );
    }

    #[test]
    fn a_release_after_a_gesture_is_not_replayed() {
        let mut g = gesture();
        g.on_event(down(100), 0);
        assert!(g.on_event(mv(200), 10).action().is_some());
        assert_eq!(g.on_event(up(200), 20).action(), None);
    }

    #[test]
    fn replaying_can_be_disabled() {
        let mut g = Gesture::new(GestureConfig {
            replay_click_when_no_gesture: false,
            ..GestureConfig::default()
        });
        g.on_event(down(100), 0);
        assert_eq!(g.on_event(up(100), 10).action(), None);
    }

    #[test]
    fn a_long_drag_walks_across_several_desktops() {
        let mut g = gesture();
        g.on_event(down(0), 0);
        // One continuous drag sampled every 60 pixels, one desktop per step once the
        // cooldown has elapsed.
        let mut switches = 0;
        for step in 1..=4 {
            let t = step as u64 * 300;
            if g.on_event(mv(step * 60), t).action() == Some(Action::SwitchRight) {
                switches += 1;
            }
        }
        assert_eq!(switches, 4);
    }

    #[test]
    fn the_cooldown_throttles_repeated_switches() {
        let mut g = gesture();
        g.on_event(down(0), 0);
        assert_eq!(g.on_event(mv(60), 0).action(), Some(Action::SwitchRight));
        // Still inside the 250 ms cooldown, even though the drag went far enough again.
        assert_eq!(g.on_event(mv(120), 100).action(), None);
        assert_eq!(g.on_event(mv(180), 200).action(), None);
        // Cooldown elapsed.
        assert_eq!(g.on_event(mv(240), 260).action(), Some(Action::SwitchRight));
    }

    #[test]
    fn a_drag_blocked_by_the_cooldown_still_counts_afterwards() {
        let mut g = gesture();
        g.on_event(down(0), 0);
        g.on_event(mv(60), 0);
        // The pointer stops moving during the cooldown; the distance already travelled
        // must still fire once the cooldown expires.
        assert_eq!(g.on_event(mv(120), 100).action(), None);
        assert_eq!(g.on_event(mv(120), 400).action(), Some(Action::SwitchRight));
    }

    #[test]
    fn vertical_drags_do_not_switch_desktops() {
        let mut g = gesture();
        g.on_event(
            RawEvent::Down {
                button: Button::Middle,
                x: 0,
                y: 0,
            },
            0,
        );
        assert_eq!(
            g.on_event(RawEvent::Move { x: 80, y: 300 }, 10).action(),
            None
        );
    }

    #[test]
    fn a_mostly_horizontal_drag_still_switches() {
        let mut g = gesture();
        g.on_event(
            RawEvent::Down {
                button: Button::Middle,
                x: 0,
                y: 0,
            },
            0,
        );
        assert_eq!(
            g.on_event(RawEvent::Move { x: 200, y: 30 }, 10).action(),
            Some(Action::SwitchRight)
        );
    }

    #[test]
    fn inverting_swaps_the_direction() {
        let mut g = Gesture::new(GestureConfig {
            invert: true,
            ..GestureConfig::default()
        });
        g.on_event(down(0), 0);
        assert_eq!(g.on_event(mv(100), 10).action(), Some(Action::SwitchLeft));
    }

    #[test]
    fn other_buttons_are_left_alone() {
        let mut g = gesture();
        let event = RawEvent::Down {
            button: Button::X1,
            x: 0,
            y: 0,
        };
        assert_eq!(g.on_event(event, 0), Verdict::PassThrough(None));
    }

    #[test]
    fn moving_without_the_trigger_held_does_nothing() {
        let mut g = gesture();
        assert_eq!(g.on_event(mv(1000), 10), Verdict::PassThrough(None));
    }

    #[test]
    fn a_release_we_never_saw_pressed_is_not_swallowed() {
        // Happens when the utility starts, or is re-enabled, while the button is held.
        let mut g = gesture();
        assert_eq!(g.on_event(up(0), 10), Verdict::PassThrough(None));
    }

    #[test]
    fn a_side_button_can_be_the_trigger() {
        let mut g = Gesture::new(GestureConfig {
            trigger: Button::X2,
            ..GestureConfig::default()
        });
        let press = RawEvent::Down {
            button: Button::X2,
            x: 0,
            y: 0,
        };
        assert!(g.on_event(press, 0).swallows());
        assert_eq!(
            g.on_event(RawEvent::Move { x: 100, y: 0 }, 10).action(),
            Some(Action::SwitchRight)
        );
    }

    #[test]
    fn reset_abandons_a_press_in_progress() {
        let mut g = gesture();
        g.on_event(down(0), 0);
        g.reset();
        assert_eq!(g.on_event(mv(500), 10).action(), None);
    }
}
