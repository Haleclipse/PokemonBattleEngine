//! What an observer sees of a battle (DESIGN.md §47.0 items 8, 9 and 11, §47.5 items 2-4): a
//! read-only view that borrows the battle — no copy of its state is made — and shows an
//! observer what it knows. A player knows their own side in full; of the other side, and a
//! spectator of both, what is public. The battle as it is, hidden things and all, is
//! `BattleEngine::omniscient_view`'s, for trusted hosts, research and AI alone.
//!
//! Of the other side a view knows what is always public, as the observer saw it (Showdown's
//! per-side protocol, §47.0 item 9; compared with @pkmn/client's, `tests/support/views.rs`): who
//! stands where, as it appears (an Illusion's disguise; a forme its details hide as its base
//! species); the members seen on the field — a member off it as it was last seen, before a heal
//! it took unseen as it left (`state::Knowledge`); level, gender, HP as the format shares it,
//! status, stat stages, a faint, Terastallization. Its item, ability, moves and volatiles are what
//! the battle has shown (`state::Knowledge`, §47.0 items 11-13). A team preview showed every
//! member each side brought, and an open team sheet more (`SideView::previewed`, §47.0 item 14).

mod names;

pub use names::Names;

use crate::BattleEngine;
use crate::dex::PokemonType;
use crate::ids::{AbilityId, ItemId, MoveId, PokemonId, PositionId, SideId, SpeciesId};
use crate::spec::Generation;
use crate::state::{
    BattlePhase, FieldConditionKind, Gender, Knowledge, MemberSheet, MoveSlotState,
    NonVolatileStatus, PokemonState, SideConditionKind, Sighting, Stat, TerrainKind,
    VolatileConditionKind, WeatherKind,
};
use std::borrow::Cow;

/// Who looks at a battle: a player, who sees their own side, or a spectator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observer {
    Side(SideId),
    Spectator,
}

/// What an observer knows of something that may be hidden (DESIGN.md §47.0 item 11): its value,
/// that there is none (an item knocked off), or nothing yet — whether there is one included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen<T> {
    Known(T),
    KnownNone,
    Unknown,
}

impl<T> Seen<T> {
    /// What an observer that knows the truth sees.
    fn known(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Known(value),
            None => Self::KnownNone,
        }
    }
}

/// The eye a view looks with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Eye {
    Observer(Observer),
    Omniscient,
}

impl Eye {
    /// Whether this eye sees everything of `side`.
    fn knows(self, side: SideId) -> bool {
        match self {
            Self::Observer(Observer::Side(own)) => own == side,
            Self::Observer(Observer::Spectator) => false,
            Self::Omniscient => true,
        }
    }
}

/// A battle as one observer sees it.
#[derive(Clone, Copy)]
pub struct BattleView<'a> {
    engine: &'a BattleEngine,
    eye: Eye,
}

impl<'a> BattleView<'a> {
    pub(crate) fn observed(engine: &'a BattleEngine, observer: Observer) -> Self {
        Self {
            engine,
            eye: Eye::Observer(observer),
        }
    }

    pub(crate) fn omniscient(engine: &'a BattleEngine) -> Self {
        Self {
            engine,
            eye: Eye::Omniscient,
        }
    }

    /// The observer, `None` for the omniscient view.
    pub fn observer(&self) -> Option<Observer> {
        match self.eye {
            Eye::Observer(observer) => Some(observer),
            Eye::Omniscient => None,
        }
    }

    pub fn turn(&self) -> u32 {
        self.engine.state().turn
    }

    pub fn phase(&self) -> BattlePhase {
        self.engine.state().phase
    }

    pub fn generation(&self) -> Generation {
        self.engine.rules().generation
    }

    pub fn side(&self, side: SideId) -> SideView<'a> {
        SideView { view: *self, side }
    }

    pub fn field(&self) -> FieldView<'a> {
        FieldView { view: *self }
    }

    /// `pokemon` as this view shows it, through the Illusion it may wear.
    fn pokemon(&self, pokemon: &'a PokemonState) -> PokemonView<'a> {
        let state = self.engine.state();
        // An Illusion fools every eye but its own side's and the omniscient one.
        let shown = pokemon
            .illusion
            .filter(|_| !self.eye.knows(pokemon.side))
            .and_then(|disguise| state.pokemon(disguise))
            .unwrap_or(pokemon);
        PokemonView {
            view: *self,
            pokemon,
            shown,
            sighting: None,
            off_field: false,
            seen_fainted: false,
        }
    }

    /// A member of a side the view does not know, as it sees it: the one standing on the field
    /// as it (itself, or a holder disguised as it), or the member as it last saw it.
    #[inline(never)]
    fn seen_member(&self, member: &'a PokemonState) -> PokemonView<'a> {
        let state = self.engine.state();
        let standing = state
            .side(member.side)
            .active
            .iter()
            .flatten()
            .filter_map(|id| state.pokemon(*id))
            .find(|standing| self.pokemon(standing).id() == member.id());
        match standing {
            Some(standing) => self.pokemon(standing),
            None => self.remembered(member),
        }
    }

    /// `pokemon`, off the field, as this view last saw it (`Sighting`): an observer that does
    /// not know its side knows it as it was when it left.
    fn remembered(&self, pokemon: &'a PokemonState) -> PokemonView<'a> {
        let known = self.eye.knows(pokemon.side);
        let knowledge = self.engine.knowledge();
        // Seen fainting, on the field: what was seen of it before is past. One that fainted
        // unseen — an Illusion's holder, in another's guise — is as it was last seen.
        let seen_fainted = !known && knowledge.seen_fainted(pokemon.id());
        let sighting = (!known && !seen_fainted)
            .then(|| knowledge.sighting(pokemon.id()))
            .flatten();
        PokemonView {
            view: *self,
            pokemon,
            shown: pokemon,
            sighting,
            off_field: !known,
            seen_fainted,
        }
    }
}

/// One side, as the view's observer sees it.
#[derive(Clone, Copy)]
pub struct SideView<'a> {
    view: BattleView<'a>,
    side: SideId,
}

impl<'a> SideView<'a> {
    pub fn id(&self) -> SideId {
        self.side
    }

    /// The side's positions on the field, in order, with what stands at each.
    pub fn active(&self) -> impl Iterator<Item = (PositionId, Option<PokemonView<'a>>)> + 'a {
        let view = self.view;
        let side = self.side;
        let topology = view.engine.rules().topology();
        topology
            .positions()
            .iter()
            .filter(move |position| position.side == side)
            .map(move |position| {
                let state = view.engine.state();
                let standing = state
                    .pokemon_at_position(position.id, topology)
                    .and_then(|id| state.pokemon(id))
                    .map(|pokemon| view.pokemon(pokemon));
                (position.id, standing)
            })
    }

    /// The team in its order: all of it for an observer that knows the side; else the members
    /// it has seen on the field (`Knowledge`: an Illusion's disguise, not its holder) — never the
    /// rest, not even after a team preview, which shows what a side brought and not what it
    /// picked. A member seen on the field now is the one standing there as it (itself, or a
    /// holder disguised as it); one off it, the way it was last seen.
    pub fn team(&self) -> impl Iterator<Item = PokemonView<'a>> + 'a {
        let view = self.view;
        let state = view.engine.state();
        let known = view.eye.knows(self.side);
        let side = state.side(self.side);
        side.team
            .iter()
            .filter_map(move |id| state.pokemon(*id))
            .filter(move |member| known || view.engine.has_appeared(member.id()))
            .map(move |member| {
                if known {
                    view.pokemon(member)
                } else {
                    view.seen_member(member)
                }
            })
    }

    /// The members a team preview showed the side brought (DESIGN.md §47.0 item 14), picked or
    /// not, in the order registered: as the observer knows them — everything to one that knows
    /// the side; else what the preview showed (`PreviewedView`). None without a team preview.
    /// A member's own entry in `team` once it has come in is another matter: what an open team
    /// sheet showed of it is known there too, what the preview alone showed is not.
    pub fn previewed(&self) -> impl Iterator<Item = PreviewedView<'a>> + 'a {
        let known = self.view.eye.knows(self.side);
        let engine: &'a BattleEngine = self.view.engine;
        let side = self.side;
        engine
            .state()
            .knowledge
            .team_preview()
            .into_iter()
            .flat_map(move |preview| preview.sides[side.index()].iter())
            .map(move |member| PreviewedView {
                sheet: if known { &member.known } else { &member.shown },
            })
    }

    /// The conditions on the side (Reflect, Spikes, Tailwind …): public.
    pub fn conditions(&self) -> impl Iterator<Item = SideConditionKind> + 'a {
        self.view
            .engine
            .state()
            .side(self.side)
            .conditions
            .entries()
            .iter()
            .map(|condition| condition.kind())
    }

    /// How many of its team have not fainted: public.
    pub fn pokemon_left(&self) -> usize {
        self.view
            .engine
            .state()
            .living_team_members(self.side)
            .len()
    }
}

/// A member a team preview showed (DESIGN.md §47.0 item 14), as the view's observer knows it:
/// to one that knows its side, all of it; else what the preview showed — its species (a forme
/// hidden as `Urshifu-*` as its base species), level and gender, whether it holds an item where
/// the generation's preview marks it, its Tera type under Tera Type Preview, and under open team
/// sheets its exact species, item, ability, moves and Tera type.
#[derive(Clone, Copy)]
pub struct PreviewedView<'a> {
    sheet: &'a MemberSheet,
}

impl<'a> PreviewedView<'a> {
    /// Its species as the observer knows it: the base species while its forme is hidden
    /// (`forme_hidden`).
    pub fn species(&self) -> SpeciesId {
        self.sheet.species
    }

    /// Whether the preview hid its forme (`Urshifu-*`): `species` is then its base species, the
    /// forme any of the species'.
    pub fn forme_hidden(&self) -> bool {
        self.sheet.forme_hidden
    }

    pub fn level(&self) -> u8 {
        self.sheet.level
    }

    pub fn gender(&self) -> Gender {
        self.sheet.gender
    }

    /// Its item: an open team sheet's; `KnownNone` where a Gen 4-7 preview marked it holding
    /// none; else `Unknown`.
    pub fn item(&self) -> Seen<ItemId> {
        match self.sheet.item {
            Some(item) => Seen::known(item),
            None => Seen::Unknown,
        }
    }

    /// Its ability: an open team sheet's, else `Unknown` (a sheet's, not a battling Pokémon's:
    /// no engine entry to read it through).
    pub fn ability(&self) -> Seen<AbilityId> {
        let MemberSheet { ability, .. } = self.sheet;
        match ability {
            Some(ability) => Seen::Known(*ability),
            None => Seen::Unknown,
        }
    }

    /// All its moves, in order, when an open team sheet showed them.
    pub fn moves(&self) -> Option<&'a [MoveId]> {
        self.sheet.moves.as_deref()
    }

    /// Its Tera type: shown by Tera Type Preview or an open team sheet; `KnownNone` before Gen 9;
    /// else `Unknown`.
    pub fn tera_type(&self) -> Seen<PokemonType> {
        match self.sheet.tera_type {
            Some(tera_type) => Seen::known(tera_type),
            None => Seen::Unknown,
        }
    }
}

/// A Pokémon's HP as an observer sees it: exactly, or as Showdown shares it with the other
/// side (`getHealth`): a percentage rounded up (99 while not full), or 48ths of the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum Hp {
    Exact { hp: u16, max: u16 },
    Percent(u8),
    Pixels(u8),
}

/// A move an observer knows a Pokémon has (DESIGN.md §47.0 item 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevealedMove {
    move_id: MoveId,
    pp: Seen<(u8, u8)>,
    observed_uses: u16,
}

impl RevealedMove {
    pub fn move_id(&self) -> MoveId {
        self.move_id
    }

    /// Its PP and maximum PP: known to an observer that knows its side alone.
    pub fn pp(&self) -> Seen<(u8, u8)> {
        self.pp
    }

    /// The times everyone saw the Pokémon use it by its own action: a two-turn move or a lock
    /// once, as it starts; not a call, a dance, a reflection. Not the PP it paid: Pressure's, a
    /// Spite's are the observer's to reckon.
    pub fn observed_uses(&self) -> u16 {
        self.observed_uses
    }
}

/// What an observer knows of a Pokémon's moves (DESIGN.md §47.0 item 13): to one that knows its
/// side, its slots; else the moves it was seen to have, never its slots or their order.
#[derive(Clone)]
pub struct MovesView<'a> {
    pokemon: PokemonView<'a>,
    knowledge: Cow<'a, Knowledge>,
}

impl MovesView<'_> {
    /// The moves the observer knows the Pokémon has now: its slots, in order, to an observer
    /// that knows its side, or that knows the side of the Pokémon it transformed into (it took
    /// them all); else the ones seen, in the order first seen — Mimic's copy in Mimic's place,
    /// a Pokémon transformed into another the moves known of that one.
    pub fn revealed(&self) -> impl Iterator<Item = RevealedMove> + '_ {
        let id = self.pokemon.pokemon.id();
        let knowledge = &*self.knowledge;
        let slots = self.slots();
        let pp_known = self.pokemon.known();
        let from_slots = slots.into_iter().flatten().map(move |slot| RevealedMove {
            move_id: slot.move_id,
            pp: if pp_known {
                Seen::Known((slot.pp, slot.max_pp))
            } else {
                Seen::Unknown
            },
            observed_uses: knowledge.observed_uses(id, slot.move_id),
        });
        let sees = self.sees();
        let seen = slots
            .is_none()
            .then(|| knowledge.moves_seen(id, self.pokemon.id(), sees))
            .into_iter()
            .flatten()
            .map(|(move_id, observed_uses)| RevealedMove {
                move_id,
                pp: Seen::Unknown,
                observed_uses,
            });
        from_slots.chain(seen)
    }

    /// Whether the observer knows these are all its moves: when it knows its slots, saw them all
    /// in the requests of one of its own that took them by Transform or on an open team sheet,
    /// or four different ones were seen (a Pokémon has four at most).
    pub fn complete(&self) -> bool {
        let pokemon = self.pokemon.pokemon.id();
        let owner = self
            .knowledge
            .transformed_into(pokemon)
            .unwrap_or(self.pokemon.id());
        self.slots().is_some()
            || self.knowledge.moves_copied_by(owner, self.sees())
            || self.knowledge.sheet_shown(owner)
            || self.revealed().count() >= 4
    }

    /// Whether the observer is on the side of `pokemon` (one that took moves by Transform).
    fn sees(&self) -> impl Fn(PokemonId) -> bool + Copy + '_ {
        let view = self.pokemon.view;
        move |pokemon| {
            view.engine
                .state()
                .pokemon(pokemon)
                .is_some_and(|pokemon| view.eye.knows(pokemon.side))
        }
    }

    /// The Pokémon's slots, when the observer knows them.
    fn slots(&self) -> Option<&[MoveSlotState]> {
        let pokemon = self.pokemon.pokemon;
        if self.pokemon.known() {
            return Some(pokemon.move_slots());
        }
        let into = self.knowledge.transformed_into(pokemon.id())?;
        let side = self.pokemon.view.engine.state().pokemon(into)?.side;
        self.pokemon
            .view
            .eye
            .knows(side)
            .then(|| pokemon.move_slots())
    }
}

/// A Pokémon as the view's observer sees it: through an Illusion that fools them.
#[derive(Clone, Copy)]
pub struct PokemonView<'a> {
    view: BattleView<'a>,
    /// The Pokémon itself.
    pokemon: &'a PokemonState,
    /// What it appears to be: itself, or the disguise an Illusion shows this observer.
    shown: &'a PokemonState,
    /// Off the field, to an observer that does not know its side: how it was last seen.
    sighting: Option<Sighting>,
    /// Off the field to the observer, which does not know its side (it may stand there under
    /// an Illusion that shows another): no stages.
    off_field: bool,
    /// Off the field and seen fainting by the observer, which does not know its side: itself, or
    /// an Illusion's holder in its guise.
    seen_fainted: bool,
}

impl<'a> PokemonView<'a> {
    /// Whether the observer knows everything of this Pokémon.
    fn known(&self) -> bool {
        self.view.eye.knows(self.pokemon.side)
    }

    /// Its id as it appears: the disguise's while an Illusion fools the observer.
    pub fn id(&self) -> PokemonId {
        self.shown.id()
    }

    pub fn side(&self) -> SideId {
        self.pokemon.side
    }

    /// Its species as it appears (a forme it changed into included). To an observer that does
    /// not know its side, a forme that shows its base species (`dex::HIDDEN_FORMES`: Rockruff
    /// with Own Tempo) is that base species.
    pub fn species(&self) -> Option<SpeciesId> {
        let species = match self.sighting {
            Some(sighting) => sighting.species?,
            None => crate::engine::species_of(self.shown)?,
        };
        if self.known() {
            return Some(species);
        }
        let dex = self.view.engine.dex();
        Some(
            dex.species_by_id(species)
                .map_or(species, |resolved| dex.shown_species(resolved).id),
        )
    }

    /// Its level: its own, through an Illusion too. Showdown's details show the disguise's
    /// species and gender with the wearer's level (`getFullDetails`); the Illusion Level Mod,
    /// which shows the disguise's, belongs to Showdown's random battles alone.
    pub fn level(&self) -> u8 {
        self.sighting
            .map_or(self.pokemon.level, |sighting| sighting.level)
    }

    pub fn gender(&self) -> Gender {
        self.shown.gender
    }

    /// Its HP: exactly for an observer that knows its side, else as the format shares it
    /// (`HpReport`).
    pub fn hp(&self) -> Hp {
        let (hp, max) = (self.pokemon.hp, self.pokemon.max_hp);
        if self.known() {
            return Hp::Exact { hp, max };
        }
        // Off the field, as it was last seen; a faint is seen.
        let (hp, max) = match self.sighting {
            _ if self.seen_fainted => (0, max),
            Some(sighting) => (sighting.hp, sighting.max_hp),
            _ => (hp, max),
        };
        match self.view.engine.rules().hp_report {
            crate::rules::HpReport::Exact => Hp::Exact { hp, max },
            crate::rules::HpReport::Percentage => Hp::Percent(shared_percent(hp, max)),
            crate::rules::HpReport::Pixels => Hp::Pixels(shared_pixels(hp, max)),
        }
    }

    /// Whether it fainted: off the field, to an observer that does not know its side, whether
    /// it was seen fainting — an Illusion's holder fainted in another's guise.
    pub fn fainted(&self) -> bool {
        if self.off_field {
            self.seen_fainted
        } else {
            self.pokemon.is_fainted()
        }
    }

    pub fn status(&self) -> Option<NonVolatileStatus> {
        match self.sighting {
            // A disguise was seen fainting in its guise: nothing of it was seen.
            _ if self.seen_fainted && !self.pokemon.is_fainted() => None,
            Some(sighting) => sighting.status,
            _ => self.pokemon.status.as_ref().map(|status| status.kind),
        }
    }

    /// Its stage in `stat`: public.
    pub fn boost(&self, stat: Stat) -> i8 {
        if self.off_field {
            return 0;
        }
        self.pokemon.stages.get(stat)
    }

    /// The type it has Terastallized into: public — off the field, as it was last seen (a
    /// disguise an Illusion's holder Terastallized in, and left in, is believed Terastallized).
    pub fn terastallized(&self) -> Option<PokemonType> {
        self.sighting
            .map_or(self.pokemon.terastallized, |sighting| {
                sighting.terastallized
            })
    }

    /// Its item: to an observer that does not know its side, the one last seen named or at work
    /// (`KnownNone` once seen lost), until then `Unknown`.
    pub fn item(&self) -> Seen<ItemId> {
        if self.known() {
            return Seen::known(self.pokemon.held_item);
        }
        match self.view.engine.knowledge().revealed(self.id()).item() {
            Some(item) => Seen::known(item),
            None => Seen::Unknown,
        }
    }

    /// Its ability: to an observer that does not know its side, the one last seen named, until
    /// then — or once it changed unnamed — `Unknown`; but one it took by Transform from a
    /// Pokémon of the observer's own side, which the observer knows.
    pub fn ability(&self) -> Seen<AbilityId> {
        if self.known() {
            return Seen::Known(self.pokemon.battle_ability());
        }
        let revealed = self.view.engine.knowledge().revealed(self.id());
        match revealed.ability() {
            Some(ability) => Seen::Known(ability),
            None if revealed.copied_from().is_some_and(|from| {
                self.view
                    .engine
                    .state()
                    .pokemon(from)
                    .is_some_and(|from| self.view.eye.knows(from.side))
            }) =>
            {
                Seen::Known(self.pokemon.battle_ability())
            }
            None => Seen::Unknown,
        }
    }

    /// Its moves, as the observer knows them (`MovesView`).
    pub fn moves(&self) -> MovesView<'a> {
        MovesView {
            pokemon: *self,
            knowledge: self.view.engine.knowledge(),
        }
    }

    /// Its volatile conditions (confusion, Substitute, Taunt …), as the observer knows them
    /// (`VolatilesView`).
    pub fn volatiles(&self) -> VolatilesView<'a> {
        VolatilesView {
            pokemon: *self,
            knowledge: self.view.engine.knowledge(),
        }
    }
}

/// What an observer knows of a Pokémon's volatile conditions (DESIGN.md §47.0 item 13): to one
/// that knows its side, all of them; else those that showed themselves as they started and are
/// still on, as far as it saw — none of the battle's own counters and marks, and an empty list
/// says only that none is known.
#[derive(Clone)]
pub struct VolatilesView<'a> {
    pokemon: PokemonView<'a>,
    knowledge: Cow<'a, Knowledge>,
}

impl VolatilesView<'_> {
    pub fn iter(&self) -> impl Iterator<Item = VolatileConditionKind> + '_ {
        let pokemon = &self.pokemon;
        let known = pokemon.known();
        let own = known
            .then(|| pokemon.pokemon.volatiles.entries().iter())
            .into_iter()
            .flatten()
            .map(|volatile| volatile.kind());
        // Off the field, to an observer that does not know its side: none.
        let seen = (!known && !pokemon.off_field)
            .then(|| self.knowledge.volatiles_seen(pokemon.pokemon.id()))
            .into_iter()
            .flatten();
        own.chain(seen)
    }

    pub fn contains(&self, kind: VolatileConditionKind) -> bool {
        self.iter().any(|seen| seen == kind)
    }
}

/// The field, as everyone sees it: public.
#[derive(Clone, Copy)]
pub struct FieldView<'a> {
    view: BattleView<'a>,
}

impl<'a> FieldView<'a> {
    pub fn weather(&self) -> Option<WeatherKind> {
        self.view
            .engine
            .state()
            .field
            .weather
            .map(|weather| weather.kind)
    }

    pub fn terrain(&self) -> Option<TerrainKind> {
        self.view
            .engine
            .state()
            .field
            .terrain
            .map(|terrain| terrain.kind)
    }

    /// The conditions on the field (Trick Room, Gravity …): the announced ones. Echoed Voice's
    /// count Showdown keeps as one without a word (`echoedvoice`'s `onFieldStart`).
    pub fn conditions(&self) -> impl Iterator<Item = FieldConditionKind> + 'a {
        self.view
            .engine
            .state()
            .field
            .conditions()
            .iter()
            .map(|condition| condition.kind())
            .filter(|kind| *kind != FieldConditionKind::EchoedVoice)
    }
}

/// Showdown `getHealth`'s shared HP in 48ths: the ratio's 48ths rounded down, 1 at the least
/// while the Pokémon stands, 0 once it fainted.
fn shared_pixels(hp: u16, max: u16) -> u8 {
    if hp == 0 || max == 0 {
        return 0;
    }
    ((u32::from(hp) * 48 / u32::from(max)).max(1)) as u8
}

/// Showdown `getHealth`'s shared HP with percentages (`HpReport::Percentage`): the ratio as a
/// percentage rounded up, 99 while the Pokémon is not full, 0 once it fainted.
fn shared_percent(hp: u16, max: u16) -> u8 {
    if hp == 0 || max == 0 {
        return 0;
    }
    let percent = (u32::from(hp) * 100).div_ceil(u32::from(max)).min(100);
    if percent == 100 && hp < max {
        99
    } else {
        percent as u8
    }
}

impl BattleEngine {
    /// The battle as `observer` sees it (DESIGN.md §47.0 item 8): borrowed, never copied.
    pub fn view(&self, observer: Observer) -> BattleView<'_> {
        BattleView::observed(self, observer)
    }

    /// The battle as it is, every hidden thing included (DESIGN.md §47.0 item 9): for trusted
    /// hosts, research and AI alone — never a client's.
    pub fn omniscient_view(&self) -> BattleView<'_> {
        BattleView::omniscient(self)
    }
}
