mod berries;
pub(crate) mod cartridge_damage;
mod curse;
mod faint_bonds;
mod field;
mod format;
mod fury_cutter;
mod future_attack;
mod generation;
mod hazards;
mod identify;
mod item_transfer;
mod kings_rock;
mod lock_on;
mod metronome;
mod pp;
mod rage;
mod rampage;
mod retaliation;
mod screens;
mod self_faint;
mod switching;
mod targeting;
mod terrain;
mod topology;
mod truant;
mod type_chart;
mod update;
mod uproar;

use crate::spec::{BattleMode, BattleSpec, FormatId, Generation};

pub use berries::{BerryHeal, BerryRules, HpShare, MicleRules, StatusBerryTiming};
pub use cartridge_damage::{CartridgeProfile, CartridgeStep};
pub use curse::CurseRules;
pub use faint_bonds::FaintBondRules;
pub use field::FieldRules;
pub use format::{BattleClauses, FormatRules, HpReport, MaybeTrappedBy, MechanicPermissions};
pub use fury_cutter::{FuryCutterModel, FuryCutterRules};
pub use future_attack::{ForeseenRoll, FutureAttackRules};
pub use generation::EffectSporeRoll;
pub use generation::{
    AbilityChangeRules, AbilitySet, AccuracyOrder, ActionOrderRules, ActionOrderStrategy,
    AnySwitchInReach, BeatUpStyle, BellyDrumRule, CallRules, Camouflage, ChanceScale, ChargeModel,
    ConditionRules, Conversion, CrashDamage, CriticalTable, DamageAlgorithm, DamageBonusStage,
    DamageModifierKind, DamageRoundingPolicy, DamageRules, DefrostUser, DurationAdjust,
    DurationEffect, FaintRules, FaintTurnEffect, FlashFireBoost, FlashFireRules, FlowerGiftRules,
    Fraction, GenerationCapabilities, GenerationRules, HazeRules, HealAmount, HitRules,
    HitStepOrder, HpBands, LifeOrbSpares, LiquidOozeRules, MeFirstBoost, MirrorMoveSource,
    MirrorTables, OhkoRules, ParalysisSpeedStage, PlusMinusPartners, PressurePaidFrom,
    ProtectStallRules, PsywaveDamage, QuashOrder, RechargeCheck, RecoilTiming, ResidualKind,
    ResidualRules, Rounding, SameWeatherMove, SemiInvulnerableKind, ShellBellCount,
    SpeciesItemStat, StageTable, StartNewAbility, StatRules, StatusResidualTiming, StatusRules,
    StockpileRule, SwitchInOrder, TargetHpPower, TeamCure, TeamCureRules, ThawOnHit, ThawTiming,
    TimedDuration, TimedVolatileRules, ToxicModel, TransformRules, TwoTurnRules, TypeItemStage,
    TypeLock, TypeRules, TypedSureHit, UNORDERED_SLOT_SUB_ORDER, UNORDERED_VOLATILE_SUB_ORDER,
    WeatherHealRules, WeatherRules,
};
pub use hazards::DefogRules;
pub use identify::{IdentifiedEvasion, IdentifyRules};
pub use item_transfer::ItemTransferRules;
pub use kings_rock::ItemFlinchRules;
pub use lock_on::{LockOnModel, LockOnRules};
pub use metronome::MetronomeRules;
pub use pp::{CalledMovePressure, MovesSlot, PpRules, PressureReach, SpiteAmount, SpiteRules};
pub use rage::{RageBuild, RageEnd, RageRules};
pub use rampage::{RampageCounter, RampageRules};
pub use retaliation::{BideRules, RetaliationRules};
pub use screens::{CrowdedSide, ScreenModel, ScreenRules, SideProtectionRules};
pub use self_faint::{SelfDestructTiming, SelfFaintRules, SlotHealRules};
pub use switching::{
    BINDING_LOCK_TURNS, BINDING_LOCK_WEIGHTS, GEN1_PARTIALLY_TRAPPED_TURNS, PursuitDestinyBond,
    PursuitRules, SwitchingRules,
};
pub use targeting::{
    DeclaredTarget, ResolvedTarget, ResolvedTargets, TargetError, TargetPattern, TargetingRules,
};
pub use terrain::TerrainRules;
pub use topology::{BattleTopology, PositionSpec};
pub use truant::{TruantModel, TruantRules};
pub use type_chart::TypeChart;
pub use update::{MoveUpdates, UpdateRules};
pub use uproar::{UproarRules, UproarWake};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecialMechanics {
    pub mega: bool,
    pub z_move: bool,
    pub dynamax: bool,
    pub tera: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EngineMechanicSupport {
    mega: bool,
    z_move: bool,
    dynamax: bool,
    tera: bool,
}

const ENGINE_MECHANIC_SUPPORT: EngineMechanicSupport = EngineMechanicSupport {
    // Mega Evolution and Ultra Burst are implemented (DESIGN.md §44.3, R-O.9): the stone's
    // forme change, the `-mega` / `-burst` records and the Gen 7 re-queue.
    mega: true,
    // Z-Moves are implemented (DESIGN.md §44.4, R-O.4): the crystal converts the chosen move and
    // the Z-Move runs in its place.
    z_move: true,
    // Terastallization is implemented (DESIGN.md §44.6, R-O.2): the choice changes the types and
    // STAB, and it runs as its own queue step.
    tera: true,
    // Dynamax is implemented (DESIGN.md §44.5, R-O.6): the HP doubles and every move runs as its
    // Max Move.
    dynamax: true,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesFingerprint(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRules {
    pub(crate) generation: Generation,
    pub(crate) mode: BattleMode,
    pub(crate) format: FormatId,
    pub(crate) topology: BattleTopology,
    pub(crate) targeting: TargetingRules,
    pub(crate) action_order: ActionOrderRules,
    pub(crate) damage: DamageRules,
    pub(crate) stats: StatRules,
    pub(crate) status: StatusRules,
    pub(crate) conditions: ConditionRules,
    pub(crate) hits: HitRules,
    pub(crate) residual: ResidualRules,
    pub(crate) faint: FaintRules,
    pub(crate) update: UpdateRules,
    pub(crate) two_turn: TwoTurnRules,
    pub(crate) weather: WeatherRules,
    pub(crate) type_chart: TypeChart,
    pub(crate) special: SpecialMechanics,
    pub(crate) clauses: BattleClauses,
    pub(crate) hp_report: HpReport,
    pub(crate) maybe_trapped_by: MaybeTrappedBy,
    pub(crate) fingerprint: RulesFingerprint,
}

/// The rules an effect (an ability, item or move handler) is allowed to see: every generation-resolved
/// table, but neither the generation nor the battle mode. Generation differences must be
/// expressed as table values (DESIGN.md §31), never as `if generation` inside an effect, and
/// singles / doubles differences through the topology, never as `if doubles`; this view makes
/// both unrepresentable rather than a convention.
#[derive(Clone, Copy)]
pub struct EffectRules<'a> {
    inner: &'a ResolvedRules,
}

impl<'a> EffectRules<'a> {
    pub(crate) const fn new(inner: &'a ResolvedRules) -> Self {
        Self { inner }
    }
    pub fn topology(&self) -> &'a BattleTopology {
        &self.inner.topology
    }
    pub const fn action_order(&self) -> ActionOrderRules {
        self.inner.action_order
    }
    pub const fn damage(&self) -> DamageRules {
        self.inner.damage
    }
    pub const fn stats(&self) -> StatRules {
        self.inner.stats
    }
    pub const fn status(&self) -> StatusRules {
        self.inner.status
    }
    pub const fn conditions(&self) -> ConditionRules {
        self.inner.conditions
    }
    pub const fn hits(&self) -> HitRules {
        self.inner.hits
    }
    pub const fn residual(&self) -> ResidualRules {
        self.inner.residual
    }
    pub const fn faint(&self) -> FaintRules {
        self.inner.faint
    }
    pub const fn update(&self) -> UpdateRules {
        self.inner.update
    }
    pub const fn two_turn(&self) -> TwoTurnRules {
        self.inner.two_turn
    }
    pub const fn weather(&self) -> WeatherRules {
        self.inner.weather
    }
    pub fn type_chart(&self) -> &'a TypeChart {
        &self.inner.type_chart
    }
}

impl ResolvedRules {
    /// The effect-facing view (no generation).
    pub const fn for_effects(&self) -> EffectRules<'_> {
        EffectRules::new(self)
    }

    pub const fn generation(&self) -> Generation {
        self.generation
    }
    pub const fn mode(&self) -> BattleMode {
        self.mode
    }
    pub fn format(&self) -> &FormatId {
        &self.format
    }
    pub fn topology(&self) -> &BattleTopology {
        &self.topology
    }
    pub fn targeting(&self) -> &TargetingRules {
        &self.targeting
    }
    pub const fn action_order(&self) -> ActionOrderRules {
        self.action_order
    }
    pub const fn damage(&self) -> DamageRules {
        self.damage
    }
    pub const fn stats(&self) -> StatRules {
        self.stats
    }
    pub const fn status(&self) -> StatusRules {
        self.status
    }
    pub const fn conditions(&self) -> ConditionRules {
        self.conditions
    }
    pub const fn hits(&self) -> HitRules {
        self.hits
    }
    pub const fn residual(&self) -> ResidualRules {
        self.residual
    }
    pub const fn faint(&self) -> FaintRules {
        self.faint
    }
    pub const fn update(&self) -> UpdateRules {
        self.update
    }
    pub const fn two_turn(&self) -> TwoTurnRules {
        self.two_turn
    }
    pub const fn weather(&self) -> WeatherRules {
        self.weather
    }
    pub fn type_chart(&self) -> &TypeChart {
        &self.type_chart
    }
    pub const fn special(&self) -> SpecialMechanics {
        self.special
    }
    pub const fn clauses(&self) -> BattleClauses {
        self.clauses
    }
    pub fn fingerprint(&self) -> &RulesFingerprint {
        &self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum RuleError {
    /// `RulesResolver::resolve` knows `custom` only; a real format's switches come from the
    /// pre-battle session (`RulesResolver::resolve_format`).
    UnknownFormat(String),
    GenerationMismatch {
        requested: Generation,
        required: Generation,
    },
    ModeMismatch {
        requested: BattleMode,
        required: BattleMode,
    },
    /// A mode the generation never had: double battles begin in Gen 3, on the cartridges and in
    /// Pokémon Showdown alike, so the engine has nothing to check a Gen 1-2 double battle against
    /// and refuses it rather than run an unverified extrapolation.
    ModeUnavailable {
        generation: Generation,
        mode: BattleMode,
    },
}

pub struct RulesResolver;

impl RulesResolver {
    /// The rules of a battle under the engine's own free ruleset (`custom`, `FormatRules::custom`).
    pub fn resolve(spec: &BattleSpec) -> Result<ResolvedRules, RuleError> {
        if spec.format.as_str() != "custom" {
            return Err(RuleError::UnknownFormat(spec.format.to_string()));
        }
        Self::resolve_format(spec, &FormatRules::custom())
    }

    /// The rules of a battle under `format`'s switches (the session derives them from a real
    /// format's rule table, DESIGN.md §45.5).
    pub fn resolve_format(
        spec: &BattleSpec,
        format: &FormatRules,
    ) -> Result<ResolvedRules, RuleError> {
        let generation = GenerationRules::for_generation(spec.generation);

        if let Some(required) = format.required_generation
            && required != spec.generation
        {
            return Err(RuleError::GenerationMismatch {
                requested: spec.generation,
                required,
            });
        }

        if let Some(required) = format.required_mode
            && required != spec.mode
        {
            return Err(RuleError::ModeMismatch {
                requested: spec.mode,
                required,
            });
        }

        if spec.mode == BattleMode::Doubles && spec.generation < Generation::Gen3 {
            return Err(RuleError::ModeUnavailable {
                generation: spec.generation,
                mode: spec.mode,
            });
        }

        let topology = match spec.mode {
            BattleMode::Singles => BattleTopology::singles(),
            BattleMode::Doubles => BattleTopology::doubles(),
        };

        // A mechanic is exposed to battle choices only when both the ruleset allows it and the
        // engine has a semantic implementation. The skeleton intentionally keeps all four off
        // rather than consuming a once-per-battle resource without applying the mechanic.
        let special = SpecialMechanics {
            mega: ENGINE_MECHANIC_SUPPORT.mega
                && generation.capabilities.mega
                && format.mechanics.mega,
            z_move: ENGINE_MECHANIC_SUPPORT.z_move
                && generation.capabilities.z_move
                && format.mechanics.z_move,
            dynamax: ENGINE_MECHANIC_SUPPORT.dynamax
                && generation.capabilities.dynamax
                && format.mechanics.dynamax,
            tera: ENGINE_MECHANIC_SUPPORT.tera
                && generation.capabilities.tera
                && format.mechanics.tera,
        };

        let type_chart = TypeChart::for_generation(spec.generation);
        let canonical = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            spec.generation,
            spec.mode,
            spec.format,
            topology,
            generation.action_order,
            generation.damage,
            generation.stats,
            generation.status,
            generation.conditions,
            generation.hits,
            generation.residual,
            generation.faint,
            generation.update,
            generation.two_turn,
            generation.weather,
            type_chart,
            special,
            format.clauses,
            format.hp_report,
            format.maybe_trapped_by,
        );
        let fingerprint = RulesFingerprint(format!("rules:{:016x}", fnv1a64(canonical.as_bytes())));

        Ok(ResolvedRules {
            generation: spec.generation,
            mode: spec.mode,
            format: spec.format.clone(),
            topology,
            targeting: TargetingRules,
            action_order: generation.action_order,
            damage: generation.damage,
            stats: generation.stats,
            status: generation.status,
            conditions: generation.conditions,
            hits: generation.hits,
            residual: generation.residual,
            faint: generation.faint,
            update: generation.update,
            two_turn: generation.two_turn,
            weather: generation.weather,
            type_chart,
            special,
            clauses: format.clauses,
            hp_report: format.hp_report,
            maybe_trapped_by: format.maybe_trapped_by.clone(),
            fingerprint,
        })
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
