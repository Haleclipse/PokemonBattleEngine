mod air_balloon;
mod berries;
mod black_sludge;
mod choice_band;
mod clear_amulet;
mod eject_button;
mod eject_pack;
mod field_items;
mod focus_band;
mod focus_sash;
pub(crate) mod hit_items;
mod leftovers;
mod life_orb;
mod metronome;
mod red_card;
mod rocky_helmet;
mod stage_items;
mod status_orb;
mod sticky_barb;

pub use crate::engine::{
    AfterHitHandler, DamageResolutionHandler, DamagingHitReactionHandler, TryStatChangeHandler,
};

/// Booster Energy's `onStart`: the item has started (`effectState.started`), and its
/// `onUpdate` check runs at once.
pub fn booster_energy_start(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if let Some(state) = ctx.item_state_mut(holder) {
        state.started = true;
    }
    ctx.use_booster_energy(holder);
}
use crate::engine::{
    BattleContext, EffectSource, OffensiveStatContext, OrderedHandler, TryHitContext, TryHitResult,
    TrySetStatusContext, TrySetStatusResult,
};
use crate::ids::MoveId;

use super::{BehaviorComponent, FractionalPriority};

/// The holder's move is decided and about to run (a Choice item locks onto it).
/// Showdown: `onModifyMove` (Gen 4: `onAfterMove`).
pub type MoveDecidedHandler = for<'a> fn(&mut BattleContext<'a>, PokemonId, MoveId);
use crate::dex::{MoveCategory, PokemonType};
use crate::effects::abilities::{
    AccuracyBoost, AccuracyModifier, AccuracyPercent, AccuracyStep, AccuracyWhen,
};
pub use crate::effects::{ResidualHandler, ResidualParticipant, SwitchHandler};
use crate::ids::{ItemId, PokemonId, SpeciesId};
pub use crate::rules::BerryHeal;
use crate::rules::{BerryRules, Fraction, HpShare, ResidualKind};
use crate::state::{NonVolatileStatus, Stat};
pub use field_items::use_if_the_field_has_it;

pub type TryHitHandler = for<'a> fn(&mut TryHitContext<'a>) -> TryHitResult;
pub type TrySetStatusHandler = for<'a> fn(&TrySetStatusContext<'a>) -> TrySetStatusResult;
pub type AfterSetStatusHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, NonVolatileStatus, EffectSource);
/// Showdown `onModifyAtk` / `onModifySpA`: the 4096-based `chainModify` the handler adds.
pub type ModifyOffenseHandler = for<'a> fn(&mut OffensiveStatContext<'a>) -> Option<u32>;

/// Showdown `onBasePower` as data: a fixed 4096-based multiplier for the moves the filter
/// admits (type-boosting items 4915 = ×1.2 at priority 15, Muscle Band / Wise Glasses 4505 =
/// ×1.1 at priority 16). Applied once, after the base-power handlers, with Showdown's
/// round-half-down `modify`.
#[derive(Clone, Copy)]
pub struct BasePowerBoost {
    pub filter: BoostFilter,
    pub numerator: u32,
    /// Showdown `onBasePowerPriority` (type boosters 15, Muscle Band / Wise Glasses 16).
    pub priority: i16,
    /// A type-boosting item's multiplier where the generation boosts the attacking stat
    /// instead (`TypeItemStage::OffensiveStat`: the `onModifyAtk` / `onModifySpA` of Gen 3's
    /// items, ×1.1 = 4505; Sea Incense ×1.05 = 4300).
    pub stat_numerator: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoostFilter {
    MoveType(PokemonType),
    Category(MoveCategory),
    /// Every move of a holder of these species (Gen 4 Light Ball: Pikachu).
    Species(&'static [SpeciesId]),
    /// A punching move (Punching Glove).
    Punch,
    /// A move of one of these types by a holder of these species (Soul Dew from Gen 7:
    /// Latias / Latios, Psychic and Dragon).
    SpeciesAndMoveTypes(&'static [SpeciesId], &'static [PokemonType]),
    /// Every move of a holder whose own species is exactly one of these, formes included
    /// (Ogerpon's masks: `baseSpecies.name.startsWith('Ogerpon-Cornerstone')` covers the
    /// forme and its Terastal one).
    OwnFormes(&'static [SpeciesId]),
}

impl BoostFilter {
    /// Whether the boost applies to `move_def` used by a holder of `species`. Showdown checks
    /// the species (Gen 3-4) or the base species (Gen 5+); the forme index is ignored, so a
    /// Pikachu in a cap and an Alolan Marowak count, and no other forme exists where the
    /// species is checked.
    pub fn admits(&self, species: Option<SpeciesId>, move_def: &crate::dex::ResolvedMove) -> bool {
        match *self {
            Self::MoveType(move_type) => move_def.move_type == move_type,
            Self::Category(category) => move_def.category == category,
            Self::Species(list) => is_one_of(species, list),
            Self::Punch => move_def.flags.punch,
            Self::SpeciesAndMoveTypes(list, types) => {
                is_one_of(species, list) && types.contains(&move_def.move_type)
            }
            Self::OwnFormes(list) => species.is_some_and(|species| list.contains(&species)),
        }
    }
}

fn is_one_of(species: Option<SpeciesId>, list: &[SpeciesId]) -> bool {
    species.is_some_and(|species| list.iter().any(|entry| entry.same_species(species)))
}

/// Showdown `onModifyAtk` / `onModifySpA` / `onModifyDef` / `onModifySpD` as data, one per
/// stat: a species' own item doubling that stat of its holder (Light Ball, Thick Club, Deep
/// Sea Tooth …) or any holder's (Assault Vest), chained into the stat's `Modify*` event at its
/// priority.
#[derive(Clone, Copy)]
pub struct StatBoost {
    /// The species the item is for (formes included, as in `BoostFilter::admits`); empty: every
    /// holder.
    pub species: &'static [SpeciesId],
    /// 4096-based.
    pub numerator: u32,
    /// Showdown `onModify*Priority`.
    pub priority: i16,
    /// Which species of the holder the item asks.
    pub check: SpeciesCheck,
}

/// Which species of its holder a species' own item asks (DESIGN.md §41.7).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpeciesCheck {
    /// The generation's (`StatRules::species_items_read_battling`).
    ByGeneration,
    /// The species it battles as, untransformed (`species.name === 'Ditto' && !transformed`):
    /// Metal Powder, Quick Powder — but Gen 3's Metal Powder asks the species alone
    /// (`StatRules::metal_powder_ignores_transform`).
    Untransformed,
}

impl StatBoost {
    /// The 4096-based modifier for `holder`, when it is one the item is for.
    pub fn modifier_for(
        &self,
        rules: &crate::rules::ResolvedRules,
        holder: &crate::state::PokemonState,
    ) -> Option<u32> {
        let species = match self.check {
            SpeciesCheck::ByGeneration => crate::engine::item_species(rules, holder),
            SpeciesCheck::Untransformed => (holder.transform.is_none()
                || rules.stats.metal_powder_ignores_transform)
                .then(|| crate::engine::species_of(holder))
                .flatten(),
        };
        (self.species.is_empty() || is_one_of(species, self.species)).then_some(self.numerator)
    }
}

/// A species' own critical-hit item (`ItemBehavior::species_crit_boost`).
#[derive(Clone, Copy)]
pub struct SpeciesCritBoost {
    pub species: &'static [SpeciesId],
    pub stages: u8,
}

impl SpeciesCritBoost {
    /// The stages for a holder of `species`, when it is one the item is for.
    pub fn stages_for(&self, species: Option<SpeciesId>) -> Option<u8> {
        is_one_of(species, self.species).then_some(self.stages)
    }
}

/// Both defenses' 4096-based modifier at a priority (`ItemBehavior::unevolved_defenses`).
#[derive(Clone, Copy)]
pub struct DefensesBoost {
    pub numerator: u32,
    /// Showdown `onModifyDefPriority` / `onModifySpDPriority`.
    pub priority: i16,
}

/// An item's `onModifyMove` on its holder's moves, as data.
#[derive(Clone, Copy, Default)]
pub struct ItemMoveEdit {
    /// A punching move loses `contact` (Punching Glove, `onModifyMovePriority` 1).
    pub punch_no_contact: bool,
    /// King's Rock, Razor Fang: the holder's damaging move gains a flinch chance of its own
    /// (a secondary marked `from_item`), for the moves `ItemFlinchRules` names — after Sheer
    /// Force has taken the move's own secondaries (Showdown: the item's `onModifyMove` at
    /// priority −1 runs after the ability's at 0; issues.md I13).
    pub adds_flinch: bool,
    /// Loaded Dice: no accuracy check after the first hit (`multiaccuracy` deleted); a 2-5
    /// hit move strikes 4 or 5 times, a 10-hit one 4 to 10 (Showdown `hitStepMoveHitLoop`:
    /// a 2 or 3 drawn becomes `5 - random(2)`, 10 becomes `10 - random(7)` — uniform either way).
    pub loaded_dice: bool,
}

/// Showdown `onModifyDamage` as data: a 4096-based multiplier chained into the ModifyDamage
/// step (Life Orb 5324 = ×1.3; Expert Belt 4915 = ×1.2 on a super-effective hit).
#[derive(Clone, Copy)]
pub struct FinalDamageBoost {
    pub numerator: u32,
    pub only_super_effective: bool,
}

/// Showdown `onAfterBoost` of the holder's item, after its ability's: the request, and what came
/// of it — whether or not the stage moved (Adrenaline Orb).
pub type ItemAfterBoostHandler = for<'a> fn(
    &mut BattleContext<'a>,
    PokemonId,
    crate::engine::StatChangeRequest,
    crate::engine::StatChangeResult,
);

/// Showdown `onAfterMoveSecondarySelf` on the user's item: once after a damaging move that
/// landed (Life Orb's recoil).
pub type AfterMoveSelfHandler =
    for<'a> fn(&mut BattleContext<'a>, PokemonId, &crate::dex::ResolvedMove);

#[derive(Clone, Copy, Default)]
pub struct ItemBehavior {
    pub base_power_boost: Option<BasePowerBoost>,
    pub final_damage_boost: Option<FinalDamageBoost>,
    pub after_move_self: Option<AfterMoveSelfHandler>,
    /// Showdown `onModifySpe` (Choice Scarf ×1.5; the Power items, Macho Brace, Iron Ball
    /// ×0.5).
    pub modify_speed: Option<Fraction>,
    /// Gen 4's `onModifyDamagePhase2` (Life Orb ×1.3, floored), used where the generation's
    /// damage order has a `ModifyDamagePhase2` step instead of the 4096-based
    /// `final_damage_boost`; the matching `after_move_self` then needs a hit on a body (the
    /// Gen 4 volatile is only set when the target had no Substitute).
    pub damage_phase2: Option<Fraction>,
    /// Safety Goggles (Showdown `onImmunity` sandstorm / hail / powder + `onTryHit`): no
    /// weather chip, powder moves refused.
    pub weather_and_powder_immune: bool,
    /// Utility Umbrella (`Pokemon.effectiveWeather` in the simulator; its `onStart` / `onUpdate`
    /// / `onEnd` only tell `WeatherChange` listeners — Forecast, Flower Gift — which are not
    /// modelled): no sun or rain for its holder (`engine::weather_for`).
    pub umbrella: bool,
    /// Showdown `ignoreKlutz`: Klutz leaves the item's effects alone (the snapshot's data, set
    /// as the registry resolves it).
    pub ignores_klutz: bool,
    /// `onSetAbility` (Ability Shield's `-block`, Gen 4 Griseous Orb's `false`): the holder's
    /// ability is not changed (Skill Swap, Role Play …: `BattleContext::set_ability`).
    pub refuses_ability_change: bool,
    /// Showdown `onPlate` (the snapshot's `plate`): the type this Plate (or type Z-Crystal)
    /// gives its holder's Multitype and its Judgment (DESIGN.md §43.4).
    pub plate: Option<crate::dex::PokemonType>,
    /// Showdown `onMemory` (the snapshot's `memory`): the type this Memory gives its holder's
    /// RKS System (Silvally) and its Multi-Attack (DESIGN.md §43.4).
    pub memory: Option<crate::dex::PokemonType>,
    /// Showdown `onDrive` (the snapshot's `drive`): the type this Drive gives Techno Blast
    /// (DESIGN.md §43.4).
    pub drive: Option<crate::dex::PokemonType>,
    /// Protective Pads (`checkMoveMakesContact`, DESIGN.md §43.4): the holder's contact moves
    /// count as no contact — for every reaction against it that asks
    /// (`engine::move_makes_contact`), but not for the items and abilities that read the move's
    /// `contact` flag itself (Tough Claws, Fluffy, Unseen Fist, Punching Glove's own edit), nor
    /// for Poison Touch: the Pads take no contact away, they only spare their holder what
    /// contact does to it (issues.md I8, R-M8-104).
    pub pads_no_contact: bool,
    /// Showdown `isChoice`: the item a `choicelock` needs its holder to keep holding (the
    /// snapshot's data, set as the registry resolves it).
    pub is_choice: bool,
    /// The Choice items' `onModifyAtk` / `onModifySpA` / `onModifySpe`: `if
    /// (pokemon.volatiles['dynamax']) return;` — their stat boost does not apply while the
    /// holder is Dynamaxed (DESIGN.md §44.5).
    pub boosts_suspended_by_dynamax: bool,
    /// Showdown `isBerry` / `isGem` (the snapshot's data, set as the registry resolves it):
    /// what Incinerate burns.
    pub is_berry: bool,
    pub is_gem: bool,
    /// Showdown `fling.basePower` (the snapshot's `fling_bp`, set as the registry resolves it):
    /// Fling's power with the item; `None` when Fling cannot throw it.
    pub fling_power: Option<u16>,
    /// Showdown `fling.status` / `fling.volatileStatus`: what Fling does with the item beyond
    /// its damage (the engine's own data, DESIGN.md R227). A berry is eaten by the target
    /// instead (`ItemBehavior::eat_effect`).
    pub fling_effect: Option<FlingEffect>,
    /// Showdown `naturalGift` (the snapshot's, set as the registry resolves it): Natural Gift's
    /// type and power with the berry.
    pub natural_gift: Option<NaturalGift>,
    /// Showdown `onTakeItem` refusing to let the item go (`engine::item_refuses_take`).
    pub take_refusal: Option<TakeRefusal>,
    /// Gen 4's Griseous Orb (`onTakeItem: false`): it is never taken, whoever holds it.
    pub refuses_take_always: bool,
    /// A Primal Orb (Showdown `isPrimalOrb`): its `formeChange` prints `-primal` instead of
    /// `-mega`, and its holder's reversion is permanent.
    pub primal_orb: bool,
    /// Sticky Barb (`onHit`): a contact move's user holding nothing takes it
    /// (`BattleContext::sticky_barb_transfer`; Gen 4 after the contact abilities,
    /// `ItemTransferRules::sticky_barb_after_contact_abilities`).
    pub sticks_to_contact: bool,
    pub try_hit: Option<TryHitHandler>,
    pub try_set_status: Option<TrySetStatusHandler>,
    pub after_set_status: Option<AfterSetStatusHandler>,
    pub modify_offense: Option<OrderedHandler<ModifyOffenseHandler>>,
    /// Reaction to a hit that reached the holder's HP (`onDamagingHitOrder` 2).
    /// Showdown: `onDamagingHit`.
    pub damaging_hit_reaction: Option<OrderedHandler<DamagingHitReactionHandler>>,
    /// Air Balloon: a move's hit on the holder's Substitute pops it as well (`onAfterSubDamage`;
    /// its `onStart` only announces the item).
    pub pops_on_substitute_hit: bool,
    /// The Resolve stage of a DamageRequest, execution order lowest first (Focus Sash, Focus
    /// Band 40).
    /// Showdown: `onDamage`.
    pub damage_resolution: Option<OrderedHandler<DamageResolutionHandler>>,
    /// An item that, armed for a move, holds every hit of it that would knock its holder out
    /// (`MovePipeline`; the holder is left at 1).
    pub holds_for_the_move: Option<HoldsForTheMove>,
    /// Custap Berry: at a quarter of its HP (half with Gluttony) the holder's move goes first in
    /// its bracket and the berry is eaten (`ActionOrderRules::custap_eaten_as_the_move_begins`).
    pub pinch_priority: bool,
    /// Absorb Bulb, Cell Battery, Luminous Moss, Snowball, Weakness Policy (`onDamagingHit` →
    /// `useItem`, the item's `boosts`): a hit that qualifies uses the item up for these stat
    /// changes (`damaging_hit_reaction`: `hit_items::use_on_hit`).
    pub used_on_hit: Option<UsedOnHit>,
    /// Electric / Grassy / Misty / Psychic Seed, Room Service (`onStart`, `onTerrainChange` /
    /// `onAnyPseudoWeatherChange` → `useItem`, the item's `boosts`): used up while the field has
    /// this (`field_items`).
    pub used_on_field: Option<UsedOnField>,
    /// Showdown `onSwitchInPriority` for the item's `switch_in` (the seeds, Room Service: −1).
    pub switch_in_priority: i8,
    /// Shed Shell (`onTrapPokemon` / `onMaybeTrapPokemon`, priority −10: after every trap): the
    /// holder can always switch out — only a move it is locked into or a recharge holds it.
    pub frees_from_traps: bool,
    /// Jaboca Berry (physical), Rowap Berry (special) (`onDamagingHit`): a hit of this category
    /// makes the holder eat it — standing or not (Showdown `eatItem` lets these two go at 0 HP) —
    /// and costs the attacker an eighth of its max HP, a quarter with the holder's Ripen
    /// (`damaging_hit_reaction`: `hit_items::recoil_on_hit`;
    /// `BerryRules::recoil_berries_need_the_attacker`).
    pub recoil_berry: Option<crate::dex::MoveCategory>,
    /// Enigma Berry (`onTryEatItem`; its `onHit` is `on_hit`): a super effective hit makes the
    /// holder eat it and restore this share of its max HP — refused with the heal (`TryHeal`:
    /// Heal Block) (`hit_items::heal_on_super_effective_hit`).
    pub super_effective_heal: Option<crate::rules::Fraction>,
    /// Showdown `onHit` of the holder's item (`runEvent('Hit')`, after its ability's: a damaging
    /// hit on the holder's HP, before the secondaries and `DamagingHit`): Enigma Berry.
    pub on_hit: Option<DamagingHitReactionHandler>,
    /// Throat Spray (`onAfterMoveSecondarySelf`): a sound move of the holder's that did something
    /// — a status one too, unless Sheer Force kept the event from running — makes it use the
    /// item up for these stat changes (`after_move`: `hit_items::use_after_a_sound_move`).
    pub used_after_a_sound_move: Option<&'static [(Stat, i8)]>,
    /// Showdown `onAfterBoost` (Adrenaline Orb: `hit_items::adrenaline_orb`; Eject Pack).
    pub after_boost: Option<ItemAfterBoostHandler>,
    /// Showdown `onFoeAfterBoost`: a foe of the holder's had a stat change (Mirror Herb).
    pub foe_after_boost: Option<ItemAfterBoostHandler>,
    /// Mirror Herb (`onUse`, `onEnd`): its use gives the holder the raises it kept
    /// (`stage_items::use_kept_raises`; `ItemState::kept_raises`).
    pub copies_foe_raises: bool,
    /// Eject Pack (`onUseItem`, `onUse`, `onEnd`): its use orders the holder out after a drop
    /// (`eject_pack::eject`; `ItemState::eject`).
    pub ejects_after_a_drop: bool,
    /// Showdown `onAnySwitchIn` (priority, handler): after a Pokémon's switch-in handlers, by
    /// their priority (White Herb −2) — `BattleEngine::run_any_switch_in`.
    pub any_switch_in: Option<(i8, crate::effects::SwitchHandler)>,
    /// Showdown `onAnyAfterMove`: after any Pokémon's move (`MovePipeline::any_after_move`).
    pub any_after_move: Option<crate::effects::SwitchHandler>,
    /// Showdown `onAnyAfterTerastallization` (Mirror Herb): after any Pokémon Terastallized
    /// (`BattleEngine::run_after_terastallization`, DESIGN.md §44.6).
    pub any_after_terastallization: Option<crate::effects::SwitchHandler>,
    /// Showdown `onAnyAfterMega` (DESIGN.md §44.3): White Herb, Eject Pack and Mirror Herb run
    /// the same handler as their `onAnyAfterMove`.
    pub any_after_mega: Option<crate::effects::SwitchHandler>,
    /// White Herb (`onUse`): its use sets the holder's lowered stats back to 0
    /// (`stage_items::restore_lowered_stats`).
    pub restores_lowered_stats: bool,
    /// Shell Bell (`onAfterMoveSecondarySelf`, priority −1): the holder heals this share of the
    /// damage its move dealt (`ConditionRules::shell_bell`; `MovePipeline::shell_bell`).
    pub heals_off_damage_dealt: Option<crate::rules::Fraction>,
    /// Big Root (`onTryHeal`, priority 1: ahead of Heal Block and Liquid Ooze): a drain, Leech
    /// Seed, Ingrain, Aqua Ring or Strength Sap heals the holder more
    /// (`BattleContext::big_root`, `ConditionRules::big_root_truncates`).
    pub boosts_absorbed_heals: bool,
    /// Destiny Knot (`onAttract`, priority −100): the holder's infatuation infatuates whoever
    /// caused it (`BattleContext::set_attract`).
    pub reciprocates_infatuation: bool,
    /// Kee Berry, Maranga Berry (`onAfterMoveSecondary` → `eatItem`): a move of this category
    /// that hit the holder makes it eat the berry (`after_hit`: `hit_items::eat_after_a_hit`).
    pub eaten_after_a_hit: Option<crate::dex::MoveCategory>,
    /// TryBoost stage of a stat change (Clear Amulet …).
    pub try_stat_change: Option<TryStatChangeHandler>,
    /// Leftovers, Black Sludge, the status orbs: take part in the residual schedule.
    /// Showdown: `onResidual`.
    pub residual: Option<ResidualParticipant>,
    /// Reaction to a damaging move that hit the holder, once per move (Showdown
    /// `onAfterMoveSecondary`); lowest priority first (Eject Button 0, Red Card 1).
    pub after_hit: Option<OrderedHandler<AfterHitHandler>>,
    /// The holder's move is decided (a Choice item locks onto it).
    /// Showdown: `onModifyMove` (Gen 4: `onAfterMove`).
    pub move_decided: Option<MoveDecidedHandler>,
    /// The holder entered the field (a Choice item forgets its lock).
    /// Showdown: `onStart`.
    pub switch_in: Option<SwitchHandler>,
    /// The volatile the item's own `condition` is (Showdown `item.condition`, Metronome's
    /// `metronome`): a Pokémon holding that one that no longer holds the item loses it when
    /// its handlers next check (`!pokemon.hasItem(...)`).
    pub condition: Option<crate::state::VolatileConditionKind>,
    /// Showdown `onModifyCritRatio` (Scope Lens, Razor Claw +1).
    pub crit_ratio_boost: u8,
    /// Showdown `onModifyCritRatio` for a species (Leek, Stick, Lucky Punch): +2 stages — Gen 2
    /// sets the stage instead (`DamageRules::species_crit_item_sets_stage`).
    pub species_crit_boost: Option<SpeciesCritBoost>,
    /// Showdown `onFractionalPriority` (Quick Claw, Lagging Tail, Full Incense).
    pub fractional_priority: Option<crate::effects::FractionalPriority>,
    /// Showdown `onModifyAtk` as data (Light Ball, Thick Club).
    pub attack_boost: Option<StatBoost>,
    /// Showdown `onModifySpA` as data (Light Ball, Deep Sea Tooth, Gen 3-6 Soul Dew).
    pub special_attack_boost: Option<StatBoost>,
    /// Showdown `onModifyDef` as data (Metal Powder).
    pub defense_boost: Option<StatBoost>,
    /// Showdown `onModifySpD` as data (Deep Sea Scale, Gen 3-6 Soul Dew).
    pub special_defense_boost: Option<StatBoost>,
    /// Showdown `onModifySpe` for a species (Quick Powder: Ditto ×2).
    pub speed_boost: Option<StatBoost>,
    /// Showdown `onModifyWeight`: the holder's weight times this, truncated (Float Stone ×1/2).
    pub weight_modifier: Option<Fraction>,
    /// Iron Ball's `onEffectiveness` (Gen 5+): a Ground move is neutral on a Flying-type holder
    /// — against each of its types, so a Skarmory's Steel weakness goes too — unless Ingrain
    /// holds it down (Smack Down and Gravity are not modelled). The grounding itself is
    /// Showdown's `isGrounded` naming the item, in every generation
    /// (`engine::environment::grounded`): Gen 4 applies the rest of the chart.
    pub flying_holder_takes_ground_neutrally: bool,
    /// Showdown `onSourceModifyAccuracy`: the accuracy of the holder's moves (Wide Lens, Zoom
    /// Lens).
    pub accuracy_boost: Option<AccuracyBoost>,
    /// Showdown `onModifyAccuracy`: the accuracy of moves aimed at the holder (Bright Powder,
    /// Lax Incense).
    pub evasion_boost: Option<AccuracyBoost>,
    /// Eviolite (`onModifyDef` + `onModifySpD`): both defenses of a holder that is not fully
    /// evolved (`ResolvedSpecies::nfe`: Showdown `baseSpecies.nfe`, the species the holder
    /// entered the battle as).
    pub unevolved_defenses: Option<DefensesBoost>,
    /// Assault Vest (`onDisableMove`): the holder cannot choose a status move.
    pub disables_status_moves: bool,
    /// Showdown `onModifyMove` on the holder's own moves (Punching Glove, Loaded Dice):
    /// `engine::modified_move`, after the ability's.
    pub move_edit: Option<ItemMoveEdit>,
    /// Power Herb (`onChargeMove`): a charging move of the holder's attacks on its first turn,
    /// the herb used up (`engine::two_turn::charge_skip`).
    pub skips_charge: bool,
    /// Ring Target (`onNegateImmunity: false`): moves the holder's types are immune to hit it,
    /// neutrally; a Ground move still misses a Levitate holder (Showdown `isGrounded` with
    /// `negateImmunity` skips only the Flying check).
    pub negates_type_immunity: bool,
    /// Showdown `onModifySecondaries` (Covert Cloak): as Shield Dust.
    pub shields_secondaries: bool,
    /// Showdown `onSourceModifyDamage` of a resist berry: a hit of its type (super effective
    /// unless it is Chilan Berry's Normal) on the holder's body — not its Substitute — makes the
    /// holder eat it (`BattleContext::eat_item`), and the damage is halved.
    pub weakening_berry: Option<WeakeningBerry>,
    /// Showdown `onEat`: what eating the berry does (`BattleContext::eat_item`).
    pub eat_effect: Option<EatEffect>,
    /// Showdown `onUpdate`: what the item does at an `Update` (DESIGN.md §40).
    pub update: Option<ItemUpdate>,
    /// A gem (`onSourceTryPrimaryHit`): the holder's damaging move of this type, as it is about
    /// to strike a target other than the holder, uses the gem up, and that move — every hit, every
    /// target, and nothing after it — gets `DamageRules::gem_boost` (issues.md I10: Showdown's
    /// `gem` volatile outlives the move).
    pub gem: Option<PokemonType>,
}

/// What a flung item does to the target besides the damage (`ItemBehavior::fling_effect`):
/// a secondary without a chance that Fling's `onPrepareHit` adds to the move, or the item's
/// `fling.effect`, which becomes the move's `onHit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlingEffect {
    /// Flame Orb, Toxic Orb, Light Ball, Poison Barb (`fling.status`).
    Status(NonVolatileStatus),
    /// King's Rock, Razor Fang (`fling.volatileStatus: 'flinch'`).
    Flinch,
    /// White Herb (`fling.effect`): the target's lowered stages back to 0 (DESIGN.md R-M8-86).
    ClearLoweredStages,
    /// Mental Herb (`fling.effect`): the target is cured as the herb's holder would be
    /// (`BattleContext::cure_mental_herb_volatiles`, DESIGN.md R-M8-86).
    CureMentalHerbVolatiles,
}

/// A berry's Natural Gift (`ItemBehavior::natural_gift`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NaturalGift {
    pub move_type: PokemonType,
    pub power: u16,
}

/// A resist berry (`ItemBehavior::weakening_berry`).
#[derive(Clone, Copy)]
pub struct WeakeningBerry {
    pub move_type: PokemonType,
    /// Only a super-effective hit (every resist berry but Chilan Berry).
    pub super_effective_only: bool,
}

/// What a hit has to be for the holder to use its item up (`ItemBehavior::used_on_hit`), and the
/// stat changes the item's `boosts` then make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsedOnHit {
    pub when: UsedOnHitWhen,
    pub boosts: &'static [(Stat, i8)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsedOnHitWhen {
    /// A move of this type (Absorb Bulb, Cell Battery, Luminous Moss, Snowball).
    MoveType(crate::dex::PokemonType),
    /// A super-effective hit that is not a fixed damage (Weakness Policy).
    SuperEffective,
}

/// What the field has to have for the holder to use its item up (`ItemBehavior::used_on_field`),
/// and the stat changes the item's `boosts` then make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsedOnField {
    pub when: FieldItemWhen,
    pub boosts: &'static [(Stat, i8)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldItemWhen {
    /// This terrain (the seeds).
    Terrain(crate::state::TerrainKind),
    /// Trick Room (Room Service).
    TrickRoom,
}

/// How an item holds a move's hits (`ItemBehavior::holds_for_the_move`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldsForTheMove {
    /// Gen 4 Focus Sash (`onTryHit`: a `focussash` condition for the move, its `onDamage` and
    /// `onAfterMoveSecondary`; pokeplatinum's `focusItemActivated` lasts the move too): armed as
    /// the move reaches the holder at full HP; the sash goes after the move if it held a hit.
    /// The Gen 4 confusion self-hit checks it as well (issues.md I39).
    FromFullHp,
    /// Focus Band where `ConditionRules::focus_band_lasts_the_move` (Gen 3-4, issues.md I38):
    /// armed by a successful roll on any hit of the move; the band stays held.
    OnChance,
}

/// A berry's `onEat` (`ItemBehavior::eat_effect`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EatEffect {
    /// Nothing of its own (a resist berry: its effect is the halving that made the holder eat
    /// it, Showdown's empty `onEat`).
    Nothing,
    /// No effect in a battle, and Showdown's `onEat: false` (Razz Berry, the berries that lower
    /// a base stat's effort …, DESIGN.md R221): eaten from its holder's hand as any berry is,
    /// but a Pokémon eating it from another's (Bug Bite, Pluck) runs no `EatItem` and has not
    /// eaten a berry (`ateBerry`).
    Inert,
    /// Cures the eater's status when it is one of these (`if (pokemon.status === …)
    /// pokemon.cureStatus()`): the berry's own trigger always holds when its holder eats it at
    /// an `Update`; one eaten otherwise (Bug Bite, Stuff Cheeks) may find nothing to cure.
    CureStatus(&'static [crate::state::NonVolatileStatus]),
    /// Ends the holder's confusion (`removeVolatile('confusion')`).
    CureConfusion,
    /// Cures any status, then ends confusion (Lum Berry, Miracle Berry).
    CureStatusAndConfusion,
    /// Restores HP (Oran Berry, Gen 2's Berry and Gold Berry). Its `onTryEatItem` refuses the
    /// berry while the heal would be (`TryHeal`: Heal Block).
    Heal(BerryHeal),
    /// Sitrus Berry: restores the generation's `BerryRules::sitrus_heal`, refused as `Heal` is.
    SitrusHeal,
    /// A confusion berry (Figy, Wiki, Mago, Aguav, Iapapa): restores the generation's share
    /// (`BerryRules::confusion_berry_heal`), then confuses a holder whose nature lowers this
    /// stat. Its `onTryEatItem` refuses it as `Heal`'s does.
    ConfusionBerryHeal(crate::state::Stat),
    /// Raises the stat one stage (`this.boost(...)`: Liechi, Ganlon, Salac, Petaya, Apicot
    /// Berry).
    Boost(crate::state::Stat),
    /// Lansat Berry: Focus Energy (`addVolatile('focusenergy')`).
    FocusEnergy,
    /// Starf Berry: one of the five stats below +6, drawn (`this.sample(...)`), rises two
    /// stages; nothing when all five are at +6.
    RandomBoost,
    /// Micle Berry: the holder's next move that checks accuracy is more accurate
    /// (`addVolatile('micleberry')`, `BerryRules::micle`).
    NextMoveAccuracy,
    /// Leppa Berry (10), Mystery Berry (5): this much PP back to the first move with none left —
    /// a forced eat (Cud Chew) finding none: to the first below its max — Ripen doubling it, up
    /// to the move's max.
    RestorePp(u8),
}

impl EatEffect {
    /// Whether the effect has room on `pokemon` (`BerryRules::stat_berries_need_room`): a
    /// stat below +6, no Focus Energy yet, one of Starf Berry's five stats below +6. Every other
    /// effect always has.
    pub fn has_room(self, pokemon: &crate::state::PokemonState) -> bool {
        match self {
            Self::Boost(stat) => pokemon.stages.get(stat) < 6,
            Self::FocusEnergy => !pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::FocusEnergy),
            Self::RandomBoost => crate::engine::STAGED_STATS
                .iter()
                .take(5)
                .any(|stat| pokemon.stages.get(*stat) < 6),
            _ => true,
        }
    }
}

/// An item's `onUpdate` (`ItemBehavior::update`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemUpdate {
    /// The holder eats it while the trigger holds (`eatItem()`).
    Eat(EatTrigger),
    /// Berry Juice (`onUpdate`): at this share of HP, unless the heal would be refused
    /// (`TryHeal`), the holder uses it up (`useItem()`: not eaten, so no Unnerve) and heals.
    UseToHeal { at: HpShare, amount: BerryHeal },
    /// Mental Herb (`onUpdate`): while the holder has one of the conditions it cures, it uses
    /// the herb up (`useItem()`) and is rid of all of them
    /// (`BattleContext::use_mental_herb`, `ConditionRules::mental_herb_cures_only_infatuation`).
    CureVolatiles,
    /// Booster Energy (`onUpdate`, once its `onStart` ran): `BattleContext::use_booster_energy`.
    BoosterEnergy,
    /// Gen 2 Berserk Gene (`onUpdate`: `if (pokemon.useItem()) pokemon.addVolatile('confusion')`,
    /// its `boosts` +2 Attack as it is used): `BattleContext::use_berserk_gene`.
    BerserkGene,
}

/// When an `Update` makes the holder eat its berry (`ItemUpdate::Eat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EatTrigger {
    /// The holder has one of these statuses.
    Status(&'static [NonVolatileStatus]),
    /// The holder is confused.
    Confused,
    /// The holder has a status or is confused.
    StatusOrConfused,
    /// The holder's HP is down to this share.
    HpAtMost(HpShare),
    /// The holder's HP is down to the generation's confusion-berry share
    /// (`BerryRules::confusion_berry_at`).
    ConfusionBerryHp,
    /// A move of the holder's has no PP left (Leppa Berry, Mystery Berry: `moveSlots.some(move
    /// => move.pp === 0)`).
    MoveOutOfPp,
}

impl EatTrigger {
    /// Whether it waits for a status or confusion (the status berries: `BerryRules::status_berries`
    /// says where those are eaten).
    pub const fn cures(self) -> bool {
        matches!(
            self,
            Self::Status(_) | Self::Confused | Self::StatusOrConfused
        )
    }

    /// Whether the generation leaves the berry to the end of the turn instead of an `Update`,
    /// though its item has no `onResidual` of its own: Gen 2's status berries
    /// (`StatusBerryTiming::InflictionOrTurnEnd`, issues.md I33), Gen 2-3's PP berries
    /// (`BerryRules::pp_berries_at_turn_end`, I49).
    pub fn at_turn_end(self, berries: &BerryRules) -> bool {
        (self.cures()
            && berries.status_berries == crate::rules::StatusBerryTiming::InflictionOrTurnEnd)
            || (matches!(self, Self::MoveOutOfPp) && berries.pp_berries_at_turn_end)
    }

    /// Whether it holds for `pokemon`; `gluttony`: its ability is Gluttony.
    pub fn holds(
        self,
        pokemon: &crate::state::PokemonState,
        berries: &BerryRules,
        gluttony: bool,
    ) -> bool {
        let confused = || {
            pokemon
                .volatiles
                .contains(crate::state::VolatileConditionKind::Confusion)
        };
        match self {
            Self::Status(statuses) => pokemon
                .status
                .is_some_and(|status| statuses.contains(&status.kind)),
            Self::Confused => confused(),
            Self::StatusOrConfused => pokemon.status.is_some() || confused(),
            Self::HpAtMost(share) => berries.reached(share, pokemon.hp, pokemon.max_hp, gluttony),
            Self::ConfusionBerryHp => berries.reached(
                berries.confusion_berry_at,
                pokemon.hp,
                pokemon.max_hp,
                gluttony,
            ),
            Self::MoveOutOfPp if berries.pp_berry_reads_own_slots => {
                pokemon.base_slots().any(|slot| slot.pp == 0)
            }
            Self::MoveOutOfPp => pokemon.move_slots().iter().any(|slot| slot.pp == 0),
        }
    }
}

/// When an item refuses to be taken (Showdown `onTakeItem`, DESIGN.md §39.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeRefusal {
    /// Mail: only Knock Off, Thief and Covet take it — Thief and Covet where
    /// `ItemTransferRules::mail_stolen` says.
    Mail,
    /// Rusted Sword, Rusted Shield: not when the holder or the taker is this species, in any
    /// forme (Showdown `baseSpecies.num`).
    Species(u16),
    /// Booster Energy: not from a Paradox Pokémon (Showdown `baseSpecies.tags` holding
    /// `Paradox`: `engine::PARADOX_SPECIES`).
    Paradox,
    /// A Z-Crystal (Showdown `onTakeItem: false`, DESIGN.md §44.4): never taken, by anything.
    ZCrystal,
    /// A Mega Stone, from the species it belongs to (Showdown `!item.megaStone[source
    /// .baseSpecies.baseSpecies]`, DESIGN.md §44.3): the dex number of that species.
    MegaStone(u16),
}

/// The item behavior components (`effects::BehaviorComponent`, DESIGN.md §25.1).
const COMPONENTS: &[BehaviorComponent<ItemBehavior>] = &[
    BehaviorComponent {
        // A Plate's `onPlate` (Arceus's type and Judgment).
        source_callbacks: |h| match h.plate {
            Some(_) => &["onPlate"],
            None => &[],
        },
        disable: |h| h.plate = None,
    },
    BehaviorComponent {
        // A Memory's `onMemory` (Silvally's type and Multi-Attack).
        source_callbacks: |h| match h.memory {
            Some(_) => &["onMemory"],
            None => &[],
        },
        disable: |h| h.memory = None,
    },
    BehaviorComponent {
        // A Drive's `onDrive` (Techno Blast).
        source_callbacks: |h| match h.drive {
            Some(_) => &["onDrive"],
            None => &[],
        },
        disable: |h| h.drive = None,
    },
    BehaviorComponent {
        // A Primal Orb's reversion is its `onSwitchIn` (the seeds' and Choice items' is
        // `onStart`).
        source_callbacks: |h| {
            if h.primal_orb { &["onSwitchIn"] } else { &[] }
        },
        disable: |h| h.primal_orb = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.take_refusal.is_some() || h.refuses_take_always {
                &["onTakeItem"]
            } else {
                &[]
            }
        },
        disable: |h| {
            h.take_refusal = None;
            h.refuses_take_always = false;
        },
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.sticks_to_contact { &["onHit"] } else { &[] }
        },
        disable: |h| h.sticks_to_contact = false,
    },
    BehaviorComponent {
        // Gen 4+ `onBasePower`; the same boost as `onModifyAtk` / `onModifySpA` in Gen 3 and
        // `onModifyDamage` in Gen 2 for a type-boosting item.
        source_callbacks: |h| match h.base_power_boost {
            Some(BasePowerBoost {
                filter: BoostFilter::MoveType(_),
                ..
            }) => &[
                "onBasePower",
                "onModifyAtk",
                "onModifySpA",
                "onModifyDamage",
            ],
            Some(_) => &["onBasePower"],
            None => &[],
        },
        disable: |h| h.base_power_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.final_damage_boost {
            Some(_) => &["onModifyDamage"],
            None => &[],
        },
        disable: |h| h.final_damage_boost = None,
    },
    BehaviorComponent {
        // Gen 4: `onBasePower` only sets the volatile that gates the recoil.
        source_callbacks: |h| match h.damage_phase2 {
            Some(_) => &["onModifyDamagePhase2", "onBasePower"],
            None => &[],
        },
        disable: |h| h.damage_phase2 = None,
    },
    BehaviorComponent {
        // Gen 4's Life Orb recoil lives in the `lifeorb` volatile its `onBasePower` sets.
        source_callbacks: |h| match (h.after_move_self, h.damage_phase2) {
            (Some(_), Some(_)) => &["onAfterMoveSecondarySelf", "onBasePower"],
            (Some(_), None) => &["onAfterMoveSecondarySelf"],
            (None, _) => &[],
        },
        disable: |h| h.after_move_self = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_speed {
            Some(_) => &["onModifySpe"],
            None => &[],
        },
        disable: |h| h.modify_speed = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.weather_and_powder_immune {
                &["onImmunity", "onTryHit"]
            } else {
                &[]
            }
        },
        disable: |h| h.weather_and_powder_immune = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.umbrella {
                &["onStart", "onUpdate", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.umbrella = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_hit {
            Some(_) => &["onTryHit"],
            None => &[],
        },
        disable: |h| h.try_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_set_status {
            Some(_) => &["onSetStatus"],
            None => &[],
        },
        disable: |h| h.try_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_set_status {
            Some(_) => &["onAfterSetStatus"],
            None => &[],
        },
        disable: |h| h.after_set_status = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.modify_offense {
            Some(_) => &["onModifyAtk", "onModifySpA"],
            None => &[],
        },
        disable: |h| h.modify_offense = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damaging_hit_reaction {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.damaging_hit_reaction = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.pops_on_substitute_hit {
                &["onAfterSubDamage", "onStart"]
            } else {
                &[]
            }
        },
        disable: |h| h.pops_on_substitute_hit = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.damage_resolution {
            Some(_) => &["onDamage"],
            None => &[],
        },
        disable: |h| h.damage_resolution = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.holds_for_the_move {
            Some(HoldsForTheMove::FromFullHp) => &["onTryHit"],
            Some(HoldsForTheMove::OnChance) => &["onDamage"],
            None => &[],
        },
        disable: |h| h.holds_for_the_move = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.pinch_priority {
                &["onBeforeTurn", "onCustap", "onFractionalPriority"]
            } else {
                &[]
            }
        },
        disable: |h| h.pinch_priority = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.used_on_hit {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.used_on_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.used_on_field {
            Some(UsedOnField {
                when: FieldItemWhen::Terrain(_),
                ..
            }) => &["onTerrainChange"],
            Some(UsedOnField {
                when: FieldItemWhen::TrickRoom,
                ..
            }) => &["onAnyPseudoWeatherChange"],
            None => &[],
        },
        disable: |h| h.used_on_field = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.refuses_ability_change {
                &["onSetAbility"]
            } else {
                &[]
            }
        },
        disable: |h| h.refuses_ability_change = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.frees_from_traps {
                &["onMaybeTrapPokemon", "onTrapPokemon"]
            } else {
                &[]
            }
        },
        disable: |h| h.frees_from_traps = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.eaten_after_a_hit {
            Some(_) => &["onAfterMoveSecondary"],
            None => &[],
        },
        disable: |h| h.eaten_after_a_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.used_after_a_sound_move {
            Some(_) => &["onAfterMoveSecondarySelf"],
            None => &[],
        },
        disable: |h| h.used_after_a_sound_move = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_boost {
            Some(_) => &["onAfterBoost"],
            None => &[],
        },
        disable: |h| h.after_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.foe_after_boost {
            Some(_) => &["onFoeAfterBoost"],
            None => &[],
        },
        disable: |h| h.foe_after_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.copies_foe_raises {
                &["onUse", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.copies_foe_raises = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.ejects_after_a_drop {
                &["onUseItem", "onUse", "onEnd"]
            } else {
                &[]
            }
        },
        disable: |h| h.ejects_after_a_drop = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_switch_in {
            Some(_) => &["onAnySwitchIn"],
            None => &[],
        },
        disable: |h| h.any_switch_in = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_move {
            Some(_) => &["onAnyAfterMove"],
            None => &[],
        },
        disable: |h| h.any_after_move = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_mega {
            Some(_) => &["onAnyAfterMega"],
            None => &[],
        },
        disable: |h| h.any_after_mega = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.any_after_terastallization {
            Some(_) => &["onAnyAfterTerastallization"],
            None => &[],
        },
        disable: |h| h.any_after_terastallization = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.restores_lowered_stats {
                &["onUse"]
            } else {
                &[]
            }
        },
        disable: |h| h.restores_lowered_stats = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.heals_off_damage_dealt {
            Some(_) => &["onAfterMoveSecondarySelf"],
            None => &[],
        },
        disable: |h| h.heals_off_damage_dealt = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.boosts_absorbed_heals {
                &["onTryHeal"]
            } else {
                &[]
            }
        },
        disable: |h| h.boosts_absorbed_heals = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.reciprocates_infatuation {
                &["onAttract"]
            } else {
                &[]
            }
        },
        disable: |h| h.reciprocates_infatuation = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.recoil_berry {
            Some(_) => &["onDamagingHit"],
            None => &[],
        },
        disable: |h| h.recoil_berry = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.super_effective_heal {
            Some(_) => &["onTryEatItem"],
            None => &[],
        },
        disable: |h| h.super_effective_heal = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.on_hit {
            Some(_) => &["onHit"],
            None => &[],
        },
        disable: |h| h.on_hit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.try_stat_change {
            Some(_) => &["onTryBoost"],
            None => &[],
        },
        disable: |h| h.try_stat_change = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.residual {
            Some(_) => &["onResidual"],
            None => &[],
        },
        disable: |h| h.residual = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.after_hit {
            Some(_) => &["onAfterMoveSecondary"],
            None => &[],
        },
        disable: |h| h.after_hit = None,
    },
    BehaviorComponent {
        // Gen 5+ lock at `onModifyMove`, Gen 4 at `onAfterMove`: the same lock, observed the
        // same way in the engine's model (DESIGN.md §31.1).
        source_callbacks: |h| match h.move_decided {
            Some(_) => &["onModifyMove", "onAfterMove"],
            None => &[],
        },
        disable: |h| h.move_decided = None,
    },
    BehaviorComponent {
        // A Primal Orb's switch-in handler is its `onSwitchIn` (the component below), every
        // other item's its `onStart`.
        source_callbacks: |h| match h.switch_in {
            Some(_) if h.primal_orb => &["onSwitchIn"],
            Some(_) => &["onStart"],
            None => &[],
        },
        disable: |h| h.switch_in = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.crit_ratio_boost > 0 {
                &["onModifyCritRatio"]
            } else {
                &[]
            }
        },
        disable: |h| h.crit_ratio_boost = 0,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.skips_charge {
                &["onChargeMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.skips_charge = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.fractional_priority {
            Some(_) => &["onFractionalPriority"],
            None => &[],
        },
        disable: |h| h.fractional_priority = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.attack_boost {
            Some(_) => &["onModifyAtk"],
            None => &[],
        },
        disable: |h| h.attack_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.special_attack_boost {
            Some(_) => &["onModifySpA"],
            None => &[],
        },
        disable: |h| h.special_attack_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.defense_boost {
            Some(_) => &["onModifyDef"],
            None => &[],
        },
        disable: |h| h.defense_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.special_defense_boost {
            Some(_) => &["onModifySpD"],
            None => &[],
        },
        disable: |h| h.special_defense_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.speed_boost {
            Some(_) => &["onModifySpe"],
            None => &[],
        },
        disable: |h| h.speed_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weight_modifier {
            Some(_) => &["onModifyWeight"],
            None => &[],
        },
        disable: |h| h.weight_modifier = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.flying_holder_takes_ground_neutrally {
                &["onEffectiveness"]
            } else {
                &[]
            }
        },
        disable: |h| h.flying_holder_takes_ground_neutrally = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.species_crit_boost {
            Some(_) => &["onModifyCritRatio"],
            None => &[],
        },
        disable: |h| h.species_crit_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.unevolved_defenses {
            Some(_) => &["onModifyDef", "onModifySpD"],
            None => &[],
        },
        disable: |h| h.unevolved_defenses = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.disables_status_moves {
                &["onDisableMove"]
            } else {
                &[]
            }
        },
        disable: |h| h.disables_status_moves = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.move_edit {
            Some(_) => &["onModifyMove"],
            None => &[],
        },
        disable: |h| h.move_edit = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.accuracy_boost {
            Some(_) => &["onSourceModifyAccuracy"],
            None => &[],
        },
        disable: |h| h.accuracy_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.evasion_boost {
            Some(_) => &["onModifyAccuracy"],
            None => &[],
        },
        disable: |h| h.evasion_boost = None,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.negates_type_immunity {
                &["onNegateImmunity"]
            } else {
                &[]
            }
        },
        disable: |h| h.negates_type_immunity = false,
    },
    BehaviorComponent {
        source_callbacks: |h| {
            if h.shields_secondaries {
                &["onModifySecondaries"]
            } else {
                &[]
            }
        },
        disable: |h| h.shields_secondaries = false,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.weakening_berry {
            Some(_) => &["onSourceModifyDamage"],
            None => &[],
        },
        disable: |h| h.weakening_berry = None,
    },
    BehaviorComponent {
        // A healing berry's `onTryEatItem` is its `TryHeal` check.
        source_callbacks: |h| match h.eat_effect {
            Some(EatEffect::Heal(_) | EatEffect::SitrusHeal | EatEffect::ConfusionBerryHeal(_)) => {
                &["onEat", "onTryEatItem"]
            }
            Some(_) => &["onEat"],
            None => &[],
        },
        disable: |h| h.eat_effect = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.update {
            Some(_) => &["onUpdate"],
            None => &[],
        },
        disable: |h| h.update = None,
    },
    BehaviorComponent {
        source_callbacks: |h| match h.gem {
            Some(_) => &["onSourceTryPrimaryHit"],
            None => &[],
        },
        disable: |h| h.gem = None,
    },
];

impl ItemBehavior {
    /// Every Showdown callback some component of these behavior stands for.
    pub fn covered_callbacks(&self) -> Vec<&'static str> {
        super::covered_callbacks(COMPONENTS, self)
    }

    /// These behavior without the components none of whose callbacks `declared` lists.
    pub fn restricted_to(self, declared: &[String]) -> Self {
        super::restricted_to(COMPONENTS, self, declared)
    }
}

/// The crowned forme the holder's Rusted Sword / Shield gives, and the item, for `BattleStart`.
pub fn battle_start_forme(
    ctx: &BattleContext<'_>,
    holder: PokemonId,
) -> Option<(ItemId, SpeciesId)> {
    // The raw field: the Pokémon is not on the field yet.
    let item = ctx.raw_held_item(holder)?;
    let (holder_species, forme) = match item {
        ItemId::RUSTED_SWORD => (SpeciesId::ZACIAN, SpeciesId::ZACIAN_CROWNED),
        ItemId::RUSTED_SHIELD => (SpeciesId::ZAMAZENTA, SpeciesId::ZAMAZENTA_CROWNED),
        _ => return None,
    };
    ctx.own_species(holder)
        .filter(|own| own.same_species(holder_species))
        .map(|_| (item, forme))
}

/// The move a Rusted Sword / Shield renames (Iron Head → Behemoth Blade / Bash).
pub const fn crowned_move_swap(item: ItemId) -> Option<(crate::ids::MoveId, crate::ids::MoveId)> {
    match item {
        ItemId::RUSTED_SWORD => Some((
            crate::ids::MoveId::IRON_HEAD,
            crate::ids::MoveId::BEHEMOTH_BLADE,
        )),
        ItemId::RUSTED_SHIELD => Some((
            crate::ids::MoveId::IRON_HEAD,
            crate::ids::MoveId::BEHEMOTH_BASH,
        )),
        _ => None,
    }
}

/// A Primal Orb's `onSwitchIn` (`isPrimalOrb`, priority −1): its holder reverts to its Primal
/// forme — permanent, no message of its own beyond the `-primal` the trace records — which is
/// what starts Primordial Sea / Desolate Land (the forme's ability starts as it is taken).
pub fn primal_reversion(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    if !ctx.is_active(holder) || ctx.has_transformed(holder) {
        return;
    }
    let Some((item, _)) = ctx.held_item(holder) else {
        return;
    };
    let Some(own) = ctx.own_species(holder) else {
        return;
    };
    let forme = match item {
        ItemId::BLUE_ORB if own.same_species(SpeciesId::KYOGRE) => SpeciesId::KYOGRE_PRIMAL,
        ItemId::RED_ORB if own.same_species(SpeciesId::GROUDON) => SpeciesId::GROUDON_PRIMAL,
        _ => return,
    };
    // `-primal|holder|Red Orb` names the orb (R-M10-41).
    if ctx.fix_own_forme(
        holder,
        forme,
        crate::engine::FormeChangeKind::Permanent,
        EffectSource::Item {
            pokemon: holder,
            item,
        },
    ) {
        ctx.announce_item(holder, item);
    }
}

/// A Plate (DESIGN.md §43.4): the type booster of its type, the type it gives its holder's
/// Multitype and Judgment (`onPlate`), and none of them can be taken off an Arceus.
fn plate(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        plate: Some(move_type),
        take_refusal: Some(TakeRefusal::Species(SpeciesId::ARCEUS.dex)),
        ..type_boost(move_type)
    }
}

/// A Memory (DESIGN.md §43.4): the type it gives RKS System (Silvally) and Multi-Attack, and
/// none of them can be taken off a Silvally. No power boost of its own (Showdown's entry has no
/// `onBasePower`).
fn memory(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        memory: Some(move_type),
        take_refusal: Some(TakeRefusal::Species(SpeciesId::SILVALLY.dex)),
        ..ItemBehavior::default()
    }
}

/// A Drive (DESIGN.md §43.4): the type it gives Techno Blast, and none of them can be taken off
/// a Genesect.
fn drive(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        drive: Some(move_type),
        take_refusal: Some(TakeRefusal::Species(SpeciesId::GENESECT.dex)),
        ..ItemBehavior::default()
    }
}

/// The Arceus forme of `type_` (the dex's own numbering).
pub fn arceus_forme(type_: PokemonType) -> SpeciesId {
    use crate::ids::SpeciesId as S;
    match type_ {
        PokemonType::Bug => S::ARCEUS_BUG,
        PokemonType::Dark => S::ARCEUS_DARK,
        PokemonType::Dragon => S::ARCEUS_DRAGON,
        PokemonType::Electric => S::ARCEUS_ELECTRIC,
        PokemonType::Fairy => S::ARCEUS_FAIRY,
        PokemonType::Fighting => S::ARCEUS_FIGHTING,
        PokemonType::Fire => S::ARCEUS_FIRE,
        PokemonType::Flying => S::ARCEUS_FLYING,
        PokemonType::Ghost => S::ARCEUS_GHOST,
        PokemonType::Grass => S::ARCEUS_GRASS,
        PokemonType::Ground => S::ARCEUS_GROUND,
        PokemonType::Ice => S::ARCEUS_ICE,
        PokemonType::Poison => S::ARCEUS_POISON,
        PokemonType::Psychic => S::ARCEUS_PSYCHIC,
        PokemonType::Rock => S::ARCEUS_ROCK,
        PokemonType::Steel => S::ARCEUS_STEEL,
        PokemonType::Water => S::ARCEUS_WATER,
        _ => SpeciesId::ARCEUS,
    }
}

/// A species' own two types ×1.2 (the creation trio's orbs and crystals; Showdown
/// `onBasePowerPriority` 15, `chainModify([4915, 4096])`).
fn species_power(species: &'static [SpeciesId], types: &'static [PokemonType]) -> BasePowerBoost {
    BasePowerBoost {
        filter: BoostFilter::SpeciesAndMoveTypes(species, types),
        numerator: 4915,
        priority: 15,
        stat_numerator: 4096,
    }
}

/// An Ogerpon mask (DESIGN.md §43.4): ×1.2 to every move of its formes, and not taken off an
/// Ogerpon.
fn mask(formes: &'static [SpeciesId], ogerpon: u16) -> ItemBehavior {
    ItemBehavior {
        base_power_boost: Some(BasePowerBoost {
            filter: BoostFilter::OwnFormes(formes),
            numerator: 4915,
            priority: 15,
            stat_numerator: 4096,
        }),
        take_refusal: Some(TakeRefusal::Species(ogerpon)),
        ..ItemBehavior::default()
    }
}

fn type_boost(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        base_power_boost: Some(BasePowerBoost {
            filter: BoostFilter::MoveType(move_type),
            numerator: 4915,
            priority: 15,
            stat_numerator: 4505,
        }),
        ..ItemBehavior::default()
    }
}

fn category_boost(category: MoveCategory) -> ItemBehavior {
    ItemBehavior {
        base_power_boost: Some(BasePowerBoost {
            filter: BoostFilter::Category(category),
            numerator: 4505,
            priority: 16,
            stat_numerator: 4096,
        }),
        ..ItemBehavior::default()
    }
}

/// A gem of `move_type` (`ItemBehavior::gem`).
fn gem(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        gem: Some(move_type),
        ..ItemBehavior::default()
    }
}

/// A resist berry: a super-effective hit of `move_type` is halved and the berry eaten.
fn resist_berry(move_type: PokemonType) -> ItemBehavior {
    ItemBehavior {
        weakening_berry: Some(WeakeningBerry {
            move_type,
            super_effective_only: true,
        }),
        eat_effect: Some(EatEffect::Nothing),
        ..ItemBehavior::default()
    }
}

/// A species' own item doubling one of its stats (`onModify*`, chainModify(2)).
const fn doubles(species: &'static [SpeciesId], priority: i16) -> Option<StatBoost> {
    Some(StatBoost {
        species,
        numerator: 8192,
        priority,
        check: SpeciesCheck::ByGeneration,
    })
}

/// Metal Powder's and Quick Powder's doubling: only an untransformed holder.
const fn untransformed_doubles(species: &'static [SpeciesId], priority: i16) -> Option<StatBoost> {
    Some(StatBoost {
        species,
        numerator: 8192,
        priority,
        check: SpeciesCheck::Untransformed,
    })
}

const PIKACHU: &[SpeciesId] = &[SpeciesId::PIKACHU];
const CLAMPERL: &[SpeciesId] = &[SpeciesId::CLAMPERL];
const LATI: &[SpeciesId] = &[SpeciesId::LATIAS, SpeciesId::LATIOS];

pub fn definition(item: ItemId) -> ItemBehavior {
    match item {
        ItemId::CHARCOAL => type_boost(PokemonType::Fire),
        ItemId::MYSTIC_WATER => type_boost(PokemonType::Water),
        ItemId::MAGNET => type_boost(PokemonType::Electric),
        ItemId::MIRACLE_SEED => type_boost(PokemonType::Grass),
        ItemId::NEVER_MELT_ICE => type_boost(PokemonType::Ice),
        ItemId::BLACK_BELT => type_boost(PokemonType::Fighting),
        ItemId::POISON_BARB => ItemBehavior {
            fling_effect: Some(FlingEffect::Status(NonVolatileStatus::Poison)),
            ..type_boost(PokemonType::Poison)
        },
        ItemId::SOFT_SAND => type_boost(PokemonType::Ground),
        ItemId::SHARP_BEAK => type_boost(PokemonType::Flying),
        ItemId::TWISTED_SPOON => type_boost(PokemonType::Psychic),
        ItemId::SILVER_POWDER => type_boost(PokemonType::Bug),
        ItemId::HARD_STONE => type_boost(PokemonType::Rock),
        ItemId::SPELL_TAG => type_boost(PokemonType::Ghost),
        ItemId::DRAGON_FANG => type_boost(PokemonType::Dragon),
        ItemId::BLACK_GLASSES => type_boost(PokemonType::Dark),
        ItemId::METAL_COAT => type_boost(PokemonType::Steel),
        ItemId::SILK_SCARF | ItemId::PINK_BOW | ItemId::POLKADOT_BOW => {
            type_boost(PokemonType::Normal)
        }
        // A type booster only where the generation's item declares it (Gen 2).
        ItemId::DRAGON_SCALE => type_boost(PokemonType::Dragon),
        ItemId::FAIRY_FEATHER => type_boost(PokemonType::Fairy),
        ItemId::ODD_INCENSE => type_boost(PokemonType::Psychic),
        ItemId::ROCK_INCENSE => type_boost(PokemonType::Rock),
        ItemId::ROSE_INCENSE => type_boost(PokemonType::Grass),
        ItemId::WAVE_INCENSE => type_boost(PokemonType::Water),
        // Gen 3: `onModifySpA`, chainModify(1.05).
        ItemId::SEA_INCENSE => ItemBehavior {
            base_power_boost: Some(BasePowerBoost {
                stat_numerator: 4300,
                ..type_boost(PokemonType::Water).base_power_boost.unwrap()
            }),
            ..ItemBehavior::default()
        },
        // Gen 3-6 ×1.5 to Latias' and Latios' Special Attack and Special Defense; from Gen 7
        // ×1.2 to their Psychic and Dragon moves.
        ItemId::SOUL_DEW => ItemBehavior {
            base_power_boost: Some(BasePowerBoost {
                filter: BoostFilter::SpeciesAndMoveTypes(
                    LATI,
                    &[PokemonType::Psychic, PokemonType::Dragon],
                ),
                numerator: 4915,
                priority: 15,
                stat_numerator: 4096,
            }),
            special_attack_boost: Some(StatBoost {
                species: LATI,
                numerator: 6144,
                priority: 1,
                check: SpeciesCheck::ByGeneration,
            }),
            special_defense_boost: Some(StatBoost {
                species: LATI,
                numerator: 6144,
                priority: 2,
                check: SpeciesCheck::ByGeneration,
            }),
            ..ItemBehavior::default()
        },
        // Gen 3 Special Attack only, Gen 4 the power of every move (priority 0), Gen 5+ Attack
        // and Special Attack; Gen 2's lives in `StatRules::species_item_stats`.
        ItemId::LIGHT_BALL => ItemBehavior {
            base_power_boost: Some(BasePowerBoost {
                filter: BoostFilter::Species(PIKACHU),
                numerator: 8192,
                priority: 0,
                stat_numerator: 4096,
            }),
            attack_boost: doubles(PIKACHU, 1),
            special_attack_boost: doubles(PIKACHU, 1),
            fling_effect: Some(FlingEffect::Status(NonVolatileStatus::Paralysis)),
            ..ItemBehavior::default()
        },
        ItemId::THICK_CLUB => ItemBehavior {
            attack_boost: doubles(&[SpeciesId::CUBONE, SpeciesId::MAROWAK], 1),
            ..ItemBehavior::default()
        },
        // A Ditto that has not transformed (`!pokemon.transformed`).
        ItemId::METAL_POWDER => ItemBehavior {
            defense_boost: untransformed_doubles(&[SpeciesId::DITTO], 2),
            ..ItemBehavior::default()
        },
        ItemId::DEEP_SEA_TOOTH => ItemBehavior {
            special_attack_boost: doubles(CLAMPERL, 1),
            ..ItemBehavior::default()
        },
        ItemId::DEEP_SEA_SCALE => ItemBehavior {
            special_defense_boost: doubles(CLAMPERL, 2),
            ..ItemBehavior::default()
        },
        ItemId::POWER_HERB => ItemBehavior {
            skips_charge: true,
            ..ItemBehavior::default()
        },
        ItemId::MUSCLE_BAND => category_boost(MoveCategory::Physical),
        ItemId::SCOPE_LENS | ItemId::RAZOR_CLAW => ItemBehavior {
            crit_ratio_boost: 1,
            ..ItemBehavior::default()
        },
        // Gen 2-7 Farfetch'd; Gen 8 Farfetch'd (the Galarian one too) and Sirfetch'd.
        ItemId::STICK | ItemId::LEEK => ItemBehavior {
            species_crit_boost: Some(SpeciesCritBoost {
                species: &[SpeciesId::FARFETCH_D, SpeciesId::SIRFETCH_D],
                stages: 2,
            }),
            ..ItemBehavior::default()
        },
        ItemId::LUCKY_PUNCH => ItemBehavior {
            species_crit_boost: Some(SpeciesCritBoost {
                species: &[SpeciesId::CHANSEY],
                stages: 2,
            }),
            ..ItemBehavior::default()
        },
        ItemId::EVIOLITE => ItemBehavior {
            unevolved_defenses: Some(DefensesBoost {
                numerator: 6144,
                priority: 2,
            }),
            ..ItemBehavior::default()
        },
        // Showdown also spares Me First, which the engine does not implement (Taunt's check
        // spares nothing either).
        ItemId::ASSAULT_VEST => ItemBehavior {
            special_defense_boost: Some(StatBoost {
                species: &[],
                numerator: 6144,
                priority: 1,
                check: SpeciesCheck::ByGeneration,
            }),
            disables_status_moves: true,
            ..ItemBehavior::default()
        },
        ItemId::PUNCHING_GLOVE => ItemBehavior {
            base_power_boost: Some(BasePowerBoost {
                filter: BoostFilter::Punch,
                numerator: 4506,
                priority: 23,
                stat_numerator: 4096,
            }),
            move_edit: Some(ItemMoveEdit {
                punch_no_contact: true,
                ..ItemMoveEdit::default()
            }),
            ..ItemBehavior::default()
        },
        ItemId::KINGS_ROCK | ItemId::RAZOR_FANG => ItemBehavior {
            move_edit: Some(ItemMoveEdit {
                adds_flinch: true,
                ..ItemMoveEdit::default()
            }),
            fling_effect: Some(FlingEffect::Flinch),
            ..ItemBehavior::default()
        },
        ItemId::LOADED_DICE => ItemBehavior {
            move_edit: Some(ItemMoveEdit {
                loaded_dice: true,
                ..ItemMoveEdit::default()
            }),
            ..ItemBehavior::default()
        },
        ItemId::WISE_GLASSES => category_boost(MoveCategory::Special),
        ItemId::LIFE_ORB => ItemBehavior {
            final_damage_boost: Some(FinalDamageBoost {
                numerator: 5324,
                only_super_effective: false,
            }),
            after_move_self: Some(life_orb::after_move_self),
            damage_phase2: Some(Fraction::new(13, 10)),
            ..ItemBehavior::default()
        },
        ItemId::EXPERT_BELT => ItemBehavior {
            final_damage_boost: Some(FinalDamageBoost {
                numerator: 4915,
                only_super_effective: true,
            }),
            ..ItemBehavior::default()
        },
        ItemId::METRONOME => ItemBehavior {
            switch_in: Some(metronome::switch_in),
            condition: Some(crate::state::VolatileConditionKind::Metronome),
            ..ItemBehavior::default()
        },
        ItemId::QUICK_CLAW => ItemBehavior {
            fractional_priority: Some(FractionalPriority::QuickClaw),
            ..ItemBehavior::default()
        },
        ItemId::LAGGING_TAIL | ItemId::FULL_INCENSE => ItemBehavior {
            fractional_priority: Some(FractionalPriority::Delay),
            ..ItemBehavior::default()
        },
        ItemId::SAFETY_GOGGLES => ItemBehavior {
            weather_and_powder_immune: true,
            ..ItemBehavior::default()
        },
        ItemId::UTILITY_UMBRELLA => ItemBehavior {
            umbrella: true,
            ..ItemBehavior::default()
        },
        ItemId::POWER_ANKLET
        | ItemId::POWER_BAND
        | ItemId::POWER_BELT
        | ItemId::POWER_BRACER
        | ItemId::POWER_LENS
        | ItemId::POWER_WEIGHT
        | ItemId::MACHO_BRACE => ItemBehavior {
            modify_speed: Some(Fraction::new(1, 2)),
            ..ItemBehavior::default()
        },
        ItemId::IRON_BALL => ItemBehavior {
            modify_speed: Some(Fraction::new(1, 2)),
            flying_holder_takes_ground_neutrally: true,
            ..ItemBehavior::default()
        },
        // A Ditto that has not transformed, like Metal Powder.
        ItemId::QUICK_POWDER => ItemBehavior {
            speed_boost: untransformed_doubles(&[SpeciesId::DITTO], 0),
            ..ItemBehavior::default()
        },
        ItemId::FLOAT_STONE => ItemBehavior {
            weight_modifier: Some(Fraction::new(1, 2)),
            ..ItemBehavior::default()
        },
        // Gen 2 `accuracy - 20` on the byte; the cartridge's hold-effect parameter 10 (Gen 3-4);
        // [3686, 4096] from Gen 5.
        ItemId::BRIGHT_POWDER => ItemBehavior {
            evasion_boost: Some(AccuracyBoost {
                when: AccuracyWhen::Always,
                modifier: AccuracyModifier {
                    byte_penalty: 20,
                    ..AccuracyModifier::new(3686, 90, AccuracyStep::TargetItem)
                },
            }),
            ..ItemBehavior::default()
        },
        ItemId::LAX_INCENSE => ItemBehavior {
            evasion_boost: Some(AccuracyBoost {
                when: AccuracyWhen::Always,
                modifier: AccuracyModifier {
                    percent: AccuracyPercent::LaxIncense,
                    ..AccuracyModifier::new(3686, 90, AccuracyStep::TargetItem)
                },
            }),
            ..ItemBehavior::default()
        },
        ItemId::WIDE_LENS => ItemBehavior {
            accuracy_boost: Some(AccuracyBoost {
                when: AccuracyWhen::Always,
                modifier: AccuracyModifier::new(4505, 110, AccuracyStep::UserItem),
            }),
            ..ItemBehavior::default()
        },
        ItemId::ZOOM_LENS => ItemBehavior {
            accuracy_boost: Some(AccuracyBoost {
                when: AccuracyWhen::OpponentMoved,
                modifier: AccuracyModifier::new(4915, 120, AccuracyStep::UserItem),
            }),
            ..ItemBehavior::default()
        },
        ItemId::BUG_GEM => gem(PokemonType::Bug),
        ItemId::DARK_GEM => gem(PokemonType::Dark),
        ItemId::DRAGON_GEM => gem(PokemonType::Dragon),
        ItemId::ELECTRIC_GEM => gem(PokemonType::Electric),
        ItemId::FIGHTING_GEM => gem(PokemonType::Fighting),
        ItemId::FIRE_GEM => gem(PokemonType::Fire),
        ItemId::FLYING_GEM => gem(PokemonType::Flying),
        ItemId::GHOST_GEM => gem(PokemonType::Ghost),
        ItemId::GRASS_GEM => gem(PokemonType::Grass),
        ItemId::GROUND_GEM => gem(PokemonType::Ground),
        ItemId::ICE_GEM => gem(PokemonType::Ice),
        ItemId::NORMAL_GEM => gem(PokemonType::Normal),
        ItemId::POISON_GEM => gem(PokemonType::Poison),
        ItemId::PSYCHIC_GEM => gem(PokemonType::Psychic),
        ItemId::ROCK_GEM => gem(PokemonType::Rock),
        ItemId::STEEL_GEM => gem(PokemonType::Steel),
        ItemId::WATER_GEM => gem(PokemonType::Water),
        ItemId::OCCA_BERRY => resist_berry(PokemonType::Fire),
        ItemId::PASSHO_BERRY => resist_berry(PokemonType::Water),
        ItemId::WACAN_BERRY => resist_berry(PokemonType::Electric),
        ItemId::RINDO_BERRY => resist_berry(PokemonType::Grass),
        ItemId::YACHE_BERRY => resist_berry(PokemonType::Ice),
        ItemId::CHOPLE_BERRY => resist_berry(PokemonType::Fighting),
        ItemId::KEBIA_BERRY => resist_berry(PokemonType::Poison),
        ItemId::SHUCA_BERRY => resist_berry(PokemonType::Ground),
        ItemId::COBA_BERRY => resist_berry(PokemonType::Flying),
        ItemId::PAYAPA_BERRY => resist_berry(PokemonType::Psychic),
        ItemId::TANGA_BERRY => resist_berry(PokemonType::Bug),
        ItemId::CHARTI_BERRY => resist_berry(PokemonType::Rock),
        ItemId::KASIB_BERRY => resist_berry(PokemonType::Ghost),
        ItemId::HABAN_BERRY => resist_berry(PokemonType::Dragon),
        ItemId::COLBUR_BERRY => resist_berry(PokemonType::Dark),
        ItemId::BABIRI_BERRY => resist_berry(PokemonType::Steel),
        ItemId::ROSELI_BERRY => resist_berry(PokemonType::Fairy),
        // Normal moves whatever their effectiveness.
        ItemId::CHILAN_BERRY => ItemBehavior {
            weakening_berry: Some(WeakeningBerry {
                move_type: PokemonType::Normal,
                super_effective_only: false,
            }),
            eat_effect: Some(EatEffect::Nothing),
            ..ItemBehavior::default()
        },
        // Arceus's Plates: a type booster that also fixes its holder's type (and Judgment's).
        ItemId::DRACO_PLATE => plate(PokemonType::Dragon),
        ItemId::DREAD_PLATE => plate(PokemonType::Dark),
        ItemId::EARTH_PLATE => plate(PokemonType::Ground),
        ItemId::FIST_PLATE => plate(PokemonType::Fighting),
        ItemId::FLAME_PLATE => plate(PokemonType::Fire),
        ItemId::ICICLE_PLATE => plate(PokemonType::Ice),
        ItemId::INSECT_PLATE => plate(PokemonType::Bug),
        ItemId::IRON_PLATE => plate(PokemonType::Steel),
        ItemId::MEADOW_PLATE => plate(PokemonType::Grass),
        ItemId::MIND_PLATE => plate(PokemonType::Psychic),
        ItemId::PIXIE_PLATE => plate(PokemonType::Fairy),
        ItemId::SKY_PLATE => plate(PokemonType::Flying),
        ItemId::SPLASH_PLATE => plate(PokemonType::Water),
        ItemId::SPOOKY_PLATE => plate(PokemonType::Ghost),
        ItemId::STONE_PLATE => plate(PokemonType::Rock),
        ItemId::TOXIC_PLATE => plate(PokemonType::Poison),
        ItemId::ZAP_PLATE => plate(PokemonType::Electric),
        // Silvally's Memories: they fix its RKS System type and Multi-Attack's type.
        ItemId::BUG_MEMORY => memory(PokemonType::Bug),
        ItemId::DARK_MEMORY => memory(PokemonType::Dark),
        ItemId::DRAGON_MEMORY => memory(PokemonType::Dragon),
        ItemId::ELECTRIC_MEMORY => memory(PokemonType::Electric),
        ItemId::FAIRY_MEMORY => memory(PokemonType::Fairy),
        ItemId::FIGHTING_MEMORY => memory(PokemonType::Fighting),
        ItemId::FIRE_MEMORY => memory(PokemonType::Fire),
        ItemId::FLYING_MEMORY => memory(PokemonType::Flying),
        ItemId::GHOST_MEMORY => memory(PokemonType::Ghost),
        ItemId::GRASS_MEMORY => memory(PokemonType::Grass),
        ItemId::GROUND_MEMORY => memory(PokemonType::Ground),
        ItemId::ICE_MEMORY => memory(PokemonType::Ice),
        ItemId::POISON_MEMORY => memory(PokemonType::Poison),
        ItemId::PSYCHIC_MEMORY => memory(PokemonType::Psychic),
        ItemId::ROCK_MEMORY => memory(PokemonType::Rock),
        ItemId::STEEL_MEMORY => memory(PokemonType::Steel),
        ItemId::WATER_MEMORY => memory(PokemonType::Water),
        // Genesect's Drives: they fix Techno Blast's type.
        ItemId::BURN_DRIVE => drive(PokemonType::Fire),
        ItemId::CHILL_DRIVE => drive(PokemonType::Ice),
        ItemId::DOUSE_DRIVE => drive(PokemonType::Water),
        ItemId::SHOCK_DRIVE => drive(PokemonType::Electric),
        // Protective Pads: no callback of its own (Showdown handles them inside the simulator's
        // `checkMoveMakesContact`), so they carry this flag alone.
        ItemId::PROTECTIVE_PADS => ItemBehavior {
            pads_no_contact: true,
            ..ItemBehavior::default()
        },
        ItemId::RING_TARGET => ItemBehavior {
            negates_type_immunity: true,
            ..ItemBehavior::default()
        },
        ItemId::MAIL => ItemBehavior {
            take_refusal: Some(TakeRefusal::Mail),
            ..ItemBehavior::default()
        },
        // The crowned formes live in the `zacian` / `zamazenta` species conditions' BattleStart
        // (DESIGN.md §43.4, R269): the item itself only refuses to be taken.
        ItemId::RUSTED_SWORD => ItemBehavior {
            take_refusal: Some(TakeRefusal::Species(SpeciesId::ZACIAN.dex)),
            ..ItemBehavior::default()
        },
        ItemId::RUSTED_SHIELD => ItemBehavior {
            take_refusal: Some(TakeRefusal::Species(SpeciesId::ZAMAZENTA.dex)),
            ..ItemBehavior::default()
        },
        // The orbs of the creation trio (and Gen 9's crystals): ×1.2 to the holder's own two
        // types. Showdown reads the species it battles as in Gen 4, its own from Gen 5
        // (`item_species`).
        ItemId::ADAMANT_ORB | ItemId::ADAMANT_CRYSTAL => ItemBehavior {
            base_power_boost: Some(species_power(
                &[SpeciesId::DIALGA],
                &[PokemonType::Steel, PokemonType::Dragon],
            )),
            take_refusal: (item == ItemId::ADAMANT_CRYSTAL)
                .then_some(TakeRefusal::Species(SpeciesId::DIALGA.dex)),
            ..ItemBehavior::default()
        },
        ItemId::LUSTROUS_ORB | ItemId::LUSTROUS_GLOBE => ItemBehavior {
            base_power_boost: Some(species_power(
                &[SpeciesId::PALKIA],
                &[PokemonType::Water, PokemonType::Dragon],
            )),
            take_refusal: (item == ItemId::LUSTROUS_GLOBE)
                .then_some(TakeRefusal::Species(SpeciesId::PALKIA.dex)),
            ..ItemBehavior::default()
        },
        ItemId::GRISEOUS_ORB | ItemId::GRISEOUS_CORE => ItemBehavior {
            base_power_boost: Some(species_power(
                &[SpeciesId::GIRATINA],
                &[PokemonType::Ghost, PokemonType::Dragon],
            )),
            // Not from a Giratina (Gen 5-8); Gen 4 refuses it outright, whoever holds it
            // (`ItemTransferRules::griseous_orb_never_taken`).
            take_refusal: Some(TakeRefusal::Species(SpeciesId::GIRATINA.dex)),
            refuses_take_always: item == ItemId::GRISEOUS_ORB,
            // Gen 4's `onSetAbility: false` (the cartridge's Skill Swap, Role Play and Worry
            // Seed checks, issues.md I81).
            refuses_ability_change: item == ItemId::GRISEOUS_ORB,
            ..ItemBehavior::default()
        },
        // Ogerpon's masks: ×1.2 to every move while it holds the matching mask, and none can be
        // taken off an Ogerpon.
        ItemId::CORNERSTONE_MASK => mask(
            &[
                SpeciesId::OGERPON_CORNERSTONE,
                SpeciesId::OGERPON_CORNERSTONE_TERA,
            ],
            SpeciesId::OGERPON.dex,
        ),
        ItemId::HEARTHFLAME_MASK => mask(
            &[
                SpeciesId::OGERPON_HEARTHFLAME,
                SpeciesId::OGERPON_HEARTHFLAME_TERA,
            ],
            SpeciesId::OGERPON.dex,
        ),
        ItemId::WELLSPRING_MASK => mask(
            &[
                SpeciesId::OGERPON_WELLSPRING,
                SpeciesId::OGERPON_WELLSPRING_TERA,
            ],
            SpeciesId::OGERPON.dex,
        ),
        // The Primal Orbs: their holder reverts as it comes in (`onSwitchInPriority` −1),
        // which is the whole switch-in handler (`species_forme`).
        ItemId::BLUE_ORB => ItemBehavior {
            switch_in: Some(primal_reversion),
            switch_in_priority: -1,
            primal_orb: true,
            take_refusal: Some(TakeRefusal::Species(SpeciesId::KYOGRE.dex)),
            ..ItemBehavior::default()
        },
        ItemId::RED_ORB => ItemBehavior {
            switch_in: Some(primal_reversion),
            switch_in_priority: -1,
            primal_orb: true,
            take_refusal: Some(TakeRefusal::Species(SpeciesId::GROUDON.dex)),
            ..ItemBehavior::default()
        },
        ItemId::CHOICE_SCARF => ItemBehavior {
            boosts_suspended_by_dynamax: true,
            modify_speed: Some(Fraction::new(3, 2)),
            move_decided: Some(choice_band::move_decided),
            switch_in: Some(choice_band::switch_in),
            ..ItemBehavior::default()
        },
        ItemId::CHOICE_SPECS => ItemBehavior {
            boosts_suspended_by_dynamax: true,
            modify_offense: Some(OrderedHandler::new(
                0,
                choice_band::modify_special_offense as ModifyOffenseHandler,
            )),
            move_decided: Some(choice_band::move_decided),
            switch_in: Some(choice_band::switch_in),
            ..ItemBehavior::default()
        },
        ItemId::CHOICE_BAND => ItemBehavior {
            boosts_suspended_by_dynamax: true,
            modify_offense: Some(OrderedHandler::new(
                0,
                choice_band::modify_offense as ModifyOffenseHandler,
            )),
            move_decided: Some(choice_band::move_decided),
            switch_in: Some(choice_band::switch_in),
            ..ItemBehavior::default()
        },
        ItemId::LEFTOVERS => ItemBehavior {
            residual: Some(ResidualParticipant {
                kind: ResidualKind::Leftovers,
                handler: leftovers::residual as ResidualHandler,
            }),
            ..ItemBehavior::default()
        },
        ItemId::BLACK_SLUDGE => ItemBehavior {
            residual: Some(ResidualParticipant {
                kind: ResidualKind::BlackSludge,
                handler: black_sludge::residual as ResidualHandler,
            }),
            ..ItemBehavior::default()
        },
        ItemId::TOXIC_ORB => ItemBehavior {
            residual: Some(ResidualParticipant {
                kind: ResidualKind::StatusOrb,
                handler: status_orb::toxic_orb as ResidualHandler,
            }),
            fling_effect: Some(FlingEffect::Status(NonVolatileStatus::Toxic)),
            ..ItemBehavior::default()
        },
        ItemId::FLAME_ORB => ItemBehavior {
            residual: Some(ResidualParticipant {
                kind: ResidualKind::StatusOrb,
                handler: status_orb::flame_orb as ResidualHandler,
            }),
            fling_effect: Some(FlingEffect::Status(NonVolatileStatus::Burn)),
            ..ItemBehavior::default()
        },
        ItemId::EJECT_BUTTON => ItemBehavior {
            after_hit: Some(OrderedHandler::new(
                0,
                eject_button::after_hit as AfterHitHandler,
            )),
            ..ItemBehavior::default()
        },
        ItemId::RED_CARD => ItemBehavior {
            after_hit: Some(OrderedHandler::new(
                1,
                red_card::after_hit as AfterHitHandler,
            )),
            ..ItemBehavior::default()
        },
        // Gen 5+ `onDamage`, Gen 4 `onTryHit` (each generation's item has one of the two).
        ItemId::FOCUS_SASH => ItemBehavior {
            damage_resolution: Some(OrderedHandler::new(
                40,
                focus_sash::damage_resolution as DamageResolutionHandler,
            )),
            holds_for_the_move: Some(HoldsForTheMove::FromFullHp),
            ..ItemBehavior::default()
        },
        ItemId::ABSORB_BULB => hit_items::used_on_hit(
            UsedOnHitWhen::MoveType(crate::dex::PokemonType::Water),
            &[(Stat::SpecialAttack, 1)],
        ),
        ItemId::CELL_BATTERY => hit_items::used_on_hit(
            UsedOnHitWhen::MoveType(crate::dex::PokemonType::Electric),
            &[(Stat::Attack, 1)],
        ),
        ItemId::LUMINOUS_MOSS => hit_items::used_on_hit(
            UsedOnHitWhen::MoveType(crate::dex::PokemonType::Water),
            &[(Stat::SpecialDefense, 1)],
        ),
        ItemId::SNOWBALL => hit_items::used_on_hit(
            UsedOnHitWhen::MoveType(crate::dex::PokemonType::Ice),
            &[(Stat::Attack, 1)],
        ),
        ItemId::WEAKNESS_POLICY => hit_items::used_on_hit(
            UsedOnHitWhen::SuperEffective,
            &[(Stat::Attack, 2), (Stat::SpecialAttack, 2)],
        ),
        ItemId::ELECTRIC_SEED => field_items::used_on_field(
            FieldItemWhen::Terrain(crate::state::TerrainKind::Electric),
            &[(Stat::Defense, 1)],
        ),
        ItemId::GRASSY_SEED => field_items::used_on_field(
            FieldItemWhen::Terrain(crate::state::TerrainKind::Grassy),
            &[(Stat::Defense, 1)],
        ),
        ItemId::MISTY_SEED => field_items::used_on_field(
            FieldItemWhen::Terrain(crate::state::TerrainKind::Misty),
            &[(Stat::SpecialDefense, 1)],
        ),
        ItemId::PSYCHIC_SEED => field_items::used_on_field(
            FieldItemWhen::Terrain(crate::state::TerrainKind::Psychic),
            &[(Stat::SpecialDefense, 1)],
        ),
        ItemId::ROOM_SERVICE => {
            field_items::used_on_field(FieldItemWhen::TrickRoom, &[(Stat::Speed, -1)])
        }
        ItemId::ABILITY_SHIELD => ItemBehavior {
            refuses_ability_change: true,
            ..ItemBehavior::default()
        },
        ItemId::SHED_SHELL => ItemBehavior {
            frees_from_traps: true,
            ..ItemBehavior::default()
        },
        ItemId::WHITE_HERB => stage_items::white_herb(),
        ItemId::MIRROR_HERB => stage_items::mirror_herb(),
        ItemId::EJECT_PACK => eject_pack::eject_pack(),
        ItemId::SHELL_BELL => ItemBehavior {
            heals_off_damage_dealt: Some(Fraction::new(1, 8)),
            ..ItemBehavior::default()
        },
        ItemId::BIG_ROOT => ItemBehavior {
            boosts_absorbed_heals: true,
            ..ItemBehavior::default()
        },
        ItemId::DESTINY_KNOT => ItemBehavior {
            reciprocates_infatuation: true,
            ..ItemBehavior::default()
        },
        ItemId::THROAT_SPRAY => ItemBehavior {
            used_after_a_sound_move: Some(&[(Stat::SpecialAttack, 1)]),
            ..ItemBehavior::default()
        },
        ItemId::ADRENALINE_ORB => ItemBehavior {
            after_boost: Some(hit_items::adrenaline_orb as ItemAfterBoostHandler),
            ..ItemBehavior::default()
        },
        ItemId::JABOCA_BERRY => hit_items::recoil_berry(crate::dex::MoveCategory::Physical),
        ItemId::ROWAP_BERRY => hit_items::recoil_berry(crate::dex::MoveCategory::Special),
        ItemId::ENIGMA_BERRY => hit_items::enigma_berry(),
        ItemId::KEE_BERRY => hit_items::eaten_after_a_hit(
            crate::dex::MoveCategory::Physical,
            EatEffect::Boost(Stat::Defense),
        ),
        ItemId::MARANGA_BERRY => hit_items::eaten_after_a_hit(
            crate::dex::MoveCategory::Special,
            EatEffect::Boost(Stat::SpecialDefense),
        ),
        ItemId::FOCUS_BAND => ItemBehavior {
            damage_resolution: Some(OrderedHandler::new(
                40,
                focus_band::damage_resolution as DamageResolutionHandler,
            )),
            holds_for_the_move: Some(HoldsForTheMove::OnChance),
            ..ItemBehavior::default()
        },
        ItemId::COVERT_CLOAK => ItemBehavior {
            shields_secondaries: true,
            ..ItemBehavior::default()
        },
        ItemId::CLEAR_AMULET => ItemBehavior {
            try_stat_change: Some(clear_amulet::try_stat_change),
            ..ItemBehavior::default()
        },
        ItemId::CHERI_BERRY | ItemId::PRZ_CURE_BERRY => berries::status_berry(berries::PARALYSIS),
        ItemId::CHESTO_BERRY | ItemId::MINT_BERRY => berries::status_berry(berries::SLEEP),
        ItemId::PECHA_BERRY | ItemId::PSN_CURE_BERRY => berries::status_berry(berries::POISON),
        // Gen 2's Ice Berry cures a burn, its Burnt Berry thaws.
        ItemId::RAWST_BERRY | ItemId::ICE_BERRY => berries::status_berry(berries::BURN),
        ItemId::ASPEAR_BERRY | ItemId::BURNT_BERRY => berries::status_berry(berries::FREEZE),
        ItemId::PERSIM_BERRY | ItemId::BITTER_BERRY => berries::confusion_berry(),
        ItemId::LUM_BERRY | ItemId::MIRACLE_BERRY => berries::cure_all_berry(),
        // Gen 4+ `onUpdate`, Gen 3 `onResidual` (issues.md I34: the cartridge's end of turn).
        // Gen 4+ `onUpdate`, Gen 3 `onResidual` (Showdown gen3, pokeemerald `ENDTURN_ITEMS1`).
        ItemId::LIECHI_BERRY => berries::pinch_berry(EatEffect::Boost(Stat::Attack)),
        ItemId::GANLON_BERRY => berries::pinch_berry(EatEffect::Boost(Stat::Defense)),
        ItemId::SALAC_BERRY => berries::pinch_berry(EatEffect::Boost(Stat::Speed)),
        ItemId::PETAYA_BERRY => berries::pinch_berry(EatEffect::Boost(Stat::SpecialAttack)),
        ItemId::APICOT_BERRY => berries::pinch_berry(EatEffect::Boost(Stat::SpecialDefense)),
        ItemId::LANSAT_BERRY => berries::pinch_berry(EatEffect::FocusEnergy),
        // `onResidual` in every generation that has it; Gen 4 eats it at an `Update` instead
        // (`BerryRules::micle`, issues.md I42).
        ItemId::MICLE_BERRY => ItemBehavior {
            residual: Some(ResidualParticipant {
                kind: ResidualKind::MicleBerry,
                handler: berries::eat_at_pinch as ResidualHandler,
            }),
            eat_effect: Some(EatEffect::NextMoveAccuracy),
            ..ItemBehavior::default()
        },
        // Gen 4 `onBeforeTurn` / `onCustap`, Gen 5+ `onFractionalPriority`.
        ItemId::CUSTAP_BERRY => ItemBehavior {
            pinch_priority: true,
            eat_effect: Some(EatEffect::Nothing),
            ..ItemBehavior::default()
        },
        ItemId::STARF_BERRY => berries::pinch_berry(EatEffect::RandomBoost),
        // `onEat: false`: nothing to do in a battle (DESIGN.md R221).
        ItemId::RAZZ_BERRY
        | ItemId::BLUK_BERRY
        | ItemId::NANAB_BERRY
        | ItemId::WEPEAR_BERRY
        | ItemId::PINAP_BERRY
        | ItemId::POMEG_BERRY
        | ItemId::KELPSY_BERRY
        | ItemId::QUALOT_BERRY
        | ItemId::HONDEW_BERRY
        | ItemId::GREPA_BERRY
        | ItemId::TAMATO_BERRY
        | ItemId::CORNN_BERRY
        | ItemId::MAGOST_BERRY
        | ItemId::RABUTA_BERRY
        | ItemId::NOMEL_BERRY
        | ItemId::SPELON_BERRY
        | ItemId::PAMTRE_BERRY
        | ItemId::WATMEL_BERRY
        | ItemId::DURIN_BERRY
        | ItemId::BELUE_BERRY => ItemBehavior {
            eat_effect: Some(EatEffect::Inert),
            ..ItemBehavior::default()
        },
        ItemId::ORAN_BERRY => berries::healing_berry(EatEffect::Heal(BerryHeal::Fixed(10))),
        ItemId::SITRUS_BERRY => berries::healing_berry(EatEffect::SitrusHeal),
        // Gen 2 `onResidual` (pokecrystal `HandleHPHealingItem`).
        ItemId::BERRY => berries::healing_berry(EatEffect::Heal(BerryHeal::Fixed(10))),
        ItemId::GOLD_BERRY => berries::healing_berry(EatEffect::Heal(BerryHeal::Fixed(30))),
        ItemId::FIGY_BERRY => berries::confusion_berry_heal(crate::state::Stat::Attack),
        ItemId::WIKI_BERRY => berries::confusion_berry_heal(crate::state::Stat::SpecialAttack),
        ItemId::MAGO_BERRY => berries::confusion_berry_heal(crate::state::Stat::Speed),
        ItemId::AGUAV_BERRY => berries::confusion_berry_heal(crate::state::Stat::SpecialDefense),
        ItemId::IAPAPA_BERRY => berries::confusion_berry_heal(crate::state::Stat::Defense),
        // Gen 4-8 `onUpdate`, Gen 2 `onResidual`.
        ItemId::MENTAL_HERB => ItemBehavior {
            update: Some(ItemUpdate::CureVolatiles),
            fling_effect: Some(FlingEffect::CureMentalHerbVolatiles),
            ..ItemBehavior::default()
        },
        // Gen 4+ `onUpdate`; Gen 2-3 at the end of the turn (issues.md I49).
        ItemId::LEPPA_BERRY => berries::pp_berry(10),
        ItemId::MYSTERY_BERRY => berries::pp_berry(5),
        // `onStart` (switch-in priority −2), `onUpdate`, and `onTakeItem`: a Paradox Pokémon
        // keeps it (DESIGN.md R248, R249).
        ItemId::BOOSTER_ENERGY => ItemBehavior {
            switch_in: Some(crate::effects::items::booster_energy_start),
            switch_in_priority: -2,
            update: Some(ItemUpdate::BoosterEnergy),
            take_refusal: Some(TakeRefusal::Paradox),
            ..ItemBehavior::default()
        },
        ItemId::STICKY_BARB => sticky_barb::sticky_barb(),
        ItemId::BERSERK_GENE => ItemBehavior {
            update: Some(ItemUpdate::BerserkGene),
            ..ItemBehavior::default()
        },
        ItemId::BERRY_JUICE => ItemBehavior {
            update: Some(ItemUpdate::UseToHeal {
                at: HpShare::Half,
                amount: BerryHeal::Fixed(20),
            }),
            residual: Some(ResidualParticipant {
                kind: ResidualKind::Berry,
                handler: berries::use_juice as ResidualHandler,
            }),
            ..ItemBehavior::default()
        },
        ItemId::AIR_BALLOON => ItemBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                crate::effects::abilities::UNORDERED_REACTION,
                air_balloon::damaging_hit_reaction as DamagingHitReactionHandler,
            )),
            switch_in: Some(air_balloon::start),
            pops_on_substitute_hit: true,
            ..ItemBehavior::default()
        },
        ItemId::ROCKY_HELMET => ItemBehavior {
            damaging_hit_reaction: Some(OrderedHandler::new(
                2,
                rocky_helmet::damaging_hit_reaction as DamagingHitReactionHandler,
            )),
            ..ItemBehavior::default()
        },
        _ => ItemBehavior::default(),
    }
}

// The helpers below take the behavior the engine resolved for the generation
// (`engine::item_behavior`), never the raw table.

pub fn try_hit(behavior: &ItemBehavior, ctx: &mut TryHitContext<'_>) -> TryHitResult {
    behavior
        .try_hit
        .map_or(TryHitResult::Allow, |handler| handler(ctx))
}

pub fn try_set_status(
    behavior: &ItemBehavior,
    ctx: &TrySetStatusContext<'_>,
) -> TrySetStatusResult {
    behavior
        .try_set_status
        .map_or(TrySetStatusResult::Allow, |handler| handler(ctx))
}

pub fn after_set_status(
    behavior: &ItemBehavior,
    ctx: &mut BattleContext<'_>,
    target: PokemonId,
    status: NonVolatileStatus,
    source: EffectSource,
) {
    if let Some(handler) = behavior.after_set_status {
        handler(ctx, target, status, source);
    }
}

/// The handler's `chainModify` for the move, if any.
pub fn modify_offense(behavior: &ItemBehavior, ctx: &mut OffensiveStatContext<'_>) -> Option<u32> {
    behavior
        .modify_offense
        .and_then(|ordered| (ordered.handler)(ctx))
}
