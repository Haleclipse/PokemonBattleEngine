//! Event history (DESIGN.md §22.8): what happened to and by a Pokémon recently, kept with
//! Showdown's own set and reset points so that move formulas reading it (Payback, Revenge,
//! Assurance, Stomping Tantrum, Rage Fist, Lash Out …) see what Showdown's do.
//!
//! Three moments shape every field: an event sets it (a hit, a stat drop, a move's result),
//! the Pokémon leaving the field clears it (Showdown `clearVolatile`), and the turn boundary
//! ages it (Showdown `endTurn`, for the Pokémon on the field).

use crate::ids::{MoveId, PokemonId, PositionId};
use serde::{Deserialize, Serialize};

/// How a Pokémon's move went (Showdown `moveThisTurnResult`), as Stomping Tantrum reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveResult {
    /// `true`: the move did something to at least one target.
    Succeeded,
    /// `false`: the move failed — it missed, was refused or immune, found no target, changed
    /// nothing, or the user could not act (full paralysis, flinch, sleep, …).
    Failed,
    /// `null`: neither — the user was recharging, charged a two-turn move, every target
    /// protected itself, or the move reported "not a failure" (Moonlight at full HP).
    Neither,
}

/// One attack a Pokémon took this turn (Showdown `attackedBy` entry with `thisTurn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttackRecord {
    #[serde(with = "crate::snapshot::wire")]
    pub source: PokemonId,
    /// The HP the move's last hit took; `None` for a status move (Showdown's non-numeric
    /// `damageValue`) or a Gen 3 hit its Substitute took (`MovePipeline::record_attack`).
    pub damage: Option<u16>,
    /// The move (Showdown `move`): Counter / Mirror Coat read its category, Gen 2 Counter
    /// checks it is the attacker's last move.
    #[serde(with = "crate::snapshot::wire")]
    pub move_id: MoveId,
    /// Where the attacker stood (Showdown `slot`): Counter and Metal Burst strike back there.
    #[serde(with = "crate::snapshot::wire")]
    pub position: Option<PositionId>,
    /// The move's category in the dex (Showdown `getCategory(move)`): what Counter and Mirror
    /// Coat answer.
    pub category: crate::dex::MoveCategory,
    /// A damaging hit that is no attack (Showdown runs `DamagingHit` but not `gotAttacked`): Gen
    /// 5+ Bide's release. Counter and Mirror Coat see it (their own `onDamagingHit`); Metal
    /// Burst, Revenge and Avalanche (`attackedBy`) do not (DESIGN.md R-M9-28).
    pub damaging_hit_only: bool,
}

/// A kind of switch-in effect the Gen 4 cartridge marks as announced for a stint on the field
/// (pokeplatinum `battleMons[].…Announced`, cleared as the Pokémon comes in): the weather
/// abilities share one (`StartNewAbility::Unannounced`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchInAnnouncement {
    Weather,
    Intimidate,
    Download,
    SlowStart,
    Frisk,
    Anticipation,
    Forewarn,
    MoldBreaker,
    Trace,
    Pressure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PokemonHistory {
    /// Showdown `newlySwitched`: the Pokémon has not been on the field across a turn boundary
    /// since it last came in. Set when it leaves (and when it is built), cleared at the next
    /// boundary it spends on the field.
    pub newly_switched: bool,
    /// Showdown `moveThisTurnResult`: `None` until the Pokémon's move of this turn has run.
    pub move_this_turn: Option<MoveResult>,
    /// Showdown `moveLastTurnResult`: the previous turn's `move_this_turn`.
    pub move_last_turn: Option<MoveResult>,
    /// Showdown `statsLoweredThisTurn`: a stat stage went down this turn.
    pub stats_lowered_this_turn: bool,
    /// Showdown `statsRaisedThisTurn`: a stat stage went up this turn (Burning Jealousy).
    pub stats_raised_this_turn: bool,
    /// Showdown `hurtThisTurn`: the Pokémon lost HP this turn (to anything but a direct cost).
    pub hurt_this_turn: bool,
    /// Showdown `attackedBy` entries with `thisTurn`: the moves that reached the Pokémon this
    /// turn. Showdown keeps older entries flagged `thisTurn = false`; every reader either
    /// requires `thisTurn` or reads the latest entry and then requires it, so they are dropped
    /// at the boundary instead.
    pub attacked_this_turn: Vec<AttackRecord>,
    /// Showdown `attackedBy` across turn boundaries, oldest first: each attacker's latest
    /// record. Showdown keeps every record, but drops an attacker's all at once — at a turn
    /// boundary that finds it off the field — and its one reader across turns wants the last
    /// (`getLastAttackedBy`, Gen 3-4 Mirror Move), so the attacker's latest stands for them all.
    pub attacked_by: Vec<AttackRecord>,
    /// Showdown `timesAttacked` (Rage Fist): hits taken by damaging moves, across switches.
    pub times_attacked: u16,
    /// Showdown `swordBoost` / `shieldBoost` (Gen 9 Intrepid Sword / Dauntless Shield): the
    /// entry boost has been given this battle — across switches.
    pub sword_boost: bool,
    pub shield_boost: bool,
    /// Showdown `syrupTriggered`: Supersweet Syrup has acted this battle — across switches.
    pub syrup_triggered: bool,
    /// Showdown `lastItem` (DESIGN.md §39.1): the item the Pokémon last used up or ate — across
    /// switches; Gen 1-4 hand it to the Pokémon that takes its place (`switchIn`).
    #[serde(with = "crate::snapshot::wire")]
    pub last_item: Option<crate::ids::ItemId>,
    /// Showdown `usedItemThisTurn`: the Pokémon used up or ate an item this turn.
    pub used_item_this_turn: bool,
    /// Showdown `ateBerry`: the Pokémon has eaten a berry this battle — across switches.
    pub ate_berry: bool,
    /// The switch-in effects its abilities started this stint on the field (Gen 4's announced
    /// flags; `BattleContext::switch_in_ability` / `start_ability`).
    pub announced: Vec<SwitchInAnnouncement>,
    /// Showdown `itemKnockedOff`: a Gen 3-4 Knock Off took its item (`ItemTransferRules`),
    /// for the rest of the battle.
    pub item_knocked_off: bool,
}

impl Default for PokemonHistory {
    /// A Pokémon that has not been on the field: newly switched, as Showdown's constructor
    /// leaves it (`clearVolatile`). The battle's first turn boundary clears it for the leads.
    fn default() -> Self {
        Self {
            newly_switched: true,
            move_this_turn: None,
            move_last_turn: None,
            stats_lowered_this_turn: false,
            stats_raised_this_turn: false,
            hurt_this_turn: false,
            attacked_this_turn: Vec::new(),
            attacked_by: Vec::new(),
            times_attacked: 0,
            sword_boost: false,
            shield_boost: false,
            syrup_triggered: false,
            last_item: None,
            used_item_this_turn: false,
            ate_berry: false,
            announced: Vec::new(),
            item_knocked_off: false,
        }
    }
}

impl PokemonHistory {
    /// Showdown `clearVolatile` (switching out, fainting) and the switch-out reset of
    /// `statsLoweredThisTurn` and `usedItemThisTurn`: everything but `times_attacked`, the
    /// once-per-battle entry boosts, the last item and the eaten berry starts over.
    pub fn leave_field(&mut self) {
        *self = Self {
            times_attacked: self.times_attacked,
            sword_boost: self.sword_boost,
            shield_boost: self.shield_boost,
            syrup_triggered: self.syrup_triggered,
            last_item: self.last_item,
            ate_berry: self.ate_berry,
            item_knocked_off: self.item_knocked_off,
            ..Self::default()
        };
    }

    /// Showdown `endTurn` for a Pokémon on the field (`active`: the Pokémon on the field then;
    /// an attacker no longer among them takes its records along, `attackedBy.splice`).
    pub fn next_turn(&mut self, active: &[crate::ids::PokemonId]) {
        self.attacked_by
            .retain(|record| active.contains(&record.source));
        self.newly_switched = false;
        self.move_last_turn = self.move_this_turn.take();
        self.stats_lowered_this_turn = false;
        self.stats_raised_this_turn = false;
        self.hurt_this_turn = false;
        self.attacked_this_turn.clear();
        self.used_item_this_turn = false;
    }

    /// Showdown `attackedBy.some(p => p.source === source && p.damage > 0 && p.thisTurn)`
    /// (Revenge, Avalanche).
    pub fn damaged_this_turn_by(&self, source: PokemonId) -> bool {
        self.attacked_this_turn.iter().any(|record| {
            !record.damaging_hit_only
                && record.source == source
                && record.damage.is_some_and(|hp| hp > 0)
        })
    }
}
