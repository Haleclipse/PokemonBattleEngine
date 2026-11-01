//! What a format asks of the battle itself (DESIGN.md §45.5): the generation and the battle
//! mode it is played in, and the special mechanics it allows. The engine reads these switches
//! and never a format. Everything a format asks of the teams before the battle — their size,
//! the picks, Team Preview, Species and Item Clause — is the pre-battle session's and the team
//! validator's (`session::BattleSession`): the battle starts from teams already registered,
//! chosen and ordered. A real format reaches the engine through the session, which derives these
//! switches from its rule table; `custom` is the engine's own free ruleset.

use crate::spec::{BattleMode, Generation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MechanicPermissions {
    pub mega: bool,
    pub z_move: bool,
    pub dynamax: bool,
    pub tera: bool,
}

impl MechanicPermissions {
    pub const NONE: Self = Self {
        mega: false,
        z_move: false,
        dynamax: false,
        tera: false,
    };
}

/// The battle-time rules a format adds (DESIGN.md §45.7): switches the status pipeline and the
/// battle's start read, never a move's or an ability's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattleClauses {
    /// Sleep Clause Mod: a foe's sleep is refused while one of the side's Pokémon still standing
    /// sleeps from a foe's doing.
    pub sleep_clause_mod: bool,
    /// Freeze Clause Mod: a foe's freeze is refused while one of the side's Pokémon is frozen.
    pub freeze_clause_mod: bool,
    /// Mega Rayquaza Clause: Rayquaza cannot Mega Evolve.
    pub mega_rayquaza_clause: bool,
    /// Endless Battle Clause: past turn 100, a battle that every side's stale Pokémon keep going
    /// ends. A side whose team started with the means to cycle restorative berries loses; otherwise
    /// it is a tie.
    pub endless_battle_clause: bool,
}

/// How a Pokémon's HP is shared with the other side (Showdown `getHealth`, DESIGN.md M10.4c):
/// exactly (`reportExactHP`: a `debug` format such as the custom games, or Exact HP Mod), as a
/// percentage (HP Percentage Mod, or any format from Gen 7), else in 48ths of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpReport {
    Exact,
    Percentage,
    Pixels,
}

/// Which of a foe's abilities make a Pokémon maybe trapped (Showdown's request `maybeTrapped`,
/// DESIGN.md §47.0 item 14): the one it has, and from Gen 3 in a format that keeps abilities
/// legal (`obtainableabilities`) any its species may have — but those the format bans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaybeTrappedBy {
    /// A custom game's or hackmons': no species tells what a foe may have.
    ItsAbility,
    SpeciesAbilities {
        banned: Vec<crate::ids::AbilityId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRules {
    pub required_generation: Option<Generation>,
    pub required_mode: Option<BattleMode>,
    pub mechanics: MechanicPermissions,
    pub clauses: BattleClauses,
    pub hp_report: HpReport,
    pub maybe_trapped_by: MaybeTrappedBy,
}

impl FormatRules {
    /// The engine's own free ruleset (`custom`): any generation and mode, every special
    /// mechanic the generation has. Tests and the differential campaigns use it.
    pub fn custom() -> Self {
        Self {
            required_generation: None,
            required_mode: None,
            mechanics: MechanicPermissions {
                mega: true,
                z_move: true,
                dynamax: true,
                tera: true,
            },
            clauses: BattleClauses::default(),
            // Showdown's custom games are `debug` formats: every HP is shared exactly.
            hp_report: HpReport::Exact,
            maybe_trapped_by: MaybeTrappedBy::ItsAbility,
        }
    }
}
