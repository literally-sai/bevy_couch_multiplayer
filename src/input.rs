use bevy_ecs::prelude::*;
use bevy_input::prelude::*;
use bevy_math::Vec2;

/// Every standard button, used to diff a gamepad's state each frame.
///
/// `GamepadButton::Other(_)` is deliberately absent — vendor-specific extras
/// aren't portable enough to build couch co-op on.
pub const ALL_BUTTONS: [GamepadButton; 19] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::North,
    GamepadButton::West,
    GamepadButton::C,
    GamepadButton::Z,
    GamepadButton::LeftTrigger,
    GamepadButton::LeftTrigger2,
    GamepadButton::RightTrigger,
    GamepadButton::RightTrigger2,
    GamepadButton::Select,
    GamepadButton::Start,
    GamepadButton::Mode,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
];

/// Buttons that count for "press any button to join". Triggers are excluded:
/// worn analog triggers rest slightly depressed and would auto-join.
pub const JOIN_BUTTONS: [GamepadButton; 15] = [
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::North,
    GamepadButton::West,
    GamepadButton::C,
    GamepadButton::Z,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger,
    GamepadButton::Start,
    GamepadButton::LeftThumb,
    GamepadButton::RightThumb,
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
];

/// A player's input for this frame, already deadzoned and slot-attributed.
///
/// Read it off the player entity — you never touch gamepad entities yourself:
///
/// ```ignore
/// fn move_players(q: Query<(&Player, &PlayerInput, &mut Transform)>) {
///     for (player, input, mut tf) in &q {
///         tf.translation += input.movement().extend(0.0) * 200.0 * dt;
///     }
/// }
/// ```
#[derive(Component, Debug, Clone, Default)]
pub struct PlayerInput {
    /// False while this player's controller is missing. Everything below is
    /// zeroed in that case, so a disconnect can't leave a player sprinting.
    pub connected: bool,
    /// Digital button state, with the same `pressed` / `just_pressed` /
    /// `just_released` API as Bevy's global input resources.
    pub buttons: ButtonInput<GamepadButton>,
    /// Left stick after radial deadzone, rescaled to 0..1.
    pub left_stick: Vec2,
    /// Right stick after radial deadzone, rescaled to 0..1.
    pub right_stick: Vec2,
    /// D-pad as a vector, so it can stand in for the stick.
    pub dpad: Vec2,
    /// Analog left trigger, deadzoned to 0..1.
    pub left_trigger: f32,
    /// Analog right trigger, deadzoned to 0..1.
    pub right_trigger: f32,
}

impl PlayerInput {
    /// Stick if it's being used, d-pad otherwise. This is what you want for
    /// character movement 95% of the time.
    pub fn movement(&self) -> Vec2 {
        if self.left_stick != Vec2::ZERO {
            self.left_stick
        } else {
            self.dpad
        }
    }

    /// Whether a button is held this frame.
    pub fn pressed(&self, button: GamepadButton) -> bool {
        self.buttons.pressed(button)
    }

    /// Whether a button went down this frame.
    pub fn just_pressed(&self, button: GamepadButton) -> bool {
        self.buttons.just_pressed(button)
    }

    /// Whether a button came up this frame.
    pub fn just_released(&self, button: GamepadButton) -> bool {
        self.buttons.just_released(button)
    }

    /// Whether any of `buttons` went down this frame.
    pub fn any_just_pressed(&self, buttons: impl IntoIterator<Item = GamepadButton>) -> bool {
        buttons.into_iter().any(|b| self.buttons.just_pressed(b))
    }

    /// Wipe to neutral. Held buttons are reported as `just_released` for one
    /// frame so charge-up / hold logic unwinds cleanly instead of hanging.
    pub(crate) fn go_neutral(&mut self) {
        self.connected = false;
        self.buttons.release_all();
        self.left_stick = Vec2::ZERO;
        self.right_stick = Vec2::ZERO;
        self.dpad = Vec2::ZERO;
        self.left_trigger = 0.0;
        self.right_trigger = 0.0;
    }

    /// Refresh from live hardware.
    pub(crate) fn sample(&mut self, gamepad: &Gamepad, stick_dz: f32, trigger_dz: f32) {
        self.connected = true;

        for button in ALL_BUTTONS {
            let down = gamepad.pressed(button);
            if down && !self.buttons.pressed(button) {
                self.buttons.press(button);
            } else if !down && self.buttons.pressed(button) {
                self.buttons.release(button);
            }
        }

        let axis = |a: GamepadAxis| gamepad.get(a).unwrap_or(0.0);
        self.left_stick = radial_deadzone(
            Vec2::new(axis(GamepadAxis::LeftStickX), axis(GamepadAxis::LeftStickY)),
            stick_dz,
        );
        self.right_stick = radial_deadzone(
            Vec2::new(axis(GamepadAxis::RightStickX), axis(GamepadAxis::RightStickY)),
            stick_dz,
        );

        let dpad_x = f32::from(gamepad.pressed(GamepadButton::DPadRight))
            - f32::from(gamepad.pressed(GamepadButton::DPadLeft));
        let dpad_y = f32::from(gamepad.pressed(GamepadButton::DPadUp))
            - f32::from(gamepad.pressed(GamepadButton::DPadDown));
        // Normalised so diagonal d-pad isn't 1.41x faster than cardinal.
        self.dpad = Vec2::new(dpad_x, dpad_y).normalize_or_zero();

        self.left_trigger = trigger(gamepad, GamepadButton::LeftTrigger2, trigger_dz);
        self.right_trigger = trigger(gamepad, GamepadButton::RightTrigger2, trigger_dz);
    }

    /// Age out `just_pressed` / `just_released` before the next sample.
    pub(crate) fn tick(&mut self) {
        self.buttons.clear();
    }
}

fn trigger(gamepad: &Gamepad, button: GamepadButton, deadzone: f32) -> f32 {
    let raw = gamepad.get(button).unwrap_or(0.0);
    if raw <= deadzone {
        0.0
    } else {
        ((raw - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0)
    }
}

/// Circular deadzone with the live zone rescaled back to 0..1.
///
/// Bevy's built-in deadzone is per-axis, which carves a square hole out of the
/// stick: pushing diagonally can clear the deadzone on one axis but not the
/// other, so slow diagonal movement snaps to a cardinal direction.
pub fn radial_deadzone(v: Vec2, deadzone: f32) -> Vec2 {
    let len = v.length();
    if len <= deadzone {
        return Vec2::ZERO;
    }
    if deadzone >= 1.0 {
        return Vec2::ZERO;
    }
    let scaled = ((len - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0);
    v / len * scaled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_ignores_resting_drift() {
        assert_eq!(radial_deadzone(Vec2::new(0.05, 0.05), 0.2), Vec2::ZERO);
    }

    #[test]
    fn deadzone_is_circular_not_square() {
        // Neither axis clears 0.2 on its own, but the stick is clearly pushed
        // diagonally. A per-axis deadzone throws this away.
        let v = radial_deadzone(Vec2::new(0.15, 0.15), 0.2);
        assert!(v.length() > 0.0);
    }

    #[test]
    fn live_zone_reaches_full_range() {
        let v = radial_deadzone(Vec2::new(1.0, 0.0), 0.2);
        assert!((v.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn diagonals_are_not_faster_than_cardinals() {
        let v = radial_deadzone(Vec2::new(1.0, 1.0), 0.1);
        assert!(v.length() <= 1.0 + 1e-5);
    }
}
