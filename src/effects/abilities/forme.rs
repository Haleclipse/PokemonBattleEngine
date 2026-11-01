//! The abilities that change their holder's forme by themselves (DESIGN.md §43.4): Zen Mode,
//! Schooling, Shields Down, Hunger Switch and Power Construct. Every one answers a species of
//! its own with a rule; the primitive is `BattleContext::forme_change`.

use crate::engine::BattleContext;
use crate::ids::{PokemonId, SpeciesId};

/// The divisor of the max HP a self-changing ability compares (Showdown `pokemon.hp >
/// pokemon.maxhp / n`, integer division).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HpDenominator {
    Half,
    Quarter,
}

impl HpDenominator {
    /// Showdown's comparison: is `hp` above `max_hp` divided by this?
    pub fn above(self, hp: u16, max_hp: u16) -> bool {
        let divisor = match self {
            Self::Half => 2,
            Self::Quarter => 4,
        };
        u32::from(hp) > u32::from(max_hp) / divisor
    }

    /// ... or at or below it (the other side of the same line)?
    pub fn at_or_below(self, hp: u16, max_hp: u16) -> bool {
        !self.above(hp, max_hp)
    }
}

/// What makes a self-changing ability switch its holder's forme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfFormeRule {
    /// Zen Mode (`onResidual`): at or below `at` of its max HP the holder wears `other` (the
    /// Galar forme its `galar` pair) and carries the `zenmode` volatile; above it its own
    /// species and no volatile. The ability's `End` puts it back silently.
    ZenMode {
        at: HpDenominator,
        other: SpeciesId,
        /// The Galar forme and its Zen forme (Darmanitan-Galar, Darmanitan-Galar-Zen).
        galar: Option<(SpeciesId, SpeciesId)>,
    },
    /// Schooling, Shields Down (`onStart`, `onResidual`): from level `min_level`, above `at` of
    /// its max HP the holder wears `high` (Wishiwashi-School, Minior-Meteor), at or below it its
    /// own species.
    Threshold {
        at: HpDenominator,
        high: SpeciesId,
        min_level: u8,
    },
    /// Power Construct (`onResidual`): like `Threshold`, and the Complete forme becomes the
    /// Pokémon's own from then on, with the max HP and the `-activate` message.
    PowerConstruct { at: HpDenominator, other: SpeciesId },
    /// Hunger Switch (`onResidual`): every end of turn the other of the two.
    Toggle { other: SpeciesId },
}

/// A self-changing ability's data (DESIGN.md §43.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfForme {
    /// The holder's own species this ability answers (Showdown `baseSpecies.baseSpecies`).
    pub holder: SpeciesId,
    pub rule: SelfFormeRule,
}

impl SelfForme {
    /// Whether this ability answers `own`: the holder's own species, in any forme, is the one
    /// (Showdown asks `baseSpecies.baseSpecies`, so it holds while it wears another forme).
    pub fn answers(self, own: Option<SpeciesId>) -> bool {
        own.is_some_and(|own| own.same_species(self.holder))
    }

    /// Zen Mode's forme at or below the line: `other`, or the Galar one for the Galar base.
    fn zen_forme(self, own: Option<SpeciesId>) -> SpeciesId {
        let SelfFormeRule::ZenMode { other, galar, .. } = self.rule else {
            return self.holder;
        };
        match galar {
            Some((base, galar_zen)) if own == Some(base) => galar_zen,
            _ => other,
        }
    }

    /// The forme to wear now, or `None` when nothing changes. `forme` is the species the holder
    /// battles as.
    pub fn forme_for(
        self,
        level: u8,
        hp: u16,
        max_hp: u16,
        own: Option<SpeciesId>,
        forme: Option<SpeciesId>,
    ) -> Option<SpeciesId> {
        let low = |at: HpDenominator| at.at_or_below(hp, max_hp);
        match self.rule {
            SelfFormeRule::ZenMode { at, .. } => {
                let zen = self.zen_forme(own);
                if low(at) {
                    (forme != Some(zen)).then_some(zen)
                } else {
                    (forme == Some(zen)).then_some(own.unwrap_or(self.holder))
                }
            }
            SelfFormeRule::Threshold {
                at,
                high,
                min_level,
            } => {
                if level < min_level {
                    return None;
                }
                let target = if low(at) { self.holder } else { high };
                (forme.or(own) != Some(target)).then_some(target)
            }
            SelfFormeRule::PowerConstruct { at, other } => {
                // One way: above the line, or already Complete, nothing changes (`pokemon.hp >
                // pokemon.maxhp / 2` and the Complete forme are both early returns).
                (!at.above(hp, max_hp) && own != Some(other)).then_some(other)
            }
            SelfFormeRule::Toggle { other } => {
                let target = if forme == Some(other) {
                    self.holder
                } else {
                    other
                };
                (forme != Some(target)).then_some(target)
            }
        }
    }
}

/// The end-of-turn part of a self-changing ability (Showdown `onResidualOrder` 29): Zen Mode
/// moves its volatile, the others change forme straight away.
pub fn residual(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    let Some(rule) = ctx.self_forme(holder) else {
        return;
    };
    let (Some(hp), Some(max_hp), Some(own)) =
        (ctx.hp(holder), ctx.max_hp(holder), ctx.own_species(holder))
    else {
        return;
    };
    if hp == 0 || !rule.answers(Some(own)) {
        return;
    }
    // Hunger Switch: `|| pokemon.terastallized` — a Terastallized Morpeko keeps its forme
    // (DESIGN.md R-M9-20).
    if matches!(rule.rule, SelfFormeRule::Toggle { .. }) && ctx.terastallized(holder) {
        return;
    }
    let forme = ctx.species_of(holder).or(Some(own));
    let level = ctx.level(holder);
    let Some(target) = rule.forme_for(level, hp, max_hp, Some(own), forme) else {
        return;
    };
    match rule.rule {
        SelfFormeRule::ZenMode { .. } => {
            let zen = ctx.has_volatile(holder, crate::state::VolatileConditionKind::ZenMode);
            if target == own {
                // Above the line: the forme goes back and the volatile with it — back to the
                // holder's own forme, a Galar Darmanitan's Galar one (`removeVolatile('zenmode')`,
                // R-M10-68).
                ctx.forme_change(
                    holder,
                    target,
                    crate::engine::FormeChangeKind::InBattle,
                    source(ctx, holder),
                );
                ctx.end_volatile(holder, crate::state::VolatileConditionKind::ZenMode);
            } else {
                // Below it: the volatile goes on, then the forme it carries.
                if !zen {
                    ctx.set_zen_mode(holder);
                }
                ctx.forme_change(
                    holder,
                    target,
                    crate::engine::FormeChangeKind::InBattle,
                    source(ctx, holder),
                );
            }
        }
        SelfFormeRule::PowerConstruct { .. } => {
            ctx.ability_activated(holder, ctx.raw_ability(holder));
            ctx.forme_change(
                holder,
                target,
                crate::engine::FormeChangeKind::Permanent,
                source(ctx, holder),
            );
        }
        _ => {
            ctx.forme_change(
                holder,
                target,
                crate::engine::FormeChangeKind::InBattle,
                source(ctx, holder),
            );
        }
    }
}

/// Schooling's and Shields Down's `onStart` (DESIGN.md §43.4): the forme the holder's HP asks
/// for as it comes in.
pub fn enter(ctx: &mut BattleContext<'_>, holder: PokemonId) {
    residual(ctx, holder);
}

/// Zen Mode's `onEnd`: the ability leaving the field, or being suppressed, puts the holder back
/// into its own forme, silently, dropping the volatile. `rule` comes from the ability the
/// Pokémon has rather than `ability_handlers`: Gastro Acid has already set it aside by the time
/// its `End` runs.
pub fn end(ctx: &mut BattleContext<'_>, holder: PokemonId, rule: SelfForme) {
    if !matches!(rule.rule, SelfFormeRule::ZenMode { .. })
        || !ctx.has_volatile(holder, crate::state::VolatileConditionKind::ZenMode)
        || ctx.hp(holder).is_none_or(|hp| hp == 0)
    {
        return;
    }
    let own = ctx.own_species(holder);
    ctx.forme_change(
        holder,
        own.unwrap_or(rule.holder),
        crate::engine::FormeChangeKind::InBattle,
        source(ctx, holder),
    );
    ctx.end_volatile(holder, crate::state::VolatileConditionKind::ZenMode);
}

fn source(ctx: &BattleContext<'_>, holder: PokemonId) -> crate::engine::EffectSource {
    crate::engine::EffectSource::Ability {
        pokemon: holder,
        ability: ctx.raw_ability(holder),
    }
}
