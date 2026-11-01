//! Heal Bell / Aromatherapy (Showdown `onHit`, target `allyTeam`): cures the status of every
//! Pokémon on the user's team, bench included — as the generation has it
//! (`ConditionRules::team_cure`, R167): some abilities keep it off (Soundproof for Heal Bell,
//! Sap Sipper for Aromatherapy, Good as Gold for both; the user's own too in Gen 3-4 and Gen
//! 6-7 Heal Bell; a benched Pokémon's only in Gen 3-4), a Substitute does for Gen 6+
//! Aromatherapy unless the user infiltrates, and from Gen 6 it fails when it cured nobody.

use crate::engine::{BattleContext, ConditionApplyError, CureReason};
use crate::ids::{MoveId, PokemonId};
use crate::rules::ResolvedTarget;
use crate::state::VolatileConditionKind;

pub fn execute(
    ctx: &mut BattleContext<'_>,
    user: PokemonId,
    targets: &[ResolvedTarget],
) -> Result<(), ConditionApplyError> {
    let Some(ResolvedTarget::Side(side)) = targets.first() else {
        return Err(ConditionApplyError::InvalidTarget);
    };
    // The move under way, not the user's last move: a called one leaves that at its caller
    // (Sleep Talk, Metronome) — Gen 2's at none (R-M8-120).
    let move_id = ctx
        .active_move()
        .ok_or(ConditionApplyError::InvalidTarget)?;
    let rules = ctx.rules().conditions().team_cure;
    let cure = if move_id == MoveId::AROMATHERAPY {
        rules.aromatherapy
    } else {
        rules.heal_bell
    };
    let infiltrates = ctx.infiltrates(user);
    let mut cured = false;
    for member in ctx.team(*side) {
        let exempt = member != user || cure.user_immune_too;
        // (`!this.suppressingAbility(ally)`: a user ignoring abilities cures through them.)
        let immune = (exempt && !ctx.suppressing_ability(member))
            .then(|| {
                cure.immune
                    .iter()
                    .copied()
                    .find(|ability| ctx.has_ability(member, *ability))
            })
            .flatten();
        // An active one says so (`-immune|ally|[from] ability: Soundproof`, R-M10-46).
        if let Some(ability) = immune
            && ctx.is_active(member)
        {
            ctx.ability_activated(member, ability);
        }
        let immune = immune.is_some();
        let behind_substitute = member != user
            && cure.substitute_blocks
            && !infiltrates
            && ctx.is_active(member)
            && ctx.has_volatile(member, VolatileConditionKind::Substitute);
        if immune || behind_substitute {
            continue;
        }
        if ctx.cure_status(member, CureReason::Move(move_id)) {
            cured = true;
        }
    }
    if cured || !cure.fails_when_nothing_cured {
        Ok(())
    } else {
        Err(ConditionApplyError::AlreadyActive)
    }
}
