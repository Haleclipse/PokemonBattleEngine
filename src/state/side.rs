use crate::ids::{MoveId, PokemonId};

use super::SideConditions;

/// Showdown gen1 `side.lastSelectedMove` (DESIGN.md §30.9): what a Pokémon that does not pick
/// from the menu — asleep, frozen, or held to a move — acts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedMove {
    /// A move picked from the menu (Struggle too).
    Move(MoveId),
    /// `cannotmove`: left by a partially trapped Pokémon's turn and by Gen 1 Haze on a sleeping
    /// or frozen one. The action is lost (a sleeping Pokémon's count does not go down) until
    /// the side picks from the menu again.
    CannotMove,
}

impl SelectedMove {
    pub const fn move_id(self) -> Option<MoveId> {
        match self {
            Self::Move(move_id) => Some(move_id),
            Self::CannotMove => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MechanicUsage {
    Unavailable,
    Available,
    Consumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideMechanicState {
    pub mega: MechanicUsage,
    /// Ultra Burst (Q1): counted apart from Mega Evolution.
    pub ultra_burst: MechanicUsage,
    pub z_move: MechanicUsage,
    pub dynamax: MechanicUsage,
    pub tera: MechanicUsage,
}

impl Default for SideMechanicState {
    fn default() -> Self {
        Self {
            mega: MechanicUsage::Unavailable,
            ultra_burst: MechanicUsage::Unavailable,
            z_move: MechanicUsage::Unavailable,
            dynamax: MechanicUsage::Unavailable,
            tera: MechanicUsage::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideState {
    pub team: Vec<PokemonId>,
    /// The party in its in-battle order (Showdown `side.pokemon`): a switch swaps the entrant
    /// with the Pokémon standing at its slot's index. `team` keeps the order the sides chose.
    pub battle_order: Vec<PokemonId>,
    pub active: Vec<Option<PokemonId>>,
    /// The Pokémon that last fainted at each active slot. Showdown keeps it in `side.active`
    /// until a replacement comes: a future move with no foe standing still aims at it (and
    /// Gen 2-4 compute the foreseen damage against it).
    pub fallen: Vec<Option<PokemonId>>,
    pub mechanics: SideMechanicState,
    pub conditions: SideConditions,
    /// An active Pokémon ordered out mid-turn (U-turn / Volt Switch). Cleared when the
    /// replacement is made or when no bench Pokémon can come in.
    pub switch_out: Option<PokemonId>,
    /// What the Pokémon in `switch_out` hands on to its replacement (Baton Pass, Shed Tail);
    /// written together with `switch_out` (DESIGN.md §30.7).
    pub passing: Option<PassKind>,
    /// The Pokémon in `switch_out` was ordered out by its own move (U-turn, Parting Shot:
    /// Showdown's `switchFlag` holds the move's id) rather than by an item or an ability
    /// (`switchFlag === true`: Eject Button, Eject Pack, Emergency Exit). Written together with
    /// `switch_out`.
    pub switch_out_by_move: bool,
    /// A Pokémon of this side fainted this turn (Showdown `faintedThisTurn`); cleared when the
    /// next turn starts. Gen 2 spares a Pokémon switching in on such a side its after-switch
    /// status damage.
    pub fainted_this_turn: bool,
    /// Showdown `faintedLastTurn`: `fainted_this_turn` of the turn before (Retaliate).
    pub fainted_last_turn: bool,
    /// Showdown `totalFainted` (Last Respects): this side's faints over the battle, at most 100.
    pub total_fainted: u8,
    /// Showdown `lastSelectedMove`: the move this side last picked from the move menu, kept
    /// across switches (Gen 1 turn order, `ActionOrderRules`), or `cannotmove`. `None` before
    /// any pick.
    pub last_selected_move: Option<SelectedMove>,
    /// Showdown `side.lastSelectedMoveSlot` (Gen 1): the slot that pick came from, kept across
    /// switches (Gen 1 Mimic overwrites it — a called Mimic overwrites the caller's slot). 0
    /// before any pick, as Showdown's initial value.
    pub last_selected_move_slot: usize,
    /// Gen 1 (Showdown `side.lastMove`): the move this side last used, kept across switches
    /// (Gen 1 Counter).
    pub last_move: Option<MoveId>,
    /// Gen 3 Follow Me (Showdown `slotCondition: 'followme'`): the active slots whose occupants
    /// draw the foes' single-target moves until the end of the turn — whoever stands there,
    /// the Follow Me user or the Pokémon that replaced it.
    pub follow_me_slots: Vec<usize>,
    /// Wish (Showdown `slotCondition: 'Wish'`): pending on an active slot, it heals whoever
    /// stands there at the end of the next turn.
    pub wishes: Vec<WishState>,
    /// Future Sight / Doom Desire foreseen at the side's active slots (Showdown's `futuremove`
    /// slot condition, DESIGN.md §15.22).
    pub future_attacks: Vec<FutureAttack>,
    /// Healing Wish / Lunar Dance (Showdown `slotCondition`): the Pokémon that comes into the
    /// slot is healed (DESIGN.md §30.5).
    pub slot_heals: Vec<SlotHeal>,
    /// Revival Blessing (Showdown `slotCondition: 'revivalblessing'`): its user, waiting for the
    /// side to name the fainted Pokémon that comes back (`BattleRequest::Revive`).
    pub reviving: Option<PokemonId>,
    /// A partner Revival Blessing brought back into its own, still empty slot: it comes in
    /// once the turn's residual is over (Showdown's `instaswitch`, queued behind it).
    pub returning: Option<PokemonId>,
    /// Gen 4 Healing Wish / Lunar Dance (`onAfterMove`: `switchFlag`): the user's slot, filled
    /// at once, mid-turn, rather than once the turn is over. Only that slot: another emptied by
    /// a faint this turn waits for the turn's end (Showdown's request is the slots' `switchFlag`).
    pub refill_mid_turn: Option<usize>,
}

/// What a Pokémon leaving by Baton Pass or Shed Tail hands on (Showdown `copyVolatileFrom`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    /// Baton Pass (`selfSwitch: 'copyvolatile'`): the stat stages and every volatile the
    /// generation lets through (`SwitchingRules::not_passed`).
    BatonPass,
    /// Shed Tail (`'shedtail'`): the Substitute alone.
    ShedTail,
}

/// A Healing Wish or Lunar Dance waiting on one of the side's active slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotHeal {
    pub slot: usize,
    pub kind: SlotHealKind,
    /// The Pokémon that used it — whose speed orders its residual handler (Gen 4-7) while it
    /// lies fainted in the slot.
    pub user: PokemonId,
    /// Gen 4-7 (`duration`): the residuals left before it lapses; Gen 8+ `None`, until a
    /// Pokémon that needs it comes in.
    pub turns_remaining: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotHealKind {
    /// HP and status.
    HealingWish,
    /// HP, status and PP.
    LunarDance,
}

/// A pending Wish on one of the side's active slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WishState {
    pub slot: usize,
    pub source: PokemonId,
    /// Gen 5+: half the wisher's max HP, taken as it wished (`effectState.hp`).
    pub amount: u16,
    /// Residuals left: the one of the turn it was made, then the one that heals.
    pub turns_remaining: u8,
}

/// A Future Sight / Doom Desire waiting on one of the side's active slots (`futuremove`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FutureAttack {
    pub slot: usize,
    /// The Pokémon that foresaw it, on the field or not.
    pub source: PokemonId,
    pub move_id: MoveId,
    /// Gen 2-4 (`FutureAttackRules::damage_at_use`): the damage computed as the move was used.
    pub damage: Option<u16>,
    /// Residuals left: the one of the turn it was foreseen, the next, then the one it strikes.
    pub turns_remaining: u8,
}

impl SideState {
    pub fn new(team: Vec<PokemonId>, active_slots: usize) -> Self {
        Self {
            battle_order: team.clone(),
            team,
            active: vec![None; active_slots],
            fallen: vec![None; active_slots],
            mechanics: SideMechanicState::default(),
            conditions: SideConditions::default(),
            switch_out: None,
            passing: None,
            switch_out_by_move: false,
            fainted_this_turn: false,
            fainted_last_turn: false,
            total_fainted: 0,
            last_selected_move: None,
            last_selected_move_slot: 0,
            last_move: None,
            follow_me_slots: Vec::new(),
            wishes: Vec::new(),
            future_attacks: Vec::new(),
            slot_heals: Vec::new(),
            reviving: None,
            returning: None,
            refill_mid_turn: None,
        }
    }

    pub fn is_active(&self, pokemon: PokemonId) -> bool {
        self.active.contains(&Some(pokemon))
    }
}
