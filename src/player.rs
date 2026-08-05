use bevy_ecs::prelude::*;

/// A stable, zero-based player slot. Player 0 is "P1" on screen.
///
/// Slots are the whole point of this crate: gamepad `Entity` ids come and go
/// as hardware connects and disconnects, but a `PlayerId` sticks to a human
/// for the whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(
    /// Zero-based slot index.
    pub u8,
);

impl PlayerId {
    /// 1-based number for UI: "Player 1", "P2 wins", etc.
    pub fn display_number(&self) -> u8 {
        self.0 + 1
    }

    /// A distinct per-slot colour as sRGB, so you don't have to invent one.
    /// Returned as a plain array to avoid depending on `bevy_color`; feed it
    /// straight to `Color::srgb(r, g, b)`.
    pub fn color_rgb(&self) -> [f32; 3] {
        const PALETTE: [[f32; 3]; 8] = [
            [0.20, 0.55, 1.00], // blue
            [1.00, 0.35, 0.30], // red
            [0.35, 0.85, 0.40], // green
            [1.00, 0.80, 0.25], // yellow
            [0.75, 0.45, 1.00], // purple
            [0.30, 0.85, 0.85], // cyan
            [1.00, 0.55, 0.20], // orange
            [0.95, 0.45, 0.75], // pink
        ];
        PALETTE[self.0 as usize % PALETTE.len()]
    }
}

impl core::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P{}", self.display_number())
    }
}

/// Marks the entity that represents a joined player.
///
/// This entity is spawned on join and lives until the player leaves — it
/// deliberately survives controller disconnects, so anything you attach to it
/// (score, character choice, inventory) survives a yanked USB cable.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Player {
    /// Which slot this entity plays in.
    pub id: PlayerId,
}

/// Which physical device is currently driving a player.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerDevice {
    /// The gamepad entity Bevy spawned for this controller.
    Gamepad(Entity),
    /// Slot is reserved but has no live hardware right now.
    Missing,
}

impl PlayerDevice {
    /// The live gamepad entity, if there is one.
    pub fn gamepad(&self) -> Option<Entity> {
        match self {
            Self::Gamepad(e) => Some(*e),
            Self::Missing => None,
        }
    }

    /// Whether hardware is currently attached.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Gamepad(_))
    }
}

/// Enough identity to recognise the same controller when it comes back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFingerprint {
    /// Device name as reported by the OS, e.g. "Wireless Controller".
    pub name: Option<String>,
    /// USB vendor id, when the platform exposes one.
    pub vendor_id: Option<u16>,
    /// USB product id, when the platform exposes one.
    pub product_id: Option<u16>,
}

impl DeviceFingerprint {
    /// Loose match: ids agree when both are known, otherwise fall back to name.
    ///
    /// Two identical controllers are indistinguishable by either measure, so
    /// this can hand P2's pad to P3 when both unplug at once. That is the
    /// least-bad outcome available without per-device serial numbers, which
    /// gilrs does not surface.
    pub fn matches(&self, other: &Self) -> bool {
        match (
            self.vendor_id,
            other.vendor_id,
            self.product_id,
            other.product_id,
        ) {
            (Some(v1), Some(v2), Some(p1), Some(p2)) => v1 == v2 && p1 == p2,
            _ => self.name.is_some() && self.name == other.name,
        }
    }
}

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

/// The source of truth for who is playing and on what.
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
        matches!(self.slots.get(id.0 as usize), Some(SlotState::Occupied { .. }))
    }

    /// The player entity for a slot, if that slot is occupied.
    pub fn player_entity(&self, id: PlayerId) -> Option<Entity> {
        match self.slots.get(id.0 as usize)? {
            SlotState::Occupied { player_entity, .. } => Some(*player_entity),
            SlotState::Empty => None,
        }
    }

    /// What hardware, if any, a slot is driving.
    pub fn device(&self, id: PlayerId) -> Option<PlayerDevice> {
        match self.slots.get(id.0 as usize)? {
            SlotState::Occupied { device, .. } => Some(*device),
            SlotState::Empty => None,
        }
    }

    /// Hardware identity of the controller in a slot — kept even while the
    /// controller is unplugged, so "P2: reconnect your DualSense" still works.
    pub fn fingerprint(&self, id: PlayerId) -> Option<&DeviceFingerprint> {
        match self.slots.get(id.0 as usize)? {
            SlotState::Occupied { fingerprint, .. } => Some(fingerprint),
            SlotState::Empty => None,
        }
    }

    /// Which button glyphs to show this player. See
    /// [`GamepadBrand::button_label`](crate::GamepadBrand::button_label).
    pub fn brand(&self, id: PlayerId) -> crate::GamepadBrand {
        self.fingerprint(id)
            .map(DeviceFingerprint::brand)
            .unwrap_or_default()
    }

    /// Slot that owns a given gamepad entity.
    pub fn player_of_gamepad(&self, gamepad: Entity) -> Option<PlayerId> {
        self.slots.iter().enumerate().find_map(|(i, slot)| match slot {
            SlotState::Occupied {
                device: PlayerDevice::Gamepad(e),
                ..
            } if *e == gamepad => Some(PlayerId(i as u8)),
            _ => None,
        })
    }

    /// Every joined slot, in order.
    pub fn players(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| match slot {
            SlotState::Occupied { .. } => Some(PlayerId(i as u8)),
            SlotState::Empty => None,
        })
    }

    /// Joined slots that currently have hardware, paired with it.
    pub fn assigned(&self) -> impl Iterator<Item = (PlayerId, Entity)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| match slot {
            SlotState::Occupied {
                device: PlayerDevice::Gamepad(e),
                ..
            } => Some((PlayerId(i as u8), *e)),
            _ => None,
        })
    }

    /// Joined players who are currently missing their controller. Show these
    /// in a "reconnect controller" overlay.
    pub fn disconnected_players(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, slot)| match slot {
            SlotState::Occupied { device, .. } if !device.is_connected() => Some(PlayerId(i as u8)),
            _ => None,
        })
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
        if !self.leave_queue.iter().any(|(queued, _)| *queued == id) {
            self.leave_queue.push((id, LeaveReason::Requested));
        }
    }

    pub(crate) fn first_free_slot(&self) -> Option<PlayerId> {
        self.slots
            .iter()
            .position(|s| matches!(s, SlotState::Empty))
            .map(|i| PlayerId(i as u8))
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
            .map(|(i, _)| PlayerId(i as u8))
    }

    /// A disconnected slot whose old controller looks like `fingerprint`.
    pub(crate) fn waiting_slot_matching(&self, fingerprint: &DeviceFingerprint) -> Option<PlayerId> {
        self.slots.iter().enumerate().find_map(|(i, slot)| match slot {
            SlotState::Occupied {
                device,
                fingerprint: known,
                ..
            } if !device.is_connected() && known.matches(fingerprint) => Some(PlayerId(i as u8)),
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
                    lost_at: Some(at), ..
                } if now - *at >= grace_secs as f64 => Some(PlayerId(i as u8)),
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
        if let Some(slot) = self.slots.get_mut(id.0 as usize) {
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
        match self.slots.get_mut(id.0 as usize)? {
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
        match self.slots.get_mut(id.0 as usize)? {
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
        let slot = self.slots.get_mut(id.0 as usize)?;
        match core::mem::replace(slot, SlotState::Empty) {
            SlotState::Occupied {
                player_entity,
                device,
                ..
            } => Some((player_entity, device)),
            SlotState::Empty => None,
        }
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

/// A new player took a slot.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerJoined {
    /// The slot they took.
    pub id: PlayerId,
    /// The spawned player entity — attach your gameplay components to this.
    pub player_entity: Entity,
    /// The gamepad they joined on.
    pub gamepad: Entity,
}

/// A player gave up their slot. The player entity is despawned right after
/// this message is written, so read anything you need from it now.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLeft {
    /// The slot they vacated. It is empty as of this message.
    pub id: PlayerId,
    /// About to be despawned — read what you need from it now.
    pub player_entity: Entity,
    /// Why they left.
    pub reason: LeaveReason,
}

/// Why a player left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveReason {
    /// Held the leave button.
    Requested,
    /// Their controller stayed gone past the reconnect grace period.
    TimedOut,
}

/// A player's controller vanished but their slot is being held.
///
/// This is your cue to pause. Their [`PlayerInput`](crate::PlayerInput) is
/// zeroed automatically so they won't keep walking into a pit.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerDisconnected {
    /// The slot being held open for them.
    pub id: PlayerId,
    /// Still alive — only their hardware went away.
    pub player_entity: Entity,
}

/// A controller was handed back to a slot that was holding it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerReconnected {
    /// The slot that got its controller back.
    pub id: PlayerId,
    /// Unchanged across the disconnect.
    pub player_entity: Entity,
    /// The gamepad entity now driving this player.
    pub gamepad: Entity,
    /// True when this is literally the same hardware as before. False means a
    /// different controller took over the slot, which only happens with
    /// [`ReconnectPolicy::adopt_any`](crate::ReconnectPolicy).
    pub same_device: bool,
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
    roster.player_count() > 0
}
