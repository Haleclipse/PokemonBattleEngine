//! A format as the session reads it: the validator, the team policy, the engine's switches, and
//! the registration of a team.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::dex::{DexResolver, ResolvedDex};
use crate::engine::BattleInitError;
use crate::format::{
    FormatEntry, FormatError, FormatLibrary, Problem, RuleTable, TeamSet, TeamValidator,
};
use crate::ids::{AbilityId, SideId};
use crate::rules::{
    BattleClauses, FormatRules, HpReport, MechanicPermissions, ResolvedRules, RuleError,
    RulesResolver,
};
use crate::spec::{BattleMode, BattleSpec, Generation};
use crate::state::{BattleRng, RandomRequestKind};

use super::bootstrap::SetBuildError;
use super::protocol::{SessionPhase, TeamChoiceError};

/// Why a session could not go on.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum SessionError {
    /// The format, a custom rule or the data directory (`FormatError`).
    Format(FormatError),
    /// A format the engine does not play: a game type other than singles and doubles, or a
    /// mod other than its generation's own (the engine's dex is the generation's) — refused,
    /// never played as the nearest thing the engine knows.
    Unplayable(String),
    Rules(RuleError),
    /// A side's registered team breaks the format: Showdown's problems, word for word.
    Team {
        side: SideId,
        problems: Vec<Problem>,
    },
    /// A set the engine cannot build from its normalised form.
    Set {
        side: SideId,
        index: usize,
        error: SetBuildError,
    },
    Init(BattleInitError),
    /// A call the session's phase does not take (a registration once the choice is open …).
    WrongPhase(SessionPhase),
    /// The side has registered, or chosen, already.
    AlreadyDone(SideId),
    /// A team choice Showdown's `chooseTeam` refuses.
    Choice(TeamChoiceError),
    /// A snapshot of another registration: the format's rules, its data or the dex changed.
    FingerprintMismatch,
}

impl From<FormatError> for SessionError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

/// What a format asks of the teams before the battle (Showdown's rule table numbers and its
/// Team Preview rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamPolicy {
    /// Showdown `minTeamSize` / `maxTeamSize`: how many Pokémon a side registers.
    pub min_team_size: u8,
    pub max_team_size: u8,
    /// Showdown `pickedTeamSize`: how many of them a side brings to the battle (VGC: 4 of 6);
    /// `None` = all of them.
    pub picked_team_size: Option<u8>,
    /// Team Preview: the sides see each other's teams before they choose their order and picks
    /// (Showdown's `onTeamPreview`). Without it a format with picks still has the sides choose,
    /// each seeing its own team only (`runPickTeam`).
    pub preview: bool,
}

/// Whether the sides see each other's team sheets (all but the stats) before they choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenTeamSheets {
    No,
    /// `Open Team Sheets`: shown once both sides accept.
    Offered,
    /// `Force Open Team Sheets`: shown at Team Preview.
    Forced,
}

/// What a side sees of the other's team before the battle (DESIGN.md §45.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityPolicy {
    /// Team Preview hides the forme of a few species (`Urshifu-*`); Species Reveal Clause shows
    /// it.
    pub species_reveal: bool,
    /// Tera Type Preview: the Tera types are shown at Team Preview.
    pub tera_types: bool,
    pub open_team_sheets: OpenTeamSheets,
}

/// The engine's switches and the team policy of a format (DESIGN.md §45.5): its generation and
/// game type, the special mechanics its clauses take away (Dynamax Clause, Terastal Clause),
/// and the team's size, picks and preview.
pub fn format_switches(
    entry: &FormatEntry,
    table: &RuleTable,
) -> Result<(BattleSpec, FormatRules, TeamPolicy), SessionError> {
    let (generation, mode) = generation_and_mode(entry)?;
    let numbers = table.numbers;
    let narrow = |value: u32| u8::try_from(value).unwrap_or(u8::MAX);
    let rules = FormatRules {
        required_generation: Some(generation),
        required_mode: Some(mode),
        mechanics: MechanicPermissions {
            mega: true,
            z_move: true,
            // `onBegin`: `side.dynamaxUsed = true` / `pokemon.canTerastallize = null`.
            dynamax: !table.has("dynamaxclause"),
            tera: !table.has("terastalclause"),
        },
        clauses: BattleClauses {
            sleep_clause_mod: table.has("sleepclausemod"),
            freeze_clause_mod: table.has("freezeclausemod"),
            mega_rayquaza_clause: table.has("megarayquazaclause"),
            endless_battle_clause: table.has("endlessbattleclause"),
        },
        // Showdown `Battle`: `reportExactHP = !!format.debug` (no target format is), then Exact
        // HP Mod's and HP Percentage Mod's `onBegin`; `getHealth` shares percentages from Gen 7
        // whatever the rules (`this.battle.gen >= 7`, R-M10-19).
        hp_report: if table.has("exacthpmod") {
            HpReport::Exact
        } else if table.has("hppercentagemod") || generation >= Generation::Gen7 {
            HpReport::Percentage
        } else {
            HpReport::Pixels
        },
        // Showdown `Battle`'s turn start: a foe's species tells what it may have unless the
        // format is hackmons (`+hackmons`, or no `obtainableabilities` and no team generator).
        maybe_trapped_by: if table.has("obtainableabilities") && !table.has("+hackmons") {
            crate::rules::MaybeTrappedBy::SpeciesAbilities {
                banned: [
                    ("-ability:shadowtag", AbilityId::SHADOW_TAG),
                    ("-ability:arenatrap", AbilityId::ARENA_TRAP),
                    ("-ability:magnetpull", AbilityId::MAGNET_PULL),
                ]
                .into_iter()
                .filter(|(ban, _)| table.has(ban))
                .map(|(_, ability)| ability)
                .collect(),
            }
        } else {
            crate::rules::MaybeTrappedBy::ItsAbility
        },
    };
    let policy = TeamPolicy {
        min_team_size: narrow(numbers.min_team_size),
        max_team_size: narrow(numbers.max_team_size),
        picked_team_size: numbers.picked_team_size.map(narrow),
        preview: table.has("teampreview") || table.has("teamtypepreview"),
    };
    let spec = BattleSpec::new(generation, mode, entry.id.clone());
    Ok((spec, rules, policy))
}

fn generation_and_mode(entry: &FormatEntry) -> Result<(Generation, BattleMode), SessionError> {
    let number = entry.generation.unwrap_or(9);
    let generation = match number {
        1 => Generation::Gen1,
        2 => Generation::Gen2,
        3 => Generation::Gen3,
        4 => Generation::Gen4,
        5 => Generation::Gen5,
        6 => Generation::Gen6,
        7 => Generation::Gen7,
        8 => Generation::Gen8,
        9 => Generation::Gen9,
        other => return Err(SessionError::Unplayable(format!("generation {other}"))),
    };
    if let Some(mod_name) = &entry.mod_name
        && *mod_name != format!("gen{number}")
    {
        return Err(SessionError::Unplayable(format!("mod {mod_name}")));
    }
    let mode = match entry.game_type.as_deref().unwrap_or("singles") {
        "singles" => BattleMode::Singles,
        "doubles" => BattleMode::Doubles,
        other => return Err(SessionError::Unplayable(format!("game type {other}"))),
    };
    Ok((generation, mode))
}

/// A side's team as registered: validated, its sets normalised, every gender fixed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredTeam {
    #[serde(with = "crate::snapshot::wire")]
    pub side: SideId,
    #[serde(with = "crate::snapshot::wire")]
    pub sets: Vec<TeamSet>,
    /// Whether its moves were checked against the learnsets (`TeamValidator::checks_learnsets`):
    /// a team registered without them is not known to be legal (DESIGN.md §45.6).
    pub learnsets_checked: bool,
}

/// A format as the session reads it.
pub struct SessionFormat<'d> {
    pub(super) validator: TeamValidator<'d>,
    pub(super) rules: FormatRules,
    pub(super) policy: TeamPolicy,
    pub(super) visibility: VisibilityPolicy,
    pub(super) spec: BattleSpec,
    pub(super) library: &'d FormatLibrary,
    pub(super) dex: Arc<ResolvedDex>,
}

impl<'d> SessionFormat<'d> {
    /// A format, its validator with the learnsets (optional data, DESIGN.md §46).
    pub fn new(library: &'d FormatLibrary, format_id: &str) -> Result<Self, SessionError> {
        Self::from_validator(TeamValidator::new(library, format_id)?)
    }

    internal!(
        /// A format with custom rules added (`format@@@rule, rule`).
        pub fn with_custom(
            library: &'d FormatLibrary,
            format_id: &str,
            custom_rules: &[String],
        ) -> Result<Self, SessionError> {
            Self::from_validator(TeamValidator::with_custom(
                library,
                format_id,
                custom_rules,
            )?)
        }
    );

    internal!(
        /// A format over a validator made elsewhere (`TeamValidator::without_learnsets`: no move
        /// checked against the learnsets).
        pub fn from_validator(validator: TeamValidator<'d>) -> Result<Self, SessionError> {
            let (spec, rules, policy) = format_switches(validator.format, &validator.table)?;
            let table = &validator.table;
            let visibility = VisibilityPolicy {
                species_reveal: table.has("speciesrevealclause"),
                tera_types: table.has("teratypepreview"),
                open_team_sheets: if table.has("forceopenteamsheets") {
                    OpenTeamSheets::Forced
                } else if table.has("openteamsheets") {
                    OpenTeamSheets::Offered
                } else {
                    OpenTeamSheets::No
                },
            };
            let dex = Arc::new(DexResolver::resolve(spec.generation));
            Ok(Self {
                library: validator.library,
                validator,
                rules,
                policy,
                visibility,
                spec,
                dex,
            })
        }
    );

    internal!(
        pub fn validator(&self) -> &TeamValidator<'d> {
            &self.validator
        }
    );

    internal!(
        pub fn switches(&self) -> &FormatRules {
            &self.rules
        }
    );

    internal!(
        pub fn policy(&self) -> TeamPolicy {
            self.policy
        }
    );

    internal!(
        pub fn visibility(&self) -> VisibilityPolicy {
            self.visibility
        }
    );

    /// The registration's identity (DESIGN.md §45.5): the format's id, its resolved rule table,
    /// whether the learnsets are checked, the data directory and the engine's dex. A session snapshot taken under another is not
    /// restored. (The battle's own snapshot carries the battle rules' fingerprint, which covers
    /// only the switches the engine reads: a new ban list does not touch a battle under way.)
    pub fn fingerprint(&self) -> String {
        let text = format!(
            "format {}\ntable {}\nlearnsets {}\n{}\n{}",
            self.validator.format.id,
            self.validator.table.fingerprint(),
            if self.validator.checks_learnsets() {
                "checked"
            } else {
                "unchecked"
            },
            self.library.data_digest(),
            self.dex.fingerprint(),
        );
        format!("session:{}", crate::digest::sha256_hex(text.as_bytes()))
    }

    internal!(
        pub fn spec(&self) -> &BattleSpec {
            &self.spec
        }
    );

    internal!(
        pub fn dex(&self) -> &Arc<ResolvedDex> {
            &self.dex
        }
    );

    internal!(
        /// The battle's rules: the generation's, with the format's switches.
        pub fn rules(&self) -> Result<ResolvedRules, SessionError> {
            RulesResolver::resolve_format(&self.spec, &self.rules).map_err(SessionError::Rules)
        }
    );

    internal!(
        /// Registers a side's team: `validate`, then `draw_genders`.
        pub fn register(
            &self,
            side: SideId,
            sets: Vec<TeamSet>,
            rng: &mut BattleRng,
        ) -> Result<RegisteredTeam, SessionError> {
            let mut team = self.validate(side, sets)?;
            self.draw_genders(&mut team, rng);
            Ok(team)
        }
    );

    internal!(
        /// Showdown's validation of a side's whole team, which normalises its sets.
        pub fn validate(
            &self,
            side: SideId,
            mut sets: Vec<TeamSet>,
        ) -> Result<RegisteredTeam, SessionError> {
            let problems = self.validator.validate_team(&mut sets);
            if !problems.is_empty() {
                return Err(SessionError::Team { side, problems });
            }
            Ok(RegisteredTeam {
                side,
                sets,
                learnsets_checked: self.validator.checks_learnsets(),
            })
        }
    );

    internal!(
        /// The gender of each set that names none of a species with two, drawn as Showdown's
        /// `Pokemon` constructor draws it (`sample(['M', 'F'])`) from the battle's generator —
        /// before the preview, which shows it. Showdown builds P1's team, then P2's.
        pub fn draw_genders(&self, team: &mut RegisteredTeam, rng: &mut BattleRng) {
            for set in &mut team.sets {
                if !set.gender.is_empty() {
                    continue;
                }
                let species = self.battle_species(set);
                let fixed = self
                    .dex
                    .species_by_name(&species)
                    .and_then(|species| species.gender);
                if fixed.is_none() {
                    let draw = rng.range_inclusive_for(RandomRequestKind::Sample, 0, 1);
                    set.gender = if draw == 0 { "M" } else { "F" }.to_string();
                }
            }
        }
    );

    /// The species a set battles as: a cosmetic forme (Vivillon-Savanna, Gastrodon-East) is its
    /// base species — it battles as it, and its look is the host's to keep, like the nickname
    /// and the shininess.
    pub(super) fn battle_species(&self, set: &TeamSet) -> String {
        let named = self.validator.species(&set.species);
        if named.is_cosmetic_forme {
            named.base_species.clone()
        } else {
            set.species.clone()
        }
    }
}
