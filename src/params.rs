//! Ergonomic system parameters for gameplay code.
//!
//! Everything here is a thin wrapper over queries you could write yourself.
//! The point is that gameplay code usually thinks in *slots* ("did P2 press
//! start?", "buzz everyone") while the ECS thinks in entities.

use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_input::prelude::GamepadButton;

use crate::haptics::{Rumble, RumbleHandle, RumblePattern};
use crate::input::PlayerInput;
use crate::player::{Player, PlayerId};

/// Read-only access to every player's input, addressed by slot.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn pause_menu(players: Players) {
///     if let Some(id) = players.who_just_pressed(GamepadButton::Start) {
///         println!("{id} opened the menu");
///     }
/// }
///
/// fn p1_only(players: Players) {
///     let Some(input) = players.get(PlayerId::P1) else {
///         return;
///     };
///     let _ = input.movement();
/// }
/// ```
#[derive(SystemParam)]
pub struct Players<'w, 's> {
    inputs: Query<'w, 's, (&'static Player, &'static PlayerInput)>,
}

impl Players<'_, '_> {
    /// One player's input, or `None` if that slot is empty.
    pub fn get(&self, id: PlayerId) -> Option<&PlayerInput> {
        self.inputs
            .iter()
            .find(|(player, _)| player.id == id)
            .map(|(_, input)| input)
    }

    /// Every joined player, including ones whose controller is unplugged
    /// (their input reads neutral).
    pub fn iter(&self) -> impl Iterator<Item = (PlayerId, &PlayerInput)> + '_ {
        self.inputs.iter().map(|(player, input)| (player.id, input))
    }

    /// Only players with a controller attached right now.
    pub fn connected(&self) -> impl Iterator<Item = (PlayerId, &PlayerInput)> + '_ {
        self.iter().filter(|(_, input)| input.connected)
    }

    /// How many players have joined.
    pub fn count(&self) -> usize {
        self.inputs.iter().count()
    }

    /// Whether anybody has joined yet.
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// The lowest-numbered player who pressed `button` this frame. Good for
    /// "press A to continue" prompts where any player may answer.
    pub fn who_just_pressed(&self, button: GamepadButton) -> Option<PlayerId> {
        self.iter()
            .filter(|(_, input)| input.just_pressed(button))
            .map(|(id, _)| id)
            .min()
    }

    /// Whether anyone pressed `button` this frame.
    pub fn any_just_pressed(&self, button: GamepadButton) -> bool {
        self.iter().any(|(_, input)| input.just_pressed(button))
    }
}

/// Play rumble on a player, or on everyone, without writing a query.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn on_explosion(mut haptics: Haptics) {
///     haptics.play_all(RumblePattern::explosion(0.9));
/// }
///
/// fn on_p1_hit(mut haptics: Haptics) {
///     haptics.play(PlayerId::P1, RumblePattern::hit(0.8));
/// }
/// ```
///
/// Requests for a player whose controller is unplugged are kept on the player
/// entity and simply never reach any motors, so callers don't have to check.
#[derive(SystemParam)]
pub struct Haptics<'w, 's> {
    rumbles: Query<'w, 's, (&'static Player, &'static mut Rumble)>,
}

impl Haptics<'_, '_> {
    /// This player's haptics channel, for anything the shortcuts below don't
    /// cover.
    pub fn get_mut(&mut self, id: PlayerId) -> Option<&mut Rumble> {
        self.rumbles
            .iter_mut()
            .find(|(player, _)| player.id == id)
            .map(|(_, rumble)| rumble.into_inner())
    }

    /// Play a pattern for one player. `None` if that slot is empty.
    pub fn play(&mut self, id: PlayerId, pattern: RumblePattern) -> Option<RumbleHandle> {
        self.get_mut(id).map(|rumble| rumble.play(pattern))
    }

    /// Play a pattern that outranks (and silences) lower-priority effects.
    pub fn play_with_priority(
        &mut self,
        id: PlayerId,
        pattern: RumblePattern,
        priority: u8,
    ) -> Option<RumbleHandle> {
        self.get_mut(id)
            .map(|rumble| rumble.play_with_priority(pattern, priority))
    }

    /// Play the same pattern for everyone — an explosion, a round ending.
    pub fn play_all(&mut self, pattern: RumblePattern) {
        for (_, mut rumble) in &mut self.rumbles {
            // Cloning a pattern is a refcount bump, not a copy of the keys.
            rumble.play(pattern.clone());
        }
    }

    /// Stop one effect on one player. See [`Rumble::stop`].
    pub fn stop(&mut self, id: PlayerId, handle: RumbleHandle) -> bool {
        self.get_mut(id).is_some_and(|rumble| rumble.stop(handle))
    }

    /// Silence one player.
    pub fn stop_player(&mut self, id: PlayerId) {
        if let Some(rumble) = self.get_mut(id) {
            rumble.stop_all();
        }
    }

    /// Silence everyone — pause menus, cutscenes, the end of a round.
    pub fn stop_all(&mut self) {
        for (_, mut rumble) in &mut self.rumbles {
            rumble.stop_all();
        }
    }
}
