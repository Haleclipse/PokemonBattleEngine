//! Grounding (DESIGN.md §41.5, K.7b): Smack Down's grounding, Magnet Rise, Telekinesis and Air
//! Balloon. The reading side is `engine::environment::grounded`.

use super::BattleContext;
use crate::engine::{ConditionApplyError, TraceEvent};
use crate::ids::{MoveId, PokemonId, SpeciesId};
use crate::state::{FieldConditionKind, VolatileConditionKind};

/// Magnet Rise's `duration`.
const MAGNET_RISE_TURNS: u8 = 5;
/// Telekinesis's `duration`.
const TELEKINESIS_TURNS: u8 = 3;
/// The species Telekinesis cannot lift (its `onStart`: `baseSpecies.baseSpecies`), and Mega
/// Gengar.
const TELEKINESIS_IMMUNE: [SpeciesId; 4] = [
    SpeciesId::DIGLETT,
    SpeciesId::DUGTRIO,
    SpeciesId::SANDYGAST,
    SpeciesId::PALOSSAND,
];

impl BattleContext<'_> {
    /// Magnet Rise's `onTry` (before Protean's): not on a user Smack Down or Ingrain holds down;
    /// Gen 4 not with Levitate in effect (`ConditionRules::magnet_rise_fails_with_levitate`; the
    /// cartridge's `Battler_Ability`, issues.md I95).
    pub fn magnet_rise_try(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(user) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let held_down = pokemon.volatiles.contains(VolatileConditionKind::SmackDown)
            || pokemon.volatiles.contains(VolatileConditionKind::Ingrain);
        let levitating = self.rules.conditions.magnet_rise_fails_with_levitate
            && crate::engine::has_ability(
                self.state,
                self.rules,
                pokemon,
                crate::ids::AbilityId::LEVITATE,
            );
        if held_down || levitating {
            return Err(ConditionApplyError::Immune);
        }
        Ok(())
    }

    /// Magnet Rise (`addVolatile('magnetrise')`): off the ground for five turns; refused while
    /// it lasts.
    pub fn set_magnet_rise(&mut self, user: PokemonId) -> Result<(), ConditionApplyError> {
        self.start_timed(user, VolatileConditionKind::MagnetRise, |v| {
            v.set_magnet_rise(MAGNET_RISE_TURNS)
        })
    }

    /// Telekinesis (`addVolatile('telekinesis')` and its `onStart`): Diglett, Dugtrio, Sandygast,
    /// Palossand and Mega Gengar cannot be lifted (`-immune`), nor a target Smack Down or Ingrain
    /// holds down; otherwise off the ground for three turns, refused while it lasts.
    pub fn set_telekinesis(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(target) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        let unliftable = pokemon.species.is_some_and(|species| {
            species == SpeciesId::GENGAR_MEGA
                || TELEKINESIS_IMMUNE
                    .iter()
                    .any(|immune| immune.same_species(species))
        });
        if unliftable
            || pokemon.volatiles.contains(VolatileConditionKind::SmackDown)
            || pokemon.volatiles.contains(VolatileConditionKind::Ingrain)
        {
            return Err(ConditionApplyError::Immune);
        }
        self.start_timed(target, VolatileConditionKind::Telekinesis, |v| {
            v.set_telekinesis(TELEKINESIS_TURNS)
        })
    }

    /// Smack Down / Thousand Arrows (`addVolatile('smackdown')`, its `onStart` / `onRestart`): a
    /// Flying type or a Levitate holder — not one an Iron Ball or Ingrain keeps down already,
    /// nor under Gravity — comes down; so does one in the air mid-Fly or mid-Bounce (its charge
    /// and its move this turn end) or lifted by Magnet Rise or Telekinesis (they end). Nothing
    /// to bring down: no grounding. Already down: only a Fly or Bounce charge ends.
    pub fn smack_down(&mut self, target: PokemonId) -> Result<(), ConditionApplyError> {
        let Some(pokemon) = self.state.pokemon(target) else {
            return Err(ConditionApplyError::InvalidTarget);
        };
        if pokemon.is_fainted() {
            return Err(ConditionApplyError::InvalidTarget);
        }
        let already = pokemon.volatiles.contains(VolatileConditionKind::SmackDown);
        // `hasAbility('levitate')`: a Mold Breaker user's Smack Down still brings it down.
        let mut applies = !already
            && (crate::engine::has_type(
                self.state,
                self.rules,
                pokemon,
                crate::dex::PokemonType::Flying,
            ) || crate::engine::has_ability(
                self.state,
                self.rules,
                pokemon,
                crate::ids::AbilityId::LEVITATE,
            ));
        if crate::engine::held_item_id(self.state, self.rules, target)
            == Some(crate::ids::ItemId::IRON_BALL)
            || pokemon.volatiles.contains(VolatileConditionKind::Ingrain)
            || self.state.field.contains(FieldConditionKind::Gravity)
        {
            applies = false;
        }
        let flying = matches!(
            pokemon.volatiles.charging_move(),
            Some(MoveId::FLY | MoveId::BOUNCE)
        );
        if flying {
            self.end_volatile(target, VolatileConditionKind::TwoTurnMove);
            self.cancel_move(target);
            applies = true;
        }
        if already {
            return if flying {
                Ok(())
            } else {
                Err(ConditionApplyError::AlreadyActive)
            };
        }
        for lift in [
            VolatileConditionKind::MagnetRise,
            VolatileConditionKind::Telekinesis,
        ] {
            if self.end_volatile(target, lift) {
                applies = true;
            }
        }
        if !applies {
            return Err(ConditionApplyError::Immune);
        }
        self.set_marker_volatile(target, VolatileConditionKind::SmackDown)
    }

    /// Air Balloon (its `onDamagingHit` / `onAfterSubDamage`): a move's hit pops it — the item is
    /// gone (`item = ''`: not the holder's last item) and `AfterUseItem` runs (Unburden).
    pub fn pop_air_balloon(&mut self, holder: PokemonId) {
        let Some(pokemon) = self.state.pokemon_mut(holder) else {
            return;
        };
        let Some(item) = pokemon.held_item.take() else {
            return;
        };
        pokemon.item_state = crate::state::ItemState::default();
        self.trace.push(TraceEvent::ItemConsumed {
            holder,
            item,
            seen: true,
        });
        self.after_use_item(holder);
    }
}
