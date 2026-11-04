//! What a battle has shown its observers (DESIGN.md §47.0 items 9, 11 and 12, M10.4c): kept for
//! the stable API's views alone — no rule of the battle reads it (`tests/architecture.rs`). The
//! engine learns it from its own events as they happen, whatever the trace keeps
//! (`TraceEvent::disclosure`, `BattleTrace::push`), so it is part of the state: a snapshot, a
//! clone and a rewind keep it.
//!
//! Observers know a Pokémon by what they see of it: an Illusion's disguise is the Pokémon they
//! saw come in. While it is on the field they watch it; once it has left, they know it as it was
//! when they last saw it (`Sighting`) — a heal it takes as it goes (Regenerator) they do not see.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::dex::PokemonType;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, SpeciesId};
use crate::state::{NonVolatileStatus, TeamPreview, VolatileConditionKind, VolatileSet};

/// What observers last saw of a Pokémon, as it left the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sighting {
    pub hp: u16,
    pub max_hp: u16,
    #[serde(with = "crate::snapshot::wire")]
    pub status: Option<NonVolatileStatus>,
    /// The species it showed (an Illusion's disguise's).
    #[serde(with = "crate::snapshot::wire")]
    pub species: Option<SpeciesId>,
    /// The level it showed: its own, through an Illusion too.
    pub level: u8,
    /// The type it showed Terastallized into: its own, through an Illusion too (a disguise that
    /// left Terastallized is believed so).
    #[serde(with = "crate::snapshot::wire")]
    pub terastallized: Option<PokemonType>,
}

/// One thing an event shows everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    /// `pokemon` came onto the field, seen as `seen_as`: itself, or the disguise its Illusion
    /// shows (an Illusion broken, the holder is seen as itself).
    Appeared {
        pokemon: PokemonId,
        seen_as: PokemonId,
    },
    /// `pokemon` left the field, seen as `seen_as`, as it was.
    Left {
        pokemon: PokemonId,
        seen_as: PokemonId,
        sighting: Sighting,
    },
    /// The Pokémon fainted, in everyone's sight — as the one it is seen as (an Illusion's holder
    /// that faints in its disguise is seen as the disguise fainting) — and leaves the field.
    Fainted(PokemonId),
    /// A fainted Pokémon off the field was seen brought back, as itself, so (Revival Blessing).
    Revived(PokemonId, Sighting),
    /// The Pokémon was seen cured of its status, as `seen_as` (the disguise an Illusion's holder
    /// went out in): one off the field so (Heal Bell, Aromatherapy, Natural Cure as it left).
    Cured {
        pokemon: PokemonId,
        seen_as: PokemonId,
    },
    /// The Pokémon was seen taking this species for good (`detailschange`): off the field so,
    /// Zero to Hero's as it leaves. `keeps_ability`: Disguise's or Ice Face's, the ability
    /// staying; another brings the forme's.
    FormeShown {
        pokemon: PokemonId,
        species: SpeciesId,
        keeps_ability: bool,
    },
    /// Its ability was named (`-ability`, `[from] ability: …`).
    AbilityShown(PokemonId, AbilityId),
    /// A forme for good gave it its ability, its own from then on (the forme shows it).
    OwnAbilityShown(PokemonId, AbilityId),
    /// Its ability changed, the new one not named by this (a Mega Evolution's, a Transform's):
    /// what was known of it is past.
    AbilityChanged(PokemonId),
    /// Its ability changed from `old` to `new`, both named (`-ability|p|new|old|[from] …`, a
    /// Gen 5+ Skill Swap's `-activate`).
    AbilityReplaced {
        pokemon: PokemonId,
        old: AbilityId,
        new: AbilityId,
    },
    /// Its item was named outright (`-item`: Frisk, Trick, a Harvest, an Air Balloon).
    ItemShown(PokemonId, ItemId),
    /// Its item was seen at work (`[from] item: …`: Leftovers, Life Orb). A berry's heal after
    /// it was eaten names the berry it no longer holds, which shows nothing new.
    ItemActed(PokemonId, ItemId),
    /// It was seen losing this item (`-enditem`: eaten, used up, knocked off, stolen).
    ItemLost(PokemonId, ItemId),
    /// It used this move of its own by its own action (DESIGN.md §47.0 item 13): not a call, nor
    /// a lock's later turn — one it took by Transform or Mimic included.
    MoveUsed(PokemonId, MoveId),
    /// A line named this move as one it has (a `cant`, Sleep Talk's call, Forewarn, Spite, a
    /// Leppa Berry …).
    MoveShown(PokemonId, MoveId),
    /// It transformed into `into`: its moves are that Pokémon's while it stays on the field. Its
    /// side sees the moves it took (`copied`) in its requests: all of `into`'s.
    Transformed {
        pokemon: PokemonId,
        into: PokemonId,
        copied: [Option<MoveId>; 4],
    },
    /// Mimic put `move_id` in its own place while the Pokémon stays on the field; Sketch, for
    /// good (`by`).
    MoveCopied {
        pokemon: PokemonId,
        move_id: MoveId,
        by: MoveId,
    },
    /// A volatile that shows itself started on the Pokémon (DESIGN.md §47.0 item 13).
    VolatileStarted(PokemonId, VolatileConditionKind),
    /// It ended.
    VolatileEnded(PokemonId, VolatileConditionKind),
    /// Baton Pass or Shed Tail handed on from `from`, which left, to `to` what it lets through
    /// (`kinds`): the observers hand on those of the kinds they saw on `from` — one whose end
    /// showed nothing too (Mean Look's `trapped` its trapper left, R-M10-41).
    VolatilesPassed {
        from: PokemonId,
        to: PokemonId,
        kinds: VolatileSet,
    },
    /// The Pokémon acted (a `move` line, a `cant`): the volatiles that last until it does are
    /// seen gone (`VolatileVisibility::until_it_acts`).
    Acted(PokemonId),
    /// The turn's end is over (`upkeep`): the volatiles of the turn are seen gone
    /// (`VolatileVisibility::for_the_turn`).
    TurnEnded,
}

/// Which volatiles the observers see start and go (DESIGN.md §47.0 item 13, R-M10-26), by
/// generation (`ConditionRules::volatile_visibility`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VolatileVisibility {
    /// Kinds whose start shows nothing: the battle's counters, marks and locks, and a
    /// generation's silent ones.
    pub unseen: VolatileSet,
    /// Kinds whose end shows nothing (no line): seen on until the Pokémon leaves the field, or
    /// acts (`until_it_acts`).
    pub ending_unseen: VolatileSet,
    /// Kinds seen gone as the Pokémon next acts (`-singlemove`, a recharge, Focus Punch's
    /// focus).
    pub until_it_acts: VolatileSet,
    /// Kinds seen gone as the turn ends (`-singleturn`).
    pub for_the_turn: VolatileSet,
}

/// What observers know of a Pokémon's ability and item, by the one seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revealed {
    /// Its ability, once named; forgotten when it changes unnamed (Skill Swap, a Transform),
    /// back to its own as it leaves the field.
    #[serde(with = "crate::snapshot::wire")]
    ability: Option<AbilityId>,
    /// Its own ability, the one it comes in with: named while unchanged since it came in;
    /// forgotten when a forme for good brings another (a Mega Evolution).
    #[serde(with = "crate::snapshot::wire")]
    own_ability: Option<AbilityId>,
    /// Its ability changed since it came in.
    ability_changed: bool,
    /// The Pokémon whose ability it took by Transform, nothing naming it since: whoever knows
    /// that one's ability knows its own (`PokemonView::ability`).
    #[serde(with = "crate::snapshot::wire")]
    copied_from: Option<PokemonId>,
    /// Its item: `Some(Some(item))` once named, `Some(None)` once seen lost, `None` unknown.
    #[serde(with = "crate::snapshot::wire::option_option")]
    item: Option<Option<ItemId>>,
    /// The item last seen lost.
    #[serde(with = "crate::snapshot::wire")]
    lost: Option<ItemId>,
}

/// What an Illusion's holder shows while it stands in its disguise (DESIGN.md §47.0 item 11):
/// the disguise's record as the observers believe it, and which of it the holder set. Unmasked,
/// what the holder set is its own; gone still disguised, it is what they believe of the disguise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Worn {
    #[serde(with = "crate::snapshot::wire")]
    holder: PokemonId,
    #[serde(with = "crate::snapshot::wire")]
    disguise: PokemonId,
    revealed: Revealed,
    ability_set: bool,
    item_set: bool,
}

/// The part of a record a disclosure sets.
#[derive(Clone, Copy)]
enum Part {
    Ability,
    Item,
}

/// A move observers have seen a Pokémon has (DESIGN.md §47.0 item 13), by the Pokémon itself and
/// the one it was seen as: an Illusion's holder shows its moves as the disguise's, and they are
/// its own once it is unmasked — what it showed in a disguise it left in is believed the
/// disguise's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct KnownMove {
    #[serde(with = "crate::snapshot::wire")]
    user: PokemonId,
    #[serde(with = "crate::snapshot::wire")]
    shown_as: PokemonId,
    /// Shown in a disguise it still stands in.
    worn: bool,
    #[serde(with = "crate::snapshot::wire")]
    move_id: MoveId,
    /// The uses seen of it by the Pokémon's own action.
    uses: u16,
    kind: MoveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum MoveKind {
    /// One of its own moves.
    Own,
    /// One of the moves it took by Transform, while it stays on the field.
    Transformed,
    /// The move Mimic put in Mimic's place, while it stays on the field.
    Mimicked,
}

impl KnownMove {
    /// What tells one record from another: the uses aside.
    fn key(&self) -> (PokemonId, PokemonId, bool, MoveKind, MoveId) {
        (self.user, self.shown_as, self.worn, self.kind, self.move_id)
    }
}

impl Revealed {
    /// Its ability, as last named (`None`: unknown).
    pub fn ability(&self) -> Option<AbilityId> {
        self.ability
    }

    /// The Pokémon whose ability it took by Transform (`Revealed::copied_from`).
    pub fn copied_from(&self) -> Option<PokemonId> {
        self.copied_from
    }

    /// Its item: `Some(Some(item))` named, `Some(None)` seen lost, `None` unknown.
    pub fn item(&self) -> Option<Option<ItemId>> {
        self.item
    }
}

/// What the battle has shown so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Knowledge {
    /// The Pokémon seen on the field, in the order they first came.
    #[serde(with = "crate::snapshot::wire")]
    appeared: Vec<PokemonId>,
    /// What was last seen of each Pokémon that has left the field, by the one seen.
    #[serde(with = "crate::snapshot::wire")]
    sightings: Vec<(PokemonId, Sighting)>,
    /// The Pokémon on the field, each with the one it is seen as: what happens to them is seen
    /// as it happens, not through a sighting.
    #[serde(with = "crate::snapshot::wire")]
    on_field: Vec<(PokemonId, PokemonId)>,
    /// The Pokémon seen fainting, by the one seen, until seen again.
    #[serde(with = "crate::snapshot::wire")]
    fainted: Vec<PokemonId>,
    /// The abilities and items seen, by the Pokémon seen.
    #[serde(with = "crate::snapshot::wire")]
    revealed: Vec<(PokemonId, Revealed)>,
    /// The Illusion holders standing in a disguise that have shown something in it.
    worn: Vec<Worn>,
    /// The moves seen, in the order they were first seen.
    moves: Vec<KnownMove>,
    /// The Pokémon standing transformed, each with the one it transformed into.
    #[serde(with = "crate::snapshot::wire")]
    transforms: Vec<(PokemonId, PokemonId)>,
    /// The moves a Pokémon took by Transform, which its side alone saw (its requests list them):
    /// the one that transformed, the one it transformed into, the move.
    #[serde(with = "crate::snapshot::wire")]
    copied: Vec<(PokemonId, PokemonId, MoveId)>,
    /// The volatiles seen on the Pokémon on the field, by the Pokémon itself.
    #[serde(with = "crate::snapshot::wire")]
    volatiles: Vec<(PokemonId, VolatileConditionKind)>,
    /// Those a Pokémon had as it last left the field, for a Baton Pass to hand on.
    #[serde(with = "crate::snapshot::wire")]
    left_with: Vec<(PokemonId, VolatileConditionKind)>,
    /// What the team preview showed (DESIGN.md §47.0 item 14), set once as the battle starts.
    #[serde(with = "crate::snapshot::option_arc")]
    preview: Option<Arc<TeamPreview>>,
    /// The Pokémon whose moves an open team sheet showed, all of them.
    #[serde(with = "crate::snapshot::wire")]
    sheets: Vec<PokemonId>,
}

impl Knowledge {
    /// Takes in what `disclosure` shows, the volatiles as `visibility` says they are seen.
    pub fn learn(&mut self, disclosure: Disclosure, visibility: &VolatileVisibility) {
        match disclosure {
            // Two of one Pokémon on the field at once tell the observers that one is an
            // Illusion's holder, but not which (both came in as it): each stays what it showed.
            Disclosure::Appeared { pokemon, seen_as } => {
                if !self.appeared.contains(&seen_as) {
                    self.appeared.push(seen_as);
                }
                // Unmasked where it stands, the holder takes back what it showed in the
                // disguise; coming in, a Pokémon's ability is its own.
                if let Some(&(_, disguise)) = self
                    .on_field
                    .iter()
                    .find(|(on, seen)| *on == pokemon && *seen != seen_as)
                {
                    self.unmask(pokemon);
                    self.unmask_moves(pokemon);
                    // The Tera the disguise showed was the holder's (its details show the
                    // holder's own; a side Terastallizes once): the disguise is believed
                    // Terastallized no more, as Showdown's client has it (`replace`).
                    if let Some(seen) = self.sighting_mut(disguise) {
                        seen.terastallized = None;
                    }
                } else {
                    self.ability_back(seen_as);
                }
                // A stay begins (an Illusion broken where it stands goes on with its own).
                self.left_with.retain(|(on, _)| *on != pokemon);
                // Seen again, standing: not fainted — one seen fainting was an Illusion's holder
                // in its guise (R-M10-47). Its sighting stays: under a disguise that breaks, the
                // one it shows was not seen, and what was seen of it holds.
                self.fainted.retain(|seen| *seen != seen_as);
                self.on_field.retain(|(on, _)| *on != pokemon);
                self.on_field.push((pokemon, seen_as));
            }
            Disclosure::Left {
                pokemon,
                seen_as,
                sighting,
            } => {
                self.leave_disguised(pokemon);
                self.moves_leave(pokemon);
                self.volatiles_leave(pokemon);
                self.on_field.retain(|(on, _)| *on != pokemon);
                match self.sighting_mut(seen_as) {
                    Some(seen) => *seen = sighting,
                    None => self.sightings.push((seen_as, sighting)),
                }
                self.ability_back(seen_as);
            }
            Disclosure::Fainted(pokemon) => {
                let seen = self.seen_as(pokemon).unwrap_or(pokemon);
                self.leave_disguised(pokemon);
                self.moves_leave(pokemon);
                self.volatiles_leave(pokemon);
                self.on_field.retain(|(on, _)| *on != pokemon);
                if !self.fainted.contains(&seen) {
                    self.fainted.push(seen);
                }
                // A faint clears its volatiles as leaving does (`clearVolatile`): an ability
                // Transform gave it is gone — one a Revival Blessing brings back has its own
                // (R-M10-54).
                self.ability_back(seen);
            }
            // Seen now, off the field, with its HP and no status: what was seen of it before it
            // fainted holds otherwise. One never seen leaving — it fainted on the field — is seen
            // so for the first time: in another's guise as it comes back, it must not show what
            // the guise takes (R-M11-50).
            Disclosure::Revived(pokemon, sighting) => {
                self.fainted.retain(|seen| *seen != pokemon);
                match self.sighting_mut(pokemon) {
                    Some(seen) => {
                        seen.hp = sighting.hp;
                        seen.status = None;
                    }
                    None => self.sightings.push((pokemon, sighting)),
                }
            }
            // Off the field, on what was seen of whom the line names: an Illusion's holder that
            // went out disguised is cured as its disguise (R-M11-51).
            Disclosure::Cured { pokemon, seen_as } => {
                if !self.is_on_field(pokemon)
                    && let Some(seen) = self.sighting_mut(seen_as)
                {
                    seen.status = None;
                }
            }
            Disclosure::FormeShown {
                pokemon,
                species,
                keeps_ability,
            } => {
                if !self.is_on_field(pokemon)
                    && let Some(seen) = self.sighting_mut(pokemon)
                {
                    seen.species = Some(species);
                }
                // A forme for good brings its own ability (`baseAbility`), unnamed — but
                // Disguise's and Ice Face's, which leave it (R-M10-40).
                if !keeps_ability {
                    self.revealed_mut(pokemon, Part::Ability).own_ability = None;
                }
            }
            // What a Pokémon on the field shows is the one it is seen as's (an Illusion's
            // disguise's).
            Disclosure::AbilityShown(pokemon, ability) => {
                let seen = self.revealed_mut(pokemon, Part::Ability);
                seen.ability = Some(ability);
                seen.copied_from = None;
                if !seen.ability_changed {
                    seen.own_ability = Some(ability);
                }
            }
            Disclosure::OwnAbilityShown(pokemon, ability) => {
                let seen = self.revealed_mut(pokemon, Part::Ability);
                seen.ability = Some(ability);
                seen.own_ability = Some(ability);
                seen.ability_changed = false;
                seen.copied_from = None;
            }
            Disclosure::AbilityChanged(pokemon) => {
                let seen = self.revealed_mut(pokemon, Part::Ability);
                seen.ability = None;
                seen.ability_changed = true;
                seen.copied_from = None;
            }
            // The old one is its own while unchanged since it came in.
            Disclosure::AbilityReplaced { pokemon, old, new } => {
                let seen = self.revealed_mut(pokemon, Part::Ability);
                if !seen.ability_changed {
                    seen.own_ability = Some(old);
                }
                seen.ability = Some(new);
                seen.ability_changed = true;
                seen.copied_from = None;
            }
            Disclosure::ItemShown(pokemon, item) => {
                self.revealed_mut(pokemon, Part::Item).item = Some(Some(item));
            }
            // The item last seen lost, at work after (a berry's heal once eaten, a Red Card's
            // drag once used up — a Symbiosis may have handed over another in between), shows
            // nothing new.
            Disclosure::ItemActed(pokemon, item) => {
                let seen = self.revealed_mut(pokemon, Part::Item);
                if seen.lost != Some(item) {
                    seen.item = Some(Some(item));
                }
            }
            Disclosure::ItemLost(pokemon, item) => {
                let seen = self.revealed_mut(pokemon, Part::Item);
                seen.item = Some(None);
                seen.lost = Some(item);
            }
            Disclosure::MoveUsed(pokemon, move_id) => {
                // Focus Punch's focus is seen gone as the move is used (the client's `useMove`);
                // another action leaves it to the turn's end (R-M10-43).
                if move_id == MoveId::FOCUS_PUNCH {
                    self.volatiles
                        .retain(|seen| *seen != (pokemon, VolatileConditionKind::FocusPunch));
                }
                self.note_move(pokemon, move_id, 1)
            }
            Disclosure::MoveShown(pokemon, move_id) => self.note_move(pokemon, move_id, 0),
            Disclosure::Transformed {
                pokemon,
                into,
                copied,
            } => {
                // Its ability is `into`'s (`setAbility(pokemon.ability, …, isTransform)`, no
                // line; Gen 1-2's none): as known of `into`, and to `into`'s side whatever it is
                // (R-M10-40).
                let theirs = self.revealed(into).ability;
                let seen = self.revealed_mut(pokemon, Part::Ability);
                seen.ability = theirs;
                seen.ability_changed = true;
                seen.copied_from = Some(into);
                // Gen 1 transforms into a transformed Pokémon: the moves are the ones that one took.
                let into = self.transformed_into(into).unwrap_or(into);
                self.moves.retain(|known| {
                    !(known.user == pokemon && known.kind == MoveKind::Transformed)
                });
                self.transforms.retain(|(on, _)| *on != pokemon);
                self.transforms.push((pokemon, into));
                for move_id in copied.into_iter().flatten() {
                    if !self.copied.contains(&(pokemon, into, move_id)) {
                        self.copied.push((pokemon, into, move_id));
                    }
                }
            }
            // Sketch becomes the move for good; Mimic's stands in its place for the stay.
            Disclosure::MoveCopied {
                pokemon,
                move_id,
                by: MoveId::SKETCH,
            } => match self.moves.iter_mut().find(|known| {
                known.user == pokemon
                    && known.kind == MoveKind::Own
                    && known.move_id == MoveId::SKETCH
            }) {
                Some(known) => {
                    known.move_id = move_id;
                    known.uses = 0;
                    self.merge_moves();
                }
                None => self.bump(pokemon, move_id, MoveKind::Own, 0),
            },
            Disclosure::MoveCopied {
                pokemon, move_id, ..
            } => {
                self.moves
                    .retain(|known| !(known.user == pokemon && known.kind == MoveKind::Mimicked));
                self.bump(pokemon, move_id, MoveKind::Mimicked, 0);
            }
            Disclosure::VolatileStarted(pokemon, kind) => {
                if !visibility.unseen.contains(kind) && !self.volatiles.contains(&(pokemon, kind)) {
                    self.volatiles.push((pokemon, kind));
                }
            }
            Disclosure::VolatileEnded(pokemon, kind) => {
                if !visibility.ending_unseen.contains(kind) {
                    self.volatiles.retain(|seen| *seen != (pokemon, kind));
                }
            }
            Disclosure::Acted(pokemon) => self
                .volatiles
                .retain(|(on, kind)| *on != pokemon || !visibility.until_it_acts.contains(*kind)),
            Disclosure::TurnEnded => self
                .volatiles
                .retain(|(_, kind)| !visibility.for_the_turn.contains(*kind)),
            Disclosure::VolatilesPassed { from, to, kinds } => {
                for index in 0..self.left_with.len() {
                    let (on, kind) = self.left_with[index];
                    if on == from && kinds.contains(kind) && !self.volatiles.contains(&(to, kind)) {
                        self.volatiles.push((to, kind));
                    }
                }
            }
        }
        // One record a move: `merge_moves` keeps it so wherever a record changes its key.
        debug_assert!(
            self.moves.iter().enumerate().all(|(index, known)| {
                self.moves[..index]
                    .iter()
                    .all(|earlier| earlier.key() != known.key())
            }),
            "{:?}",
            self.moves
        );
    }

    /// `pokemon` leaves the field: its volatiles go with it, kept for a Baton Pass to hand on.
    fn volatiles_leave(&mut self, pokemon: PokemonId) {
        self.left_with.retain(|(on, _)| *on != pokemon);
        let mut index = 0;
        while index < self.volatiles.len() {
            if self.volatiles[index].0 == pokemon {
                let left = self.volatiles.remove(index);
                self.left_with.push(left);
            } else {
                index += 1;
            }
        }
    }

    /// The volatiles seen on `pokemon` now, in the order they started.
    pub fn volatiles_seen(
        &self,
        pokemon: PokemonId,
    ) -> impl Iterator<Item = VolatileConditionKind> + '_ {
        self.volatiles
            .iter()
            .filter(move |(on, _)| *on == pokemon)
            .map(|(_, kind)| *kind)
    }

    /// `pokemon` showed `move_id` as one it has now, `uses` of it by its own action: one it took
    /// by Transform is the one it transformed into's too.
    fn note_move(&mut self, pokemon: PokemonId, move_id: MoveId, uses: u16) {
        if let Some(into) = self.transformed_into(pokemon) {
            self.bump(pokemon, move_id, MoveKind::Transformed, uses);
            self.bump(into, move_id, MoveKind::Own, 0);
            return;
        }
        let kind = if self.moves.iter().any(|known| {
            known.user == pokemon && known.kind == MoveKind::Mimicked && known.move_id == move_id
        }) {
            MoveKind::Mimicked
        } else {
            MoveKind::Own
        };
        self.bump(pokemon, move_id, kind, uses);
    }

    /// Adds `uses` to `user`'s record of `move_id`, as it is seen now.
    fn bump(&mut self, user: PokemonId, move_id: MoveId, kind: MoveKind, uses: u16) {
        let shown_as = self.seen_as(user).unwrap_or(user);
        let worn = shown_as != user;
        match self
            .moves
            .iter_mut()
            .find(|known| known.key() == (user, shown_as, worn, kind, move_id))
        {
            Some(known) => known.uses = known.uses.saturating_add(uses),
            None => self.moves.push(KnownMove {
                user,
                shown_as,
                worn,
                move_id,
                uses,
                kind,
            }),
        }
    }

    /// `pokemon` leaves the field: what it took by Transform or Mimic goes, and what it showed in
    /// a disguise stays the disguise's.
    fn moves_leave(&mut self, pokemon: PokemonId) {
        self.moves
            .retain(|known| !(known.user == pokemon && known.kind != MoveKind::Own));
        self.transforms.retain(|(on, _)| *on != pokemon);
        // What it showed in a disguise is the one record of it there may now be two of.
        let mut unworn = false;
        for known in &mut self.moves {
            if known.user == pokemon && known.worn {
                known.worn = false;
                unworn = true;
            }
        }
        if unworn {
            self.merge_moves();
        }
    }

    /// The holder `pokemon`, unmasked where it stands: the moves it showed in the disguise are
    /// its own.
    fn unmask_moves(&mut self, pokemon: PokemonId) {
        let mut unworn = false;
        for known in &mut self.moves {
            if known.user == pokemon && known.worn {
                known.shown_as = pokemon;
                known.worn = false;
                unworn = true;
            }
        }
        if unworn {
            self.merge_moves();
        }
    }

    /// One record a move, the first: the uses of the others join it.
    fn merge_moves(&mut self) {
        let mut index = 0;
        while index < self.moves.len() {
            let first = self.moves[index];
            let mut other = index + 1;
            while other < self.moves.len() {
                let known = self.moves[other];
                if known.key() == first.key() {
                    self.moves[index].uses = self.moves[index].uses.saturating_add(known.uses);
                    self.moves.remove(other);
                } else {
                    other += 1;
                }
            }
            index += 1;
        }
    }

    /// The Pokémon `pokemon` stands transformed into, as observers saw it.
    pub fn transformed_into(&self, pokemon: PokemonId) -> Option<PokemonId> {
        self.transforms
            .iter()
            .find(|(on, _)| *on == pokemon)
            .map(|(_, into)| *into)
    }

    /// The moves observers know `pokemon` — seen as `shown` — has now (DESIGN.md §47.0 item 13),
    /// each once, in the order they were first seen, with the uses seen of it: those seen as
    /// `shown`'s own, Mimic's in Mimic's place; transformed, those known of the one it took them
    /// from, then those it used that no one had shown. Then those the observer's own took by
    /// Transform from it (`sees`: whether the observer is on the side of the one that took them).
    pub fn moves_seen<'s>(
        &'s self,
        pokemon: PokemonId,
        shown: PokemonId,
        sees: impl Fn(PokemonId) -> bool + Copy + 's,
    ) -> impl Iterator<Item = (MoveId, u16)> + 's {
        let into = self.transformed_into(pokemon);
        let mimicked = self
            .moves
            .iter()
            .find(|known| known.user == pokemon && known.kind == MoveKind::Mimicked)
            .filter(|_| into.is_none());
        let owner = into.unwrap_or(shown);
        let listed = self
            .distinct(owner)
            .map(move |(move_id, uses)| match (into, mimicked) {
                (Some(_), _) => (move_id, self.uses(pokemon, move_id, MoveKind::Transformed)),
                (None, Some(mimic)) if move_id == MoveId::MIMIC => (mimic.move_id, mimic.uses),
                _ => (move_id, uses),
            });
        let extra = self
            .moves
            .iter()
            .filter(move |known| {
                known.user == pokemon
                    && match into {
                        Some(into) => {
                            known.kind == MoveKind::Transformed
                                && !self.distinct(into).any(|(id, _)| id == known.move_id)
                        }
                        None => {
                            known.kind == MoveKind::Mimicked
                                && !self.distinct(owner).any(|(id, _)| id == MoveId::MIMIC)
                        }
                    }
            })
            .map(|known| (known.move_id, known.uses));
        let listed_or_extra = move |move_id: MoveId| {
            self.distinct(owner).any(|(id, _)| id == move_id)
                || self.moves.iter().any(|known| {
                    known.user == pokemon && known.kind != MoveKind::Own && known.move_id == move_id
                })
        };
        let copied = self
            .copied
            .iter()
            .enumerate()
            .filter(move |(index, (by, into, move_id))| {
                *into == owner
                    && sees(*by)
                    && !listed_or_extra(*move_id)
                    && !self.copied[..*index]
                        .iter()
                        .any(|(earlier, of, id)| of == into && id == move_id && sees(*earlier))
            })
            .map(|(_, (_, _, move_id))| (*move_id, 0));
        listed.chain(extra).chain(copied)
    }

    /// Whether the observer (`sees`, as for `moves_seen`) saw all of `pokemon`'s moves: one of its
    /// own took them by Transform.
    pub fn moves_copied_by(&self, pokemon: PokemonId, sees: impl Fn(PokemonId) -> bool) -> bool {
        self.copied
            .iter()
            .any(|(by, into, _)| *into == pokemon && sees(*by))
    }

    /// The own moves seen as `shown`'s, each once, in the order first seen, their uses summed
    /// (a disguise's and its holder's).
    fn distinct(&self, shown: PokemonId) -> impl Iterator<Item = (MoveId, u16)> + '_ {
        let of = move |known: &KnownMove| known.kind == MoveKind::Own && known.shown_as == shown;
        self.moves
            .iter()
            .enumerate()
            .filter(move |(index, known)| {
                of(known)
                    && !self.moves[..*index]
                        .iter()
                        .any(|earlier| of(earlier) && earlier.move_id == known.move_id)
            })
            .map(move |(_, known)| {
                let uses = self
                    .moves
                    .iter()
                    .filter(|other| of(other) && other.move_id == known.move_id)
                    .fold(0u16, |uses, other| uses.saturating_add(other.uses));
                (known.move_id, uses)
            })
    }

    /// The uses seen of `pokemon`'s `move_id` of `kind`, whatever it was seen as.
    fn uses(&self, pokemon: PokemonId, move_id: MoveId, kind: MoveKind) -> u16 {
        self.moves
            .iter()
            .filter(|known| known.user == pokemon && known.kind == kind && known.move_id == move_id)
            .fold(0u16, |uses, known| uses.saturating_add(known.uses))
    }

    /// The uses everyone saw `pokemon` make of `move_id`, one of the moves it has now: its own,
    /// Mimic's, or one it took by Transform.
    pub fn observed_uses(&self, pokemon: PokemonId, move_id: MoveId) -> u16 {
        let kind = if self.transformed_into(pokemon).is_some() {
            MoveKind::Transformed
        } else if self.moves.iter().any(|known| {
            known.user == pokemon && known.kind == MoveKind::Mimicked && known.move_id == move_id
        }) {
            MoveKind::Mimicked
        } else {
            MoveKind::Own
        };
        self.uses(pokemon, move_id, kind)
    }

    /// `pokemon` (the one seen) comes in or leaves: its ability is its own again.
    fn ability_back(&mut self, pokemon: PokemonId) {
        if let Some((_, seen)) = self.revealed.iter_mut().find(|(id, _)| *id == pokemon) {
            seen.ability = seen.own_ability;
            seen.ability_changed = false;
            seen.copied_from = None;
        }
    }

    /// The holder `pokemon`, unmasked where it stands: what it showed in its disguise is its own;
    /// the disguise is as it was believed before.
    fn unmask(&mut self, pokemon: PokemonId) {
        let Some(index) = self.worn.iter().position(|worn| worn.holder == pokemon) else {
            return;
        };
        let worn = self.worn.remove(index);
        let own = self.own_record(pokemon);
        if worn.ability_set {
            own.ability = worn.revealed.ability;
            own.ability_changed = worn.revealed.ability_changed;
            if !worn.revealed.ability_changed {
                own.own_ability = worn.revealed.ability;
            }
        }
        if worn.item_set {
            own.item = worn.revealed.item;
            own.lost = worn.revealed.lost;
        }
    }

    /// The holder `pokemon` leaves the field (or faints) still disguised: what it showed is what
    /// the observers believe of the disguise.
    fn leave_disguised(&mut self, pokemon: PokemonId) {
        let Some(index) = self.worn.iter().position(|worn| worn.holder == pokemon) else {
            return;
        };
        let worn = self.worn.remove(index);
        *self.own_record(worn.disguise) = worn.revealed;
    }

    /// The record of `pokemon` itself.
    fn own_record(&mut self, pokemon: PokemonId) -> &mut Revealed {
        let index = match self.revealed.iter().position(|(id, _)| *id == pokemon) {
            Some(index) => index,
            None => {
                self.revealed.push((pokemon, Revealed::default()));
                self.revealed.len() - 1
            }
        };
        &mut self.revealed[index].1
    }

    /// The record of what `pokemon` — as it is seen — has shown, `part` of it to be set: a
    /// holder in its disguise sets the disguise's as the observers believe it (`Worn`).
    fn revealed_mut(&mut self, pokemon: PokemonId, part: Part) -> &mut Revealed {
        let seen = self.seen_as(pokemon).unwrap_or(pokemon);
        if seen != pokemon {
            let index = match self.worn.iter().position(|worn| worn.holder == pokemon) {
                Some(index) => index,
                None => {
                    let revealed = self.revealed(seen);
                    self.worn.push(Worn {
                        holder: pokemon,
                        disguise: seen,
                        revealed,
                        ability_set: false,
                        item_set: false,
                    });
                    self.worn.len() - 1
                }
            };
            let worn = &mut self.worn[index];
            match part {
                Part::Ability => worn.ability_set = true,
                Part::Item => worn.item_set = true,
            }
            return &mut worn.revealed;
        }
        let index = match self.revealed.iter().position(|(id, _)| *id == seen) {
            Some(index) => index,
            None => {
                self.revealed.push((seen, Revealed::default()));
                self.revealed.len() - 1
            }
        };
        &mut self.revealed[index].1
    }

    /// What `pokemon` (the one seen) has shown of its ability and item: while a holder stands
    /// disguised as it and has shown something, what the observers believe of it then.
    pub fn revealed(&self, pokemon: PokemonId) -> Revealed {
        if let Some(worn) = self.worn.iter().find(|worn| worn.disguise == pokemon) {
            return worn.revealed;
        }
        self.revealed
            .iter()
            .find(|(id, _)| *id == pokemon)
            .map(|(_, revealed)| *revealed)
            .unwrap_or_default()
    }

    fn is_on_field(&self, pokemon: PokemonId) -> bool {
        self.on_field.iter().any(|(on, _)| *on == pokemon)
    }

    /// Whether `pokemon` was seen fainting (and not seen since).
    pub fn seen_fainted(&self, pokemon: PokemonId) -> bool {
        self.fainted.contains(&pokemon)
    }

    /// The Pokémon observers see `pokemon` as, while it is on the field.
    pub fn seen_as(&self, pokemon: PokemonId) -> Option<PokemonId> {
        self.on_field
            .iter()
            .find(|(on, _)| *on == pokemon)
            .map(|(_, seen)| *seen)
    }

    fn sighting_mut(&mut self, pokemon: PokemonId) -> Option<&mut Sighting> {
        self.sightings
            .iter_mut()
            .find(|(seen, _)| *seen == pokemon)
            .map(|(_, sighting)| sighting)
    }

    /// Whether `pokemon` has been seen on the field.
    pub fn has_appeared(&self, pokemon: PokemonId) -> bool {
        self.appeared.contains(&pokemon)
    }

    /// What was last seen of `pokemon` as it left the field, if it has.
    pub fn sighting(&self, pokemon: PokemonId) -> Option<Sighting> {
        self.sightings
            .iter()
            .find(|(seen, _)| *seen == pokemon)
            .map(|(_, sighting)| *sighting)
    }

    /// The team preview (DESIGN.md §47.0 item 14): kept for the views, and what an open team
    /// sheet showed of each member its side brought known of that Pokémon from the start — its
    /// item, its ability (its own), all its moves, none of them seen used.
    pub fn show_team_preview(&mut self, preview: TeamPreview) {
        for member in preview.sides.iter().flatten() {
            let (Some(pokemon), Some(moves)) = (member.pokemon, &member.shown.moves) else {
                continue;
            };
            let seen = self.own_record(pokemon);
            if let Some(ability) = member.shown.ability {
                seen.ability = Some(ability);
                seen.own_ability = Some(ability);
            }
            if let Some(item) = member.shown.item {
                seen.item = Some(item);
            }
            for move_id in moves {
                self.bump(pokemon, *move_id, MoveKind::Own, 0);
            }
            if !self.sheets.contains(&pokemon) {
                self.sheets.push(pokemon);
            }
        }
        self.preview = Some(Arc::new(preview));
    }

    /// What the team preview showed, if there was one.
    pub fn team_preview(&self) -> Option<&TeamPreview> {
        self.preview.as_deref()
    }

    /// Whether an open team sheet showed all of `pokemon`'s moves.
    pub fn sheet_shown(&self, pokemon: PokemonId) -> bool {
        self.sheets.contains(&pokemon)
    }
}
