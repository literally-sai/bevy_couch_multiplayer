# bevy_couch_multiplayer

Local co-op gamepad management and haptics for [Bevy](https://bevy.org) 0.19.

Bevy hands you gamepads as entities and stops there. Which is the right
primitive, and also the point where every couch co-op game starts rewriting the
same layer: mapping controllers to player slots, letting a fourth person join
mid-game, surviving a yanked USB cable, and driving rumble without stomping on
itself.

```toml
[dependencies]
bevy_couch_multiplayer = "0.1"
```

```rust
use bevy::prelude::*;
use bevy_couch_multiplayer::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(CouchMultiplayerPlugin::new(4))
        .add_systems(Update, move_players)
        .run();
}

fn move_players(time: Res<Time>, mut players: Query<(&PlayerInput, &mut Transform)>) {
    for (input, mut transform) in &mut players {
        transform.translation += (input.movement() * 250.0 * time.delta_secs()).extend(0.0);
    }
}
```

That's the whole integration. Controllers join by pressing Start, each gets a
player entity, and `PlayerInput` is deadzoned and attributed to the right slot
before your `Update` systems run.

## The model

| Thing | Lifetime | Who owns it |
| --- | --- | --- |
| `PlayerId` (slot) | The whole session. P2 stays P2. | This crate |
| Player entity | Join → leave. **Survives disconnects.** | You: attach score, character, sprite |
| Gamepad entity | Whenever hardware is plugged in | Bevy. You never touch it |

The separation is the whole idea. Gameplay code queries player entities and
never learns that a controller was unplugged and replaced.

---

## Cheat sheet

| | |
| --- | --- |
| **Components** on the player entity | `Player`, `PlayerInput`, `PlayerDevice`, `Rumble`, `LeaveHold` |
| **Resources** | `PlayerRoster` (who's playing), `CouchConfig` (live settings) |
| **Messages** | `PlayerJoined`, `PlayerLeft`, `PlayerDisconnected`, `PlayerReconnected` |
| **Run conditions** | `any_player_joined`, `any_player_disconnected` |
| **Ordering** | `CouchSystems::{Devices, Membership, Input, Haptics}` |

---

## Reacting to players joining

The plugin spawns a bare player entity; gameplay components are yours to add.
Anything you attach survives disconnects and is despawned automatically when
the player leaves.

```rust
fn give_new_players_a_body(mut joined: MessageReader<PlayerJoined>, mut commands: Commands) {
    for event in joined.read() {
        let [r, g, b] = event.id.color_rgb(); // built-in per-slot palette

        commands.entity(event.player_entity).insert((
            Sprite {
                color: Color::srgb(r, g, b),
                custom_size: Some(Vec2::splat(56.0)),
                ..default()
            },
            Transform::from_xyz(-300.0 + event.id.0 as f32 * 200.0, 0.0, 0.0),
            Score(0),
        ));
    }
}
```

`PlayerLeft` is written *before* the entity is despawned, so you can still read
components off it: persist a score, drop their inventory on the floor:

```rust
fn save_score(mut left: MessageReader<PlayerLeft>, scores: Query<&Score>, mut hi: ResMut<HighScores>) {
    for event in left.read() {
        if let Ok(score) = scores.get(event.player_entity) {
            hi.record(event.id, score.0);
        }
    }
}
```

## Reading input

Everything comes off the player entity: you never query a gamepad yourself.

```rust
fn player_actions(mut players: Query<(&Player, &PlayerInput, &mut Velocity)>) {
    for (player, input, mut velocity) in &mut players {
        // Stick if it's being pushed, d-pad otherwise. Already deadzoned.
        velocity.0 = input.movement() * 200.0;

        // Same API shape as Bevy's own ButtonInput.
        if input.just_pressed(GamepadButton::South) {
            velocity.0.y = 400.0;
        }
        if input.any_just_pressed([GamepadButton::Start, GamepadButton::Mode]) {
            println!("{player:?} opened the menu");
        }

        // Analog triggers, deadzoned to 0..1.
        let throttle = input.right_trigger;

        // Right stick for twin-stick aiming.
        if input.right_stick != Vec2::ZERO {
            velocity.0 += input.right_stick;
        }

        // False while their controller is missing; every field above is
        // zeroed in that case, so nobody keeps sprinting into a pit.
        if !input.connected {
            continue;
        }
    }
}
```

## Pausing on a disconnect

A dropped controller never drops the player. The slot is reserved, their input
goes neutral: *including released-this-frame events, so charge-up and hold
logic unwinds instead of hanging*: and you get a message.

The one-liner:

```rust
app.add_systems(Update, gameplay.run_if(not(any_player_disconnected)));
```

Or drive a state, so you can show a "P2, reconnect your controller" overlay:

```rust
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum GameState {
    #[default]
    Playing,
    ControllerLost,
}

fn pause_on_disconnect(
    mut disconnected: MessageReader<PlayerDisconnected>,
    mut next: ResMut<NextState<GameState>>,
) {
    for event in disconnected.read() {
        println!("waiting for {}", event.id);
        next.set(GameState::ControllerLost);
    }
}

fn resume_when_everyone_is_back(roster: Res<PlayerRoster>, mut next: ResMut<NextState<GameState>>) {
    if !roster.any_disconnected() {
        next.set(GameState::Playing);
    }
}
```

For the overlay text, the roster remembers what hardware each waiting slot had:

```rust
for id in roster.disconnected_players() {
    let name = roster
        .fingerprint(id)
        .and_then(|device| device.name.clone())
        .unwrap_or_else(|| "controller".into());

    println!("{id}: reconnect your {name}");
}
```

A returning controller is matched by vendor/product id (falling back to name)
and handed straight back to its original slot: P3 doesn't come back as P1. If
a *different* pad joins while someone is waiting, it takes over the
longest-waiting slot instead of opening a new one, which is what makes "my
battery died, pass me the spare" work. Both behaviours are `ReconnectPolicy`
flags.

## Join and leave flows

```rust
// Straight into gameplay: every controller that appears is a player.
CouchMultiplayerPlugin::new(2).with_join_mode(JoinMode::Automatic)

// Any button joins. Idle plugged-in pads stay out until someone presses.
CouchMultiplayerPlugin::new(4).with_join_mode(JoinMode::PressToJoin(JoinTrigger::AnyButton))

// Your own lobby drives it.
CouchMultiplayerPlugin::new(4).with_join_mode(JoinMode::Manual)
```

With `Manual`, ask the roster to admit a specific pad: after your character
select screen says that player is ready, say:

```rust
fn lobby(mut roster: ResMut<PlayerRoster>, gamepads: Query<&Gamepad>, ready: Res<ReadyList>) {
    let waiting = roster.unassigned_gamepads().to_vec();

    for gamepad in waiting {
        let Ok(pad) = gamepads.get(gamepad) else { continue };

        if pad.just_pressed(GamepadButton::Start) && ready.contains(gamepad) {
            roster.request_join(gamepad);
        }
    }
}
```

Leaving is a **hold**, not a tap, so nobody quits the party by fat-fingering
Select. Wire `LeaveHold` to a filling ring so the hold is visible:

```rust
fn show_leave_progress(config: Res<CouchConfig>, mut holds: Query<(&LeaveHold, &mut Node)>) {
    let LeaveMode::HoldButton { secs, .. } = config.leave_mode else { return };

    for (hold, mut node) in &mut holds {
        node.width = Val::Percent(hold.progress(secs) * 100.0);
    }
}
```

You can also drop someone from code: a boss kicking an idle player, a
character-select "back" button:

```rust
roster.request_leave(PlayerId(2));
```

## Haptics

Rumble is requested per *player*, on a `Rumble` component, so it keeps working
across a disconnect and reconnect without touching a gamepad entity.

```rust
fn everyone_feels_the_explosion(mut players: Query<&mut Rumble>) {
    for mut rumble in &mut players {
        rumble.play(RumblePattern::explosion(0.9));
    }
}
```

Just one player, by slot:

```rust
fn hurt(player: PlayerId, roster: Res<PlayerRoster>, mut rumbles: Query<&mut Rumble>) {
    let Some(entity) = roster.player_entity(player) else { return };

    if let Ok(mut rumble) = rumbles.get_mut(entity) {
        rumble.play(RumblePattern::hit(0.7));
    }
}
```

### Patterns, not buzzes

Built in: `hit`, `explosion`, `heartbeat`, `ramp_up`, `texture` (looping) and
`constant`. A shaped envelope reads as *what* happened; a flat one only as
*something* happened. Hand-roll your own from keyframes: here, a stuttering
engine misfire:

```rust
let misfire = RumblePattern::new(vec![
    RumbleKey::new(0.00, 1.0, 0.0), // at 0.00s: strong motor full, weak silent
    RumbleKey::new(0.06, 0.0, 0.0),
    RumbleKey::new(0.12, 0.8, 0.0),
    RumbleKey::new(0.18, 0.0, 0.0),
]);
```

Intensity is interpolated between keys. The strong (low-frequency) motor is the
heavy thumping one; the weak (high-frequency) motor is buzzy and detailed.

Looping patterns run until you stop them:

```rust
fn engine_hum(mut players: Query<(&PlayerInput, &mut Rumble)>) {
    for (input, mut rumble) in &mut players {
        if input.just_pressed(GamepadButton::RightTrigger) {
            rumble.play(RumblePattern::texture(0.25)); // already looping
        }
        if input.just_released(GamepadButton::RightTrigger) {
            rumble.stop_all();
        }
    }
}
```

### Mixing rules

Two rules stop concurrent effects turning to mush:

- **Priority wins outright.** Only tracks at the highest active priority are
  audible. An explosion at priority 5 silences the idle engine hum rather than
  being muddied by it.
- **Max, not sum.** Same-priority tracks combine per-motor with `max`, so five
  simultaneous effects don't pin both motors at 1.0, which just feels like
  noise.

```rust
rumble.play_with_priority(RumblePattern::explosion(1.0), 5); // drowns out everything below
```

Over `CouchConfig::max_rumble_tracks`, the lowest-priority (then oldest) track
is evicted.

### Under the hood

The driver keeps exactly **one live effect per gamepad**: it sends `Stop` then
a single `Add`, and only when the intensity meaningfully changes or a sustained
effect needs refreshing. Firing a request per frame would pile effects up
inside `bevy_gilrs` and re-upload to the device 60×/second.

### Accessibility

`CouchConfig` is a live resource: wire these straight to your options menu:

```rust
fn apply_settings(mut config: ResMut<CouchConfig>, options: Res<Options>) {
    config.rumble_enabled = options.rumble_on;
    config.rumble_scale = options.rumble_strength; // 0..1
    config.stick_deadzone = options.deadzone;
}
```

## Button prompts

One "press A" prompt on a shared screen is wrong for at least one player.

```rust
let brand = roster.brand(player_id);

format!("Press {} to jump", brand.button_label(GamepadButton::South));
// Xbox: "A"   PlayStation: "✕"   Switch: "B"   unknown: falls back to Xbox

format!("{id} - {}", brand.name()); // "P2 - PlayStation"
```

Note the Switch mapping: `GamepadButton::South` is physically labelled **B** on
a Switch Pro controller. Telling a Switch player to "press A" when you meant
`South` sends them to the wrong button.

## Ordering your own systems

Everything runs in `PreUpdate` after Bevy's `InputSystems`, in the order
`Devices → Membership → Input`, so `PlayerInput` is current by `Update`.
Haptics flush in `PostUpdate`.

```rust
// See joins before gameplay does.
app.add_systems(PreUpdate, assign_teams.after(CouchSystems::Membership));

// Post-process input: remapping, recording, replay.
app.add_systems(PreUpdate, remap_input.after(CouchSystems::Input));
```

## Testing without hardware

Bevy's `Gamepad` component exposes mutable input state, so integration tests
can fake a controller without one plugged in:

```rust
fn fake_a_press(mut gamepads: Query<&mut Gamepad>) {
    for mut pad in &mut gamepads {
        pad.digital_mut().press(GamepadButton::Start);
    }
}
```

`cargo test` also covers the rumble mixer, envelope sampling, deadzone shape,
and brand detection directly.

## Configuration reference

| Field | Default | Notes |
| --- | --- | --- |
| `max_players` | 4 | Slot count. Stable for the session. |
| `join_mode` | `PressToJoin(Button(Start))` | `Automatic`, `PressToJoin`, `Manual` |
| `leave_mode` | `HoldButton { Select, 1.5s }` | Or `Manual` |
| `reconnect.grace_secs` | `None` | `None` reserves the slot forever |
| `reconnect.match_device` | `true` | Return a pad to its original slot |
| `reconnect.adopt_any` | `true` | Let a spare pad take over a waiting slot |
| `stick_deadzone` | `0.12` | Radial, on top of Bevy's per-axis one |
| `trigger_deadzone` | `0.05` | |
| `rumble_enabled` / `rumble_scale` | `true` / `1.0` | Accessibility |
| `max_rumble_tracks` | `8` | Concurrent effects per player |

### Why a second deadzone?

Bevy's built-in deadzone is per-axis, which carves a **square** hole out of the
stick: a slow diagonal push clears the deadzone on one axis but not the other,
so it snaps to a cardinal direction. `radial_deadzone` uses the vector length
instead and rescales the live zone back to 0..1. D-pad input is normalised too,
so diagonals aren't 1.41× faster than cardinals.

## Example

```sh
cargo run --example couch
```

Four squares, hot-join, per-player rumble. Yank a controller out mid-game and
watch the slot stay open.

## Not handled

- **Keyboard players.** Deliberate: mixing a keyboard "player 0" in changes the
  input model enough that it deserves its own design pass rather than a flag.
- **Split-screen cameras.** Out of scope, but pairs well: the player entity is
  the natural place to hang a viewport.
- **Netcode.** Strictly local.
- **Two identical controllers unplugged at once.** They're indistinguishable by
  vendor/product id and name, so slots may swap. gilrs doesn't expose per-device
  serials, so nothing else is available to key on.

## Compatibility

| This crate | Bevy |
| --- | --- |
| 0.1 | 0.19 |

## License

MIT
