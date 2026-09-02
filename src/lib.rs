//! Couch co-op gamepad management for Bevy.
//!
//! Bevy gives you gamepads as entities and leaves the rest to you. That's the
//! right primitive, but every local-multiplayer game then rewrites the same
//! awkward layer on top of it: mapping controllers to player slots, letting
//! people join mid-game, surviving a yanked USB cable, and driving rumble
//! without stomping on itself. This crate is that layer.
//!
//! # The model
//!
//! - A **slot** ([`PlayerId`]) is stable for the whole session. P2 stays P2.
//! - A **player entity** is spawned on join and is yours to build on — attach
//!   score, character choice, a sprite. It survives disconnects.
//! - A **gamepad entity** is Bevy's, ephemeral, and you never touch it. It
//!   gets swapped underneath the player when hardware comes and goes.
//!
//! # Quick start
//!
//! ```no_run
//! use bevy::prelude::*;
//! use bevy_couch_multiplayer::prelude::*;
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(DefaultPlugins)
//!         .add_plugins(CouchMultiplayerPlugin::new(4))
//!         .add_systems(Update, move_players)
//!         .run();
//! }
//!
//! fn move_players(time: Res<Time>, mut players: Query<(&PlayerInput, &mut Transform)>) {
//!     for (input, mut transform) in &mut players {
//!         let step = input.movement() * 250.0 * time.delta_secs();
//!         transform.translation += step.extend(0.0);
//!     }
//! }
//! ```
//!
//! Controllers join by pressing Start, each gets a player entity, and
//! [`PlayerInput`] is deadzoned and attributed to the right slot before your
//! `Update` systems run.
//!
//! # Three ways to reach a player
//!
//! Pick whichever fits the system you're writing:
//!
//! ```
//! use bevy::prelude::*;
//! use bevy_couch_multiplayer::prelude::*;
//!
//! // 1. A query, when you want gameplay components alongside the input.
//! fn shoot(players: Query<(&PlayerInput, &Transform)>) {}
//!
//! // 2. `Players`, when you think in slots.
//! fn menu(players: Players) {
//!     if players.any_just_pressed(GamepadButton::Start) {}
//!     let _ = players.get(PlayerId::P2);
//! }
//!
//! // 3. `PlayerRoster`, for lobby UI: who's in, on what, is anyone unplugged.
//! fn hud(roster: Res<PlayerRoster>) {
//!     for slot in roster.iter() {
//!         let _ = (slot.id(), slot.is_connected(), slot.brand().name());
//!     }
//! }
//! ```
//!
//! # Joining and leaving
//!
//! [`PlayerJoined`] arrives in [`PreUpdate`], so a system ordered
//! `.after(CouchSystems::Membership)` can dress the new player before anything
//! in `Update` sees it:
//!
//! ```
//! use bevy::prelude::*;
//! use bevy_couch_multiplayer::prelude::*;
//! # #[derive(Component)] struct Score(u32);
//! # #[derive(Component)] struct TeamColor([f32; 3]);
//!
//! fn on_join(mut joined: MessageReader<PlayerJoined>, mut commands: Commands) {
//!     for event in joined.read() {
//!         commands
//!             .entity(event.player_entity)
//!             .insert((Score(0), TeamColor(event.id.color_rgb())));
//!     }
//! }
//!
//! # let mut app = App::new();
//! app.add_systems(PreUpdate, on_join.after(CouchSystems::Membership));
//! ```
//!
//! Anything you attach survives a disconnect and is despawned with the player
//! when they leave.
//!
//! # Rumble
//!
//! Request haptics for a *player*, never for a gamepad:
//!
//! ```
//! use bevy::prelude::*;
//! use bevy_couch_multiplayer::prelude::*;
//!
//! fn on_explosion(mut haptics: Haptics) {
//!     haptics.play_all(RumblePattern::explosion(0.9));
//! }
//! ```
//!
//! Patterns are envelopes, not flat buzzes, and the driver keeps exactly one
//! live effect per gamepad however many you stack. [`Rumble::play`] hands back
//! a [`RumbleHandle`] so a looping effect can be stopped again without
//! silencing everything else. See [`Rumble`] for the mixing rules.
//!
//! # Scheduling
//!
//! Everything runs in [`PreUpdate`] after Bevy's `InputSystems`, so
//! [`PlayerInput`] is current by `Update`. Haptics flush in [`PostUpdate`].
//! Order against [`CouchSystems`] if you need to slot in between.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod brand;
mod config;
mod haptics;
mod input;
mod join;
mod params;
mod player;
mod roster;

pub use brand::{GamepadBrand, vendor};
pub use config::{CouchConfig, JoinMode, JoinTrigger, LeaveMode, ReconnectPolicy};
pub use haptics::{Rumble, RumbleHandle, RumbleKey, RumblePattern};
pub use input::{ALL_BUTTONS, JOIN_BUTTONS, PlayerInput, radial_deadzone};
pub use join::LeaveHold;
pub use params::{Haptics, Players};
pub use player::{
    DeviceFingerprint, LeaveReason, Player, PlayerDevice, PlayerDisconnected, PlayerId,
    PlayerJoined, PlayerLeft, PlayerReconnected,
};
pub use roster::{
    PlayerRoster, PlayerSlot, any_player_disconnected, any_player_joined, lobby_is_full,
};

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_input::InputSystems;
use bevy_input::prelude::GamepadButton;

/// Everything you normally need.
pub mod prelude {
    pub use crate::{
        CouchConfig, CouchMultiplayerPlugin, CouchSystems, DeviceFingerprint, GamepadBrand,
        Haptics, JoinMode, JoinTrigger, LeaveHold, LeaveMode, LeaveReason, Player, PlayerDevice,
        PlayerDisconnected, PlayerId, PlayerInput, PlayerJoined, PlayerLeft, PlayerReconnected,
        PlayerRoster, PlayerSlot, Players, ReconnectPolicy, Rumble, RumbleHandle, RumbleKey,
        RumblePattern, any_player_disconnected, any_player_joined, lobby_is_full,
    };
}

/// Ordering handles for this crate's work.
///
/// The three [`PreUpdate`] sets run in listed order, after Bevy's
/// `InputSystems`. To react to a join before gameplay sees the new player,
/// order yourself `.after(CouchSystems::Membership)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CouchSystems {
    /// Connection and disconnection handling; slot reservation and recovery.
    Devices,
    /// Join and leave requests; spawning and despawning player entities.
    Membership,
    /// Refreshing every player's [`PlayerInput`].
    Input,
    /// Mixing rumble and talking to the hardware. Runs in [`PostUpdate`].
    Haptics,
}

/// Adds local co-op player management and haptics.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// // Four players, join by pressing any button, hold Select for 2s to drop out.
/// let plugin = CouchMultiplayerPlugin::new(4)
///     .with_join_mode(JoinMode::PressToJoin(JoinTrigger::AnyButton))
///     .with_hold_to_leave(GamepadButton::Select, 2.0);
/// ```
#[derive(Debug, Clone, Default)]
pub struct CouchMultiplayerPlugin {
    /// Starting configuration. Also inserted as a mutable [`CouchConfig`]
    /// resource, so an options menu can change it at runtime.
    pub config: CouchConfig,
}

/// Forwards a [`CouchConfig`] builder method onto the plugin, so both spell
/// setup the same way.
macro_rules! forward_config {
    ($(#[$meta:meta])* $name:ident ( $( $arg:ident : $ty:ty ),* )) => {
        $(#[$meta])*
        pub fn $name(mut self, $($arg: $ty),*) -> Self {
            self.config = self.config.$name($($arg),*);
            self
        }
    };
}

impl CouchMultiplayerPlugin {
    /// A lobby with `max_players` slots and default behaviour: press Start to
    /// join, hold Select to leave, slots held forever through disconnects.
    pub fn new(max_players: u8) -> Self {
        Self {
            config: CouchConfig::default().with_max_players(max_players),
        }
    }

    /// Start from a fully specified config.
    pub fn from_config(config: CouchConfig) -> Self {
        Self { config }
    }

    forward_config!(
        /// Slot count, clamped to at least 1.
        with_max_players(max: u8)
    );
    forward_config!(
        /// See [`JoinMode`].
        with_join_mode(mode: JoinMode)
    );
    forward_config!(
        /// See [`LeaveMode`].
        with_leave_mode(mode: LeaveMode)
    );
    forward_config!(
        /// Shorthand for [`LeaveMode::HoldButton`].
        with_hold_to_leave(button: GamepadButton, secs: f32)
    );
    forward_config!(
        /// See [`ReconnectPolicy`].
        with_reconnect(policy: ReconnectPolicy)
    );
    forward_config!(
        /// Radial stick deadzone, applied on top of Bevy's per-axis one.
        with_stick_deadzone(deadzone: f32)
    );
    forward_config!(
        /// Below this, a trigger reads as 0.0.
        with_trigger_deadzone(deadzone: f32)
    );
    forward_config!(
        /// Master rumble multiplier, clamped to 0..1.
        with_rumble_scale(scale: f32)
    );
    forward_config!(
        /// Concurrent rumble effects kept per player.
        with_max_rumble_tracks(max: usize)
    );
    forward_config!(
        /// Start with rumble off. Players can turn it back on through the
        /// [`CouchConfig`] resource.
        without_rumble()
    );
}

impl From<CouchConfig> for CouchMultiplayerPlugin {
    fn from(config: CouchConfig) -> Self {
        Self::from_config(config)
    }
}

impl Plugin for CouchMultiplayerPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();

        if matches!(config.join_mode, JoinMode::Automatic)
            && matches!(config.leave_mode, LeaveMode::HoldButton { .. })
        {
            log::warn!(
                "CouchMultiplayerPlugin: JoinMode::Automatic re-joins a controller the instant it \
                 leaves, so LeaveMode::HoldButton will look like it does nothing"
            );
        }

        let roster = PlayerRoster::new(config.max_players);

        app.insert_resource(config)
            .insert_resource(roster)
            // Buffered messages — Bevy 0.17+ calls these messages, not events.
            .add_message::<PlayerJoined>()
            .add_message::<PlayerLeft>()
            .add_message::<PlayerDisconnected>()
            .add_message::<PlayerReconnected>()
            // Normally `GamepadPlugin` registers this. Doing it here too (the
            // call is idempotent) keeps our systems runnable on a headless app
            // with no gamepad backend, where a missing message queue would
            // otherwise skip them entirely.
            .add_message::<bevy_input::gamepad::GamepadRumbleRequest>()
            // Bevy spawns and updates gamepad entities inside `InputSystems`,
            // so everything here has to come after it. Sync points between
            // these sets are inserted for us, which is what lets `Input` see
            // players that `Membership` spawned on the same frame.
            .configure_sets(
                PreUpdate,
                (
                    CouchSystems::Devices,
                    CouchSystems::Membership,
                    CouchSystems::Input,
                )
                    .chain()
                    .after(InputSystems),
            )
            .add_systems(
                PreUpdate,
                (
                    join::sync_devices.in_set(CouchSystems::Devices),
                    join::process_membership.in_set(CouchSystems::Membership),
                    join::read_player_input.in_set(CouchSystems::Input),
                ),
            )
            // Late, so a rumble requested anywhere in `Update` reaches the
            // motors on the same frame it was asked for.
            .add_systems(
                PostUpdate,
                haptics::drive_haptics.in_set(CouchSystems::Haptics),
            );
    }
}
