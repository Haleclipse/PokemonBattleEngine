//! Side conditions (DESIGN.md §30): hazards, screens, Tailwind, the slot conditions — their
//! kinds and values, and the set a side holds.

use crate::ids::PokemonId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum SideConditionKind {
    StealthRock,
    Spikes,
    ToxicSpikes,
    StickyWeb,
    Tailwind,
    /// Wide Guard / Quick Guard: the side is guarded for the turn.
    WideGuard,
    QuickGuard,
    /// Crafty Shield / Mat Block (Gen 6-8): the side is guarded for the turn against status
    /// moves / against every move Protect would stop.
    CraftyShield,
    MatBlock,
    /// Reflect / Light Screen / Aurora Veil (Gen 2+): hits on the side are weakened while the
    /// screen lasts (`SideCondition::Timed`).
    Reflect,
    LightScreen,
    AuroraVeil,
    /// Mist (Gen 3+), Safeguard (Gen 2+), Lucky Chant (Gen 4-7): other Pokémon cannot lower the
    /// stats of the side's Pokémon / give them a status or confusion; no critical hit lands on
    /// them (`SideCondition::Timed`).
    Mist,
    /// The Pledge combo's field effects (DESIGN.md §42.9): Fire Pledge chips the side's
    /// non-Fire types every turn, Grass Pledge quarters their Speed, Water Pledge doubles the
    /// secondaries of the side's moves.
    FirePledge,
    GrassPledge,
    WaterPledge,
    Safeguard,
    LuckyChant,
}

/// Entry hazards and side effects. Entries keep insertion order: entry hazards trigger in the
/// order they were laid (Showdown runs the side's SwitchIn handlers in effect order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideCondition {
    StealthRock {
        source: PokemonId,
    },
    Spikes {
        source: PokemonId,
        layers: u8,
    },
    ToxicSpikes {
        source: PokemonId,
        layers: u8,
    },
    StickyWeb {
        source: PokemonId,
    },
    Tailwind {
        source: PokemonId,
        turns_remaining: u8,
    },
    WideGuard {
        source: PokemonId,
    },
    QuickGuard {
        source: PokemonId,
    },
    CraftyShield {
        source: PokemonId,
    },
    MatBlock {
        source: PokemonId,
    },
    /// A side condition that only lasts a number of turns (Showdown `duration`, counted down
    /// in the residual): the screens, Mist, Safeguard, Lucky Chant.
    Timed {
        kind: SideConditionKind,
        turns_remaining: u8,
    },
}

impl SideCondition {
    pub const fn kind(self) -> SideConditionKind {
        match self {
            Self::Timed { kind, .. } => kind,
            Self::StealthRock { .. } => SideConditionKind::StealthRock,
            Self::Spikes { .. } => SideConditionKind::Spikes,
            Self::ToxicSpikes { .. } => SideConditionKind::ToxicSpikes,
            Self::StickyWeb { .. } => SideConditionKind::StickyWeb,
            Self::Tailwind { .. } => SideConditionKind::Tailwind,
            Self::WideGuard { .. } => SideConditionKind::WideGuard,
            Self::QuickGuard { .. } => SideConditionKind::QuickGuard,
            Self::CraftyShield { .. } => SideConditionKind::CraftyShield,
            Self::MatBlock { .. } => SideConditionKind::MatBlock,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideConditions {
    entries: Vec<SideCondition>,
}

impl SideConditions {
    pub fn entries(&self) -> &[SideCondition] {
        &self.entries
    }

    pub fn contains(&self, kind: SideConditionKind) -> bool {
        self.entries.iter().any(|entry| entry.kind() == kind)
    }

    pub fn set_stealth_rock(&mut self, source: PokemonId) -> bool {
        if self.contains(SideConditionKind::StealthRock) {
            return false;
        }
        self.entries.push(SideCondition::StealthRock { source });
        true
    }

    /// Adds a layer of Spikes (up to `max_layers`); returns the new layer count, or `None`
    /// when the side is already full.
    pub fn add_spikes(&mut self, source: PokemonId, max_layers: u8) -> Option<u8> {
        self.add_layer(source, max_layers, false)
    }

    pub fn add_toxic_spikes(&mut self, source: PokemonId, max_layers: u8) -> Option<u8> {
        self.add_layer(source, max_layers, true)
    }

    fn add_layer(&mut self, source: PokemonId, max_layers: u8, toxic: bool) -> Option<u8> {
        for entry in &mut self.entries {
            match (entry, toxic) {
                (SideCondition::Spikes { layers, .. }, false)
                | (SideCondition::ToxicSpikes { layers, .. }, true) => {
                    if *layers >= max_layers {
                        return None;
                    }
                    *layers += 1;
                    return Some(*layers);
                }
                _ => {}
            }
        }
        self.entries.push(if toxic {
            SideCondition::ToxicSpikes { source, layers: 1 }
        } else {
            SideCondition::Spikes { source, layers: 1 }
        });
        Some(1)
    }

    pub fn spikes_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                SideCondition::Spikes { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn toxic_spikes_layers(&self) -> u8 {
        self.entries
            .iter()
            .find_map(|entry| match entry {
                SideCondition::ToxicSpikes { layers, .. } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn set_sticky_web(&mut self, source: PokemonId) -> bool {
        if self.contains(SideConditionKind::StickyWeb) {
            return false;
        }
        self.entries.push(SideCondition::StickyWeb { source });
        true
    }

    /// Removes a condition (a Poison type absorbing Toxic Spikes, Defog …).
    pub fn remove(&mut self, kind: SideConditionKind) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.kind() != kind);
        self.entries.len() != before
    }

    /// Takes out the conditions `moves` picks, in order, leaving the others where they are
    /// (Court Change).
    pub fn take(&mut self, moves: impl Fn(SideConditionKind) -> bool) -> Vec<SideCondition> {
        let (taken, kept) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| moves(entry.kind()));
        self.entries = kept;
        taken
    }

    /// Adds conditions after the ones already there (Court Change).
    pub fn extend(&mut self, entries: Vec<SideCondition>) {
        self.entries.extend(entries);
    }

    pub fn set_tailwind(&mut self, source: PokemonId, turns: u8) -> bool {
        if self.contains(SideConditionKind::Tailwind) {
            return false;
        }
        self.entries.push(SideCondition::Tailwind {
            source,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// Wide Guard / Quick Guard for the turn; false when the side already has it.
    pub fn set_guard(&mut self, kind: SideConditionKind, source: PokemonId) -> bool {
        if self.contains(kind) {
            return false;
        }
        match kind {
            SideConditionKind::WideGuard => {
                self.entries.push(SideCondition::WideGuard { source });
            }
            SideConditionKind::QuickGuard => {
                self.entries.push(SideCondition::QuickGuard { source });
            }
            SideConditionKind::CraftyShield => {
                self.entries.push(SideCondition::CraftyShield { source });
            }
            SideConditionKind::MatBlock => {
                self.entries.push(SideCondition::MatBlock { source });
            }
            _ => return false,
        }
        true
    }

    pub fn tailwind_turns(&self) -> Option<u8> {
        self.entries.iter().find_map(|entry| match entry {
            SideCondition::Tailwind {
                turns_remaining, ..
            } => Some(*turns_remaining),
            _ => None,
        })
    }

    /// Starts a timed side condition (a screen) for `turns`; false when the side has it.
    pub fn set_timed(&mut self, kind: SideConditionKind, turns: u8) -> bool {
        if self.contains(kind) {
            return false;
        }
        self.entries.push(SideCondition::Timed {
            kind,
            turns_remaining: turns.max(1),
        });
        true
    }

    /// The timed side conditions' countdown alone (Gen 3-4 count the screens down first in
    /// the residual); returns the ones that ended.
    pub fn tick_timed(&mut self) -> Vec<SideConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Timed {
                kind,
                turns_remaining,
            } => count_down(*kind, turns_remaining, &mut expired),
            _ => true,
        });
        expired
    }

    /// The end-of-turn countdown; `tailwind` false leaves Tailwind to `tick_tailwind` (Gen 4
    /// counts it down first in the residual), `timed` false the screens to `tick_timed`.
    pub fn tick_end_turn(&mut self, tailwind: bool, timed: bool) -> Vec<SideConditionKind> {
        let mut expired = Vec::new();
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Tailwind { .. } if !tailwind => true,
            SideCondition::Timed { .. } if !timed => true,
            SideCondition::Timed {
                kind,
                turns_remaining,
            } => count_down(*kind, turns_remaining, &mut expired),
            SideCondition::StealthRock { .. }
            | SideCondition::Spikes { .. }
            | SideCondition::ToxicSpikes { .. }
            | SideCondition::StickyWeb { .. } => true,
            // Single-turn guards (Showdown duration 1).
            SideCondition::WideGuard { .. } => {
                expired.push(SideConditionKind::WideGuard);
                false
            }
            SideCondition::QuickGuard { .. } => {
                expired.push(SideConditionKind::QuickGuard);
                false
            }
            SideCondition::CraftyShield { .. } => {
                expired.push(SideConditionKind::CraftyShield);
                false
            }
            SideCondition::MatBlock { .. } => {
                expired.push(SideConditionKind::MatBlock);
                false
            }
            SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    expired.push(SideConditionKind::Tailwind);
                    false
                } else {
                    *turns_remaining -= 1;
                    true
                }
            }
        });
        expired
    }

    /// Whether `kind` counts down in the residual: everything but the entry hazards.
    pub fn counts_down(&self, kind: SideConditionKind) -> bool {
        !matches!(
            kind,
            SideConditionKind::StealthRock
                | SideConditionKind::Spikes
                | SideConditionKind::ToxicSpikes
                | SideConditionKind::StickyWeb
        )
    }

    /// One residual count off `kind`: `Some(true)` when it ran out and is gone, `Some(false)`
    /// while it goes on, `None` when the side does not have it (or it never counts down).
    pub fn count_down(&mut self, kind: SideConditionKind) -> Option<bool> {
        let index = self.entries.iter().position(|entry| entry.kind() == kind)?;
        let ended = match &mut self.entries[index] {
            SideCondition::StealthRock { .. }
            | SideCondition::Spikes { .. }
            | SideCondition::ToxicSpikes { .. }
            | SideCondition::StickyWeb { .. } => return None,
            SideCondition::WideGuard { .. }
            | SideCondition::QuickGuard { .. }
            | SideCondition::CraftyShield { .. }
            | SideCondition::MatBlock { .. } => true,
            SideCondition::Timed {
                turns_remaining, ..
            }
            | SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    true
                } else {
                    *turns_remaining -= 1;
                    false
                }
            }
        };
        if ended {
            self.entries.remove(index);
        }
        Some(ended)
    }

    /// Tailwind's countdown alone; true when it ended.
    pub fn tick_tailwind(&mut self) -> bool {
        let mut ended = false;
        self.entries.retain_mut(|entry| match entry {
            SideCondition::Tailwind {
                turns_remaining, ..
            } => {
                if *turns_remaining <= 1 {
                    ended = true;
                    false
                } else {
                    *turns_remaining -= 1;
                    true
                }
            }
            _ => true,
        });
        ended
    }
}

/// One turn off a timed side condition; false (and `kind` noted as ended) when it runs out.
fn count_down(
    kind: SideConditionKind,
    turns: &mut u8,
    expired: &mut Vec<SideConditionKind>,
) -> bool {
    if *turns <= 1 {
        expired.push(kind);
        false
    } else {
        *turns -= 1;
        true
    }
}
