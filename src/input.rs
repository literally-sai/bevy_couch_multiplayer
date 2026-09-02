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

/// Buttons that count for "press any button to join".
///
/// Three deliberate omissions:
///
/// - `LeftTrigger2` / `RightTrigger2`, the analog triggers — worn ones rest
///   slightly depressed and would auto-join a pad nobody touched. The bumpers
///   (`LeftTrigger` / `RightTrigger`) are digital and are included.
/// - `Select`, which is the default *leave* button.
/// - `Mode`, the guide/home button, which platform overlays claim.
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
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn move_players(time: Res<Time>, mut players: Query<(&PlayerInput, &mut Transform)>) {
///     for (input, mut transform) in &mut players {
///         let step = input.movement() * 250.0 * time.delta_secs();
///         transform.translation += step.extend(0.0);
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
    /// D-pad as a unit vector, so it can stand in for the stick.
    pub dpad: Vec2,
    /// Analog left trigger, deadzoned to 0..1.
    pub left_trigger: f32,
    /// Analog right trigger, deadzoned to 0..1.
    pub right_trigger: f32,
}

impl PlayerInput {
    /// Left stick if it's being used, d-pad otherwise. This is what you want
    /// for character movement 95% of the time.
    pub fn movement(&self) -> Vec2 {
        if self.left_stick != Vec2::ZERO {
            self.left_stick
        } else {
            self.dpad
        }
    }

    /// Right stick, for aiming or a free camera.
    pub fn look(&self) -> Vec2 {
        self.right_stick
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

    /// Whether any of `buttons` is held.
    pub fn any_pressed(&self, buttons: impl IntoIterator<Item = GamepadButton>) -> bool {
        buttons.into_iter().any(|b| self.buttons.pressed(b))
    }

    /// Whether any of `buttons` went down this frame.
    pub fn any_just_pressed(&self, buttons: impl IntoIterator<Item = GamepadButton>) -> bool {
        buttons.into_iter().any(|b| self.buttons.just_pressed(b))
    }

    /// Nothing pushed and nothing held — useful for attract modes and idle
    /// timers. Always true while disconnected.
    pub fn is_neutral(&self) -> bool {
        self.movement() == Vec2::ZERO
            && self.right_stick == Vec2::ZERO
            && self.left_trigger == 0.0
            && self.right_trigger == 0.0
            && self.buttons.get_pressed().next().is_none()
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
/// other, so slow diagonal movement snaps to a cardinal direction. This also
/// clamps the result to the unit circle, so a stick that reports past its
/// corners can't hand you a 1.4x speed boost on diagonals.
pub fn radial_deadzone(v: Vec2, deadzone: f32) -> Vec2 {
    if deadzone >= 1.0 {
        return Vec2::ZERO;
    }
    let len = v.length();
    // The NaN guard matters: a garbage axis reading would otherwise propagate
    // into a player's position and never come back.
    if len.is_nan() || len <= deadzone {
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
    fn deadzone_keeps_the_direction_it_was_given() {
        let v = radial_deadzone(Vec2::new(0.6, 0.8), 0.2);
        assert!((v.normalize().x - 0.6).abs() < 1e-5);
        assert!((v.normalize().y - 0.8).abs() < 1e-5);
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

    #[test]
    fn a_full_deadzone_kills_everything() {
        assert_eq!(radial_deadzone(Vec2::new(1.0, 0.0), 1.0), Vec2::ZERO);
    }

    #[test]
    fn nonsense_input_does_not_leak_nan() {
        assert_eq!(radial_deadzone(Vec2::new(f32::NAN, 0.0), 0.1), Vec2::ZERO);
        assert_eq!(radial_deadzone(Vec2::ZERO, 0.0), Vec2::ZERO);
    }

    #[test]
    fn movement_falls_back_to_the_dpad() {
        let mut input = PlayerInput {
            dpad: Vec2::new(1.0, 0.0),
            ..Default::default()
        };
        assert_eq!(input.movement(), Vec2::new(1.0, 0.0));

        input.left_stick = Vec2::new(0.0, 0.5);
        assert_eq!(input.movement(), Vec2::new(0.0, 0.5), "stick wins when used");
    }

    #[test]
    fn going_neutral_reports_releases_once() {
        let mut input = PlayerInput {
            connected: true,
            left_stick: Vec2::new(1.0, 1.0),
            ..Default::default()
        };
        input.buttons.press(GamepadButton::South);

        input.go_neutral();

        assert!(!input.connected);
        assert!(!input.pressed(GamepadButton::South));
        assert!(
            input.just_released(GamepadButton::South),
            "hold-to-charge logic needs the release to unwind"
        );
        assert_eq!(input.movement(), Vec2::ZERO);
        assert!(input.is_neutral());
    }
}
