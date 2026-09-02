use core::time::Duration;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_input::gamepad::{Gamepad, GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy_time::Time;

use crate::config::CouchConfig;
use crate::player::PlayerDevice;

/// How long each rumble request we send lasts.
const EFFECT_DURATION: f32 = 0.30;
/// How often we refresh a sustained effect. Comfortably shorter than
/// `EFFECT_DURATION`, so the motors never gap between requests.
const REFRESH_INTERVAL: f32 = 0.15;
/// Intensity change below this doesn't justify a new request.
const CHANGE_EPSILON: f32 = 0.02;

/// One point in a rumble envelope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RumbleKey {
    /// Seconds from the start of the pattern.
    pub at: f32,
    /// Low-frequency motor, 0..1. The heavy, thumping one.
    pub strong: f32,
    /// High-frequency motor, 0..1. The buzzy, detailed one.
    pub weak: f32,
}

impl RumbleKey {
    /// A keyframe at `at` seconds with the given motor intensities.
    pub const fn new(at: f32, strong: f32, weak: f32) -> Self {
        Self { at, strong, weak }
    }
}

/// An envelope of rumble intensity over time, interpolated between keys.
///
/// Patterns beat one-shot vibrations because a flat buzz reads as "something
/// happened" and nothing more; a shaped envelope reads as *what* happened.
///
/// Cloning is cheap (the keyframes are shared), so build your patterns once
/// and keep them in a resource:
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// #[derive(Resource)]
/// struct GameFeel {
///     land: RumblePattern,
/// }
///
/// fn on_land(feel: Res<GameFeel>, mut players: Query<&mut Rumble>) {
///     for mut rumble in &mut players {
///         rumble.play(feel.land.clone());
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RumblePattern {
    keys: Arc<[RumbleKey]>,
    duration: f32,
    repeat: bool,
}

impl RumblePattern {
    /// Build a pattern from keyframes. They're sorted for you.
    pub fn new(keys: impl Into<Vec<RumbleKey>>) -> Self {
        let mut keys: Vec<RumbleKey> = keys.into();
        keys.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(core::cmp::Ordering::Equal));
        let duration = keys.last().map_or(0.0, |k| k.at);
        Self {
            keys: keys.into(),
            duration,
            repeat: false,
        }
    }

    /// The keyframes, sorted by time.
    pub fn keys(&self) -> &[RumbleKey] {
        &self.keys
    }

    /// Total length in seconds — the time of the last key.
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Whether playback wraps instead of ending.
    pub fn repeats(&self) -> bool {
        self.repeat
    }

    /// Loop until stopped. Keep the [`RumbleHandle`] from
    /// [`Rumble::play`] so you can stop it again, or it runs forever.
    pub fn looping(mut self) -> Self {
        self.repeat = true;
        self
    }

    /// Scale both motors by `factor`, for a weaker or stronger variant of an
    /// existing pattern.
    pub fn scaled(&self, factor: f32) -> Self {
        let factor = factor.max(0.0);
        Self {
            keys: self
                .keys
                .iter()
                .map(|k| RumbleKey::new(k.at, k.strong * factor, k.weak * factor))
                .collect(),
            duration: self.duration,
            repeat: self.repeat,
        }
    }

    /// Flat intensity for a fixed time.
    pub fn constant(strong: f32, weak: f32, secs: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, strong, weak),
            RumbleKey::new(secs, strong, weak),
        ])
    }

    /// Sharp attack, quick decay. Melee hits, landing, bumping a wall.
    pub fn hit(strength: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, strength, strength * 0.8),
            RumbleKey::new(0.05, strength, strength * 0.6),
            RumbleKey::new(0.18, 0.0, 0.0),
        ])
    }

    /// Instant slam then a long low rumble-off.
    pub fn explosion(strength: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, strength, strength),
            RumbleKey::new(0.08, strength * 0.9, strength * 0.35),
            RumbleKey::new(0.45, strength * 0.35, 0.05),
            RumbleKey::new(0.9, 0.0, 0.0),
        ])
    }

    /// Two soft thumps. Low health, tension, a held breath.
    pub fn heartbeat(strength: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, strength, 0.0),
            RumbleKey::new(0.10, 0.0, 0.0),
            RumbleKey::new(0.22, strength * 0.7, 0.0),
            RumbleKey::new(0.32, 0.0, 0.0),
            RumbleKey::new(1.10, 0.0, 0.0),
        ])
    }

    /// Buzz that swells in. Charging a shot, a door grinding open.
    pub fn ramp_up(strength: f32, secs: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, 0.0, 0.0),
            RumbleKey::new(secs, strength * 0.6, strength),
        ])
    }

    /// Light continuous texture — engines, rolling, wind. Loops.
    pub fn texture(strength: f32) -> Self {
        Self::new([
            RumbleKey::new(0.0, 0.0, strength),
            RumbleKey::new(0.25, 0.0, strength),
        ])
        .looping()
    }

    /// Intensity at time `t`, linearly interpolated.
    pub fn sample(&self, t: f32) -> (f32, f32) {
        let (Some(&first), Some(&last)) = (self.keys.first(), self.keys.last()) else {
            return (0.0, 0.0);
        };

        let t = if self.repeat && self.duration > 0.0 {
            t.rem_euclid(self.duration)
        } else {
            t
        };

        if t.is_nan() || t <= first.at {
            return (first.strong, first.weak);
        }
        if t >= last.at {
            return (last.strong, last.weak);
        }

        // `t` sits strictly inside the envelope, so there is always a key on
        // either side of it.
        let next = self.keys.partition_point(|k| k.at <= t);
        let (a, b) = (self.keys[next - 1], self.keys[next]);
        let span = b.at - a.at;
        let f = if span <= f32::EPSILON {
            0.0
        } else {
            (t - a.at) / span
        };
        (
            a.strong + (b.strong - a.strong) * f,
            a.weak + (b.weak - a.weak) * f,
        )
    }
}

/// Identifies one playing effect, so you can stop it without stopping
/// everything else. Returned by [`Rumble::play`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RumbleHandle(u64);

#[derive(Debug, Clone)]
struct RumbleTrack {
    id: u64,
    pattern: RumblePattern,
    elapsed: f32,
    priority: u8,
}

/// A player's haptics channel. Sits on the player entity, so you request
/// rumble for a *player*, not for a gamepad entity that may not exist anymore.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn on_hit(mut players: Query<(&Player, &mut Rumble)>) {
///     for (player, mut rumble) in &mut players {
///         rumble.play(RumblePattern::hit(0.8));
///     }
/// }
/// ```
///
/// **Mixing rule:** only tracks at the highest active priority are audible,
/// and those are combined per-motor with `max`. That keeps a big explosion
/// from being muddied by an idle engine hum, and stops five simultaneous
/// effects from pinning both motors at 1.0 — which just feels like noise.
#[derive(Component, Debug, Default)]
pub struct Rumble {
    tracks: Vec<RumbleTrack>,
    next_id: u64,
    last_sent: (f32, f32),
    since_send: f32,
    emitting: bool,
    /// Which gamepad `last_sent` describes. Hardware swaps invalidate it.
    last_gamepad: Option<Entity>,
}

impl Rumble {
    /// Queue a pattern at the default priority.
    pub fn play(&mut self, pattern: RumblePattern) -> RumbleHandle {
        self.play_with_priority(pattern, 0)
    }

    /// Queue a pattern that outranks (and silences) lower-priority ones.
    pub fn play_with_priority(&mut self, pattern: RumblePattern, priority: u8) -> RumbleHandle {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.tracks.push(RumbleTrack {
            id,
            pattern,
            elapsed: 0.0,
            priority,
        });
        RumbleHandle(id)
    }

    /// One-shot convenience: flat intensity for a duration.
    pub fn pulse(&mut self, strong: f32, weak: f32, secs: f32) -> RumbleHandle {
        self.play(RumblePattern::constant(strong, weak, secs))
    }

    /// Stop one effect — the way to end a looping pattern without silencing
    /// everything else. Returns whether it was still playing.
    ///
    /// ```
    /// use bevy::prelude::*;
    /// use bevy_couch_multiplayer::prelude::*;
    ///
    /// #[derive(Component)]
    /// struct EngineHum(RumbleHandle);
    ///
    /// fn start(mut rumble: Mut<Rumble>, mut commands: Commands, player: Entity) {
    ///     let handle = rumble.play(RumblePattern::texture(0.25));
    ///     commands.entity(player).insert(EngineHum(handle));
    /// }
    ///
    /// fn stop(mut players: Query<(&EngineHum, &mut Rumble)>) {
    ///     for (hum, mut rumble) in &mut players {
    ///         rumble.stop(hum.0);
    ///     }
    /// }
    /// ```
    pub fn stop(&mut self, handle: RumbleHandle) -> bool {
        let before = self.tracks.len();
        self.tracks.retain(|t| t.id != handle.0);
        self.tracks.len() != before
    }

    /// Whether a specific effect is still running.
    pub fn is_active(&self, handle: RumbleHandle) -> bool {
        self.tracks.iter().any(|t| t.id == handle.0)
    }

    /// Cancel everything on this player, immediately.
    pub fn stop_all(&mut self) {
        self.tracks.clear();
    }

    /// Whether any track is still running.
    pub fn is_playing(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// How many tracks are live, after budget eviction.
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// The intensity last sent to the motors, as `(strong, weak)`. Handy for a
    /// debug overlay; it is `(0.0, 0.0)` while nothing is playing.
    pub fn current(&self) -> (f32, f32) {
        if self.emitting { self.last_sent } else { (0.0, 0.0) }
    }

    /// Forget what we believe the motors are doing. Used when the hardware
    /// goes away or is swapped underneath the player.
    fn forget_output(&mut self) {
        self.last_sent = (0.0, 0.0);
        self.since_send = 0.0;
        self.emitting = false;
        self.last_gamepad = None;
    }

    /// Drop the lowest-priority (then oldest) tracks over the budget.
    fn enforce_budget(&mut self, max: usize) {
        if max == 0 {
            self.tracks.clear();
            return;
        }
        while self.tracks.len() > max {
            let victim = self
                .tracks
                .iter()
                .enumerate()
                .min_by_key(|(i, t)| (t.priority, *i))
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.tracks.remove(i);
                }
                None => break,
            }
        }
    }

    /// Advance and mix. Returns the intensity for this frame.
    fn advance(&mut self, dt: f32, max_tracks: usize) -> (f32, f32) {
        self.enforce_budget(max_tracks);

        for track in &mut self.tracks {
            track.elapsed += dt;
        }
        self.tracks
            .retain(|t| t.pattern.repeat || t.elapsed <= t.pattern.duration);

        let Some(top) = self.tracks.iter().map(|t| t.priority).max() else {
            return (0.0, 0.0);
        };

        let mut mixed = (0.0f32, 0.0f32);
        for track in self.tracks.iter().filter(|t| t.priority == top) {
            let (s, w) = track.pattern.sample(track.elapsed);
            mixed.0 = mixed.0.max(s);
            mixed.1 = mixed.1.max(w);
        }
        mixed
    }
}

/// Advances every player's rumble and talks to the hardware.
///
/// We never fire a request per frame — that would pile effects up inside
/// `bevy_gilrs`. Instead each change (or a periodic refresh of a sustained
/// effect) sends a `Stop` followed by a single `Add`, so exactly one effect is
/// live per gamepad at any moment.
pub(crate) fn drive_haptics(
    time: Res<Time>,
    config: Res<CouchConfig>,
    mut players: Query<(&PlayerDevice, &mut Rumble)>,
    live_gamepads: Query<(), With<Gamepad>>,
    mut requests: MessageWriter<GamepadRumbleRequest>,
) {
    let dt = time.delta_secs();

    for (device, mut rumble) in &mut players {
        let Some(gamepad) = device.gamepad() else {
            // No hardware: drop everything so a reconnecting player doesn't
            // inherit a stale buzz.
            if rumble.is_playing() || rumble.emitting || rumble.last_gamepad.is_some() {
                rumble.stop_all();
                rumble.forget_output();
            }
            continue;
        };

        // The gamepad entity can be despawned in the same frame it drops;
        // `sync_devices` will move this player to `Missing` next frame.
        if !live_gamepads.contains(gamepad) {
            continue;
        }

        // Different hardware than we last talked to (a reconnect, or a
        // borrowed pad): whatever we thought the motors were doing is void.
        if rumble.last_gamepad != Some(gamepad) {
            rumble.forget_output();
            rumble.last_gamepad = Some(gamepad);
        }

        let (mut strong, mut weak) = rumble.advance(dt, config.max_rumble_tracks);

        if config.rumble_enabled {
            let scale = config.rumble_scale.clamp(0.0, 1.0);
            strong = (strong * scale).clamp(0.0, 1.0);
            weak = (weak * scale).clamp(0.0, 1.0);
        } else {
            strong = 0.0;
            weak = 0.0;
        }

        rumble.since_send += dt;

        let silent = strong <= f32::EPSILON && weak <= f32::EPSILON;
        if silent {
            if rumble.emitting {
                requests.write(GamepadRumbleRequest::Stop { gamepad });
                rumble.emitting = false;
                rumble.last_sent = (0.0, 0.0);
                rumble.since_send = 0.0;
            }
            continue;
        }

        let changed = (strong - rumble.last_sent.0).abs() > CHANGE_EPSILON
            || (weak - rumble.last_sent.1).abs() > CHANGE_EPSILON;
        let needs_refresh = rumble.emitting && rumble.since_send >= REFRESH_INTERVAL;

        if changed || needs_refresh || !rumble.emitting {
            requests.write(GamepadRumbleRequest::Stop { gamepad });
            requests.write(GamepadRumbleRequest::Add {
                gamepad,
                duration: Duration::from_secs_f32(EFFECT_DURATION),
                intensity: GamepadRumbleIntensity {
                    strong_motor: strong,
                    weak_motor: weak,
                },
            });
            rumble.last_sent = (strong, weak);
            rumble.since_send = 0.0;
            rumble.emitting = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_between_keys() {
        let p = RumblePattern::new([RumbleKey::new(0.0, 0.0, 0.0), RumbleKey::new(1.0, 1.0, 0.5)]);
        let (s, w) = p.sample(0.5);
        assert!((s - 0.5).abs() < 1e-5);
        assert!((w - 0.25).abs() < 1e-5);
    }

    #[test]
    fn keys_are_sorted_on_construction() {
        let p = RumblePattern::new([RumbleKey::new(1.0, 1.0, 0.0), RumbleKey::new(0.0, 0.0, 0.0)]);
        assert_eq!(p.keys()[0].at, 0.0);
        assert_eq!(p.duration(), 1.0);
    }

    #[test]
    fn clamps_outside_range() {
        let p = RumblePattern::constant(0.4, 0.2, 1.0);
        assert_eq!(p.sample(-1.0), (0.4, 0.2));
        assert_eq!(p.sample(99.0), (0.4, 0.2));
    }

    #[test]
    fn an_empty_pattern_is_silent() {
        assert_eq!(
            RumblePattern::new(Vec::<RumbleKey>::new()).sample(0.5),
            (0.0, 0.0)
        );
    }

    #[test]
    fn samples_correctly_across_many_keys() {
        // Exercises the binary search rather than the first/last shortcuts.
        let p = RumblePattern::new([
            RumbleKey::new(0.0, 0.0, 0.0),
            RumbleKey::new(1.0, 1.0, 0.0),
            RumbleKey::new(2.0, 0.0, 0.0),
            RumbleKey::new(3.0, 1.0, 0.0),
        ]);
        assert!((p.sample(2.5).0 - 0.5).abs() < 1e-5);
        assert!((p.sample(1.0).0 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn looping_wraps() {
        let p = RumblePattern::new([RumbleKey::new(0.0, 1.0, 0.0), RumbleKey::new(1.0, 0.0, 0.0)])
            .looping();
        let (a, _) = p.sample(0.25);
        let (b, _) = p.sample(1.25);
        assert!((a - b).abs() < 1e-5);
    }

    #[test]
    fn scaling_keeps_the_shape() {
        let quiet = RumblePattern::hit(1.0).scaled(0.5);
        assert!((quiet.sample(0.0).0 - 0.5).abs() < 1e-5);
        assert_eq!(quiet.duration(), RumblePattern::hit(1.0).duration());
    }

    #[test]
    fn only_top_priority_is_audible() {
        let mut r = Rumble::default();
        r.play_with_priority(RumblePattern::constant(0.2, 0.2, 10.0), 0);
        r.play_with_priority(RumblePattern::constant(0.9, 0.0, 10.0), 5);
        let (s, w) = r.advance(0.016, 8);
        assert!((s - 0.9).abs() < 1e-5);
        assert_eq!(w, 0.0, "low-priority track should be silenced entirely");
    }

    #[test]
    fn budget_evicts_lowest_priority() {
        let mut r = Rumble::default();
        for _ in 0..5 {
            r.play_with_priority(RumblePattern::constant(0.1, 0.1, 10.0), 0);
        }
        r.play_with_priority(RumblePattern::constant(1.0, 1.0, 10.0), 9);
        r.advance(0.0, 2);
        assert_eq!(r.track_count(), 2);
        assert!(r.tracks.iter().any(|t| t.priority == 9));
    }

    #[test]
    fn budget_keeps_the_freshest_of_equal_priority() {
        let mut r = Rumble::default();
        r.play(RumblePattern::constant(0.1, 0.0, 10.0));
        r.play(RumblePattern::constant(0.9, 0.0, 10.0));
        let (strong, _) = r.advance(0.0, 1);
        assert!((strong - 0.9).abs() < 1e-5, "the newest effect should survive");
    }

    #[test]
    fn finished_tracks_are_dropped() {
        let mut r = Rumble::default();
        r.play(RumblePattern::constant(1.0, 1.0, 0.1));
        r.advance(0.2, 8);
        assert!(!r.is_playing());
    }

    #[test]
    fn a_loop_can_be_stopped_by_handle() {
        let mut r = Rumble::default();
        let hum = r.play(RumblePattern::texture(0.3));
        let hit = r.play(RumblePattern::hit(1.0));

        assert!(r.is_active(hum));
        assert!(r.stop(hum));
        assert!(!r.is_active(hum));
        assert!(r.is_active(hit), "unrelated effects keep playing");
        assert!(!r.stop(hum), "stopping twice is a no-op");
    }

    #[test]
    fn loops_outlive_their_duration() {
        let mut r = Rumble::default();
        r.play(RumblePattern::texture(0.3));
        r.advance(10.0, 8);
        assert!(r.is_playing());
    }
}
