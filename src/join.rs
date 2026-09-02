use bevy_ecs::name::Name;
use bevy_ecs::prelude::*;
use bevy_input::gamepad::{Gamepad, GamepadConnectionEvent, GamepadRumbleRequest};
use bevy_time::Time;

use crate::config::{CouchConfig, JoinMode, JoinTrigger, LeaveMode};
use crate::haptics::Rumble;
use crate::input::{JOIN_BUTTONS, PlayerInput};
use crate::player::{
    DeviceFingerprint, LeaveReason, Player, PlayerDevice, PlayerDisconnected, PlayerId,
    PlayerJoined, PlayerLeft, PlayerReconnected,
};
use crate::roster::PlayerRoster;

/// Every live gamepad, with the name Bevy attaches to it.
type Gamepads<'w, 's> = Query<'w, 's, (Entity, &'static Gamepad, Option<&'static Name>)>;

/// Tracks how long a player has held the leave button.
///
/// [`progress`](Self::progress) is 0..1 — wire it to a filling ring so players
/// can see the hold register instead of guessing:
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn show_leave_rings(players: Query<(&Player, &LeaveHold)>) {
///     for (player, hold) in &players {
///         if hold.is_holding() {
///             println!("{} is {:.0}% of the way out", player.id, hold.progress() * 100.0);
///         }
///     }
/// }
/// ```
#[derive(Component, Debug, Default)]
pub struct LeaveHold {
    /// Seconds the leave button has been held.
    pub held_secs: f32,
    /// Kept in sync with [`LeaveMode`] by the plugin, so UI code doesn't have
    /// to reach for the config to draw a progress ring.
    required_secs: f32,
}

impl LeaveHold {
    /// How long the button has to be held, per the current
    /// [`CouchConfig`]. Zero when [`LeaveMode::Manual`] is in effect.
    pub fn required_secs(&self) -> f32 {
        self.required_secs
    }

    /// Whether the leave button is down right now.
    pub fn is_holding(&self) -> bool {
        self.held_secs > 0.0
    }

    /// Hold progress in 0..1, for a filling UI indicator.
    pub fn progress(&self) -> f32 {
        if self.required_secs <= 0.0 {
            return if self.is_holding() { 1.0 } else { 0.0 };
        }
        (self.held_secs / self.required_secs).clamp(0.0, 1.0)
    }
}

fn fingerprint_of(gamepad: Entity, gamepads: &Gamepads) -> DeviceFingerprint {
    match gamepads.get(gamepad) {
        Ok((_, pad, name)) => DeviceFingerprint {
            name: name.map(|n| n.as_str().to_string()),
            vendor_id: pad.vendor_id(),
            product_id: pad.product_id(),
        },
        Err(_) => DeviceFingerprint::default(),
    }
}

/// Keeps slots pointed at live hardware.
///
/// A disconnect never drops the player: the slot is held, their input goes
/// neutral, and a [`PlayerDisconnected`] message goes out so you can pause.
pub(crate) fn sync_devices(
    mut connections: MessageReader<GamepadConnectionEvent>,
    mut roster: ResMut<PlayerRoster>,
    config: Res<CouchConfig>,
    time: Res<Time>,
    gamepads: Gamepads,
    mut players: Query<(&mut PlayerDevice, &mut PlayerInput)>,
    mut disconnected: MessageWriter<PlayerDisconnected>,
    mut reconnected: MessageWriter<PlayerReconnected>,
) {
    let now = time.elapsed_secs_f64();

    let dropped: Vec<Entity> = connections
        .read()
        .filter(|event| event.disconnected())
        .map(|event| event.gamepad)
        .collect();

    // Cross-checked against the query rather than trusting messages alone: a
    // gamepad entity can also go away without one, and a message missed while
    // this system was skipped would otherwise strand a player on dead
    // hardware forever.
    let assigned: Vec<(PlayerId, Entity)> = roster.assigned().collect();
    for (id, gamepad) in assigned {
        if !dropped.contains(&gamepad) && gamepads.contains(gamepad) {
            continue;
        }
        let Some(player_entity) = roster.mark_disconnected(id, now) else {
            continue;
        };
        if let Ok((mut device, mut input)) = players.get_mut(player_entity) {
            *device = PlayerDevice::Missing;
            // Without this the player keeps holding whatever direction they
            // were pushing when the cable came out.
            input.go_neutral();
        }
        log::warn!("{id} lost their controller; slot reserved");
        disconnected.write(PlayerDisconnected { id, player_entity });
    }

    // Rebuilt from the world every frame, so controllers plugged in before
    // the app started are picked up the same as ones plugged in later.
    let mut free: Vec<Entity> = gamepads
        .iter()
        .map(|(entity, _, _)| entity)
        .filter(|entity| roster.player_of_gamepad(*entity).is_none())
        .collect();

    if config.reconnect.match_device {
        free.retain(|gamepad| {
            let fingerprint = fingerprint_of(*gamepad, &gamepads);
            if !fingerprint.is_known() {
                // Nothing to match on; let it join as a new player instead of
                // handing it the first slot that happens to be waiting.
                return true;
            }
            let Some(id) = roster.waiting_slot_matching(&fingerprint) else {
                return true;
            };
            let Some(player_entity) = roster.attach_device(id, *gamepad, fingerprint) else {
                return true;
            };
            if let Ok((mut device, _)) = players.get_mut(player_entity) {
                *device = PlayerDevice::Gamepad(*gamepad);
            }
            log::info!("{id} reconnected on their own controller");
            reconnected.write(PlayerReconnected {
                id,
                player_entity,
                gamepad: *gamepad,
                same_device: true,
            });
            false
        });
    }

    roster.set_unassigned(free);
}

/// Applies join and leave rules, spawning and despawning player entities.
pub(crate) fn process_membership(
    mut commands: Commands,
    mut roster: ResMut<PlayerRoster>,
    config: Res<CouchConfig>,
    time: Res<Time>,
    gamepads: Gamepads,
    mut holds: Query<(&Player, &PlayerInput, &mut LeaveHold)>,
    mut devices: Query<&mut PlayerDevice>,
    mut joined: MessageWriter<PlayerJoined>,
    mut left: MessageWriter<PlayerLeft>,
    mut reconnected: MessageWriter<PlayerReconnected>,
    mut rumble_requests: MessageWriter<GamepadRumbleRequest>,
) {
    // An options menu can change the lobby size mid-session.
    if config.is_changed() && roster.resize(config.max_players) {
        log::info!("lobby resized to {} slots", roster.capacity());
    }

    queue_join_requests(&mut roster, &config, &gamepads);
    admit_players(
        &mut commands,
        &mut roster,
        &config,
        &gamepads,
        &mut devices,
        &mut joined,
        &mut reconnected,
    );
    track_leave_holds(&mut roster, &config, &time, &mut holds);

    if let Some(grace) = config.reconnect.grace_secs {
        for id in roster.expired_slots(grace, time.elapsed_secs_f64()) {
            roster.queue_leave(id, LeaveReason::TimedOut);
        }
    }

    let leaving = roster.take_leave_requests();
    for (id, reason) in leaving {
        let Some((player_entity, device)) = roster.vacate(id) else {
            continue;
        };

        left.write(PlayerLeft {
            id,
            player_entity,
            reason,
        });
        log::info!("{id} left ({reason:?})");
        commands.entity(player_entity).despawn();

        if let Some(gamepad) = device.gamepad() {
            if gamepads.contains(gamepad) {
                // Their `Rumble` is about to be despawned, so nothing would
                // ever tell the motors to stop.
                rumble_requests.write(GamepadRumbleRequest::Stop { gamepad });
                // A still-plugged-in pad goes back in the pool so it can rejoin.
                roster.release_unassigned(gamepad);
            }
        }
    }
}

/// Turns queued join requests into players, restoring waiting slots first.
fn admit_players(
    commands: &mut Commands,
    roster: &mut PlayerRoster,
    config: &CouchConfig,
    gamepads: &Gamepads,
    devices: &mut Query<&mut PlayerDevice>,
    joined: &mut MessageWriter<PlayerJoined>,
    reconnected: &mut MessageWriter<PlayerReconnected>,
) {
    for gamepad in roster.take_join_requests() {
        if roster.player_of_gamepad(gamepad).is_some() {
            continue;
        }
        let fingerprint = fingerprint_of(gamepad, gamepads);

        // Someone still waiting on dead hardware gets first refusal, so a
        // borrowed controller restores that player — with their score and
        // whatever else you hung off their entity — instead of opening a
        // second slot beside the one they're still holding.
        let waiting = if config.reconnect.adopt_any {
            roster.longest_waiting_slot()
        } else {
            None
        };

        if let Some(id) = waiting {
            // Cloned rather than moved: if the slot turns out not to be
            // adoptable we fall through to a fresh join, which needs it.
            if let Some(player_entity) = roster.attach_device(id, gamepad, fingerprint.clone()) {
                if let Ok(mut device) = devices.get_mut(player_entity) {
                    *device = PlayerDevice::Gamepad(gamepad);
                }
                roster.claim_unassigned(gamepad);
                log::info!("{id} came back on a different controller");
                reconnected.write(PlayerReconnected {
                    id,
                    player_entity,
                    gamepad,
                    same_device: false,
                });
                continue;
            }
        }

        let Some(id) = roster.first_free_slot() else {
            log::info!("gamepad {gamepad} tried to join but every slot is taken");
            continue;
        };

        let player_entity = commands
            .spawn((
                Player { id },
                PlayerDevice::Gamepad(gamepad),
                PlayerInput::default(),
                Rumble::default(),
                LeaveHold::default(),
                Name::new(format!("Player {}", id.number())),
            ))
            .id();

        roster.occupy(id, player_entity, gamepad, fingerprint);
        roster.claim_unassigned(gamepad);
        log::info!("{id} joined on gamepad {gamepad}");
        joined.write(PlayerJoined {
            id,
            player_entity,
            gamepad,
        });
    }
}

/// Advances the hold-to-leave timer and queues anyone who completed it.
fn track_leave_holds(
    roster: &mut PlayerRoster,
    config: &CouchConfig,
    time: &Time,
    holds: &mut Query<(&Player, &PlayerInput, &mut LeaveHold)>,
) {
    let required = config.leave_hold_secs();
    let button = match config.leave_mode {
        LeaveMode::HoldButton { button, .. } => Some(button),
        LeaveMode::Manual => None,
    };
    let dt = time.delta_secs();

    for (player, input, mut hold) in holds.iter_mut() {
        let holding = button.is_some_and(|b| input.connected && input.pressed(b));

        if !holding {
            // Touched only when it would actually change, to keep change
            // detection quiet for the common idle case.
            if hold.held_secs != 0.0 || hold.required_secs != required {
                hold.held_secs = 0.0;
                hold.required_secs = required;
            }
            continue;
        }

        hold.held_secs += dt;
        hold.required_secs = required;
        if hold.held_secs >= required {
            hold.held_secs = 0.0;
            roster.queue_leave(player.id, LeaveReason::Requested);
        }
    }
}

fn queue_join_requests(roster: &mut PlayerRoster, config: &CouchConfig, gamepads: &Gamepads) {
    let candidates = roster.unassigned_gamepads().to_vec();

    match &config.join_mode {
        JoinMode::Automatic => {
            for gamepad in candidates {
                roster.request_join(gamepad);
            }
        }
        JoinMode::PressToJoin(trigger) => {
            for gamepad in candidates {
                let Ok((_, pad, _)) = gamepads.get(gamepad) else {
                    continue;
                };
                let wants_in = match trigger {
                    JoinTrigger::AnyButton => JOIN_BUTTONS.iter().any(|b| pad.just_pressed(*b)),
                    JoinTrigger::Button(button) => pad.just_pressed(*button),
                    JoinTrigger::AnyOf(buttons) => buttons.iter().any(|b| pad.just_pressed(*b)),
                };
                if wants_in {
                    roster.request_join(gamepad);
                }
            }
        }
        JoinMode::Manual => {}
    }
}

/// Refreshes every player's [`PlayerInput`] from their gamepad.
pub(crate) fn read_player_input(
    config: Res<CouchConfig>,
    gamepads: Query<&Gamepad>,
    mut players: Query<(&PlayerDevice, &mut PlayerInput)>,
) {
    for (device, mut input) in &mut players {
        input.tick();

        match device.gamepad().and_then(|e| gamepads.get(e).ok()) {
            Some(pad) => input.sample(pad, config.stick_deadzone, config.trigger_deadzone),
            None => {
                if input.connected {
                    input.go_neutral();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(held: f32, required: f32) -> LeaveHold {
        LeaveHold {
            held_secs: held,
            required_secs: required,
        }
    }

    #[test]
    fn progress_fills_over_the_hold() {
        assert_eq!(hold(0.0, 2.0).progress(), 0.0);
        assert_eq!(hold(1.0, 2.0).progress(), 0.5);
        assert_eq!(hold(9.0, 2.0).progress(), 1.0, "clamped");
    }

    #[test]
    fn an_idle_ring_is_empty_even_without_a_hold_configured() {
        assert_eq!(hold(0.0, 0.0).progress(), 0.0);
        assert!(!hold(0.0, 0.0).is_holding());
    }
}
