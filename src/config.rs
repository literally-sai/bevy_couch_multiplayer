use bevy_ecs::prelude::*;
use bevy_input::prelude::GamepadButton;

/// How an unassigned gamepad becomes a player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinMode {
    /// Any gamepad that connects is immediately given the lowest free slot.
    ///
    /// Good for games that start straight into gameplay.
    Automatic,
    /// A gamepad joins when the given button is pressed on it.
    ///
    /// This is what most couch co-op games want: a controller can be plugged
    /// in and sit idle until its owner actually presses something.
    PressToJoin(JoinTrigger),
    /// Nothing joins on its own. Drive it yourself with
    /// [`PlayerRoster::request_join`](crate::PlayerRoster::request_join).
    Manual,
}

impl Default for JoinMode {
    fn default() -> Self {
        Self::PressToJoin(JoinTrigger::default())
    }
}

/// Which button press counts as "I want in".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinTrigger {
    /// Any face/shoulder/d-pad button. Triggers are excluded, since resting
    /// triggers on some pads report spurious presses.
    AnyButton,
    /// One specific button.
    Button(GamepadButton),
}

impl Default for JoinTrigger {
    fn default() -> Self {
        Self::Button(GamepadButton::Start)
    }
}

/// How a player drops out.
#[derive(Debug, Clone, PartialEq)]
pub enum LeaveMode {
    /// Players never leave on their own; call
    /// [`PlayerRoster::request_leave`](crate::PlayerRoster::request_leave).
    Manual,
    /// Hold a button for `secs` to drop out. A hold (rather than a tap) avoids
    /// someone rage-quitting the party by fat-fingering Select.
    HoldButton { button: GamepadButton, secs: f32 },
}

impl Default for LeaveMode {
    fn default() -> Self {
        Self::HoldButton {
            button: GamepadButton::Select,
            secs: 1.5,
        }
    }
}

/// What happens when a gamepad drops off mid-game.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconnectPolicy {
    /// Keep the player's slot (and their player entity) reserved this long
    /// after a disconnect. `None` reserves it forever.
    pub grace_secs: Option<f32>,
    /// Automatically hand a controller back to the slot whose old device had
    /// the same vendor/product id and name. Costs the player nothing — they
    /// plug back in and they're playing again.
    pub match_device: bool,
    /// Let a *different* controller take over the longest-waiting slot when
    /// it joins, instead of opening a fresh one beside it.
    ///
    /// This is what makes "my battery died, pass me the spare pad" work: the
    /// borrowed controller presses to join and restores P2, score and all.
    /// The trade-off is that a genuinely new player who joins while someone
    /// is disconnected inherits that slot rather than getting their own. Turn
    /// it off if a returning player should only be restored by their original
    /// hardware.
    pub adopt_any: bool,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            grace_secs: None,
            match_device: true,
            adopt_any: true,
        }
    }
}

/// Plugin-wide settings. Inserted as a resource; mutate it at runtime to
/// change behaviour (e.g. flip `rumble_enabled` from an options menu).
#[derive(Resource, Debug, Clone)]
pub struct CouchConfig {
    /// Number of slots. Slots are stable: player 2 stays player 2.
    pub max_players: u8,
    /// How an unassigned gamepad becomes a player.
    pub join_mode: JoinMode,
    /// How a player gives up their slot.
    pub leave_mode: LeaveMode,
    /// What happens when a controller drops mid-game.
    pub reconnect: ReconnectPolicy,
    /// Radial deadzone applied on top of Bevy's per-axis
    /// [`GamepadSettings`](bevy_input::gamepad::GamepadSettings). Per-axis
    /// deadzones make diagonals feel wrong; this fixes that.
    pub stick_deadzone: f32,
    /// Below this, a trigger reads as 0.0.
    pub trigger_deadzone: f32,
    /// Master rumble switch, for accessibility settings.
    pub rumble_enabled: bool,
    /// Multiplied into every rumble. Expose this as a 0-1 slider.
    pub rumble_scale: f32,
    /// Concurrent rumble effects kept per player. Extra effects evict the
    /// lowest-priority track.
    pub max_rumble_tracks: usize,
}

impl Default for CouchConfig {
    fn default() -> Self {
        Self {
            max_players: 4,
            join_mode: JoinMode::default(),
            leave_mode: LeaveMode::default(),
            reconnect: ReconnectPolicy::default(),
            stick_deadzone: 0.12,
            trigger_deadzone: 0.05,
            rumble_enabled: true,
            rumble_scale: 1.0,
            max_rumble_tracks: 8,
        }
    }
}

impl CouchConfig {
    /// Set the slot count. Clamped to at least 1.
    pub fn with_max_players(mut self, max: u8) -> Self {
        self.max_players = max.max(1);
        self
    }

    /// Set how gamepads join.
    pub fn with_join_mode(mut self, mode: JoinMode) -> Self {
        self.join_mode = mode;
        self
    }

    /// Set how players drop out.
    pub fn with_leave_mode(mut self, mode: LeaveMode) -> Self {
        self.leave_mode = mode;
        self
    }

    /// Set the disconnect/reconnect behaviour.
    pub fn with_reconnect(mut self, policy: ReconnectPolicy) -> Self {
        self.reconnect = policy;
        self
    }
}
