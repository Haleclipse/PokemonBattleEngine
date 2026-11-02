//! Which volatile conditions a generation shows its observers, and when.

use super::*;

/// Which volatiles the observers see start and go (DESIGN.md §47.0 item 13, R-M10-26): Showdown's
/// lines (`-start`, `-singleturn`, `-singlemove`, `-mustrecharge` … and their ends), as
/// @pkmn/client reads them.
pub(super) fn volatile_visibility(generation: Generation) -> crate::state::VolatileVisibility {
    use crate::state::{VolatileConditionKind as V, VolatileSet};
    // The battle's own counters, marks and locks, and what shows itself otherwise (a forme, an
    // ability's line, a `-prepare`, Lock-On's and Sky Drop's `-activate`).
    let mut unseen: VolatileSet = [
        V::Flinch,
        V::ChoiceLock,
        V::TwoTurnMove,
        V::Stall,
        V::AllySwitch,
        V::SubstituteBroken,
        V::ResidualDamage,
        V::SureHit,
        V::MeFirst,
        V::Unburden,
        V::DefenseCurl,
        V::GastroAcid,
        V::MicleBerry,
        V::Fling,
        V::Minimize,
        V::ZenMode,
        V::Commanding,
        V::Commanded,
        V::SparklingAria,
        V::Metronome,
        V::ChillyReception,
        V::Trapper,
        V::PartialTrappingLock,
        V::FakePartiallyTrapped,
        V::Counter,
        V::MirrorCoat,
        V::Pursuit,
        V::Rolling,
        V::RolloutStorage,
        V::Rampage,
        V::FuryCutter,
        V::Truant,
        V::Invulnerability,
        V::SkyDrop,
        V::SkyDropped,
        V::LockOn,
        V::LockedOn,
    ]
    .into_iter()
    .collect();
    // Gen 1 starts Rage and a partial trap with no line of their own.
    if generation == Generation::Gen1 {
        unseen.insert(V::Rage);
        unseen.insert(V::PartiallyTrapped);
    }
    // `-singlemove`'s and the recharge's: no `-end`, gone as the Pokémon next acts. (Charge's
    // `-start` lasts until an Electric move, its `-end … [silent]` shows.)
    let single_move = [
        V::MustRecharge,
        V::DestinyBond,
        V::Grudge,
        V::Rage,
        V::GlaiveRush,
    ];
    // (Focus Punch's focus, `-singleturn`, goes with the turn, as its user uses the move, or as
    // it loses its focus — not as it acts otherwise: a sleeping user keeps it, R-M10-43.)
    let until_it_acts: VolatileSet = single_move.into_iter().collect();
    // `trapped` has no `onEnd`: its trapper leaving frees it without a word.
    let mut ending_unseen: VolatileSet = single_move.into_iter().collect();
    ending_unseen.insert(V::Trapped);
    crate::state::VolatileVisibility {
        unseen,
        ending_unseen,
        until_it_acts,
        // `-singleturn`'s: some Showdown removes before the turn ends with no word (Beak Blast's
        // after the move, Snatch's as it steals).
        for_the_turn: [
            V::Protect,
            V::Endure,
            V::FollowMe,
            V::RagePowder,
            V::HelpingHand,
            V::MagicCoat,
            V::Snatch,
            V::Roost,
            V::Powder,
            V::Electrify,
            V::Spotlight,
            V::FocusPunch,
            V::BeakBlast,
            V::ShellTrap,
        ]
        .into_iter()
        .collect(),
    }
}
