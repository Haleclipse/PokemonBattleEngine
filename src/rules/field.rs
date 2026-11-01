//! The pseudo-weathers as a generation has them (DESIGN.md §23.4): Gravity, Wonder Room, Ion
//! Deluge, Mud Sport, Water Sport, Fairy Lock.

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldRules {
    /// Showdown `duration`: Gravity, Wonder Room and the Gen 6+ sports 5, Fairy Lock 2 (this
    /// turn and the next), Ion Deluge 1 (the rest of this turn).
    pub gravity_turns: u8,
    pub wonder_room_turns: u8,
    pub magic_room_turns: u8,
    pub sport_turns: u8,
    pub fairy_lock_turns: u8,
    pub ion_deluge_turns: u8,
    /// Mud Sport / Water Sport: Gen 6+ a pseudo-weather; Gen 3-5 (the gen5 mod's
    /// `volatileStatus`) a volatile on the user, for as long as it stays in.
    pub sports_on_field: bool,
    /// Their weakening of Electric / Fire moves, 4096-based: Gen 3-4 `chainModify(0.5)`, Gen 5+
    /// [1352, 4096].
    pub sport_modifier: u32,
    /// Their BasePower priority: Gen 3-4 `onAnyBasePowerPriority` 3, Gen 5 1, Gen 6+
    /// `onBasePowerPriority` 1.
    pub sport_priority: i16,
    /// Wish heals half the wisher's max HP, taken as it wished (Gen 5+ `effectState.hp`); Gen
    /// 3-4 (the gen4 mod's `onEnd`) half the recipient's.
    pub wish_heals_wisher_share: bool,
}

impl FieldRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        let early = matches!(
            generation,
            Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
        );
        Self {
            gravity_turns: 5,
            wonder_room_turns: 5,
            magic_room_turns: 5,
            sport_turns: 5,
            fairy_lock_turns: 2,
            ion_deluge_turns: 1,
            sports_on_field: !early && !matches!(generation, Generation::Gen5),
            sport_modifier: if early { 2048 } else { 1352 },
            sport_priority: if early { 3 } else { 1 },
            wish_heals_wisher_share: !early,
        }
    }
}
