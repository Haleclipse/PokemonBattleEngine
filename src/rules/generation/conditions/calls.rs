//! Moves that call or copy moves (`CallRules`).

/// A move that calls or copies other moves (DESIGN.md §42.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallRules {
    /// Gen 1 (`useMove`): the called move becomes the user's last move (Encore, Disable, Mimic
    /// read it). Gen 2 clears it as the chosen move is used; Gen 3+ write the chosen move.
    pub called_sets_last_move: bool,
    /// Gen 2, Gen 4: Metronome cannot draw a move the user's own move slots hold.
    pub metronome_excludes_known: bool,
    /// Where Mirror Move takes its move from.
    pub mirror_move: MirrorMoveSource,
    /// The moves each generation's Mirror Move refuses beside its source rule (Showdown's
    /// `noMirror` tables).
    pub mirror_tables: MirrorTables,
    /// Gen 4: Metronome, Assist and Copycat cannot draw a move Gravity grounds or a Heal Block
    /// on the user blocks.
    pub filters_gravity_and_heal_block: bool,
    /// Nature Power's fixed move before Gen 6 (Swift, Tri Attack, Earthquake); `None` from
    /// Gen 6, where it follows the terrain.
    pub nature_power: Option<crate::ids::MoveId>,
    /// Gen 3: a Sleep Talk draw that lands on a move with no PP left is `cant|p|nopp|move`.
    pub sleep_talk_names_depleted: bool,
    /// Gen 3-4 (`onTryHit`, which the gen3 mod inherits from gen4's): Sleep Talk fails while the
    /// user is choice-locked or Encored — pokeemerald's `Cmd_trychoosesleeptalkmove` finds every
    /// other move barred by `CheckMoveLimitations` (Choice Band, Encore) (DESIGN.md R-M8-158).
    pub sleep_talk_fails_locked: bool,
    /// Gen 4 (Showdown restores `activeMove` at the end of the outermost `runMove`): Copycat
    /// reads the outermost move of the action; Gen 5+ the innermost.
    pub copycat_reads_outermost: bool,
    /// Gen 3-4 (the gen4 mod's Magic Coat `onTryHit` removes the volatile, and Gen 3 inherits
    /// it; pokeemerald's `Cmd_attackcanceler` clears `bounceMove`): reflecting ends the coat at
    /// once; Gen 5+ leave it for the turn (DESIGN.md §42.8, R-M8-68).
    pub magic_coat_ends_on_reflection: bool,
    /// Gen 6+ (`onAllyTryHitSide` returns for a semi-invulnerable `target`; the gen5 mod's has
    /// no such check): a reflectable move aimed at the foes' side — reflected by the first of
    /// them, fastest first, wearing Magic Coat or holding Magic Bounce — is not when the
    /// Pokémon in the side's first position, the event's target (`foes(true)[0]`), is hiding
    /// (Fly, Dig … Sky Drop). Gen 3-4's side moves are not `reflectable` (the gen4 data;
    /// pokeplatinum's hazards lack `MOVE_FLAG_CAN_MAGIC_COAT`). Side moves are the only ones a
    /// partner reflects: a move aimed at a Pokémon is the target's own (`onTryHit`, DESIGN.md
    /// R-M8-77).
    pub hiding_first_stops_side_reflection: bool,
    /// Gen 3-4: a Pokémon that takes another's move over (Magic Coat, Snatch) pays one PP when
    /// that Pokémon has Pressure, whatever its side (pokeemerald `PressurePPLose`, pokeplatinum
    /// `BattleSystem_DecPPForPressure`; Showdown's gen4 Snatch, which Gen 3 inherits, but not its
    /// Magic Coat — issues.md I169). Gen 5+: nobody pays.
    pub taker_pays_pressure: Option<PressurePaidFrom>,
    /// Gen 1 (pokered `MetronomePickMove` / `MirrorMoveCopyMove` write `wPlayerSelectedMove`;
    /// Showdown's gen1 `onHit`): Metronome and Mirror Move make the move they call the side's
    /// last selected move — what a frozen or sleeping Pokémon's next action and Counter read
    /// (DESIGN.md R-M8-72).
    pub caller_selects_the_called_move: bool,
    /// Snatch is Gen 3-7 (`isNonstandard: Past` from Gen 8).
    pub snatch_exists: bool,
    /// Gen 3-4: a move Snatch took is taken again by another holder (the gen4 handler has no
    /// `sourceEffect === 'snatch'` check, and Gen 3 inherits it; pokeemerald's
    /// `BattleScript_SnatchedMove` returns into `attackcanceler`, which the snatcher runs
    /// again). Gen 5+ leave a snatched move alone (DESIGN.md R-M8-68).
    pub snatch_takes_a_snatched_move: bool,
    /// Gen 3 (the gen3 `tryMoveHit` runs the `PrepareHit` event before the move's `Try`;
    /// pokeemerald's `attackcanceler` takes a snatched move before Swallow's
    /// `stockpiletohpheal` or Rest's `trysetrest` runs): Snatch takes a move its own `onTry`
    /// would fail (a Swallow with nothing stockpiled). Gen 4+ (`trySpreadMoveHit`) run `Try`
    /// first (DESIGN.md R-M8-102).
    pub snatch_before_try: bool,
    /// Gen 5+: two allies' different Pledge moves used in the same turn combine (the second
    /// one runs first, at 150 power, with the pair's type and side condition; DESIGN.md §42.9).
    pub pledge_combines: bool,
    /// A combined Water Pledge doubles the side's secondaries (its `onModifyMove`).
    pub water_pledge_doubles_secondaries: bool,
    /// How long a Pledge combination's side condition lasts (Showdown's `duration: 4`).
    pub pledge_turns: u8,
    /// Gen 9 (`onPrepareHit`): Ally Switch rolls its consecutive-use chance (1 / the
    /// `allyswitch` counter) and fails on a miss; Gen 5-8 always succeed.
    pub ally_switch_consecutive_chance: bool,
    /// Gen 4+ (pokeplatinum `BattleControllerPlayer_FightCommand` asks Encore before the lock;
    /// Showdown's `onOverrideAction` from Gen 5): Encore turns a locked action — a charge or
    /// rampage a caller brought in, which the user's slots do not hold — into the encored move.
    /// Gen 3 (pokeemerald `HandleAction_UseMove`, issues.md I182) keeps the lock; Gen 2 too, as
    /// before (unchecked) (R-M8-137).
    pub encore_overrides_a_called_lock: bool,
    /// Gen 2+ (`source.moves.includes(move.id)`): Mimic refuses a move its user already knows.
    /// Gen 1's `onHit` writes the draw straight in, however bad a match it is.
    pub mimic_refuses_a_known_move: bool,
    /// Gen 1 Mimic (`this.sample(target.moves)`): the copied move is drawn from the target's
    /// slots rather than being its last move (DESIGN.md §42.5).
    pub mimic_draws_from_the_slots: bool,
    /// Gen 2+ (`if (source.transformed …) return false`): a transformed Pokémon's Mimic fails.
    /// Gen 1's goes ahead and writes the copies Transform made (DESIGN.md R-M8-75).
    pub mimic_fails_when_transformed: bool,
    /// Gen 1's Transform + Mirror Move / Metronome PP error (the gen1 `useMove`): a transformed
    /// Pokémon's move called by Metronome or Mirror Move gives the own slot the side last picked
    /// one PP back (`baseMoveSlots[lastSelectedMoveSlot].pp += 1`, mod 64; DESIGN.md R-M8-64).
    pub transformed_caller_restores_own_pp: bool,
    /// Gen 1 Mimic keeps the slot's PP (`source.moveSlots[moveslot].pp`): the copy and the slot
    /// share one PP byte, so what is spent from the copy is the slot's too (the gen1
    /// `deductPP` syncs a `virtual` slot into `baseMoveSlots`, DESIGN.md R-M8-54).
    pub mimic_keeps_the_slot_pp: bool,
    /// Gen 2-4 Mimic caps the copied move's PP at 5 (the gen4 mod's `onHit`, which Gen 2-3
    /// inherit; Gen 5+ use the base's `pp: move.pp` again).
    pub mimic_pp_capped: bool,
    /// Gen 2-4 (the gen4 mod's `onHit` checks `target.volatiles['substitute']`; the base's
    /// checks `source.transformed` and the target's `lastMove`).
    pub mimic_fails_behind_substitute: bool,
    /// Gen 2 Sketch always fails in a link battle (the gen2 `onHit`'s `-nothing`).
    pub sketch_always_fails: bool,
    /// Where Me First's ×1.5 applies (Gen 4's `onModifyDamagePhase2`, Gen 5+'s `onBasePower`
    /// priority 12); `None` before the move exists (DESIGN.md §42.8).
    pub me_first_boost: Option<MeFirstBoost>,
    /// Gen 5+ (Sucker Punch's `onTry`: `move.category === 'Status' && move.id !== 'mefirst'`):
    /// a target about to use Me First counts as attacking. Gen 4 (the gen4 `onTry`; pokeplatinum
    /// `BtlCmd_TrySuckerPunch` fails on a move of power 0) does not (DESIGN.md R-M8-70).
    pub me_first_counts_as_an_attack: bool,
}

/// Where Me First's ×1.5 applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeFirstBoost {
    /// Gen 5+ (`onBasePowerPriority: 12`): the BasePower chain.
    BasePower,
    /// Gen 4 (`onModifyDamagePhase2`): after the critical hit, before the random roll.
    DamagePhase2,
}

/// Which of its moves a Pokémon taking another's move over pays Pressure's PP from
/// (`CallRules::taker_pays_pressure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PressurePaidFrom {
    /// Gen 3 (pokeemerald `PressurePPLose(…, MOVE_MAGIC_COAT / MOVE_SNATCH)`): the slot holding
    /// the taking move — none when a caller (Assist, Metronome) used it.
    TheTakingMove,
    /// Gen 4 (pokeplatinum `moveSlot[battler]`): the slot the taker chose this turn, a caller's
    /// when one called the taking move.
    TheChosenMove,
}

/// Where a generation's Mirror Move takes the move it copies from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirrorMoveSource {
    /// Gen 1 (`onHit`): the opposite active Pokémon's last move, unless it is Mirror Move
    /// itself.
    Gen1FoeLastMove,
    /// Gen 2 (`onHit`): the same, without the calling moves (`MirrorTables::gen2`) and never one
    /// the user already knows.
    Gen2FoeLastMove,
    /// Gen 3-4: the last move that attacked the user (`getLastAttackedBy`), its user still
    /// knowing it.
    LastAttacker,
    /// Gen 5+: the target's last move, which must carry `mirror`.
    TargetLastMove,
}

/// The moves a generation's Mirror Move refuses whatever its source rule says (Showdown's
/// `noMirror` lists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirrorTables {
    pub gen2: &'static [crate::ids::MoveId],
    pub gen3: &'static [crate::ids::MoveId],
    pub gen4: &'static [crate::ids::MoveId],
}

impl Default for MirrorTables {
    fn default() -> Self {
        Self {
            gen2: &[
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::TRANSFORM,
            ],
            gen3: &[
                crate::ids::MoveId::ASSIST,
                crate::ids::MoveId::CURSE,
                crate::ids::MoveId::DOOM_DESIRE,
                crate::ids::MoveId::FOCUS_PUNCH,
                crate::ids::MoveId::FUTURE_SIGHT,
                crate::ids::MoveId::MAGIC_COAT,
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::NATURE_POWER,
                crate::ids::MoveId::PSYCH_UP,
                crate::ids::MoveId::ROLE_PLAY,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::SPIT_UP,
                crate::ids::MoveId::TAUNT,
                crate::ids::MoveId::TEETER_DANCE,
                crate::ids::MoveId::TRANSFORM,
            ],
            gen4: &[
                crate::ids::MoveId::ACUPRESSURE,
                crate::ids::MoveId::AROMATHERAPY,
                crate::ids::MoveId::ASSIST,
                crate::ids::MoveId::CHATTER,
                crate::ids::MoveId::COPYCAT,
                crate::ids::MoveId::COUNTER,
                crate::ids::MoveId::CURSE,
                crate::ids::MoveId::DOOM_DESIRE,
                crate::ids::MoveId::FEINT,
                crate::ids::MoveId::FOCUS_PUNCH,
                crate::ids::MoveId::FUTURE_SIGHT,
                crate::ids::MoveId::GRAVITY,
                crate::ids::MoveId::HAIL,
                crate::ids::MoveId::HAZE,
                crate::ids::MoveId::HEAL_BELL,
                crate::ids::MoveId::HELPING_HAND,
                crate::ids::MoveId::LIGHT_SCREEN,
                crate::ids::MoveId::LUCKY_CHANT,
                crate::ids::MoveId::MAGIC_COAT,
                crate::ids::MoveId::ME_FIRST,
                crate::ids::MoveId::METRONOME,
                crate::ids::MoveId::MIMIC,
                crate::ids::MoveId::MIRROR_COAT,
                crate::ids::MoveId::MIRROR_MOVE,
                crate::ids::MoveId::MIST,
                crate::ids::MoveId::MUD_SPORT,
                crate::ids::MoveId::NATURE_POWER,
                crate::ids::MoveId::PERISH_SONG,
                crate::ids::MoveId::PSYCH_UP,
                crate::ids::MoveId::RAIN_DANCE,
                crate::ids::MoveId::REFLECT,
                crate::ids::MoveId::ROLE_PLAY,
                crate::ids::MoveId::SAFEGUARD,
                crate::ids::MoveId::SANDSTORM,
                crate::ids::MoveId::SKETCH,
                crate::ids::MoveId::SLEEP_TALK,
                crate::ids::MoveId::SNATCH,
                crate::ids::MoveId::SPIT_UP,
                crate::ids::MoveId::SPIKES,
                crate::ids::MoveId::STEALTH_ROCK,
                crate::ids::MoveId::STRUGGLE,
                crate::ids::MoveId::SUNNY_DAY,
                crate::ids::MoveId::TAILWIND,
                crate::ids::MoveId::TOXIC_SPIKES,
                crate::ids::MoveId::TRANSFORM,
                crate::ids::MoveId::WATER_SPORT,
            ],
        }
    }
}

impl MirrorTables {
    /// The table the generation's Mirror Move refuses (Gen 3's stands for Gen 4 too where the
    /// gen4 mod only changed the source).
    pub fn for_generation(
        &self,
        generation: crate::spec::Generation,
    ) -> &'static [crate::ids::MoveId] {
        match generation {
            crate::spec::Generation::Gen2 => self.gen2,
            crate::spec::Generation::Gen3 => self.gen3,
            _ => self.gen4,
        }
    }
}
