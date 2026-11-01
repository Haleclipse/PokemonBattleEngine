//! The session's protocol (DESIGN.md §45.5): both sides register, then — when the format
//! previews the teams or has picks — each orders its team and picks, then the battle starts and
//! belongs to the engine. Each side's choice stays its own until both have chosen.

use std::sync::Arc;

use crate::BattleEngine;
use crate::engine::BattleSnapshot;
use crate::format::TeamSet;
use crate::ids::SideId;
use crate::state::BattleRng;

use super::format::{OpenTeamSheets, RegisteredTeam, SessionError, SessionFormat};
use super::view::PreviewEntry;

/// A Pokémon's place in its side's registered team (0 first), not a battle id: before the
/// battle there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TeamSlot(pub u8);

/// A side's answer to the team choice: its Pokémon in the order it wants them, leads first.
/// Under picks the first `picked` are brought. As Showdown's `chooseTeam`: a longer order is
/// cut to `picked`, a shorter one completed with the team's other Pokémon in their order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamChoice {
    pub order: Vec<TeamSlot>,
}

impl TeamChoice {
    /// Showdown `chooseTeam`: the slots a team of `team_size` brings when `picked` of it go, in
    /// order — the choice cut to `picked`, completed with the team's other slots in their order;
    /// a slot the team lacks or a slot twice refused.
    pub fn resolve(
        &self,
        team_size: usize,
        picked: usize,
    ) -> Result<Vec<TeamSlot>, TeamChoiceError> {
        let mut order: Vec<usize> = self.order.iter().map(|slot| usize::from(slot.0)).collect();
        order.truncate(picked);
        if order.len() < picked {
            for slot in 0..picked {
                if !order.contains(&slot) {
                    order.push(slot);
                }
                if order.len() >= picked {
                    break;
                }
            }
        }
        for (at, &slot) in order.iter().enumerate() {
            let named = TeamSlot(u8::try_from(slot).unwrap_or(u8::MAX));
            if slot >= team_size {
                return Err(TeamChoiceError::NoSuchSlot(named));
            }
            if order.iter().position(|other| *other == slot) != Some(at) {
                return Err(TeamChoiceError::Duplicate(named));
            }
        }
        Ok(order.into_iter().map(|slot| TeamSlot(slot as u8)).collect())
    }
}

/// Why a team choice is refused (Showdown's `chooseTeam` errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "internals"), non_exhaustive)]
pub enum TeamChoiceError {
    NoSuchSlot(TeamSlot),
    Duplicate(TeamSlot),
}

impl std::fmt::Display for TeamChoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchSlot(slot) => write!(
                f,
                "Can't choose for Team Preview: You do not have a Pokémon in slot {}",
                slot.0 + 1
            ),
            Self::Duplicate(slot) => write!(
                f,
                "Can't choose for Team Preview: The Pokémon in slot {} can only switch in once",
                slot.0 + 1
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Registering,
    /// The team choice: Team Preview, or a private pick.
    Choosing,
    /// The battle is under way, the engine's.
    InBattle,
}

/// What the session waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRequest {
    /// The sides still to register.
    Register(Vec<SideId>),
    /// The sides still to choose: each orders its team, leads (the active slots) first, and
    /// brings the first `picked` of it.
    Choose {
        sides: Vec<SideId>,
        picked: [usize; 2],
        leads: usize,
    },
    /// The engine's request (`BattleEngine::step`).
    Battle,
}

/// What a side sees before the battle. Its own registered team; of the other's, what the
/// format shows — Team Preview's entries, the open team sheets once shown — and whether it has
/// registered and chosen, never what it chose.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub viewer: SideId,
    pub own: Option<Vec<TeamSet>>,
    pub own_choice: Option<Vec<TeamSlot>>,
    pub opponent_registered: bool,
    pub opponent_chosen: bool,
    pub opponent_preview: Option<Vec<PreviewEntry>>,
    pub opponent_sheet: Option<Vec<TeamSet>>,
}

/// A session at a moment it waits (`snapshot` / `restore`): the registration's fingerprint, the
/// registered teams, the choices made, the battle's generator, the battle once started.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    #[cfg(feature = "internals")]
    pub fingerprint: String,
    #[cfg(not(feature = "internals"))]
    pub(crate) fingerprint: String,
    #[cfg(feature = "internals")]
    pub phase: SessionPhase,
    #[cfg(not(feature = "internals"))]
    pub(crate) phase: SessionPhase,
    #[cfg(feature = "internals")]
    pub registered: [Option<RegisteredTeam>; 2],
    #[cfg(not(feature = "internals"))]
    pub(crate) registered: [Option<RegisteredTeam>; 2],
    #[cfg(feature = "internals")]
    pub choices: [Option<Vec<TeamSlot>>; 2],
    #[cfg(not(feature = "internals"))]
    pub(crate) choices: [Option<Vec<TeamSlot>>; 2],
    #[cfg(feature = "internals")]
    pub sheets_accepted: [bool; 2],
    #[cfg(not(feature = "internals"))]
    pub(crate) sheets_accepted: [bool; 2],
    #[cfg(feature = "internals")]
    pub rng: BattleRng,
    #[cfg(not(feature = "internals"))]
    pub(crate) rng: BattleRng,
    #[cfg(feature = "internals")]
    pub battle: Option<BattleSnapshot>,
    #[cfg(not(feature = "internals"))]
    pub(crate) battle: Option<BattleSnapshot>,
}

/// One battle of a format, from the registration to the battle.
pub struct BattleSession<'d> {
    format: SessionFormat<'d>,
    phase: SessionPhase,
    registered: [Option<RegisteredTeam>; 2],
    choices: [Option<Vec<TeamSlot>>; 2],
    sheets_accepted: [bool; 2],
    rng: BattleRng,
    engine: Option<BattleEngine>,
}

fn index(side: SideId) -> usize {
    match side {
        SideId::P1 => 0,
        SideId::P2 => 1,
    }
}

fn other(side: SideId) -> SideId {
    match side {
        SideId::P1 => SideId::P2,
        SideId::P2 => SideId::P1,
    }
}

impl<'d> BattleSession<'d> {
    pub fn new(format: SessionFormat<'d>, seed: u64) -> Self {
        Self {
            format,
            phase: SessionPhase::Registering,
            registered: [None, None],
            choices: [None, None],
            sheets_accepted: [false, false],
            rng: BattleRng::new(seed),
            engine: None,
        }
    }

    pub fn format(&self) -> &SessionFormat<'d> {
        &self.format
    }

    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// The battle, once started.
    pub fn engine(&self) -> Option<&BattleEngine> {
        self.engine.as_ref()
    }

    pub fn engine_mut(&mut self) -> Option<&mut BattleEngine> {
        self.engine.as_mut()
    }

    pub fn request(&self) -> SessionRequest {
        let pending = |done: &dyn Fn(usize) -> bool| -> Vec<SideId> {
            [SideId::P1, SideId::P2]
                .into_iter()
                .filter(|side| !done(index(*side)))
                .collect()
        };
        match self.phase {
            SessionPhase::Registering => {
                SessionRequest::Register(pending(&|i| self.registered[i].is_some()))
            }
            SessionPhase::Choosing => SessionRequest::Choose {
                sides: pending(&|i| self.choices[i].is_some()),
                picked: [self.picked(0), self.picked(1)],
                leads: match self.format.spec.mode {
                    crate::spec::BattleMode::Singles => 1,
                    crate::spec::BattleMode::Doubles => 2,
                },
            },
            SessionPhase::InBattle => SessionRequest::Battle,
        }
    }

    /// How many Pokémon side `i` brings: Showdown `pickedTeamSize()`, the format's picks or the
    /// whole team.
    fn picked(&self, i: usize) -> usize {
        let size = self.registered[i]
            .as_ref()
            .map_or(0, |team| team.sets.len());
        self.format
            .policy
            .picked_team_size
            .map_or(size, |picked| size.min(usize::from(picked)))
    }

    /// Registers a side's team (`SessionFormat::validate`). Once both sides have, the genders
    /// the sets leave open are drawn, P1's team first (Showdown builds the sides in that order,
    /// whichever registered first); then the team choice, or — when the format has neither
    /// preview nor picks — the battle in the registered order.
    pub fn register(&mut self, side: SideId, sets: Vec<TeamSet>) -> Result<(), SessionError> {
        if self.phase != SessionPhase::Registering {
            return Err(SessionError::WrongPhase(self.phase));
        }
        if self.registered[index(side)].is_some() {
            return Err(SessionError::AlreadyDone(side));
        }
        let team = self.format.validate(side, sets)?;
        self.registered[index(side)] = Some(team);
        if self.registered.iter().all(Option::is_some) {
            for team in self.registered.iter_mut().flatten() {
                self.format.draw_genders(team, &mut self.rng);
            }
            let policy = self.format.policy;
            if policy.preview || policy.picked_team_size.is_some() {
                self.phase = SessionPhase::Choosing;
            } else {
                self.choices = [Some(Vec::new()), Some(Vec::new())];
                self.start()?;
            }
        }
        Ok(())
    }

    /// A side's team choice, kept from the other side until both have chosen; then the battle
    /// starts from the chosen Pokémon, leads in.
    pub fn choose(&mut self, side: SideId, choice: TeamChoice) -> Result<(), SessionError> {
        if self.phase != SessionPhase::Choosing {
            return Err(SessionError::WrongPhase(self.phase));
        }
        let i = index(side);
        if self.choices[i].is_some() {
            return Err(SessionError::AlreadyDone(side));
        }
        let size = self.registered[i]
            .as_ref()
            .map_or(0, |team| team.sets.len());
        let order = choice
            .resolve(size, self.picked(i))
            .map_err(SessionError::Choice)?;
        self.choices[i] = Some(order);
        if self.choices.iter().all(Option::is_some) {
            self.start()?;
        }
        Ok(())
    }

    /// `Open Team Sheets`: a side agrees to show its sheet; both sheets show once both agree.
    pub fn accept_open_team_sheets(&mut self, side: SideId) -> Result<(), SessionError> {
        if self.phase != SessionPhase::Choosing
            || self.format.visibility.open_team_sheets != OpenTeamSheets::Offered
        {
            return Err(SessionError::WrongPhase(self.phase));
        }
        self.sheets_accepted[index(side)] = true;
        Ok(())
    }

    fn sheets_shown(&self) -> bool {
        match self.format.visibility.open_team_sheets {
            OpenTeamSheets::No => false,
            OpenTeamSheets::Forced => self.phase != SessionPhase::Registering,
            OpenTeamSheets::Offered => self.sheets_accepted.iter().all(|accepted| *accepted),
        }
    }

    fn start(&mut self) -> Result<(), SessionError> {
        // Each side's registered indices, in the order brought.
        let picked: Vec<Vec<usize>> = (0..2)
            .map(|i| {
                let sets = &self.registered[i].as_ref().expect("registered").sets;
                let order = self.choices[i].as_ref().expect("chosen");
                if order.is_empty() {
                    (0..sets.len()).collect()
                } else {
                    order.iter().map(|slot| usize::from(slot.0)).collect()
                }
            })
            .collect();
        let registered: Vec<&[TeamSet]> = (0..2)
            .map(|i| {
                self.registered[i]
                    .as_ref()
                    .expect("registered")
                    .sets
                    .as_slice()
            })
            .collect();
        let chosen: Vec<Vec<TeamSet>> = (0..2)
            .map(|i| {
                picked[i]
                    .iter()
                    .map(|&j| registered[i][j].clone())
                    .collect()
            })
            .collect();
        let mut engine = self
            .format
            .start([&chosen[0], &chosen[1]], self.rng.clone())?;
        // What the preview showed, the open team sheets with it once shown (DESIGN.md §47.0
        // item 14).
        if let Some(preview) = self.format.team_preview(
            [registered[0], registered[1]],
            [&picked[0], &picked[1]],
            self.sheets_shown(),
        )? {
            engine.show_team_preview(preview);
        }
        engine.run_lead_switch_ins();
        self.engine = Some(engine);
        self.phase = SessionPhase::InBattle;
        Ok(())
    }

    /// What `viewer` sees before the battle.
    pub fn view(&self, viewer: SideId) -> SessionView {
        let own = self.registered[index(viewer)].as_ref();
        let opponent = self.registered[index(other(viewer))].as_ref();
        let shown = self.phase != SessionPhase::Registering;
        SessionView {
            viewer,
            own: own.map(|team| team.sets.clone()),
            own_choice: self.choices[index(viewer)].clone(),
            opponent_registered: opponent.is_some(),
            opponent_chosen: self.choices[index(other(viewer))].is_some(),
            opponent_preview: opponent
                .filter(|_| shown && self.format.policy.preview)
                .map(|team| {
                    team.sets
                        .iter()
                        .map(|set| self.format.preview_entry(set))
                        .collect()
                }),
            opponent_sheet: opponent.filter(|_| self.sheets_shown()).map(|team| {
                team.sets
                    .iter()
                    .map(|set| self.format.open_sheet(set))
                    .collect()
            }),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            fingerprint: self.format.fingerprint(),
            phase: self.phase,
            registered: self.registered.clone(),
            choices: self.choices.clone(),
            sheets_accepted: self.sheets_accepted,
            rng: self.rng.clone(),
            battle: self.engine.as_ref().map(BattleEngine::snapshot),
        }
    }

    /// A session from its snapshot, under the same registration (`SessionFormat::fingerprint`):
    /// a format whose rules or data changed is another registration, refused.
    pub fn restore(
        format: SessionFormat<'d>,
        snapshot: SessionSnapshot,
    ) -> Result<Self, SessionError> {
        if snapshot.fingerprint != format.fingerprint() {
            return Err(SessionError::FingerprintMismatch);
        }
        let engine = match snapshot.battle {
            Some(battle) => Some(
                BattleEngine::restore(Arc::new(format.rules()?), format.dex.clone(), battle)
                    .map_err(SessionError::Init)?,
            ),
            None => None,
        };
        Ok(Self {
            format,
            phase: snapshot.phase,
            registered: snapshot.registered,
            choices: snapshot.choices,
            sheets_accepted: snapshot.sheets_accepted,
            rng: snapshot.rng,
            engine,
        })
    }
}
