//! Guessing what kind of controller a player is holding.
//!
//! In couch co-op the players are looking at *one* screen, so a single "press
//! A" prompt is wrong for at least one of them. `GamepadBrand` lets you label
//! prompts per player instead.

use bevy_input::prelude::GamepadButton;

use crate::player::DeviceFingerprint;

/// USB vendor ids, as reported by the OS.
pub mod vendor {
    /// Sony Interactive Entertainment (DualShock, DualSense).
    pub const SONY: u16 = 0x054C;
    /// Microsoft (Xbox pads).
    pub const MICROSOFT: u16 = 0x045E;
    /// Nintendo (Switch Pro, Joy-Con).
    pub const NINTENDO: u16 = 0x057E;
    /// Valve (Steam Controller, Steam Deck).
    pub const VALVE: u16 = 0x28DE;
    /// 8BitDo.
    pub const EIGHT_BITDO: u16 = 0x2DC8;
}

/// The button-label family a controller belongs to.
///
/// Bevy names buttons by *position* (`South`, `East`, ...) which is the right
/// call for gameplay code — but exactly the wrong thing to show a player,
/// because the same position is A, ✕, or B depending on the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GamepadBrand {
    /// Xbox and the many pads that pretend to be one. A/B/X/Y.
    Xbox,
    /// PlayStation. ✕/○/□/△.
    PlayStation,
    /// Nintendo. A/B/X/Y, but mirrored versus Xbox.
    Nintendo,
    /// Steam Controller / Steam Deck. Xbox labels.
    Steam,
    /// Unrecognised. Falls back to Xbox labels, the most common default.
    #[default]
    Generic,
}

impl GamepadBrand {
    /// Best guess from a vendor id, falling back to the device name.
    ///
    /// Vendor id is checked first because names are wildly inconsistent
    /// between platforms and driver stacks.
    pub fn detect(name: Option<&str>, vendor_id: Option<u16>) -> Self {
        match vendor_id {
            Some(vendor::SONY) => return Self::PlayStation,
            Some(vendor::MICROSOFT) => return Self::Xbox,
            Some(vendor::NINTENDO) => return Self::Nintendo,
            Some(vendor::VALVE) => return Self::Steam,
            _ => {}
        }

        let Some(name) = name else {
            return Self::Generic;
        };
        let lower = name.to_ascii_lowercase();

        const PLAYSTATION: [&str; 6] = [
            "dualsense",
            "dualshock",
            "playstation",
            "ps3",
            "ps4",
            "ps5",
        ];
        const XBOX: [&str; 3] = ["xbox", "xinput", "x-box"];
        const NINTENDO: [&str; 4] = ["nintendo", "switch", "joy-con", "joycon"];
        const STEAM: [&str; 2] = ["steam", "valve"];

        if PLAYSTATION.iter().any(|k| lower.contains(k)) {
            Self::PlayStation
        } else if XBOX.iter().any(|k| lower.contains(k)) {
            Self::Xbox
        } else if NINTENDO.iter().any(|k| lower.contains(k)) {
            Self::Nintendo
        } else if STEAM.iter().any(|k| lower.contains(k)) {
            Self::Steam
        } else {
            Self::Generic
        }
    }

    /// What this player should see written on screen for a button.
    ///
    /// Note the Nintendo mapping: the *south* button is physically labelled
    /// "B" on a Switch Pro controller, and east is "A". Showing "press A" to a
    /// Switch player when you meant `GamepadButton::South` sends them to the
    /// wrong button.
    pub fn button_label(self, button: GamepadButton) -> &'static str {
        use GamepadButton::*;

        // The trailing wildcard is deliberate: it keeps this compiling if
        // `GamepadButton` ever grows a variant.
        #[allow(unreachable_patterns)]
        match (self, button) {
            (Self::PlayStation, South) => "✕",
            (Self::PlayStation, East) => "○",
            (Self::PlayStation, North) => "△",
            (Self::PlayStation, West) => "□",
            (Self::PlayStation, LeftTrigger) => "L1",
            (Self::PlayStation, LeftTrigger2) => "L2",
            (Self::PlayStation, RightTrigger) => "R1",
            (Self::PlayStation, RightTrigger2) => "R2",
            (Self::PlayStation, LeftThumb) => "L3",
            (Self::PlayStation, RightThumb) => "R3",
            (Self::PlayStation, Select) => "Create",
            (Self::PlayStation, Start) => "Options",

            (Self::Nintendo, South) => "B",
            (Self::Nintendo, East) => "A",
            (Self::Nintendo, North) => "X",
            (Self::Nintendo, West) => "Y",
            (Self::Nintendo, LeftTrigger) => "L",
            (Self::Nintendo, LeftTrigger2) => "ZL",
            (Self::Nintendo, RightTrigger) => "R",
            (Self::Nintendo, RightTrigger2) => "ZR",
            (Self::Nintendo, Select) => "-",
            (Self::Nintendo, Start) => "+",

            // Xbox / Steam / Generic all share the Xbox vocabulary.
            (_, South) => "A",
            (_, East) => "B",
            (_, North) => "Y",
            (_, West) => "X",
            (_, LeftTrigger) => "LB",
            (_, LeftTrigger2) => "LT",
            (_, RightTrigger) => "RB",
            (_, RightTrigger2) => "RT",
            (_, LeftThumb) => "LS",
            (_, RightThumb) => "RS",
            (_, Select) => "View",
            (_, Start) => "Menu",

            (_, DPadUp) => "D-Up",
            (_, DPadDown) => "D-Down",
            (_, DPadLeft) => "D-Left",
            (_, DPadRight) => "D-Right",
            (_, Mode) => "Guide",
            (_, C) => "C",
            (_, Z) => "Z",
            (_, Other(_)) => "?",

            _ => "?",
        }
    }

    /// Short human-readable family name, for "P2 — PlayStation" style UI.
    pub fn name(self) -> &'static str {
        match self {
            Self::Xbox => "Xbox",
            Self::PlayStation => "PlayStation",
            Self::Nintendo => "Nintendo",
            Self::Steam => "Steam",
            Self::Generic => "Gamepad",
        }
    }
}

impl DeviceFingerprint {
    /// Which button-label family this controller belongs to.
    pub fn brand(&self) -> GamepadBrand {
        GamepadBrand::detect(self.name.as_deref(), self.vendor_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_id_wins_over_name() {
        // Plenty of third-party pads put "Xbox" in the product string while
        // reporting their own vendor id, so ids are checked first.
        let brand = GamepadBrand::detect(Some("Xbox-style pad"), Some(vendor::SONY));
        assert_eq!(brand, GamepadBrand::PlayStation);
    }

    #[test]
    fn falls_back_to_name() {
        assert_eq!(
            GamepadBrand::detect(Some("Wireless DualSense Controller"), None),
            GamepadBrand::PlayStation
        );
        assert_eq!(
            GamepadBrand::detect(Some("Pro Controller (Switch)"), None),
            GamepadBrand::Nintendo
        );
        assert_eq!(GamepadBrand::detect(Some("weird thing"), None), GamepadBrand::Generic);
    }

    #[test]
    fn nintendo_south_is_b() {
        assert_eq!(GamepadBrand::Nintendo.button_label(GamepadButton::South), "B");
        assert_eq!(GamepadBrand::Xbox.button_label(GamepadButton::South), "A");
        assert_eq!(GamepadBrand::PlayStation.button_label(GamepadButton::South), "✕");
    }

    #[test]
    fn unknown_brand_uses_xbox_labels() {
        assert_eq!(GamepadBrand::Generic.button_label(GamepadButton::East), "B");
    }
}
