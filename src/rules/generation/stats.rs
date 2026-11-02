//! A generation's stat stages and stat items (`StatRules`).

use super::*;

/// Stat stage multipliers: Gen 3+ `(2 + n) / 2` up and `2 / (2 + n)` down; Gen 1-2 Showdown's
/// tables, ×1, 1.5, 2 … 4 up and 100, 66, 50, 40, 33, 28, 25 % down (each floored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTable {
    Modern,
    Legacy,
}

/// How an in-battle stat comes from the stored one (DESIGN.md §31.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatRules {
    pub stage_table: StageTable,
    /// Gen 1-2 clamp every in-battle stat to `1..=999` after stages, paralysis and burn.
    pub cap: Option<u16>,
    /// Gen 1 (Showdown `modifiedStats`): each stat's in-battle value is kept on the Pokémon.
    /// A stage change recomputes it from the stored stat (dropping a paralysis / burn cut on
    /// it), paralysis and burn cut it when they are inflicted and again whenever the other
    /// side uses a move that changes stat stages, and a switch-out resets it.
    pub modified_stats: bool,
    /// Gen 6+ `boost`: no stage changes once the side facing the Pokémon has none left
    /// (`foePokemonLeft`) — after the hit that knocked out its last Pokémon, the move's
    /// `selfBoost` and Fell Stinger's boost change nothing.
    pub boosts_need_foe_pokemon_left: bool,
    /// Gen 2 (the gen2 `boostBy`): a raise changes nothing once the stat reads 999 — at its
    /// stage, with paralysis, clamped, the species item after (`stats::capped_stat_reading`).
    pub raise_stops_at_cap: bool,
    /// Gen 2 (the gen2 mod's `getStat`, after the clamp): a species' own item multiplies the
    /// stat itself — the items declare no callback there. Empty elsewhere, where the items'
    /// own `onModify*` handlers do it (`ItemBehavior::attack_boost` …).
    pub species_item_stats: &'static [SpeciesItemStat],
    /// Gen 2-4: a species' own item (Light Ball, Thick Club, Deep Sea Tooth / Scale, Stick,
    /// Lucky Punch) asks the species its holder battles as (Showdown `species`: a Ditto
    /// transformed into Pikachu gets the Light Ball's boost); Gen 5+ its own (`baseSpecies`).
    /// Gen 2's stat items (`species_item_stats`) read its own. Gen 3 on the cartridge
    /// (pokeemerald `CalculateBaseDamage`: `attacker->species`, which Transform copies) Soul Dew
    /// too, where Showdown asks `baseSpecies` (issues.md I113).
    pub species_items_read_battling: bool,
    /// Gen 3 on the cartridge (pokeemerald: `defender->species == SPECIES_DITTO`, nothing else):
    /// Metal Powder asks the species alone, a Ditto transformed into a Ditto included; Showdown
    /// and Gen 4+ also ask `!transformed` (issues.md I113).
    pub metal_powder_ignores_transform: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_CompareBattlerSpeed`, issues.md I52): an item that
    /// halves its holder's Speed (Macho Brace, Iron Ball, the Power items: `modify_speed`
    /// below 1) is read from the held item itself — "not ignored by any negation effect": it
    /// slows its holder under Klutz and Embargo too.
    pub slowing_items_ignore_suppression: bool,
    /// Gen 5+ (Showdown: its `onAnyModifyBoost` is found through `foes()` /
    /// `alliesAndSelf()`, which skip a Pokémon at 0 HP): an Unaware holder knocked out by its
    /// own move first (`selfdestruct: 'always'`) meets its target's stages. Gen 4 on the
    /// cartridge (pokeplatinum `BattleSystem_CalcMoveDamage` and the accuracy check read
    /// `Battler_Ability`, whatever the HP) keeps its Unaware (issues.md I178).
    pub unaware_needs_hp: bool,
}

/// One species item of `StatRules::species_item_stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeciesItemStat {
    pub item: crate::ids::ItemId,
    /// The holder's species (by national dex number).
    pub species: &'static [crate::ids::SpeciesId],
    pub stats: &'static [crate::state::Stat],
    /// Floored.
    pub multiplier: Fraction,
}

/// Gen 2's `getStat`: Thick Club doubles Cubone's and Marowak's Attack, Light Ball Pikachu's
/// Special Attack, Metal Powder gives Ditto ×1.5 (floored) Defense and Special Defense.
const GEN2_SPECIES_ITEM_STATS: &[SpeciesItemStat] = &[
    SpeciesItemStat {
        item: crate::ids::ItemId::THICK_CLUB,
        species: &[
            crate::ids::SpeciesId::CUBONE,
            crate::ids::SpeciesId::MAROWAK,
        ],
        stats: &[crate::state::Stat::Attack],
        multiplier: Fraction::new(2, 1),
    },
    SpeciesItemStat {
        item: crate::ids::ItemId::LIGHT_BALL,
        species: &[crate::ids::SpeciesId::PIKACHU],
        stats: &[crate::state::Stat::SpecialAttack],
        multiplier: Fraction::new(2, 1),
    },
    SpeciesItemStat {
        item: crate::ids::ItemId::METAL_POWDER,
        species: &[crate::ids::SpeciesId::DITTO],
        stats: &[
            crate::state::Stat::Defense,
            crate::state::Stat::SpecialDefense,
        ],
        multiplier: Fraction::new(3, 2),
    },
];

/// The generation's stat stages and stat items.
pub(super) fn resolve(generation: Generation) -> StatRules {
    if generation <= Generation::Gen2 {
        StatRules {
            stage_table: StageTable::Legacy,
            cap: Some(999),
            modified_stats: generation == Generation::Gen1,
            boosts_need_foe_pokemon_left: false,
            raise_stops_at_cap: generation == Generation::Gen2,
            species_item_stats: if generation == Generation::Gen2 {
                GEN2_SPECIES_ITEM_STATS
            } else {
                &[]
            },
            species_items_read_battling: generation <= Generation::Gen4,
            metal_powder_ignores_transform: generation == Generation::Gen3,
            slowing_items_ignore_suppression: false,
            unaware_needs_hp: false,
        }
    } else {
        StatRules {
            stage_table: StageTable::Modern,
            cap: None,
            modified_stats: false,
            species_item_stats: &[],
            species_items_read_battling: generation <= Generation::Gen4,
            metal_powder_ignores_transform: generation == Generation::Gen3,
            boosts_need_foe_pokemon_left: generation >= Generation::Gen6,
            raise_stops_at_cap: false,
            slowing_items_ignore_suppression: generation == Generation::Gen4,
            unaware_needs_hp: generation >= Generation::Gen5,
        }
    }
}
