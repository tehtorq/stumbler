//! Gamepad input: analogue steering and pedals from whichever pad was used
//! last, plus buttons for the game's actions. Works alongside the keyboard.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};

/// Stick travel ignored around the centre.
const DEADZONE: f32 = 0.1;
/// Steering response past the deadzone: above 1 is gentler near the
/// centre for fine corrections, with full lock still at full travel.
const STEER_CURVE: f32 = 1.4;
/// Trigger travel ignored at rest.
const TRIGGER_DEADZONE: f32 = 0.05;

/// The pad's driving controls this frame.
#[derive(Clone, Copy, Default)]
pub struct PadState {
    /// -1 left to 1 right.
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: bool,
}

/// Buttons that do something other than drive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadAction {
    /// Race from a start line, or race again after finishing (Start).
    Race,
    /// Time trial from a start line (left bumper).
    TimeTrial,
    /// A tour from a start line (right bumper).
    Tour,
    /// The car's setup, at a start line (d-pad left or right).
    Setup,
    /// Rivals' difficulty, at a start line (d-pad up or down).
    Difficulty,
    /// Leave the race (B).
    Abandon,
    /// World map (Back / Select).
    Map,
    /// Back onto the road (Y).
    Reset,
    /// Next paint (X).
    Paint,
}

pub struct Gamepad {
    gilrs: Gilrs,
    active: Option<GamepadId>,
    pub state: PadState,
}

impl Gamepad {
    /// None if gamepads aren't available on this system.
    pub fn new() -> Option<Self> {
        let gilrs = Gilrs::new().ok()?;
        let active = gilrs.gamepads().next().map(|(id, _)| id);
        Some(Self { gilrs, active, state: PadState::default() })
    }

    /// Read what's happened since last time; returns the actions pressed.
    pub fn poll(&mut self) -> Vec<PadAction> {
        let mut actions = Vec::new();
        while let Some(event) = self.gilrs.next_event() {
            if let EventType::Disconnected = event.event {
                if self.active == Some(event.id) {
                    self.active = None;
                }
                continue;
            }
            // Whichever pad was touched last is the one that drives
            self.active = Some(event.id);
            if let EventType::ButtonPressed(button, _) = event.event {
                let action = match button {
                    Button::Start => Some(PadAction::Race),
                    Button::LeftTrigger => Some(PadAction::TimeTrial),
                    Button::RightTrigger => Some(PadAction::Tour),
                    Button::DPadUp | Button::DPadDown => Some(PadAction::Difficulty),
                    Button::DPadLeft | Button::DPadRight => Some(PadAction::Setup),
                    Button::East => Some(PadAction::Abandon),
                    Button::Select => Some(PadAction::Map),
                    Button::North => Some(PadAction::Reset),
                    Button::West => Some(PadAction::Paint),
                    _ => None,
                };
                actions.extend(action);
            }
        }
        self.state = self.active.map(|id| {
            let pad = self.gilrs.gamepad(id);
            let trigger = |b: Button| {
                let v = pad.button_data(b).map_or(0.0, |d| d.value());
                ((v - TRIGGER_DEADZONE) / (1.0 - TRIGGER_DEADZONE)).clamp(0.0, 1.0)
            };
            PadState {
                steer: shape_steer(pad.value(Axis::LeftStickX)),
                throttle: trigger(Button::RightTrigger2),
                brake: trigger(Button::LeftTrigger2),
                handbrake: pad.is_pressed(Button::South),
            }
        }).unwrap_or_default();
        actions
    }
}

/// Stick position to steering: nothing inside the deadzone, then a gentle
/// curve out to full lock.
fn shape_steer(x: f32) -> f32 {
    let past = ((x.abs() - DEADZONE) / (1.0 - DEADZONE)).clamp(0.0, 1.0);
    past.powf(STEER_CURVE) * x.signum()
}

#[cfg(test)]
mod tests {
    use super::shape_steer;

    #[test]
    fn steering_has_a_deadzone_and_reaches_full_lock() {
        assert_eq!(shape_steer(0.05), 0.0);
        assert_eq!(shape_steer(-0.08), 0.0);
        assert!((shape_steer(1.0) - 1.0).abs() < 1e-5);
        assert!((shape_steer(-1.0) + 1.0).abs() < 1e-5);
        // Gentler than linear part way
        assert!(shape_steer(0.5) > 0.0 && shape_steer(0.5) < 0.45);
    }
}
