use bevy_ecs::prelude::*;

use crate::brand::GamepadBrand;
use crate::player::{DeviceFingerprint, LeaveReason, PlayerDevice, PlayerId};

#[derive(Debug, Clone, Default)]
pub(crate) enum SlotState {
    #[default]
    Empty,
    Occupied {
        player_entity: Entity,
        device: PlayerDevice,
        fingerprint: DeviceFingerprint,
        /// Seconds (since app start) when the device dropped, if it has.
        lost_at: Option<f64>,
    },
}

/// A read-only view of one occupied slot.
///
/// Cheaper to pass around than four separate roster lookups, and it can't
/// disagree with itself:
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
///
/// fn hud(roster: Res<PlayerRoster>) {
///     for slot in roster.iter() {
///         if slot.is_connected() {
///             println!("{} on a {}", slot.id(), slot.brand().name());
///         } else {
///             println!("{} — reconnect your controller", slot.id());
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct PlayerSlot<'a> {
    id: PlayerId,
    player_entity: Entity,
    device: PlayerDevice,
    fingerprint: &'a DeviceFingerprint,
    lost_at: Option<f64>,
}

impl<'a> PlayerSlot<'a> {
    /// Which slot this is.
    pub fn id(self) -> PlayerId {
        self.id
    }

    /// The player entity — yours to attach gameplay components to.
    pub fn entity(self) -> Entity {
        self.player_entity
    }

    /// What hardware is driving this slot, if any.
    pub fn device(self) -> PlayerDevice {
        self.device
    }

    /// The live gamepad entity, if the controller is plugged in.
    pub fn gamepad(self) -> Option<Entity> {
        self.device.gamepad()
    }

    /// Whether the controller is plugged in right now.
    pub fn is_connected(self) -> bool {
        self.device.is_connected()
    }

    /// Identity of the controller in this slot — kept even while it's
    /// unplugged, so "P2: reconnect your DualSense" still works.
    pub fn fingerprint(self) -> &'a DeviceFingerprint {
        self.fingerprint
    }

    /// Which button glyphs to show this player. See
    /// [`GamepadBrand::button_label`].
    pub fn brand(self) -> GamepadBrand {
        self.fingerprint.brand()
    }

    /// How long this player has been waiting for hardware, in seconds.
    /// `None` while they're connected.
    pub fn waiting_secs(self, now: f64) -> Option<f32> {
        self.lost_at.map(|at| (now - at).max(0.0) as f32)
    }
}

/// The source of truth for who is playing and on what.
///
/// The plugin keeps this in sync; you read it for UI and call
/// [`request_join`](Self::request_join) / [`request_leave`](Self::request_leave)
/// to drive a lobby yourself.
#[derive(Resource, Debug)]
pub struct PlayerRoster {
    slots: Vec<SlotState>,
    unassigned: Vec<Entity>,
    join_queue: Vec<Entity>,
    leave_queue: Vec<(PlayerId, LeaveReason)>,
}

impl PlayerRoster {
    pub(crate) fn new(max_players: u8) -> Self {
        Self {
            slots: vec![SlotState::Empty; max_players.max(1) as usize],
            unassigned: Vec::new(),
            join_queue: Vec::new(),
            leave_queue: Vec::new(),
        }
    }

    /// Total slots, joined or not.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many players have joined (including ones whose pad is unplugged).
    pub fn player_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s, SlotState::Occupied { .. }))
            .count()
    }

    /// Whether nobody has joined yet.
    pub fn is_empty(&self) -> bool {
        self.player_count() == 0
    }

    /// Players whose controller is live right now.
    pub fn connected_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s, SlotState::Occupied { device, .. } if device.is_connected()))
            .count()
    }

    /// Whether every slot is taken.
    pub fn is_full(&self) -> bool {
        self.player_count() >= self.capacity()
    }

    /// Gamepads that are plugged in but haven't joined.
    pub fn unassigned_gamepads(&self) -> &[Entity] {
        &self.unassigned
    }

    /// Whether a slot currently holds a player.
    pub fn is_joined(&self, id: PlayerId) -> bool {
        matches!(self.slots.get(id.index()), Some(SlotState::Occupied { .. }))
    }

    /// Everything known about one slot, or `None` if nobody is in it.
    pub fn get(&self, id: PlayerId) -> Option<PlayerSlot<'_>> {
        match self.slots.get(id.index())? {
            SlotState::Occupied {
                player_entity,
                device,
                fingerprint,
                lost_at,
            } => Some(PlayerSlot {
                id,
                player_entity: *player_entity,
                device: *device,
                fingerprint,
                lost_at: *lost_at,
            }),
            SlotState::Empty => None,
        }
    }

    /// Every joined slot, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = PlayerSlot<'_>> + '_ {
        (0..self.slots.len()).filter_map(|i| self.get(PlayerId::new(i as u8)))
    }

    /// Just the ids of joined slots, in order.
    pub fn players(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.iter().map(PlayerSlot::id)
    }

    /// The player entity for a slot, if that slot is occupied.
    pub fn player_entity(&self, id: PlayerId) -> Option<Entity> {
        self.get(id).map(PlayerSlot::entity)
    }

    /// What hardware, if any, a slot is driving.
    pub fn device(&self, id: PlayerId) -> Option<PlayerDevice> {
        self.get(id).map(PlayerSlot::device)
    }

    /// Whether a slot is joined *and* holding live hardware.
    pub fn is_connected(&self, id: PlayerId) -> bool {
        self.get(id).is_some_and(PlayerSlot::is_connected)
    }

    /// Hardware identity of the controller in a slot — kept even while the
    /// controller is unplugged.
    pub fn fingerprint(&self, id: PlayerId) -> Option<&DeviceFingerprint> {
        match self.slots.get(id.index())? {
            SlotState::Occupied { fingerprint, .. } => Some(fingerprint),
            SlotState::Empty => None,
        }
    }

    /// Which button glyphs to show this player. See
    /// [`GamepadBrand::button_label`].
    pub fn brand(&self, id: PlayerId) -> GamepadBrand {
        self.fingerprint(id)
            .map(DeviceFingerprint::brand)
            .unwrap_or_default()
    }

    /// Slot that owns a given gamepad entity.
    pub fn player_of_gamepad(&self, gamepad: Entity) -> Option<PlayerId> {
        self.iter()
            .find(|slot| slot.gamepad() == Some(gamepad))
            .map(PlayerSlot::id)
    }

    /// Joined slots that currently have hardware, paired with it.
    pub fn assigned(&self) -> impl Iterator<Item = (PlayerId, Entity)> + '_ {
        self.iter()
            .filter_map(|slot| slot.gamepad().map(|pad| (slot.id(), pad)))
    }

    /// Joined players who are currently missing their controller. Show these
    /// in a "reconnect controller" overlay.
    pub fn disconnected_players(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.iter()
            .filter(|slot| !slot.is_connected())
            .map(PlayerSlot::id)
    }

    /// Whether anyone is waiting on a controller right now.
    pub fn any_disconnected(&self) -> bool {
        self.disconnected_players().next().is_some()
    }

    /// Ask for a specific gamepad to join on the next update. Used by
    /// [`JoinMode::Manual`](crate::JoinMode), e.g. from your own lobby UI.
    pub fn request_join(&mut self, gamepad: Entity) {
        if !self.join_queue.contains(&gamepad) {
            self.join_queue.push(gamepad);
        }
    }

    /// Drop a player on the next update.
    pub fn request_leave(&mut self, id: PlayerId) {
        self.queue_leave(id, LeaveReason::Requested);
    }

    pub(crate) fn first_free_slot(&self) -> Option<PlayerId> {
        self.slots
            .iter()
            .position(|s| matches!(s, SlotState::Empty))
            .map(|i| PlayerId::new(i as u8))
    }

    /// The disconnected slot that has been waiting longest for hardware.
    pub(crate) fn longest_waiting_slot(&self) -> Option<PlayerId> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| match slot {
                SlotState::Occupied {
                    device,
                    lost_at: Some(at),
                    ..
                } if !device.is_connected() => Some((i, *at)),
                _ => None,
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal))
            .map(|(i, _)| PlayerId::new(i as u8))
    }

    /// A disconnected slot whose old controller looks like `fingerprint`.
    pub(crate) fn waiting_slot_matching(&self, fingerprint: &DeviceFingerprint) -> Option<PlayerId> {
        self.slots
            .iter()
            .enumerate()
            .find_map(|(i, slot)| match slot {
                SlotState::Occupied {
                    device,
                    fingerprint: known,
                    ..
                } if !device.is_connected() && known.matches(fingerprint) => {
                    Some(PlayerId::new(i as u8))
                }
                _ => None,
            })
    }

    /// Slots whose reconnect grace period has run out.
    pub(crate) fn expired_slots(&self, grace_secs: f32, now: f64) -> Vec<PlayerId> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| match slot {
                SlotState::Occupied {
                    device,
                    lost_at: Some(at),
                    ..
                } if !device.is_connected() && now - *at >= grace_secs as f64 => {
                    Some(PlayerId::new(i as u8))
                }
                _ => None,
            })
            .collect()
    }

    pub(crate) fn occupy(
        &mut self,
        id: PlayerId,
        player_entity: Entity,
        gamepad: Entity,
        fingerprint: DeviceFingerprint,
    ) {
        if let Some(slot) = self.slots.get_mut(id.index()) {
            *slot = SlotState::Occupied {
                player_entity,
                device: PlayerDevice::Gamepad(gamepad),
                fingerprint,
                lost_at: None,
            };
        }
    }

    /// Hand hardware to an occupied slot. Returns its player entity.
    pub(crate) fn attach_device(
        &mut self,
        id: PlayerId,
        gamepad: Entity,
        fingerprint: DeviceFingerprint,
    ) -> Option<Entity> {
        match self.slots.get_mut(id.index())? {
            SlotState::Occupied {
                player_entity,
                device,
                fingerprint: known,
                lost_at,
            } => {
                *device = PlayerDevice::Gamepad(gamepad);
                *known = fingerprint;
                *lost_at = None;
                Some(*player_entity)
            }
            SlotState::Empty => None,
        }
    }

    /// Returns the player entity, or `None` if the slot was already flagged.
    pub(crate) fn mark_disconnected(&mut self, id: PlayerId, now: f64) -> Option<Entity> {
        match self.slots.get_mut(id.index())? {
            SlotState::Occupied {
                player_entity,
                device,
                lost_at,
                ..
            } if device.is_connected() => {
                *device = PlayerDevice::Missing;
                *lost_at = Some(now);
                Some(*player_entity)
            }
            _ => None,
        }
    }

    /// Empty a slot, returning what was in it.
    pub(crate) fn vacate(&mut self, id: PlayerId) -> Option<(Entity, PlayerDevice)> {
        let slot = self.slots.get_mut(id.index())?;
        match core::mem::replace(slot, SlotState::Empty) {
            SlotState::Occupied {
                player_entity,
                device,
                ..
            } => Some((player_entity, device)),
            SlotState::Empty => None,
        }
    }

    /// Grow or shrink the lobby. Shrinking only ever drops empty trailing
    /// slots, so raising and lowering the setting can't kick anyone out
    /// mid-game. Returns whether the capacity actually changed.
    pub(crate) fn resize(&mut self, max_players: u8) -> bool {
        let target = (max_players.max(1)) as usize;
        if target > self.slots.len() {
            self.slots.resize(target, SlotState::Empty);
            return true;
        }

        let occupied_through = self
            .slots
            .iter()
            .rposition(|s| matches!(s, SlotState::Occupied { .. }))
            .map_or(0, |i| i + 1);
        let new_len = target.max(occupied_through).max(1);
        if new_len == self.slots.len() {
            return false;
        }
        self.slots.truncate(new_len);
        true
    }

    pub(crate) fn set_unassigned(&mut self, gamepads: Vec<Entity>) {
        self.unassigned = gamepads;
    }

    pub(crate) fn claim_unassigned(&mut self, gamepad: Entity) {
        self.unassigned.retain(|e| *e != gamepad);
    }

    pub(crate) fn release_unassigned(&mut self, gamepad: Entity) {
        if !self.unassigned.contains(&gamepad) {
            self.unassigned.push(gamepad);
        }
    }

    pub(crate) fn take_join_requests(&mut self) -> Vec<Entity> {
        core::mem::take(&mut self.join_queue)
    }

    pub(crate) fn take_leave_requests(&mut self) -> Vec<(PlayerId, LeaveReason)> {
        core::mem::take(&mut self.leave_queue)
    }

    pub(crate) fn queue_leave(&mut self, id: PlayerId, reason: LeaveReason) {
        if !self.leave_queue.iter().any(|(queued, _)| *queued == id) {
            self.leave_queue.push((id, reason));
        }
    }
}

impl Default for PlayerRoster {
    fn default() -> Self {
        Self::new(4)
    }
}

/// Run condition: true while any joined player is missing their controller.
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
/// # fn gameplay() {}
/// # let mut app = App::new();
/// app.add_systems(Update, gameplay.run_if(not(any_player_disconnected)));
/// ```
pub fn any_player_disconnected(roster: Res<PlayerRoster>) -> bool {
    roster.any_disconnected()
}

/// Run condition: true once at least one player has joined.
pub fn any_player_joined(roster: Res<PlayerRoster>) -> bool {
    !roster.is_empty()
}

/// Run condition: true while every slot is taken.
pub fn lobby_is_full(roster: Res<PlayerRoster>) -> bool {
    roster.is_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `Entity` ids without depending on any raw-id constructor.
    fn ids(n: usize) -> Vec<Entity> {
        let mut world = World::new();
        (0..n).map(|_| world.spawn_empty().id()).collect()
    }

    fn fingerprint(product: u16) -> DeviceFingerprint {
        DeviceFingerprint {
            name: Some("Test Pad".into()),
            vendor_id: Some(0x054C),
            product_id: Some(product),
        }
    }

    /// Joins `pad` into the first free slot, as `process_membership` does.
    fn join(roster: &mut PlayerRoster, player: Entity, pad: Entity, fp: DeviceFingerprint) -> PlayerId {
        let id = roster.first_free_slot().expect("a free slot");
        roster.occupy(id, player, pad, fp);
        id
    }

    #[test]
    fn slots_fill_from_the_lowest_free_one() {
        let e = ids(6);
        let mut roster = PlayerRoster::new(4);

        assert_eq!(join(&mut roster, e[0], e[1], fingerprint(1)), PlayerId::P1);
        assert_eq!(join(&mut roster, e[2], e[3], fingerprint(2)), PlayerId::P2);
        assert_eq!(roster.player_count(), 2);
        assert!(!roster.is_full());
    }

    #[test]
    fn vacated_slots_are_reused_before_higher_ones() {
        // P1 leaving must not renumber P2 into P1.
        let e = ids(8);
        let mut roster = PlayerRoster::new(4);
        join(&mut roster, e[0], e[1], fingerprint(1));
        join(&mut roster, e[2], e[3], fingerprint(2));

        roster.vacate(PlayerId::P1);

        assert!(roster.is_joined(PlayerId::P2));
        assert_eq!(roster.first_free_slot(), Some(PlayerId::P1));
    }

    #[test]
    fn a_full_lobby_has_no_free_slot() {
        let e = ids(4);
        let mut roster = PlayerRoster::new(2);
        join(&mut roster, e[0], e[1], fingerprint(1));
        join(&mut roster, e[2], e[3], fingerprint(2));

        assert!(roster.is_full());
        assert_eq!(roster.first_free_slot(), None);
    }

    #[test]
    fn disconnect_holds_the_slot() {
        let e = ids(2);
        let mut roster = PlayerRoster::new(4);
        let id = join(&mut roster, e[0], e[1], fingerprint(1));

        assert_eq!(roster.mark_disconnected(id, 10.0), Some(e[0]));

        assert!(roster.is_joined(id), "the slot is held, not freed");
        assert!(!roster.is_connected(id));
        assert_eq!(roster.player_entity(id), Some(e[0]), "entity survives");
        assert!(roster.any_disconnected());
    }

    #[test]
    fn disconnecting_twice_is_ignored() {
        let e = ids(2);
        let mut roster = PlayerRoster::new(4);
        let id = join(&mut roster, e[0], e[1], fingerprint(1));

        roster.mark_disconnected(id, 10.0);
        assert_eq!(
            roster.mark_disconnected(id, 20.0),
            None,
            "the second one must not reset the waiting clock"
        );
        assert_eq!(roster.get(id).unwrap().waiting_secs(30.0), Some(20.0));
    }

    #[test]
    fn the_same_controller_finds_its_own_slot() {
        let e = ids(6);
        let mut roster = PlayerRoster::new(4);
        join(&mut roster, e[0], e[1], fingerprint(1));
        let p2 = join(&mut roster, e[2], e[3], fingerprint(2));
        roster.mark_disconnected(p2, 5.0);

        assert_eq!(roster.waiting_slot_matching(&fingerprint(2)), Some(p2));
        assert_eq!(
            roster.waiting_slot_matching(&fingerprint(99)),
            None,
            "an unrelated pad must not steal a held slot"
        );

        assert_eq!(roster.attach_device(p2, e[4], fingerprint(2)), Some(e[2]));
        assert!(roster.is_connected(p2));
        assert_eq!(roster.get(p2).unwrap().gamepad(), Some(e[4]));
    }

    #[test]
    fn a_connected_slot_is_never_offered_to_a_matching_pad() {
        let e = ids(4);
        let mut roster = PlayerRoster::new(4);
        join(&mut roster, e[0], e[1], fingerprint(1));

        // Two identical controllers: the second must not adopt the first's slot.
        assert_eq!(roster.waiting_slot_matching(&fingerprint(1)), None);
    }

    #[test]
    fn borrowed_pads_go_to_whoever_waited_longest() {
        let e = ids(6);
        let mut roster = PlayerRoster::new(4);
        let p1 = join(&mut roster, e[0], e[1], fingerprint(1));
        let p2 = join(&mut roster, e[2], e[3], fingerprint(2));

        roster.mark_disconnected(p2, 5.0);
        roster.mark_disconnected(p1, 9.0);

        assert_eq!(roster.longest_waiting_slot(), Some(p2));
    }

    #[test]
    fn grace_expiry_only_counts_the_disconnected() {
        let e = ids(6);
        let mut roster = PlayerRoster::new(4);
        let p1 = join(&mut roster, e[0], e[1], fingerprint(1));
        let p2 = join(&mut roster, e[2], e[3], fingerprint(2));
        roster.mark_disconnected(p2, 10.0);

        assert!(roster.expired_slots(30.0, 35.0).is_empty(), "still in grace");
        assert_eq!(roster.expired_slots(30.0, 41.0), vec![p2]);
        assert!(
            !roster.expired_slots(0.0, 41.0).contains(&p1),
            "a connected player never times out"
        );
    }

    #[test]
    fn reconnecting_clears_the_grace_timer() {
        let e = ids(4);
        let mut roster = PlayerRoster::new(4);
        let id = join(&mut roster, e[0], e[1], fingerprint(1));
        roster.mark_disconnected(id, 10.0);
        roster.attach_device(id, e[2], fingerprint(1));

        assert!(roster.expired_slots(1.0, 1_000.0).is_empty());
        assert_eq!(roster.get(id).unwrap().waiting_secs(1_000.0), None);
    }

    #[test]
    fn leave_requests_are_deduplicated() {
        let e = ids(2);
        let mut roster = PlayerRoster::new(4);
        let id = join(&mut roster, e[0], e[1], fingerprint(1));

        roster.request_leave(id);
        roster.request_leave(id);
        roster.queue_leave(id, LeaveReason::TimedOut);

        assert_eq!(roster.take_leave_requests().len(), 1);
        assert!(roster.take_leave_requests().is_empty(), "queue drains");
    }

    #[test]
    fn unassigned_pads_are_tracked() {
        let e = ids(3);
        let mut roster = PlayerRoster::new(4);

        roster.set_unassigned(vec![e[0], e[1]]);
        roster.claim_unassigned(e[0]);
        assert_eq!(roster.unassigned_gamepads(), &[e[1]]);

        roster.release_unassigned(e[0]);
        roster.release_unassigned(e[0]);
        assert_eq!(roster.unassigned_gamepads().len(), 2, "no duplicates");
    }

    #[test]
    fn growing_the_lobby_opens_slots() {
        let mut roster = PlayerRoster::new(2);
        assert!(roster.resize(4));
        assert_eq!(roster.capacity(), 4);
        assert!(!roster.resize(4), "no change reported when nothing moves");
    }

    #[test]
    fn shrinking_never_kicks_a_seated_player() {
        let e = ids(8);
        let mut roster = PlayerRoster::new(4);
        join(&mut roster, e[0], e[1], fingerprint(1));
        join(&mut roster, e[2], e[3], fingerprint(2));
        join(&mut roster, e[4], e[5], fingerprint(3));

        roster.resize(1);

        assert_eq!(roster.capacity(), 3, "trailing empty slot dropped, P3 kept");
        assert!(roster.is_joined(PlayerId::P3));
    }
}
