//! Life Orb (Gen 5+): ×1.3 at the ModifyDamage step (`FinalDamageBoost` 5324 / 4096) and,
//! once after a damaging move that landed, 1/10 of the user's max HP (Showdown
//! `onAfterMoveSecondarySelf`; a residual-style `damage`, so Magic Guard refuses it) — except
//! after a one-hit KO move (Gen 5-6) or for a user a Red Card is dragging out (Gen 7+,
//! `ConditionRules::life_orb_spares`). Gen 4's Life Orb is the `damage_phase2` component and the
//! same recoil, paid for the item held once the move is over when a hit took real HP (pokeplatinum,
//! issues.md I193).

use crate::dex::ResolvedMove;
use crate::engine::{
    BattleContext, DamageAmount, DamageFlags, DamageKind, DamageRequest, EffectSource,
};
use crate::ids::{ItemId, PokemonId};
use crate::rules::LifeOrbSpares;

pub fn after_move_self(ctx: &mut BattleContext<'_>, user: PokemonId, move_def: &ResolvedMove) {
    let Some(max_hp) = ctx.base_max_hp(user) else {
        return;
    };
    if ctx.is_fainted(user) {
        return;
    }
    let spared = match ctx.rules().conditions().life_orb_spares {
        LifeOrbSpares::Nothing => false,
        LifeOrbSpares::OneHitKo => move_def.ohko.is_some(),
        LifeOrbSpares::ForcedOut => ctx.is_drag_pending(user),
    };
    if spared {
        return;
    }
    ctx.apply_damage(DamageRequest {
        target: user,
        amount: DamageAmount((max_hp / 10).max(1)),
        kind: DamageKind::Recoil,
        source: EffectSource::Item {
            pokemon: user,
            item: ItemId::LIFE_ORB,
        },
        flags: DamageFlags {
            bypass_substitute: true,
            spare_one_hp: false,
            instafaint: false,
            single_strike: false,
            future_move: false,
        },
    });
}
