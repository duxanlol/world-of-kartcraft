//! Karting the profession: on a World of Kartcraft server the kart is a spell. Casting Summon
//! Kart puts its aura on us and the aura is the ride: we are in the kart exactly while it lasts,
//! and `J` casts or cancels it. The kart abilities are auras too, cast from the D-pad (keys 1–4
//! on the keyboard) or the action bar; while one is on us it changes the driving model's
//! modifiers. Each banked turbo chain is reported to the server, which raises the skill and gives
//! experience. On a stock server (no Summon Kart in our spellbook) `J` simply climbs in and out.

use benilla_formats::kartcraft::{
    SPELL_AKU_AKU, SPELL_SUMMON_KART, SPELL_SUPER_ENGINE, SPELL_TURBO_BOOST, SPELL_WUMPA_JUICE,
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::net::{ClientCommand, NetCommands, ObjectStore, SelfPlayer};

/// The abilities by D-pad direction: up, right, down, left.
const DPAD: [u32; 4] = [
    SPELL_WUMPA_JUICE,
    SPELL_TURBO_BOOST,
    SPELL_SUPER_ENGINE,
    SPELL_AKU_AKU,
];

/// The kart auras on us this frame.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct KartAuras {
    pub(super) kart: bool,
    pub(super) juice: bool,
    pub(super) turbo: bool,
    pub(super) engine: bool,
    pub(super) aku: bool,
}

/// What the profession reads and sends from the kart update.
#[derive(SystemParam)]
pub(crate) struct Profession<'w, 's> {
    self_q: Query<'w, 's, &'static ObjectStore, With<SelfPlayer>>,
    actions: Option<Res<'w, crate::ui_action::PlayerActions>>,
    commands: Option<Res<'w, NetCommands>>,
}

impl Profession<'_, '_> {
    /// Whether the kart is a spell here: we know Summon Kart.
    pub(super) fn server_kart(&self) -> bool {
        self.knows(SPELL_SUMMON_KART)
    }

    fn knows(&self, spell: u32) -> bool {
        self.actions
            .as_ref()
            .is_some_and(|a| a.spells.contains(&spell))
    }

    /// Whether Summon Kart is on us, or `None` while we have no object of our own: a worldport
    /// drops it (`SMSG_NEW_WORLD`) until the new map has us, and that is no answer either way.
    pub(super) fn kart_aura(&self) -> Option<bool> {
        self.self_q.single().ok()?;
        Some(self.auras().kart)
    }

    pub(super) fn auras(&self) -> KartAuras {
        let mut auras = KartAuras::default();
        let Ok(store) = self.self_q.single() else {
            return auras;
        };
        for slot in store.0.unit_auras() {
            match slot.spell_id {
                SPELL_SUMMON_KART => auras.kart = true,
                SPELL_WUMPA_JUICE => auras.juice = true,
                SPELL_TURBO_BOOST => auras.turbo = true,
                SPELL_SUPER_ENGINE => auras.engine = true,
                SPELL_AKU_AKU => auras.aku = true,
                _ => {}
            }
        }
        auras
    }

    fn send_command(&self, command: ClientCommand) {
        if let Some(commands) = &self.commands {
            let _ = commands.0.send(command);
        }
    }

    fn cast(&self, spell_id: u32) {
        self.send_command(ClientCommand::CastSpell {
            spell_id,
            target: None,
        });
    }

    pub(super) fn summon(&self) {
        self.cast(SPELL_SUMMON_KART);
    }

    pub(super) fn dismiss(&self) {
        self.send_command(ClientCommand::CancelAura {
            spell_id: SPELL_SUMMON_KART,
        });
    }

    /// Casts the ability for each D-pad direction pressed (up, right, down, left), if known.
    pub(super) fn dpad(&self, pressed: [bool; 4]) {
        for (spell, pressed) in DPAD.into_iter().zip(pressed) {
            if pressed && self.knows(spell) {
                self.cast(spell);
            }
        }
    }

    /// Sends a kart addon line (the server relays state lines to the players around us).
    pub(super) fn send(&self, text: String) {
        self.send_command(ClientCommand::AddonMessage {
            distribution: benilla_ui::script::AddonDistribution::Party,
            text,
        });
    }

    /// Reports a banked chain of `points` for the skill and experience.
    pub(super) fn landed(&self, points: f64, label: &str) {
        info!("kart: banked {label} for {points:.0} points");
        self.send_command(ClientCommand::AddonMessage {
            distribution: benilla_ui::script::AddonDistribution::Party,
            text: format!("KARTCRAFT\tL\t{points:.0}"),
        });
    }
}
