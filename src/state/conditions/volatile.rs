//! The volatile conditions a Pokémon holds (`VolatileState`, DESIGN.md §30) and their core —
//! what is there, what goes, what passes on, the end of the turn's counting down — with the
//! accessors of the conditions that are neither a restriction (`volatile_choices.rs`), a lock
//! (`volatile_locks.rs`) nor a trap (`volatile_traps.rs`); their kinds are in
//! `volatile_kinds.rs`.

use super::*;
use crate::ids::{ItemId, MoveId, PokemonId, PositionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolatileState {
    pub(super) entries: Vec<VolatileCondition>,
}

impl VolatileState {
    pub fn entries(&self) -> &[VolatileCondition] {
        &self.entries
    }

    pub fn contains(&self, kind: VolatileConditionKind) -> bool {
        self.entries.iter().any(|entry| entry.kind() == kind)
    }

    pub fn set_protect(&mut self, source: PokemonId, turn: u32, kind: ProtectKind) {
        self.remove(VolatileConditionKind::Protect);
        self.entries.push(VolatileCondition::Protect {
            source,
            started_turn: turn,
            kind,
        });
    }

    /// The protecting move behind the `Protect` volatile, if any.
    pub fn protect_kind(&self) -> Option<ProtectKind> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Protect { kind, .. } => Some(*kind),
            _ => None,
        })
    }

    /// Dynamax (DESIGN.md §44.5): the volatile with its counter and the max HP it doubled.
    pub fn set_dynamax(&mut self, base_max_hp: u16) {
        self.remove(VolatileConditionKind::Dynamax);
        self.entries.push(VolatileCondition::Dynamax {
            turns: 0,
            base_max_hp,
        });
    }

    /// The Dynamax volatile's `effectState.turns`, counted at the residual.
    pub fn dynamax_turns(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Dynamax { turns, .. } => Some(*turns),
            _ => None,
        })
    }

    /// The max HP before the doubling (`baseMaxhp`).
    pub fn dynamax_base_max_hp(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Dynamax { base_max_hp, .. } => Some(*base_max_hp),
            _ => None,
        })
    }

    /// A permanent forme change's new `baseMaxhp` under the Dynamax (`updateMaxHp`).
    pub fn set_dynamax_base_max_hp(&mut self, value: u16) {
        for entry in &mut self.entries {
            if let VolatileCondition::Dynamax { base_max_hp, .. } = entry {
                *base_max_hp = value;
            }
        }
    }

    /// The counter's next value; `None` when the volatile is gone.
    pub fn count_a_dynamax_turn(&mut self) -> Option<u8> {
        self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::Dynamax { turns, .. } => {
                *turns = turns.saturating_add(1);
                Some(*turns)
            }
            _ => None,
        })
    }

    pub fn set_endure(&mut self, turn: u32) {
        self.remove(VolatileConditionKind::Endure);
        self.entries
            .push(VolatileCondition::Endure { started_turn: turn });
    }

    pub fn mark_substitute_broken(&mut self, by: MoveId) {
        self.remove(VolatileConditionKind::SubstituteBroken);
        self.entries
            .push(VolatileCondition::SubstituteBroken { by });
    }

    /// Gen 2 Toxic (`residualdmg` `onStart`): the counter starts over at 0, the volatile is
    /// added if missing. Returns whether it was added.
    pub fn start_residual_damage(&mut self) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::ResidualDamage { counter } = entry {
                *counter = 0;
                return false;
            }
        }
        self.entries
            .push(VolatileCondition::ResidualDamage { counter: 0 });
        true
    }

    pub fn residual_damage_counter(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::ResidualDamage { counter } => Some(*counter),
            _ => None,
        })
    }

    /// One after-action tick of a statused holder (`residualdmg` `onAfterMoveSelf`).
    pub fn advance_residual_damage(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::ResidualDamage { counter } = entry {
                *counter = counter.saturating_add(1);
            }
        }
    }

    pub fn substitute_broken_by(&self) -> Option<MoveId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SubstituteBroken { by } => Some(*by),
            _ => None,
        })
    }

    pub fn set_rage_powder(&mut self) -> bool {
        if self.contains(VolatileConditionKind::RagePowder) {
            return false;
        }
        self.entries.push(VolatileCondition::RagePowder);
        true
    }

    pub fn set_follow_me(&mut self) -> bool {
        if self.contains(VolatileConditionKind::FollowMe) {
            return false;
        }
        self.entries.push(VolatileCondition::FollowMe);
        true
    }

    /// Showdown `helpinghand` onStart (×1.5) / onRestart (`multiplier *= 1.5`).
    pub fn add_helping_hand(&mut self) {
        for entry in &mut self.entries {
            if let VolatileCondition::HelpingHand { multiplier } = entry {
                *multiplier = *multiplier * 3 / 2;
                return;
            }
        }
        self.entries
            .push(VolatileCondition::HelpingHand { multiplier: 6144 });
    }

    pub fn helping_hand_multiplier(&self) -> Option<u32> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::HelpingHand { multiplier } => Some(*multiplier),
            _ => None,
        })
    }

    /// Zen Mode's volatile (`addVolatile('zenmode')`): false when the holder already has it
    /// (Showdown's `addVolatile` then runs no `onStart`, so the forme stays as it is).
    pub fn set_zen_mode(&mut self) -> bool {
        if self.contains(VolatileConditionKind::ZenMode) {
            return false;
        }
        self.entries.push(VolatileCondition::ZenMode);
        true
    }

    /// Quark Drive's or Protosynthesis's volatile (`kind`) with the stat it raises, started by
    /// Booster Energy or not; false when the holder already has it (`addVolatile` fails: the
    /// stat stays the one picked first) or `kind` is neither.
    pub fn set_paradox_boost(
        &mut self,
        kind: VolatileConditionKind,
        best: crate::state::Stat,
        from_booster: bool,
    ) -> bool {
        let entry = match kind {
            VolatileConditionKind::QuarkDrive => {
                VolatileCondition::QuarkDrive { best, from_booster }
            }
            VolatileConditionKind::Protosynthesis => {
                VolatileCondition::Protosynthesis { best, from_booster }
            }
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// The stat Quark Drive or Protosynthesis raises, with the volatile, while it does.
    pub fn paradox_boost(&self) -> Option<(VolatileConditionKind, crate::state::Stat)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::QuarkDrive { best, .. } => {
                Some((VolatileConditionKind::QuarkDrive, *best))
            }
            VolatileCondition::Protosynthesis { best, .. } => {
                Some((VolatileConditionKind::Protosynthesis, *best))
            }
            _ => None,
        })
    }

    /// Whether the Quark Drive or Protosynthesis volatile came from Booster Energy.
    pub fn paradox_boost_from_booster(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(
                entry,
                VolatileCondition::QuarkDrive {
                    from_booster: true,
                    ..
                } | VolatileCondition::Protosynthesis {
                    from_booster: true,
                    ..
                }
            )
        })
    }

    /// Gen 1 Reflect / Light Screen, Gen 1-2 Mist on the user; false when it already has it
    /// (Gen 1's `onTryHit`, `addVolatile` failing) or `kind` is none of them.
    pub fn set_user_guard(&mut self, kind: VolatileConditionKind) -> bool {
        let entry = match kind {
            VolatileConditionKind::Reflect => VolatileCondition::Reflect,
            VolatileConditionKind::LightScreen => VolatileCondition::LightScreen,
            VolatileConditionKind::Mist => VolatileCondition::Mist,
            VolatileConditionKind::MudSport => VolatileCondition::MudSport,
            VolatileConditionKind::WaterSport => VolatileCondition::WaterSport,
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Yawning for `turns`, at the doing of `source`'s side; false when already yawning.
    pub fn set_yawn(&mut self, turns: u8, source: Option<crate::ids::SideId>) -> bool {
        if self.contains(VolatileConditionKind::Yawn) {
            return false;
        }
        self.entries.push(VolatileCondition::Yawn { turns, source });
        true
    }

    /// The side whose Pokémon yawned at the holder.
    pub fn yawn_source(&self) -> Option<crate::ids::SideId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Yawn { source, .. } => *source,
            _ => None,
        })
    }

    /// Nightmare; false when already having one.
    pub fn set_nightmare(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Nightmare) {
            return false;
        }
        self.entries.push(VolatileCondition::Nightmare);
        true
    }

    /// Magnet Rise for `turns`; false when already there.
    pub fn set_magnet_rise(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::MagnetRise) {
            return false;
        }
        self.entries.push(VolatileCondition::MagnetRise { turns });
        true
    }

    /// Telekinesis for `turns`; false when already there.
    pub fn set_telekinesis(&mut self, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::Telekinesis) {
            return false;
        }
        self.entries.push(VolatileCondition::Telekinesis { turns });
        true
    }

    /// A volatile without state of its own (Salt Cure, Tar Shot, Defense Curl, Aqua Ring,
    /// Gastro Acid, Sparkling Aria's mark); false
    /// when already there.
    pub fn set_marker(&mut self, kind: VolatileConditionKind) -> bool {
        let entry = match kind {
            VolatileConditionKind::SaltCure => VolatileCondition::SaltCure,
            VolatileConditionKind::TarShot => VolatileCondition::TarShot,
            VolatileConditionKind::Unburden => VolatileCondition::Unburden,
            VolatileConditionKind::DefenseCurl => VolatileCondition::DefenseCurl,
            VolatileConditionKind::AquaRing => VolatileCondition::AquaRing,
            VolatileConditionKind::PowerTrick => VolatileCondition::PowerTrick,
            VolatileConditionKind::GastroAcid => VolatileCondition::GastroAcid,
            VolatileConditionKind::FocusEnergy => VolatileCondition::FocusEnergy,
            VolatileConditionKind::Charge => VolatileCondition::Charge,
            VolatileConditionKind::NoRetreat => VolatileCondition::NoRetreat,
            VolatileConditionKind::Minimize => VolatileCondition::Minimize,
            VolatileConditionKind::Roost => VolatileCondition::Roost,
            VolatileConditionKind::ZenMode => VolatileCondition::ZenMode,
            VolatileConditionKind::Commanding => VolatileCondition::Commanding,
            VolatileConditionKind::Commanded => VolatileCondition::Commanded,
            VolatileConditionKind::Powder => VolatileCondition::Powder,
            VolatileConditionKind::Electrify => VolatileCondition::Electrify,
            VolatileConditionKind::Foresight => VolatileCondition::Foresight,
            VolatileConditionKind::MiracleEye => VolatileCondition::MiracleEye,
            VolatileConditionKind::SmackDown => VolatileCondition::SmackDown,
            VolatileConditionKind::FlashFire => VolatileCondition::FlashFire,
            VolatileConditionKind::SparklingAria => VolatileCondition::SparklingAria,
            VolatileConditionKind::DestinyBond => VolatileCondition::DestinyBond,
            VolatileConditionKind::Grudge => VolatileCondition::Grudge,
            VolatileConditionKind::Spotlight => VolatileCondition::Spotlight,
            VolatileConditionKind::Torment => VolatileCondition::Torment,
            VolatileConditionKind::Imprison => VolatileCondition::Imprison,
            VolatileConditionKind::LockedOn => VolatileCondition::LockedOn,
            VolatileConditionKind::Curse => VolatileCondition::Curse,
            VolatileConditionKind::GlaiveRush => VolatileCondition::GlaiveRush,
            VolatileConditionKind::Truant => VolatileCondition::Truant,
            _ => return false,
        };
        if self.contains(kind) {
            return false;
        }
        self.entries.push(entry);
        true
    }

    /// Pursuit's mark (`addVolatile('pursuit')`, then `sources.push`): `source` joins the
    /// holder's pursuers.
    pub fn mark_pursuit(&mut self, source: PokemonId) -> bool {
        for entry in &mut self.entries {
            if let VolatileCondition::Pursuit { sources } = entry {
                return match sources.iter_mut().find(|slot| slot.is_none()) {
                    Some(slot) => {
                        *slot = Some(source);
                        true
                    }
                    None => false,
                };
            }
        }
        self.entries.push(VolatileCondition::Pursuit {
            sources: [Some(source), None, None],
        });
        true
    }

    /// The holder's pursuers, in marking order.
    pub fn pursuers(&self) -> Vec<PokemonId> {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Pursuit { sources } => {
                    Some(sources.iter().flatten().copied().collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Lock-On on its user: the target and the turns left (Gen 3+).
    pub fn lock_on(&self) -> Option<(PokemonId, u8)> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::LockOn { target, turns } => Some((*target, *turns)),
            _ => None,
        })
    }

    /// Sets Lock-On on its user anew, the one there replaced.
    pub fn set_lock_on(&mut self, target: PokemonId, turns: u8) {
        self.remove(VolatileConditionKind::LockOn);
        self.entries
            .push(VolatileCondition::LockOn { target, turns });
    }

    /// A Lock-On on `from` follows it to `to` (Gen 3-4 Baton Pass), its turns kept or set.
    pub fn retarget_lock_on(&mut self, from: PokemonId, to: PokemonId, turns: Option<u8>) {
        for entry in &mut self.entries {
            if let VolatileCondition::LockOn {
                target,
                turns: left,
            } = entry
                && *target == from
            {
                *target = to;
                if let Some(turns) = turns {
                    *left = turns;
                }
            }
        }
    }

    /// The Pokémon that threw the holder's Syrup Bomb.
    pub fn syrup_bomb_source(&self) -> Option<PokemonId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::SyrupBomb { source, .. } => Some(*source),
            _ => None,
        })
    }

    /// Dragon Cheer adding `stages`; false when already there.
    pub fn set_dragon_cheer(&mut self, stages: u8) -> bool {
        if self.contains(VolatileConditionKind::DragonCheer) {
            return false;
        }
        self.entries.push(VolatileCondition::DragonCheer { stages });
        true
    }

    /// The critical-hit stages Dragon Cheer adds (0 without it).
    pub fn dragon_cheer_stages(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::DragonCheer { stages } => Some(*stages),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Fling's mark: the holder threw `item` and loses it at the next `Update`.
    pub fn set_fling(&mut self, item: ItemId) {
        self.remove(VolatileConditionKind::Fling);
        self.entries.push(VolatileCondition::Fling { item });
    }

    /// The item Fling's mark says the holder threw.
    pub fn flung_item(&self) -> Option<ItemId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Fling { item } => Some(*item),
            _ => None,
        })
    }

    /// Micle Berry's boost, for `turns` (none: until used); false when it already has one
    /// (`addVolatile` fails, no `onRestart`).
    pub fn set_micle_berry(&mut self, turns: Option<u8>) -> bool {
        if self.contains(VolatileConditionKind::MicleBerry) {
            return false;
        }
        self.entries.push(VolatileCondition::MicleBerry { turns });
        true
    }

    /// Laser Focus for `turns`, started or restarted (Showdown `onRestart`: `duration = 2`).
    pub fn set_laser_focus(&mut self, turns: u8) {
        self.remove(VolatileConditionKind::LaserFocus);
        self.entries.push(VolatileCondition::LaserFocus { turns });
    }

    /// Stockpile's layers (0 without one).
    /// How many times Autotomize took 100 kg off the holder (0 without it).
    pub fn autotomize_count(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Autotomize { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Autotomize once more (the first one starts it): `lighter` counts another 100 kg off.
    pub fn add_autotomize(&mut self, lighter: bool) {
        for entry in &mut self.entries {
            if let VolatileCondition::Autotomize { count } = entry {
                *count += u8::from(lighter);
                return;
            }
        }
        self.entries.push(VolatileCondition::Autotomize {
            count: u8::from(lighter),
        });
    }

    /// Gen 6+ Transform: the holder's Autotomize count becomes `count` (none at 0).
    pub fn set_autotomize_count(&mut self, count: u8) {
        self.remove(VolatileConditionKind::Autotomize);
        if count > 0 {
            self.entries.push(VolatileCondition::Autotomize { count });
        }
    }

    pub fn stockpile_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                VolatileCondition::Stockpile { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// One more Stockpile layer (the first one starts it); returns the layers.
    pub fn add_stockpile_layer(&mut self) -> u8 {
        for entry in &mut self.entries {
            if let VolatileCondition::Stockpile { layers, .. } = entry {
                *layers += 1;
                return *layers;
            }
        }
        self.entries.push(VolatileCondition::Stockpile {
            layers: 1,
            defense: 0,
            special_defense: 0,
        });
        1
    }

    /// Stockpile's end will take back one more stage of `defense` / `special_defense`.
    pub fn count_stockpile_stages(&mut self, defense: bool, special_defense: bool) {
        for entry in &mut self.entries {
            if let VolatileCondition::Stockpile {
                defense: def,
                special_defense: spd,
                ..
            } = entry
            {
                *def -= i8::from(defense);
                *spd -= i8::from(special_defense);
            }
        }
    }

    /// Removes Stockpile; the stages its end takes back (Defense, Special Defense).
    pub fn take_stockpile(&mut self) -> Option<(i8, i8)> {
        let taken = self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Stockpile {
                defense,
                special_defense,
                ..
            } => Some((*defense, *special_defense)),
            _ => None,
        })?;
        self.remove(VolatileConditionKind::Stockpile);
        Some(taken)
    }

    /// Syrup Bomb thrown by `source`, for `turns`; false when already there.
    pub fn set_syrup_bomb(&mut self, source: PokemonId, turns: u8) -> bool {
        if self.contains(VolatileConditionKind::SyrupBomb) {
            return false;
        }
        self.entries
            .push(VolatileCondition::SyrupBomb { source, turns });
        true
    }

    /// Perish Song's count (Showdown `duration: 4`); false when one is already running.
    pub fn set_perish_song(&mut self) -> bool {
        if self.contains(VolatileConditionKind::PerishSong) {
            return false;
        }
        self.entries
            .push(VolatileCondition::PerishSong { turns: 4 });
        true
    }

    /// One residual of Perish Song: the count left, `Some(0)` when it ran out (the volatile is
    /// then gone); `None` without the volatile.
    pub fn tick_perish_song(&mut self) -> Option<u8> {
        let left = self.entries.iter_mut().find_map(|entry| match entry {
            VolatileCondition::PerishSong { turns } => {
                *turns = turns.saturating_sub(1);
                Some(*turns)
            }
            _ => None,
        })?;
        if left == 0 {
            self.remove(VolatileConditionKind::PerishSong);
        }
        Some(left)
    }

    /// Infatuated with `source`; false when already infatuated.
    pub fn set_attract(&mut self, source: PokemonId) -> bool {
        if self.contains(VolatileConditionKind::Attract) {
            return false;
        }
        self.entries.push(VolatileCondition::Attract { source });
        true
    }

    pub fn stall_counter(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Stall { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// Ally Switch's own counter (Gen 9): `addVolatile('allyswitch')` starting at 3 and
    /// multiplying by 3 each consecutive use, capped at 729.
    pub fn ally_switch_counter(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::AllySwitch { counter, .. } => Some(*counter),
            _ => None,
        })
    }

    /// Showdown `allyswitch`'s `onStart` (3) and `onRestart` (×3, at most 729): a fresh counter
    /// or the next one, with the duration reset. False when the chance failed (no counter).
    pub fn bump_ally_switch(&mut self, turns: u8) -> u16 {
        for entry in &mut self.entries {
            if let VolatileCondition::AllySwitch {
                counter,
                turns: left,
            } = entry
            {
                *counter = (*counter * 3).min(729);
                *left = turns;
                return *counter;
            }
        }
        self.entries
            .push(VolatileCondition::AllySwitch { counter: 3, turns });
        3
    }

    /// `onRestart`'s failure: the volatile goes (the move fails with it).
    pub fn remove_ally_switch(&mut self) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| !matches!(entry, VolatileCondition::AllySwitch { .. }));
        self.entries.len() != before
    }

    /// Showdown `stall` onStart / onRestart: a fresh counter (`start`), or the live one
    /// advanced by `next` with its duration reset.
    pub fn bump_stall(&mut self, start: u16, next: impl FnOnce(u16) -> u16, turns: u8) -> u16 {
        for entry in &mut self.entries {
            if let VolatileCondition::Stall {
                counter,
                turns: left,
            } = entry
            {
                *counter = next(*counter);
                *left = turns;
                return *counter;
            }
        }
        self.entries.push(VolatileCondition::Stall {
            counter: start,
            turns,
        });
        start
    }

    /// Remaining turns of a timed condition (Taunt / Encore / Disable).
    pub fn turns_left(&self, kind: VolatileConditionKind) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Taunt { turns } if kind == VolatileConditionKind::Taunt => {
                Some(*turns)
            }
            VolatileCondition::Encore { turns, .. } if kind == VolatileConditionKind::Encore => {
                Some(*turns)
            }
            VolatileCondition::Disable { turns, .. } if kind == VolatileConditionKind::Disable => {
                Some(*turns)
            }
            VolatileCondition::TwoTurnMove { turns, .. }
                if kind == VolatileConditionKind::TwoTurnMove =>
            {
                Some(*turns)
            }
            VolatileCondition::PartiallyTrapped { turns, .. }
                if kind == VolatileConditionKind::PartiallyTrapped =>
            {
                Some(*turns)
            }
            VolatileCondition::Stall { turns, .. } if kind == VolatileConditionKind::Stall => {
                Some(*turns)
            }
            VolatileCondition::PartialTrappingLock { turns, .. }
                if kind == VolatileConditionKind::PartialTrappingLock =>
            {
                Some(*turns)
            }
            VolatileCondition::FakePartiallyTrapped { turns, .. }
                if kind == VolatileConditionKind::FakePartiallyTrapped =>
            {
                Some(*turns)
            }
            _ => None,
        })
    }

    /// Showdown `duration--` in the residual schedule: returns the turns left, removing the
    /// condition when it reaches 0 (the caller traces the end).
    pub fn tick_timed(&mut self, kind: VolatileConditionKind) -> Option<u8> {
        let index = self.entries.iter().position(|entry| entry.kind() == kind)?;
        let turns = timed_turns(&mut self.entries[index])?;
        *turns = turns.saturating_sub(1);
        let left = *turns;
        if left == 0 {
            self.entries.remove(index);
        }
        Some(left)
    }

    /// The turns a condition `tick_timed` counts down has left before its next `duration--`:
    /// 1 where the next residual ends it (R-M10-66).
    pub fn timed_turns_left(&self, kind: VolatileConditionKind) -> Option<u8> {
        let mut entry = *self.entries.iter().find(|entry| entry.kind() == kind)?;
        timed_turns(&mut entry).map(|turns| *turns)
    }

    /// Chilly Reception's announcement for the turn (`addVolatile('chillyreception')`).
    pub fn set_chilly_reception(&mut self) -> bool {
        self.set_turn_start(VolatileConditionKind::ChillyReception)
    }

    /// A move's turn-start volatile (`priorityChargeCallback` / `beforeTurnCallback`: Chilly
    /// Reception, Focus Punch, Beak Blast, Shell Trap), for the turn; false when already there.
    pub fn set_turn_start(&mut self, kind: VolatileConditionKind) -> bool {
        if self.contains(kind) {
            return false;
        }
        let entry = match kind {
            VolatileConditionKind::ChillyReception => {
                VolatileCondition::ChillyReception { turns: 1 }
            }
            VolatileConditionKind::FocusPunch => VolatileCondition::FocusPunch {
                lost_focus: false,
                turns: 1,
            },
            VolatileConditionKind::BeakBlast => VolatileCondition::BeakBlast { turns: 1 },
            VolatileConditionKind::ShellTrap => VolatileCondition::ShellTrap {
                got_hit: false,
                turns: 1,
            },
            VolatileConditionKind::Counter => VolatileCondition::Counter { turns: 1 },
            VolatileConditionKind::MirrorCoat => VolatileCondition::MirrorCoat { turns: 1 },
            _ => return false,
        };
        self.entries.push(entry);
        true
    }

    /// Showdown `addVolatile('toxic')`: fails while one is already there.
    pub fn set_sure_hit(&mut self) -> bool {
        if self.contains(VolatileConditionKind::SureHit) {
            return false;
        }
        self.entries.push(VolatileCondition::SureHit { turns: 1 });
        true
    }

    /// Showdown `addVolatile('mefirst')`: the boost marker for the move Me First calls.
    pub fn set_me_first(&mut self) -> bool {
        if self.contains(VolatileConditionKind::MeFirst) {
            return false;
        }
        self.entries.push(VolatileCondition::MeFirst { turns: 1 });
        true
    }

    /// Showdown `addVolatile('snatch')`: like the coat, for the turn; a second is no failure.
    pub fn set_snatch(&mut self) -> bool {
        if self.contains(VolatileConditionKind::Snatch) {
            return false;
        }
        self.entries.push(VolatileCondition::Snatch { turns: 1 });
        true
    }

    /// Showdown `addVolatile('magiccoat')`: the move announces itself and stays for the turn.
    /// A second Magic Coat is no failure (`addVolatile` restarts it: the duration is the same).
    pub fn set_magic_coat(&mut self, prankster_boosted: bool) -> bool {
        if self.contains(VolatileConditionKind::MagicCoat) {
            return false;
        }
        self.entries.push(VolatileCondition::MagicCoat {
            turns: 1,
            prankster_boosted,
        });
        true
    }

    /// Whether the Magic Coat on this Pokémon was Prankster-boosted.
    pub fn magic_coat_prankster(&self) -> bool {
        self.entries.iter().any(|volatile| {
            matches!(
                volatile,
                VolatileCondition::MagicCoat {
                    prankster_boosted: true,
                    ..
                }
            )
        })
    }

    pub fn set_leech_seed(&mut self, source: PokemonId, source_position: PositionId) -> bool {
        if self.contains(VolatileConditionKind::LeechSeed) {
            return false;
        }
        self.entries.push(VolatileCondition::LeechSeed {
            source,
            source_position,
        });
        true
    }

    pub fn leech_seed_source_position(&self) -> Option<PositionId> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::LeechSeed {
                source_position, ..
            } => Some(*source_position),
            _ => None,
        })
    }

    pub fn set_substitute(&mut self, source: PokemonId, hp: u16) {
        self.remove(VolatileConditionKind::Substitute);
        self.entries
            .push(VolatileCondition::Substitute { source, hp });
    }

    pub fn set_flinch(&mut self, source: PokemonId) {
        self.remove(VolatileConditionKind::Flinch);
        self.entries.push(VolatileCondition::Flinch { source });
    }

    /// Returns false when already confused (Showdown: the effect fails to stack).
    pub fn set_confusion(&mut self, source: PokemonId, time: u16) -> bool {
        if self.contains(VolatileConditionKind::Confusion) {
            return false;
        }
        self.entries
            .push(VolatileCondition::Confusion { source, time });
        true
    }

    pub fn confusion_time(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Confusion { time, .. } => Some(*time),
            _ => None,
        })
    }

    /// Decrements the confusion counter. Returns the new value; `Some(0)` means the confusion
    /// just ended and has been removed.
    pub fn tick_confusion(&mut self) -> Option<u16> {
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, VolatileCondition::Confusion { .. }))?;
        let remaining = match &mut self.entries[index] {
            VolatileCondition::Confusion { time, .. } => {
                *time = time.saturating_sub(1);
                *time
            }
            _ => unreachable!(),
        };
        if remaining == 0 {
            self.entries.remove(index);
        }
        Some(remaining)
    }

    pub fn substitute_hp(&self) -> Option<u16> {
        self.entries.iter().find_map(|entry| match entry {
            VolatileCondition::Substitute { hp, .. } => Some(*hp),
            _ => None,
        })
    }

    /// Applies damage to a substitute. Returns `(applied, before, after, broken)` when present.
    pub fn damage_substitute(&mut self, amount: u16) -> Option<(u16, u16, u16, bool)> {
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, VolatileCondition::Substitute { .. }))?;

        let (applied, before, after, broken) = match &mut self.entries[index] {
            VolatileCondition::Substitute { hp, .. } => {
                let before = *hp;
                let applied = amount.min(before);
                *hp -= applied;
                (applied, before, *hp, *hp == 0)
            }
            _ => unreachable!(),
        };

        if broken {
            self.entries.remove(index);
        }

        Some((applied, before, after, broken))
    }

    pub fn remove(&mut self, kind: VolatileConditionKind) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.kind() != kind);
        before != self.entries.len()
    }

    /// Volatile state is cleared when a Pokémon leaves the active field.
    /// Baton Pass / Shed Tail (Showdown `copyVolatileFrom`): the entries `passed` lets through,
    /// as they stand (counts, turns, the Substitute's HP).
    pub fn passed_on(
        &self,
        passed: impl Fn(VolatileConditionKind) -> bool,
    ) -> Vec<VolatileCondition> {
        self.entries
            .iter()
            .copied()
            .filter(|entry| passed(entry.kind()))
            .collect()
    }

    /// The entries handed on by the Pokémon this one replaced.
    pub fn take_over(&mut self, entries: &[VolatileCondition]) {
        for entry in entries {
            self.remove(entry.kind());
            self.entries.push(*entry);
        }
    }

    pub fn clear_on_switch(&mut self) -> Vec<VolatileConditionKind> {
        let expired = self.entries.iter().map(|entry| entry.kind()).collect();
        self.entries.clear();
        expired
    }

    /// End-of-turn lifecycle for turn-scoped volatile conditions.
    pub fn expire_end_turn(&mut self, current_turn: u32) -> Vec<VolatileConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain(|entry| {
            let should_expire = matches!(
                entry,
                VolatileCondition::Protect { started_turn, .. }
                    | VolatileCondition::Endure { started_turn }
                    if *started_turn <= current_turn
            ) || matches!(
                entry,
                VolatileCondition::Flinch { .. }
                    | VolatileCondition::FollowMe
                    | VolatileCondition::RagePowder
                    | VolatileCondition::Spotlight
                    | VolatileCondition::Pursuit { .. }
                    | VolatileCondition::HelpingHand { .. }
            );
            if should_expire {
                expired.push(entry.kind());
                false
            } else {
                true
            }
        });
        expired
    }

    /// The volatiles that only count down in the residual (Showdown's `duration`-only
    /// handlers, none with a residual order of its own), in the order they started: the ones
    /// `tick_timed` counts and the ones of a single turn (`expire_end_turn`). The volatiles with
    /// a residual of their own (Taunt, Perish Song …) are residual instances instead.
    pub fn counting_down(&self) -> Vec<VolatileConditionKind> {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    VolatileCondition::TwoTurnMove { .. }
                        | VolatileCondition::MustRecharge { turns: Some(_) }
                        | VolatileCondition::Stall { .. }
                        | VolatileCondition::AllySwitch { .. }
                        | VolatileCondition::SureHit { .. }
                        | VolatileCondition::MagicCoat { .. }
                        | VolatileCondition::Snatch { .. }
                        | VolatileCondition::MeFirst { .. }
                        | VolatileCondition::ChillyReception { .. }
                        | VolatileCondition::FocusPunch { .. }
                        | VolatileCondition::BeakBlast { .. }
                        | VolatileCondition::ShellTrap { .. }
                        | VolatileCondition::Counter { .. }
                        | VolatileCondition::MirrorCoat { .. }
                        | VolatileCondition::Rolling { .. }
                        | VolatileCondition::Rampage(_)
                        | VolatileCondition::LockOn { .. }
                        | VolatileCondition::SkyDrop { .. }
                        | VolatileCondition::FuryCutter { turns: Some(_), .. }
                        | VolatileCondition::Bide(BideState { turns: Some(_), .. })
                        | VolatileCondition::Protect { .. }
                        | VolatileCondition::Endure { .. }
                        | VolatileCondition::Flinch { .. }
                        | VolatileCondition::FollowMe
                        | VolatileCondition::RagePowder
                        | VolatileCondition::Spotlight
                        | VolatileCondition::Pursuit { .. }
                        | VolatileCondition::HelpingHand { .. }
                )
            })
            .map(|entry| entry.kind())
            .collect()
    }

    /// One residual count off `kind` (`counting_down`): `Some(true)` when it ran out and is
    /// gone, `Some(false)` while it goes on, `None` when the Pokémon does not have it.
    pub fn count_down(&mut self, kind: VolatileConditionKind) -> Option<bool> {
        let single_turn = matches!(
            kind,
            VolatileConditionKind::Protect
                | VolatileConditionKind::Endure
                | VolatileConditionKind::Flinch
                | VolatileConditionKind::FollowMe
                | VolatileConditionKind::RagePowder
                | VolatileConditionKind::Spotlight
                | VolatileConditionKind::Pursuit
                | VolatileConditionKind::HelpingHand
        );
        if single_turn {
            return self.remove(kind).then_some(true);
        }
        self.tick_timed(kind).map(|left| left == 0)
    }
}

/// The count of a timed condition (Showdown `duration`), the one `VolatileConditions::tick_timed`
/// decrements; `None` for one with no count.
fn timed_turns(entry: &mut VolatileCondition) -> Option<&mut u8> {
    match entry {
        VolatileCondition::Taunt { turns }
        | VolatileCondition::Encore { turns, .. }
        | VolatileCondition::Disable { turns, .. }
        | VolatileCondition::TwoTurnMove { turns, .. }
        | VolatileCondition::MustRecharge { turns: Some(turns) }
        | VolatileCondition::MicleBerry { turns: Some(turns) }
        | VolatileCondition::PartiallyTrapped { turns, .. }
        | VolatileCondition::Stall { turns, .. }
        | VolatileCondition::AllySwitch { turns, .. }
        | VolatileCondition::Yawn { turns, .. }
        | VolatileCondition::HealBlock { turns }
        | VolatileCondition::Embargo { turns }
        | VolatileCondition::MagnetRise { turns }
        | VolatileCondition::Telekinesis { turns }
        | VolatileCondition::SyrupBomb { turns, .. }
        | VolatileCondition::ThroatChop { turns }
        | VolatileCondition::LaserFocus { turns }
        | VolatileCondition::SureHit { turns }
        | VolatileCondition::MagicCoat { turns, .. }
        | VolatileCondition::Snatch { turns }
        | VolatileCondition::MeFirst { turns }
        | VolatileCondition::LockOn { turns, .. }
        | VolatileCondition::SkyDrop { turns, .. }
        | VolatileCondition::FuryCutter {
            turns: Some(turns), ..
        }
        | VolatileCondition::ChillyReception { turns }
        | VolatileCondition::PartialTrappingLock { turns, .. }
        | VolatileCondition::FakePartiallyTrapped { turns, .. }
        | VolatileCondition::FocusPunch { turns, .. }
        | VolatileCondition::BeakBlast { turns }
        | VolatileCondition::ShellTrap { turns, .. }
        | VolatileCondition::Counter { turns }
        | VolatileCondition::MirrorCoat { turns }
        | VolatileCondition::Rolling { turns, .. }
        | VolatileCondition::Bide(BideState {
            turns: Some(turns), ..
        }) => Some(turns),
        _ => None,
    }
}
