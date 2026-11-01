//! How items change hands as a generation has it (DESIGN.md §39.3): Knock Off, Thief / Covet,
//! Sticky Hold, and the cartridge's own checks where they differ from Showdown (issues.md I53,
//! I54, I55, I90-I93).

use crate::spec::Generation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemTransferRules {
    /// Gen 4 (the gen4 mod's Griseous Orb, `onTakeItem: false`): the Orb is never taken,
    /// whoever holds it (Gen 5+ refuses it only from a Giratina, and Gen 9 has no such check).
    pub griseous_orb_never_taken: bool,
    /// Gen 5+ (a Plate's `onTakeItem`): a Plate stays with an Arceus, its holder or its taker.
    /// Gen 4's Plates are taken like any item (the gen4 mod's `onTakeItem: true`; pokeplatinum
    /// `BtlCmd_TryStealItem` refuses only a Multitype holder or taker, which the ability keeps:
    /// R-M8-105).
    pub plates_stay_with_arceus: bool,
    /// Gen 3-4 Knock Off (the gen4 mod's `onAfterHit`): `runEvent('TakeItem')` without
    /// `takeItem`'s knocked-off check, and the holder is marked for the battle
    /// (`PokemonState::item_knocked_off`); Gen 5+ the target takes its own item (`takeItem()`).
    pub knock_off_marks: bool,
    /// Gen 4 (Showdown `takeItem`: `itemKnockedOff` on either side; pokeplatinum
    /// `BtlCmd_TryStealItem`, `BattleSystem_CanStealItem`): a marked holder or taker takes
    /// nothing. Gen 3 link battles skip the check (pokeemerald `MOVE_EFFECT_STEAL_ITEM`,
    /// issues.md I54).
    pub knocked_off_blocks_taking: bool,
    /// Gen 3-4 (pokeemerald `Cmd_switchindataupdate`, pokeplatinum `BattleSystem_InitBattleMon`):
    /// a marked Pokémon enters holding nothing (issues.md I54).
    pub knocked_off_enters_empty: bool,
    /// Gen 3 (pokeemerald `SetMoveEffect`, issues.md I53): Knock Off takes nothing from a
    /// Pokémon its hit knocked out.
    pub knock_off_spares_fainted: bool,
    /// Gen 4 (pokeplatinum `subscript_knock_off.s`, `BtlCmd_TryStealItem`, issues.md I53):
    /// Knock Off, Thief and Covet take nothing from a target whose Quick Claw or Custap Berry
    /// went off this turn and that has not moved yet (`BattleState::priority_item_pending`).
    pub priority_item_keeps_item: bool,
    /// Gen 4 (pokeplatinum `BtlCmd_TryStealItem`, issues.md I53): Thief and Covet steal nothing
    /// when the user or the target has Multitype.
    pub steal_needs_no_multitype: bool,
    /// Gen 5 (Showdown Mail's `onTakeItem`): Thief and Covet may take Mail. Gen 2-4 only Knock
    /// Off may (pokecrystal `ItemIsMail`, pokeemerald `IS_ITEM_MAIL`, pokeplatinum
    /// `BattleSystem_CanStealItem`; issues.md I53).
    pub mail_stolen: bool,
    /// Gen 5+ Sticky Hold lets a fainted holder's item be taken (and a Sticky Barb, with that
    /// item); Gen 3-4 (the gen4 mod) refuses anyway.
    pub sticky_hold_spares_fainted: bool,
    /// Gen 6+ Incinerate burns a gem as well as a berry (the gen5 mod: berries alone).
    pub incinerate_burns_gems: bool,
    /// Gen 7+ Symbiosis gives nothing to a partner that is leaving (`pokemon.switchFlag`); the
    /// gen6 mod does not ask.
    pub symbiosis_skips_a_leaving_partner: bool,
    /// Gen 3-4 Choice items lock their holder in after the move (the gen4 mod's `onAfterMove`,
    /// onto `pokemon.lastMove`, which the gen3 mod inherits; pokeemerald's `MOVEEND_CHOICE_MOVE`
    /// writes `gChosenMove` then), with the item held then — one Trick took or gave locks as it
    /// now stands; from Gen 5 as the move is decided (`onModifyMove`). A Gen 3 Sleep Talk is not
    /// yet locked as it first runs (DESIGN.md R-M8-158).
    pub choice_locks_after_the_move: bool,
    /// Gen 3: a Pokémon that received an item during its own move (its Trick, Thief, Covet) is
    /// not locked by it as the move ends — pokeemerald hands the item over in
    /// `MOVEEND_CHANGED_ITEMS`, after `MOVEEND_CHOICE_MOVE` (issues.md I184: the oracle patch
    /// skips that holder's `onAfterMove`; DESIGN.md R-M8-158).
    pub choice_lock_skips_an_item_received_in_the_move: bool,
    /// Gen 3 (pokeemerald `Cmd_tryswapitems`, issues.md I184): a Trick that swapped clears both
    /// battlers' Choice locks (`choicedMove`), and the item the user took waits in
    /// `changedItems` until after `MOVEEND_CHOICE_MOVE`, so it locks nobody this move. The gen4
    /// mod's Choice items (Gen 3-4) have no `onStart`, so in Showdown a Choice item handed over
    /// leaves the receiver's lock; pokeplatinum's swap leaves `choiceLockedMove` too (R-M8-108).
    pub trick_frees_choice_locks: bool,
    /// Gen 4 (pokeplatinum `UpdateMoveBuffers`, issues.md I55): a Choice lock ends once its
    /// holder's item is set aside (Klutz, Embargo: `Battler_HeldItem` reads nothing) — the
    /// cartridge clears it after any move the holder uses or is hit by, the first of which is
    /// the one that set the item aside. Showdown keeps the lock, held off while the item is set
    /// aside and back in force after.
    pub choice_lock_ends_while_set_aside: bool,
    /// Gen 7+ (`choicelock.onBeforeMove`): a lock ends as its holder gets to move without a
    /// Choice item (Knock Off, Corrosive Gas took it) — a frame before the turn's end sees it
    /// gone — and refuses any other move the holder gets to (a charge a caller brought in,
    /// R-M8-139). The gen6 mod (Gen 3-6) has no `onBeforeMove`: the lock waits for
    /// `DisableMove` and holds only the request (Gen 3-4 cartridges: `CheckInvalidMoves`).
    pub choice_lock_before_move: bool,
    /// Gen 5+ Klutz keeps its holder from flinging (Showdown `ignoringItem(true)`: the Klutz
    /// step only from Gen 5, whatever the item is; Embargo and Magic Room always). Gen 4 Klutz
    /// flings: pokeplatinum reads the held item itself (`Battler_ItemFlingPower`).
    pub fling_klutz_blocks: bool,
    /// Gen 4 (pokeplatinum `effect_script_0233.s`, issues.md I90): Fling fails for a user with
    /// Multitype or holding a Griseous Orb, whatever it holds. Showdown asks only the item's own
    /// `TakeItem`.
    pub fling_refuses_multitype: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_FlingItem`, issues.md I91): a flung berry's effect
    /// reaches a target with Klutz — only Embargo keeps it away. Showdown's `singleEvent('Eat')`
    /// skips it for a target that sets items aside.
    pub flung_berry_ignores_klutz: bool,
    /// Gen 4 (pokeplatinum `BattleSystem_FlingItem`, issues.md I92): Embargo on the target keeps
    /// a flung item's status or flinch away too (`flingScript = 0`). Showdown's are secondaries,
    /// which Embargo does not touch.
    pub embargo_stops_flung_effects: bool,
    /// Gen 4 (pokeplatinum `subscript_pluck.s`: `RemoveItem` on the target writes its
    /// `recycleItem`, issues.md I93): the berry Bug Bite or Pluck takes is the target's last
    /// item, which Recycle brings back. Showdown's `takeItem` leaves `lastItem` alone.
    pub plucked_berry_recyclable: bool,
    /// Gen 4 Sticky Barb (pokeplatinum `BattleSystem_TriggerHeldItemOnHit` and
    /// `…OnPivotMove`, issues.md I100): the barb changes hands after the contact abilities
    /// (Rough Skin) rather than at the `Hit` event before them, and not to a user knocked out by
    /// then, nor for a Knock Off, nor to a user a Knock Off marked. Showdown hands it over first,
    /// for any contact move, asking only the holder's mark (`takeItem()`).
    pub sticky_barb_after_contact_abilities: bool,
    /// Gen 4 (pokeplatinum `subscript_future_sight_damage`, issues.md I165): a foreseen strike
    /// (Future Sight, Doom Desire) takes the HP and runs no `BattleSystem_TriggerHeldItemOnHit`
    /// — Jaboca, Rowap, Enigma Berry and Sticky Barb do not answer it. Showdown runs the
    /// strike's hit events as any hit's.
    pub foreseen_strike_skips_hit_items: bool,
}

impl ItemTransferRules {
    pub(crate) const fn for_generation(generation: Generation) -> Self {
        Self {
            choice_locks_after_the_move: matches!(generation, Generation::Gen3 | Generation::Gen4),
            choice_lock_skips_an_item_received_in_the_move: matches!(generation, Generation::Gen3),
            trick_frees_choice_locks: matches!(generation, Generation::Gen3),
            choice_lock_ends_while_set_aside: matches!(generation, Generation::Gen4),
            choice_lock_before_move: matches!(
                generation,
                Generation::Gen7 | Generation::Gen8 | Generation::Gen9
            ),
            fling_klutz_blocks: !matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
            fling_refuses_multitype: matches!(generation, Generation::Gen4),
            flung_berry_ignores_klutz: matches!(generation, Generation::Gen4),
            embargo_stops_flung_effects: matches!(generation, Generation::Gen4),
            plucked_berry_recyclable: matches!(generation, Generation::Gen4),
            sticky_barb_after_contact_abilities: matches!(generation, Generation::Gen4),
            foreseen_strike_skips_hit_items: matches!(generation, Generation::Gen4),
            symbiosis_skips_a_leaving_partner: !matches!(
                generation,
                Generation::Gen1
                    | Generation::Gen2
                    | Generation::Gen3
                    | Generation::Gen4
                    | Generation::Gen5
                    | Generation::Gen6
            ),
            incinerate_burns_gems: !matches!(
                generation,
                Generation::Gen1
                    | Generation::Gen2
                    | Generation::Gen3
                    | Generation::Gen4
                    | Generation::Gen5
            ),
            knock_off_marks: matches!(generation, Generation::Gen3 | Generation::Gen4),
            knocked_off_blocks_taking: matches!(generation, Generation::Gen4),
            knocked_off_enters_empty: matches!(generation, Generation::Gen3 | Generation::Gen4),
            knock_off_spares_fainted: matches!(generation, Generation::Gen3),
            priority_item_keeps_item: matches!(generation, Generation::Gen4),
            steal_needs_no_multitype: matches!(generation, Generation::Gen4),
            griseous_orb_never_taken: matches!(generation, Generation::Gen4),
            plates_stay_with_arceus: !matches!(generation, Generation::Gen4),
            mail_stolen: !matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
            sticky_hold_spares_fainted: !matches!(
                generation,
                Generation::Gen1 | Generation::Gen2 | Generation::Gen3 | Generation::Gen4
            ),
        }
    }
}
