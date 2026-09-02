//! Four-player couch demo.
//!
//! ```sh
//! cargo run --example couch
//! ```
//!
//! Plug in some controllers and press any button to join. Each player gets a
//! square. Try yanking a controller out mid-game: the square freezes, the slot
//! is held open, and plugging the same pad back in returns it to the same
//! player.
//!
//! - left stick / d-pad — move
//! - South (A / ✕ / B) — bump, with a sharp rumble
//! - East (B / ○ / A) — explosion rumble
//! - North (Y / △ / X) — toggle a looping engine hum
//! - right trigger — a high-priority buzz that silences the rest
//! - hold Select — leave

use bevy::prelude::*;
use bevy::text::FontSize;
use bevy_couch_multiplayer::prelude::*;

const SPEED: f32 = 320.0;
const HALF_BOUNDS: Vec2 = Vec2::new(560.0, 320.0);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(
            CouchMultiplayerPlugin::new(4)
                .with_join_mode(JoinMode::PressToJoin(JoinTrigger::AnyButton)),
        )
        .add_systems(Startup, setup)
        // In `PreUpdate` after `Membership`, so a joining player has a body
        // before any `Update` system looks at them.
        .add_systems(
            PreUpdate,
            give_new_players_a_body.after(CouchSystems::Membership),
        )
        .add_systems(
            Update,
            (
                move_players,
                rumble_on_input,
                toggle_engine_hum,
                show_disconnects,
                update_hud,
            ),
        )
        .run();
}

#[derive(Component)]
struct Hud;

/// A looping rumble we started, kept so we can stop that one specifically.
#[derive(Component)]
struct EngineHum(RumbleHandle);

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        Hud,
        Text::new("Press any button on a controller to join"),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
    ));
}

/// Player entities are spawned by the plugin; gameplay components are ours to
/// add. Anything attached here survives a controller disconnect and is cleaned
/// up automatically when the player leaves.
fn give_new_players_a_body(mut joined: MessageReader<PlayerJoined>, mut commands: Commands) {
    for event in joined.read() {
        let [r, g, b] = event.id.color_rgb();
        let slot = event.id.index() as f32;

        commands.entity(event.player_entity).insert((
            Sprite {
                color: Color::srgb(r, g, b),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_xyz(-300.0 + slot * 200.0, 0.0, 0.0),
        ));
    }
}

fn move_players(time: Res<Time>, mut players: Query<(&PlayerInput, &mut Transform)>) {
    for (input, mut transform) in &mut players {
        let delta = input.movement() * SPEED * time.delta_secs();
        let position = (transform.translation.truncate() + delta).clamp(-HALF_BOUNDS, HALF_BOUNDS);
        transform.translation = position.extend(0.0);
    }
}

/// Haptics are requested per player, so this keeps working across a disconnect
/// and reconnect without touching a gamepad entity.
fn rumble_on_input(mut players: Query<(&PlayerInput, &mut Rumble)>) {
    for (input, mut rumble) in &mut players {
        if input.just_pressed(GamepadButton::South) {
            rumble.play(RumblePattern::hit(0.8));
        }
        if input.just_pressed(GamepadButton::East) {
            rumble.play(RumblePattern::explosion(0.9));
        }
        // Priority 5 outranks the effects above, so a full trigger pull
        // silences incidental bumps rather than muddying them together.
        if input.right_trigger > 0.5 {
            rumble.play_with_priority(RumblePattern::constant(0.0, input.right_trigger, 0.1), 5);
        }
    }
}

/// A looping pattern runs until you stop it, which is what the handle is for.
fn toggle_engine_hum(
    mut commands: Commands,
    mut players: Query<(Entity, &PlayerInput, &mut Rumble, Option<&EngineHum>)>,
) {
    for (entity, input, mut rumble, hum) in &mut players {
        if !input.just_pressed(GamepadButton::North) {
            continue;
        }
        match hum {
            Some(hum) => {
                rumble.stop(hum.0);
                commands.entity(entity).remove::<EngineHum>();
            }
            None => {
                let handle = rumble.play(RumblePattern::texture(0.3));
                commands.entity(entity).insert(EngineHum(handle));
            }
        }
    }
}

fn show_disconnects(
    mut disconnected: MessageReader<PlayerDisconnected>,
    mut reconnected: MessageReader<PlayerReconnected>,
    mut sprites: Query<&mut Sprite>,
) {
    for event in disconnected.read() {
        if let Ok(mut sprite) = sprites.get_mut(event.player_entity) {
            sprite.color = sprite.color.with_alpha(0.25);
        }
    }
    for event in reconnected.read() {
        if let Ok(mut sprite) = sprites.get_mut(event.player_entity) {
            sprite.color = sprite.color.with_alpha(1.0);
        }
    }
}

fn update_hud(
    roster: Res<PlayerRoster>,
    holds: Query<(&Player, &LeaveHold)>,
    mut hud: Query<&mut Text, With<Hud>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let mut lines = vec![format!(
        "{}/{} players — {} unassigned pad(s)",
        roster.player_count(),
        roster.capacity(),
        roster.unassigned_gamepads().len()
    )];

    for slot in roster.iter() {
        let brand = slot.brand();
        let status = if slot.is_connected() {
            format!(
                "{} — bump: {}",
                brand.name(),
                brand.button_label(GamepadButton::South)
            )
        } else {
            "controller disconnected — plug it back in".to_string()
        };

        // The hold lives on the player entity, so a leave ring is one lookup.
        let leaving = holds
            .iter()
            .find(|(player, _)| player.id == slot.id())
            .map(|(_, hold)| hold.progress())
            .filter(|progress| *progress > 0.0)
            .map(|progress| format!("  leaving… {:.0}%", progress * 100.0))
            .unwrap_or_default();

        lines.push(format!("{}: {status}{leaving}", slot.id()));
    }

    if !roster.is_full() {
        lines.push("Press any button on another controller to join".to_string());
    }

    **text = lines.join("\n");
}
