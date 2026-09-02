use bevy_ecs::prelude::*;

/// A stable, zero-based player slot. Slot 0 is "P1" on screen.
///
/// Slots are the whole point of this crate: gamepad `Entity` ids come and go
/// as hardware connects and disconnects, but a `PlayerId` sticks to a human
/// for the whole session.
///
/// ```
/// use bevy_couch_multiplayer::PlayerId;
///
/// assert_eq!(PlayerId::P2.number(), 2);   // for UI
/// assert_eq!(PlayerId::P2.index(), 1);    // for arrays
/// assert_eq!(PlayerId::P2.to_string(), "P2");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(
    /// Zero-based slot index.
    pub u8,
);

impl PlayerId {
    /// Slot 0.
    pub const P1: Self = Self(0);
    /// Slot 1.
    pub const P2: Self = Self(1);
    /// Slot 2.
    pub const P3: Self = Self(2);
    /// Slot 3.
    pub const P4: Self = Self(3);
    /// Slot 4.
    pub const P5: Self = Self(4);
    /// Slot 5.
    pub const P6: Self = Self(5);
    /// Slot 6.
    pub const P7: Self = Self(6);
    /// Slot 7.
    pub const P8: Self = Self(7);

    /// A slot from its zero-based index.
    pub const fn new(slot: u8) -> Self {
        Self(slot)
    }

    /// Zero-based, for indexing your own per-player arrays.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// One-based, for UI: "Player 1", "P2 wins".
    pub const fn number(self) -> u8 {
        self.0.saturating_add(1)
    }

    /// A distinct per-slot colour as sRGB, so you don't have to invent one.
    /// Returned as a plain array to avoid depending on `bevy_color`; feed it
    /// straight to `Color::srgb(r, g, b)`.
    pub fn color_rgb(self) -> [f32; 3] {
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
        PALETTE[self.index() % PALETTE.len()]
    }
}

impl From<u8> for PlayerId {
    fn from(slot: u8) -> Self {
        Self(slot)
    }
}

impl core::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "P{}", self.number())
    }
}

/// Marks the entity that represents a joined player.
///
/// This entity is spawned on join and lives until the player leaves — it
/// deliberately survives controller disconnects, so anything you attach to it
/// (score, character choice, inventory) survives a yanked USB cable.
///
/// It always carries [`PlayerDevice`](crate::PlayerDevice),
/// [`PlayerInput`](crate::PlayerInput), [`Rumble`](crate::Rumble) and
/// [`LeaveHold`](crate::LeaveHold).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Player {
    /// Which slot this entity plays in.
    pub id: PlayerId,
}

impl Player {
    /// A player in the given slot.
    pub const fn new(id: PlayerId) -> Self {
        Self { id }
    }
}

/// Which physical device is currently driving a player.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerDevice {
    /// The gamepad entity Bevy spawned for this controller.
    Gamepad(Entity),
    /// Slot is reserved but has no live hardware right now.
    #[default]
    Missing,
}

impl PlayerDevice {
    /// The live gamepad entity, if there is one.
    pub const fn gamepad(&self) -> Option<Entity> {
        match self {
            Self::Gamepad(e) => Some(*e),
            Self::Missing => None,
        }
    }

    /// Whether hardware is currently attached.
    pub const fn is_connected(&self) -> bool {
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
    /// Whether this fingerprint carries anything worth matching on.
    pub fn is_known(&self) -> bool {
        self.name.is_some() || self.vendor_id.is_some()
    }

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

/// A new player took a slot.
///
/// Read this in [`PreUpdate`](bevy_app::PreUpdate) after
/// [`CouchSystems::Membership`](crate::CouchSystems) to attach your gameplay
/// components before anything in `Update` sees the new player:
///
/// ```
/// use bevy::prelude::*;
/// use bevy_couch_multiplayer::prelude::*;
/// # #[derive(Component)] struct Score(u32);
///
/// fn dress_new_players(mut joined: MessageReader<PlayerJoined>, mut commands: Commands) {
///     for event in joined.read() {
///         commands.entity(event.player_entity).insert(Score(0));
///     }
/// }
///
/// # let mut app = App::new();
/// app.add_systems(PreUpdate, dress_new_players.after(CouchSystems::Membership));
/// ```
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
    /// Held the leave button, or you called
    /// [`PlayerRoster::request_leave`](crate::PlayerRoster::request_leave).
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(name: &str, vendor: u16, product: u16) -> DeviceFingerprint {
        DeviceFingerprint {
            name: Some(name.to_string()),
            vendor_id: Some(vendor),
            product_id: Some(product),
        }
    }

    #[test]
    fn ids_read_naturally() {
        assert_eq!(PlayerId::P1.number(), 1);
        assert_eq!(PlayerId::P4.index(), 3);
        assert_eq!(PlayerId::from(2), PlayerId::P3);
    }

    #[test]
    fn ids_are_ordered_by_slot() {
        let mut ids = vec![PlayerId::P3, PlayerId::P1, PlayerId::P2];
        ids.sort();
        assert_eq!(ids, vec![PlayerId::P1, PlayerId::P2, PlayerId::P3]);
    }

    #[test]
    fn same_hardware_matches() {
        assert!(pad("DualSense", 0x054C, 0x0CE6).matches(&pad("DualSense", 0x054C, 0x0CE6)));
    }

    #[test]
    fn different_product_does_not_match() {
        assert!(!pad("Pad", 0x054C, 0x0CE6).matches(&pad("Pad", 0x054C, 0x09CC)));
    }

    #[test]
    fn falls_back_to_name_without_ids() {
        let named = DeviceFingerprint {
            name: Some("Wireless Controller".into()),
            ..Default::default()
        };
        assert!(named.matches(&named.clone()));
        assert!(!named.matches(&DeviceFingerprint::default()));
    }

    #[test]
    fn anonymous_devices_never_match() {
        // Otherwise every unidentifiable pad would claim the first held slot.
        assert!(!DeviceFingerprint::default().matches(&DeviceFingerprint::default()));
    }
}
