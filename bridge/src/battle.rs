//! One local battle. The engine advances; this module only carries choices and a filtered view.

use pokemon_battle_engine::format::to_id;
use pokemon_battle_engine::view::{Hp, Observer, Seen};
use pokemon_battle_engine::{
    ActionChoice, BattleOutcome, BattleRequest, BattleSession, DeclaredTarget, EngineStep,
    ForcedReplacement, ForcedSwitchChoice, MechanicChoice, MoveSlot, PokemonId, ReviveChoice,
    SessionFormat, SessionPhase, SessionRequest, SideChoice, SideId, SlotChoice, TeamChoice,
    TeamSet, TeamSlot,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ai::Policy;
use crate::catalog::Host;
use crate::log::{self, status_name, terrain_zh, type_zh, weather_zh};
use crate::teams::{SetInput, session_team_error};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ai,
    Hotseat,
}

#[derive(Clone)]
enum Recorded {
    Choose(SideId, Vec<u8>),
    Sheets(SideId),
    Turn([SideChoice; 2]),
    Forced([ForcedSwitchChoice; 2]),
    Revive(ReviveChoice),
}

#[derive(Clone)]
struct Origin {
    format: String,
    seed: u64,
    mode: Mode,
    teams: [Vec<TeamSet>; 2],
}

struct LogLine {
    seq: u64,
    text: String,
    tone: &'static str,
}

struct CueLine {
    seq: u64,
    pokemon: u8,
    clip: &'static str,
}

pub struct BattleRoom {
    origin: Origin,
    session: BattleSession<'static>,
    mode: Mode,
    policy: Policy,
    pending_turn: [Option<SideChoice>; 2],
    pending_forced: [Option<ForcedSwitchChoice>; 2],
    logs: [Vec<LogLine>; 3],
    cues: [Vec<CueLine>; 3],
    counters: [u64; 3],
    outcome: Option<BattleOutcome>,
    last_request: Option<BattleRequest>,
    recorded: Vec<Recorded>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionIn {
    pub actor: u8,
    pub kind: String,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub target: Option<usize>,
    #[serde(default)]
    pub mechanic: Option<String>,
    #[serde(default)]
    pub pokemon: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementIn {
    pub position: usize,
    pub pokemon: u8,
}

impl BattleRoom {
    pub fn create(
        host: &Host,
        format: &str,
        seed: u64,
        mode: &str,
        p1: Vec<SetInput>,
        p2: Vec<SetInput>,
    ) -> Result<Self, String> {
        let mode = match mode {
            "ai" => Mode::Ai,
            "hotseat" => Mode::Hotseat,
            _ => return Err("对战方式只能是 ai 或 hotseat".into()),
        };
        let origin = Origin {
            format: format.to_string(),
            seed,
            mode,
            teams: [into_sets(p1), into_sets(p2)],
        };
        let mut room = Self::register(host, origin)?;
        if room.mode == Mode::Ai {
            room.choose_side(host, SideId::P2, Vec::new())?;
        }
        Ok(room)
    }

    fn register(host: &Host, origin: Origin) -> Result<Self, String> {
        let format = SessionFormat::new(host.library, &origin.format)
            .map_err(|error| format!("{error:?}"))?;
        let mut session = BattleSession::new(format, origin.seed);
        for (side, team) in [SideId::P1, SideId::P2]
            .into_iter()
            .zip(origin.teams.clone())
        {
            session
                .register(side, team)
                .map_err(|error| team_error(&error))?;
        }
        let mut room = Self {
            policy: Policy(origin.seed),
            mode: origin.mode,
            origin,
            session,
            pending_turn: [None, None],
            pending_forced: [None, None],
            logs: [Vec::new(), Vec::new(), Vec::new()],
            cues: [Vec::new(), Vec::new(), Vec::new()],
            counters: [0, 0, 0],
            outcome: None,
            last_request: None,
            recorded: Vec::new(),
        };
        if room.session.phase() == SessionPhase::InBattle {
            room.settle(host)?;
        }
        Ok(room)
    }

    pub fn state(&self, host: &Host, observer: &str) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        Ok(self.project(host, observer))
    }

    pub fn choose(&mut self, host: &Host, observer: &str, order: Vec<u8>) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        let Observer::Side(side) = observer else {
            return Err("观战不能选择队伍".into());
        };
        self.choose_side(host, side, order)?;
        Ok(self.project(host, observer))
    }

    pub fn sheets(&mut self, host: &Host, observer: &str) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        let Observer::Side(side) = observer else {
            return Err("观战不能接受公开队伍表".into());
        };
        self.session
            .accept_open_team_sheets(side)
            .map_err(|error| format!("{error:?}"))?;
        self.recorded.push(Recorded::Sheets(side));
        if self.mode == Mode::Ai {
            let _ = self.session.accept_open_team_sheets(SideId::P2);
            self.recorded.push(Recorded::Sheets(SideId::P2));
        }
        Ok(self.project(host, observer))
    }

    pub fn act(
        &mut self,
        host: &Host,
        observer: &str,
        actions: Vec<ActionIn>,
    ) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        let Observer::Side(side) = observer else {
            return Err("观战不能行动".into());
        };
        let choice = self.build_turn(side, &actions)?;
        self.session
            .engine()
            .ok_or("对战还没开始")?
            .validate_choice(side, &choice)
            .map_err(|error| format!("{error:?}"))?;
        self.pending_turn[side.index()] = Some(choice);
        self.pump(host)?;
        Ok(self.project(host, observer))
    }

    pub fn force(
        &mut self,
        host: &Host,
        observer: &str,
        replacements: Vec<ReplacementIn>,
    ) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        let Observer::Side(side) = observer else {
            return Err("观战不能替换".into());
        };
        let choice = self.build_forced(side, &replacements)?;
        self.pending_forced[side.index()] = Some(choice);
        self.pump(host)?;
        Ok(self.project(host, observer))
    }

    pub fn revive(&mut self, host: &Host, observer: &str, pokemon: u8) -> Result<Value, String> {
        let observer = parse_observer(self.mode, observer)?;
        let Observer::Side(side) = observer else {
            return Err("观战不能复活".into());
        };
        let BattleRequest::Revive {
            side: expected,
            candidates,
        } = self.last_request.clone().ok_or("现在不用复活")?
        else {
            return Err("现在不用复活".into());
        };
        if expected != side {
            return Err("还没轮到这一方复活".into());
        }
        let pokemon = PokemonId(pokemon);
        if !candidates.contains(&pokemon) {
            return Err("不能复活这只宝可梦".into());
        }
        self.resolve_revive(host, ReviveChoice { side, pokemon })?;
        self.pump(host)?;
        Ok(self.project(host, observer))
    }

    pub fn restart(&mut self, host: &Host, observer: &str) -> Result<Value, String> {
        let observer_name = observer.to_string();
        let origin = self.origin.clone();
        *self = Self::register(host, origin)?;
        if self.mode == Mode::Ai {
            self.choose_side(host, SideId::P2, Vec::new())?;
        }
        self.state(host, &observer_name)
    }

    pub fn replay(&self, host: &Host, observer: &str) -> Result<Vec<Value>, String> {
        let observer = parse_observer(self.mode, observer)?;
        let mut ghost = Self::register(host, self.origin.clone())?;
        let mut frames = vec![ghost.project(host, observer)];
        for step in &self.recorded {
            ghost.apply(host, step.clone())?;
            frames.push(ghost.project(host, observer));
        }
        Ok(frames)
    }

    fn choose_side(&mut self, host: &Host, side: SideId, order: Vec<u8>) -> Result<(), String> {
        if self.session.phase() != SessionPhase::Choosing {
            return Err("现在不是队伍预览".into());
        }
        let choice = TeamChoice {
            order: order.iter().copied().map(TeamSlot).collect(),
        };
        self.session
            .choose(side, choice)
            .map_err(|error| format!("{error:?}"))?;
        self.recorded.push(Recorded::Choose(side, order));
        if self.session.phase() == SessionPhase::InBattle {
            self.settle(host)?;
        }
        Ok(())
    }

    fn apply(&mut self, host: &Host, step: Recorded) -> Result<(), String> {
        match step {
            Recorded::Choose(side, order) => {
                let choice = TeamChoice {
                    order: order.into_iter().map(TeamSlot).collect(),
                };
                self.session
                    .choose(side, choice)
                    .map_err(|error| format!("{error:?}"))?;
                if self.session.phase() == SessionPhase::InBattle {
                    self.settle(host)?;
                }
            }
            Recorded::Sheets(side) => {
                let _ = self.session.accept_open_team_sheets(side);
            }
            Recorded::Turn(choices) => {
                self.session
                    .engine_mut()
                    .ok_or("对战还没开始")?
                    .submit_turn_choices(choices)
                    .map_err(|error| format!("{error:?}"))?;
                self.settle(host)?;
            }
            Recorded::Forced(choices) => {
                self.session
                    .engine_mut()
                    .ok_or("对战还没开始")?
                    .resolve_forced_switches(choices)
                    .map_err(|error| format!("{error:?}"))?;
                self.consume(host);
                self.settle(host)?;
            }
            Recorded::Revive(choice) => self.resolve_revive(host, choice)?,
        }
        Ok(())
    }

    fn pump(&mut self, host: &Host) -> Result<(), String> {
        if self.session.phase() != SessionPhase::InBattle {
            return Ok(());
        }
        loop {
            if self.outcome.is_some() {
                return Ok(());
            }
            if self.answer_waiting(host)? {
                continue;
            }
            let step = {
                let engine = self.session.engine_mut().ok_or("对战还没开始")?;
                engine.step().map_err(|error| format!("{error:?}"))?
            };
            self.consume(host);
            match step {
                EngineStep::Progressed { .. } => continue,
                EngineStep::Finished(outcome) => {
                    self.outcome = Some(outcome);
                    self.last_request = None;
                    return Ok(());
                }
                EngineStep::Awaiting(request) => {
                    self.last_request = Some(request);
                    if self.answer_waiting(host)? {
                        continue;
                    }
                    return Ok(());
                }
            }
        }
    }

    fn settle(&mut self, host: &Host) -> Result<(), String> {
        self.pump(host)
    }

    fn answer_waiting(&mut self, host: &Host) -> Result<bool, String> {
        let Some(engine_phase) = self
            .session
            .engine()
            .map(|engine| engine.view(Observer::Spectator).phase())
        else {
            return Ok(false);
        };
        let request = match engine_phase {
            pokemon_battle_engine::BattlePhase::TurnChoice => BattleRequest::TurnChoices {
                sides: [SideId::P1, SideId::P2],
            },
            pokemon_battle_engine::BattlePhase::ForcedSwitch => {
                let requirements = self
                    .session
                    .engine()
                    .ok_or("对战还没开始")?
                    .replacement_requirements();
                BattleRequest::ForcedSwitch {
                    replacements: requirements,
                }
            }
            pokemon_battle_engine::BattlePhase::Revive => {
                self.last_request.clone().ok_or("现在不用复活")?
            }
            _ => return Ok(false),
        };
        self.last_request = Some(request.clone());
        match request {
            BattleRequest::TurnChoices { .. } => {
                self.fill_ai_turns()?;
                if self.pending_turn.iter().all(Option::is_some) {
                    let choices = [
                        self.pending_turn[0].take().expect("p1"),
                        self.pending_turn[1].take().expect("p2"),
                    ];
                    self.recorded.push(Recorded::Turn(choices.clone()));
                    self.session
                        .engine_mut()
                        .ok_or("对战还没开始")?
                        .submit_turn_choices(choices)
                        .map_err(|error| format!("{error:?}"))?;
                    self.last_request = None;
                    let _ = host;
                    return Ok(true);
                }
            }
            BattleRequest::ForcedSwitch { replacements } => {
                self.fill_forced(&replacements)?;
                if self.pending_forced.iter().all(Option::is_some) {
                    let choices = [
                        self.pending_forced[0].take().expect("p1"),
                        self.pending_forced[1].take().expect("p2"),
                    ];
                    self.recorded.push(Recorded::Forced(choices.clone()));
                    self.session
                        .engine_mut()
                        .ok_or("对战还没开始")?
                        .resolve_forced_switches(choices)
                        .map_err(|error| format!("{error:?}"))?;
                    self.pending_forced = [None, None];
                    self.last_request = None;
                    self.consume(host);
                    return Ok(true);
                }
            }
            BattleRequest::Revive { side, candidates } => {
                if self.is_ai(side) {
                    if candidates.is_empty() {
                        return Err("没有可以复活的宝可梦".into());
                    }
                    let mut policy = self.policy.clone();
                    let choice = policy
                        .revive(&BattleRequest::Revive { side, candidates })
                        .expect("revive");
                    self.policy = policy;
                    self.resolve_revive(host, choice)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn resolve_revive(&mut self, host: &Host, choice: ReviveChoice) -> Result<(), String> {
        self.recorded.push(Recorded::Revive(choice));
        self.session
            .engine_mut()
            .ok_or("对战还没开始")?
            .resolve_revive(choice)
            .map_err(|error| format!("{error:?}"))?;
        self.last_request = None;
        self.consume(host);
        Ok(())
    }

    fn fill_ai_turns(&mut self) -> Result<(), String> {
        for side in [SideId::P1, SideId::P2] {
            if self.is_ai(side) && self.pending_turn[side.index()].is_none() {
                let mut policy = self.policy.clone();
                let choice = {
                    let engine = self.session.engine().ok_or("对战还没开始")?;
                    policy.turn(engine, side)
                };
                self.policy = policy;
                self.pending_turn[side.index()] = Some(choice);
            }
        }
        Ok(())
    }

    fn fill_forced(
        &mut self,
        replacements: &[pokemon_battle_engine::ReplacementRequirement; 2],
    ) -> Result<(), String> {
        for requirement in replacements {
            let index = requirement.side.index();
            if self.pending_forced[index].is_some() {
                continue;
            }
            if requirement.required_count == 0 {
                self.pending_forced[index] = Some(ForcedSwitchChoice {
                    side: requirement.side,
                    replacements: Vec::new(),
                });
            } else if self.is_ai(requirement.side) {
                if requirement.available_pokemon.is_empty() {
                    return Err("电脑没有可换上的宝可梦".into());
                }
                let mut policy = self.policy.clone();
                let mut available = requirement.available_pokemon.clone();
                let picked = requirement
                    .positions
                    .iter()
                    .take(requirement.required_count)
                    .map(|slot| {
                        let at = policy.below(available.len());
                        ForcedReplacement {
                            position: slot.position,
                            pokemon: available.remove(at),
                        }
                    })
                    .collect();
                self.policy = policy;
                self.pending_forced[index] = Some(ForcedSwitchChoice {
                    side: requirement.side,
                    replacements: picked,
                });
            }
        }
        Ok(())
    }

    fn is_ai(&self, side: SideId) -> bool {
        self.mode == Mode::Ai && side == SideId::P2
    }

    fn consume(&mut self, host: &Host) {
        let events = {
            let Some(engine) = self.session.engine_mut() else {
                return;
            };
            engine.take_trace_events()
        };
        let Some(engine) = self.session.engine() else {
            return;
        };
        for event in &events {
            for (index, observer) in [
                Observer::Side(SideId::P1),
                Observer::Side(SideId::P2),
                Observer::Spectator,
            ]
            .into_iter()
            .enumerate()
            {
                for note in log::narrate(engine, observer, event, host) {
                    self.counters[index] += 1;
                    let seq = self.counters[index];
                    if let Some(text) = note.text {
                        self.logs[index].push(LogLine {
                            seq,
                            text,
                            tone: note.tone,
                        });
                    }
                    if let Some((pokemon, clip)) = note.cue {
                        self.cues[index].push(CueLine { seq, pokemon, clip });
                    }
                }
            }
        }
    }

    fn build_turn(&self, side: SideId, actions: &[ActionIn]) -> Result<SideChoice, String> {
        let engine = self.session.engine().ok_or("对战还没开始")?;
        let options = engine.choice_options(side).ok_or("现在不能选择招式")?;
        if actions.len() != options.actors.len() {
            return Err("行动数量和场上的宝可梦不一致".into());
        }
        let mut slots = Vec::new();
        for action in actions {
            let actor = options
                .actors
                .iter()
                .find(|actor| actor.actor.0 == action.actor)
                .ok_or("场上没有这只宝可梦")?;
            let choice = if action.kind == "locked" {
                actor.locked.clone().ok_or("这只宝可梦没有被锁定的行动")?
            } else if let Some(locked) = &actor.locked {
                locked.clone()
            } else {
                match action.kind.as_str() {
                    "pass" => ActionChoice::Pass,
                    "switch" => {
                        let pokemon = PokemonId(action.pokemon.ok_or("没有指定替换宝可梦")?);
                        if !actor.switches.contains(&pokemon) {
                            return Err("现在不能换上这只宝可梦".into());
                        }
                        ActionChoice::Switch { pokemon }
                    }
                    "move" => {
                        let slot = MoveSlot::new(action.slot.ok_or("没有选择招式")?)
                            .ok_or("招式栏无效")?;
                        let option = actor
                            .moves
                            .iter()
                            .find(|option| option.slot == slot)
                            .ok_or("没有这招")?;
                        if option.refused.is_some() {
                            return Err("这招现在不能使用".into());
                        }
                        ActionChoice::Move {
                            slot,
                            target: pick_target(option.targets.clone(), action.target)?,
                            mechanic: mechanic(action.mechanic.as_deref())?,
                        }
                    }
                    _ => return Err("未知行动".into()),
                }
            };
            slots.push(SlotChoice {
                actor: actor.actor,
                action: choice,
            });
        }
        Ok(SideChoice {
            side,
            actions: slots,
        })
    }

    fn build_forced(
        &self,
        side: SideId,
        replacements: &[ReplacementIn],
    ) -> Result<ForcedSwitchChoice, String> {
        let BattleRequest::ForcedSwitch {
            replacements: requirements,
        } = self.last_request.clone().ok_or("现在不用替换")?
        else {
            return Err("现在不用替换".into());
        };
        let requirement = requirements
            .iter()
            .find(|requirement| requirement.side == side)
            .ok_or("这一方不用替换")?;
        if replacements.len() != requirement.required_count {
            return Err("替换数量不对".into());
        }
        let mut picked = Vec::new();
        for replacement in replacements {
            let position = requirement
                .positions
                .iter()
                .find(|slot| slot.position.index() == replacement.position)
                .map(|slot| slot.position)
                .ok_or("没有这个位置")?;
            let pokemon = PokemonId(replacement.pokemon);
            if !requirement.available_pokemon.contains(&pokemon) {
                return Err("不能换上这只宝可梦".into());
            }
            picked.push(ForcedReplacement { position, pokemon });
        }
        Ok(ForcedSwitchChoice {
            side,
            replacements: picked,
        })
    }

    fn project(&self, host: &Host, observer: Observer) -> Value {
        let index = match observer {
            Observer::Side(SideId::P1) => 0,
            Observer::Side(SideId::P2) => 1,
            Observer::Spectator => 2,
        };
        let your_side = match observer {
            Observer::Side(side) => Some(side_name(side)),
            Observer::Spectator => None,
        };
        let session_view = match observer {
            Observer::Side(side) => Some(self.session.view(side)),
            Observer::Spectator => None,
        };
        let (battle, request) = if let Some(engine) = self.session.engine() {
            (
                Some(project_battle(engine, observer, host)),
                Some(self.project_request(engine, observer, host)),
            )
        } else {
            (None, None)
        };
        let your_turn = your_side.is_some() && self.waiting_for(observer);
        json!({
            "format": self.origin.format,
            "seed": self.origin.seed.to_string(),
            "mode": match self.mode { Mode::Ai => "ai", Mode::Hotseat => "hotseat" },
            "phase": phase_name(self.session.phase(), self.outcome),
            "observer": match observer {
                Observer::Side(side) => side_name(side),
                Observer::Spectator => "spectator",
            },
            "yourSide": your_side,
            "yourTurn": your_turn,
            "finished": self.outcome.map(|outcome| match outcome {
                BattleOutcome::Winner(side) => json!({ "winner": side_name(side) }),
                BattleOutcome::Draw => json!({ "winner": "draw" }),
            }),
            "preview": session_view.as_ref().map(|view| json!({
                "yourOrder": view.own_choice.as_ref().map(|order| order.iter().map(|slot| slot.0).collect::<Vec<_>>()),
                "opponent": view.opponent_preview.as_ref().map(|entries| entries.iter().map(|entry| json!({
                    "details": entry.details,
                    "item": entry.item,
                    "teraType": entry.tera_type,
                })).collect::<Vec<_>>()),
                "sheet": view.opponent_sheet.as_ref().map(|sets| sets.iter().cloned().map(SetInput::from).collect::<Vec<_>>()),
            })),
            "battle": battle,
            "request": request,
            "log": self.logs[index].iter().map(|line| json!({
                "seq": line.seq,
                "text": line.text,
                "tone": line.tone,
            })).collect::<Vec<_>>(),
            "cues": self.cues[index].iter().map(|cue| json!({
                "seq": cue.seq,
                "pokemon": cue.pokemon,
                "clip": cue.clip,
            })).collect::<Vec<_>>(),
        })
    }

    fn waiting_for(&self, observer: Observer) -> bool {
        let Observer::Side(side) = observer else {
            return false;
        };
        if self.outcome.is_some() || self.is_ai(side) {
            return false;
        }
        match self.session.phase() {
            SessionPhase::Choosing => self.session.request().let_choose_pending(side),
            SessionPhase::InBattle => match &self.last_request {
                Some(BattleRequest::TurnChoices { .. }) => {
                    self.pending_turn[side.index()].is_none()
                }
                Some(BattleRequest::ForcedSwitch { replacements }) => {
                    replacements.iter().any(|requirement| {
                        requirement.side == side
                            && requirement.required_count > 0
                            && self.pending_forced[side.index()].is_none()
                    })
                }
                Some(BattleRequest::Revive { side: reviving, .. }) => *reviving == side,
                _ => false,
            },
            SessionPhase::Registering => false,
        }
    }

    fn project_request(
        &self,
        engine: &pokemon_battle_engine::BattleEngine,
        observer: Observer,
        host: &Host,
    ) -> Option<Value> {
        let Observer::Side(side) = observer else {
            return None;
        };
        if !self.waiting_for(observer) {
            return None;
        }
        match &self.last_request {
            Some(BattleRequest::TurnChoices { .. }) => {
                let options = engine.choice_options(side)?;
                Some(json!({
                    "kind": "turn",
                    "actors": options.actors.iter().map(|actor| {
                        json!({
                            "actor": actor.actor.0,
                            "position": actor.position.index(),
                            "trapped": format!("{:?}", actor.trapped).to_lowercase(),
                            "locked": actor.locked.is_some(),
                            "tera": actor.mechanics.tera.map(|kind| type_zh(kind, host)),
                            "teraEn": actor.mechanics.tera.map(|kind| format!("{kind:?}")),
                            "mega": actor.mechanics.mega.is_some(),
                            "ultra": actor.mechanics.ultra_burst.is_some(),
                            "dynamax": actor.mechanics.dynamax,
                            "moves": actor.moves.iter().map(|option| {
                                let names = engine.names();
                                let english = names.move_(option.move_id).unwrap_or("招式").to_string();
                                json!({
                                    "slot": option.slot.index(),
                                    "name": host.zh_move(&english),
                                    "nameEn": english,
                                    "pp": option.pp,
                                    "maxPp": option.max_pp,
                                    "disabled": option.refused.is_some(),
                                    "targets": option.targets.iter().flatten().map(|target| target_json(engine, observer, *target)).collect::<Vec<_>>(),
                                })
                            }).collect::<Vec<_>>(),
                            "switches": actor.switches.iter().map(|pokemon| {
                                let names = engine.names();
                                let english = engine.view(observer).side(side).team().find(|member| member.id() == *pokemon)
                                    .and_then(|member| member.species())
                                    .and_then(|id| names.species(id))
                                    .unwrap_or("宝可梦")
                                    .to_string();
                                json!({
                                    "pokemon": pokemon.0,
                                    "name": host.zh_species(&to_id(&english), &english),
                                    "nameEn": english,
                                })
                            }).collect::<Vec<_>>(),
                        })
                    }).collect::<Vec<_>>(),
                }))
            }
            Some(BattleRequest::ForcedSwitch { replacements }) => {
                let requirement = replacements
                    .iter()
                    .find(|requirement| requirement.side == side)?;
                Some(json!({
                    "kind": "forced",
                    "required": requirement.required_count,
                    "positions": requirement.positions.iter().map(|slot| slot.position.index()).collect::<Vec<_>>(),
                    "pokemon": requirement.available_pokemon.iter().map(|pokemon| bench_name(engine, observer, side, *pokemon, host)).collect::<Vec<_>>(),
                }))
            }
            Some(BattleRequest::Revive {
                side: reviving,
                candidates,
            }) if *reviving == side => Some(json!({
                "kind": "revive",
                "pokemon": candidates.iter().map(|pokemon| bench_name(engine, observer, side, *pokemon, host)).collect::<Vec<_>>(),
            })),
            _ => None,
        }
    }
}

trait ChoosePending {
    fn let_choose_pending(&self, side: SideId) -> bool;
}

impl ChoosePending for SessionRequest {
    fn let_choose_pending(&self, side: SideId) -> bool {
        match self {
            Self::Choose { sides, .. } => sides.contains(&side),
            _ => false,
        }
    }
}

fn into_sets(sets: Vec<SetInput>) -> Vec<TeamSet> {
    sets.into_iter().map(TeamSet::from).collect()
}

fn team_error(error: &pokemon_battle_engine::SessionError) -> String {
    if let Some(problems) = session_team_error(error) {
        return problems
            .into_iter()
            .map(|problem| problem.message)
            .collect::<Vec<_>>()
            .join("\n");
    }
    format!("{error:?}")
}

fn parse_observer(mode: Mode, text: &str) -> Result<Observer, String> {
    let observer = match text {
        "p1" => Observer::Side(SideId::P1),
        "p2" => Observer::Side(SideId::P2),
        "spectator" => Observer::Spectator,
        _ => return Err("观察者应为 p1、p2 或 spectator".into()),
    };
    if mode == Mode::Ai && matches!(observer, Observer::Side(SideId::P2)) {
        return Err("人机对战不会交出对手的私有视角".into());
    }
    Ok(observer)
}

fn side_name(side: SideId) -> &'static str {
    match side {
        SideId::P1 => "p1",
        SideId::P2 => "p2",
    }
}

fn phase_name(phase: SessionPhase, outcome: Option<BattleOutcome>) -> &'static str {
    if outcome.is_some() {
        return "finished";
    }
    match phase {
        SessionPhase::Registering => "register",
        SessionPhase::Choosing => "preview",
        SessionPhase::InBattle => "battle",
    }
}

fn mechanic(name: Option<&str>) -> Result<Option<MechanicChoice>, String> {
    Ok(match name {
        None | Some("") => None,
        Some("tera") => Some(MechanicChoice::Terastallize),
        Some("mega") => Some(MechanicChoice::MegaEvolve),
        Some("ultra") => Some(MechanicChoice::UltraBurst),
        Some("z") => Some(MechanicChoice::ZMove),
        Some("dynamax") => Some(MechanicChoice::Dynamax),
        Some(other) => return Err(format!("未知的特殊机制 {other}")),
    })
}

fn pick_target(
    targets: Vec<Option<DeclaredTarget>>,
    wanted: Option<usize>,
) -> Result<Option<DeclaredTarget>, String> {
    let concrete: Vec<DeclaredTarget> = targets.iter().copied().flatten().collect();
    if let Some(index) = wanted {
        return concrete
            .into_iter()
            .find(|target| target.0.index() == index)
            .map(Some)
            .ok_or_else(|| "这个目标不合法".into());
    }
    if concrete.len() <= 1 {
        return Ok(concrete.first().copied());
    }
    Err("请选择目标".into())
}

fn target_json(
    engine: &pokemon_battle_engine::BattleEngine,
    observer: Observer,
    target: DeclaredTarget,
) -> Value {
    let mut label = format!("位置 {}", target.0.index());
    let mut pokemon = None;
    let view = engine.view(observer);
    for side in [SideId::P1, SideId::P2] {
        for (position, member) in view.side(side).active() {
            if position == target.0 {
                if let Some(member) = member {
                    pokemon = Some(member.id().0);
                    let names = engine.names();
                    if let Some(species) = member.species().and_then(|id| names.species(id)) {
                        label = format!("{} {species}", side_name(side));
                    }
                }
            }
        }
    }
    json!({ "position": target.0.index(), "pokemon": pokemon, "label": label })
}

fn bench_name(
    engine: &pokemon_battle_engine::BattleEngine,
    observer: Observer,
    side: SideId,
    pokemon: PokemonId,
    host: &Host,
) -> Value {
    let names = engine.names();
    let english = engine
        .view(observer)
        .side(side)
        .team()
        .find(|member| member.id() == pokemon)
        .and_then(|member| member.species())
        .and_then(|species| names.species(species))
        .unwrap_or("宝可梦")
        .to_string();
    json!({
        "pokemon": pokemon.0,
        "name": host.zh_species(&to_id(&english), &english),
        "nameEn": english,
    })
}

fn project_battle(
    engine: &pokemon_battle_engine::BattleEngine,
    observer: Observer,
    host: &Host,
) -> Value {
    let view = engine.view(observer);
    let names = engine.names();
    let mut sides = json!({});
    for side in [SideId::P1, SideId::P2] {
        let side_view = view.side(side);
        let active: Vec<Option<u8>> = side_view
            .active()
            .map(|(_, member)| member.map(|member| member.id().0))
            .collect();
        let pokemon: Vec<Value> = side_view
            .team()
            .map(|member| {
                let english = member
                    .species()
                    .and_then(|species| names.species(species))
                    .unwrap_or("?");
                let species_id = to_id(english);
                let (dex, form) = member.species().map(|species| (species.dex, species.form)).unwrap_or((0, 0));
                json!({
                    "id": member.id().0,
                    "speciesId": species_id,
                    "name": host.zh_species(&species_id, english),
                    "nameEn": english,
                    "dex": dex,
                    "form": form,
                    "level": member.level(),
                    "gender": gender_name(member.gender()),
                    "hp": hp_json(member.hp()),
                    "fainted": member.fainted(),
                    "status": member.status().map(status_name),
                    "boosts": {
                        "atk": member.boost(pokemon_battle_engine::Stat::Attack),
                        "def": member.boost(pokemon_battle_engine::Stat::Defense),
                        "spa": member.boost(pokemon_battle_engine::Stat::SpecialAttack),
                        "spd": member.boost(pokemon_battle_engine::Stat::SpecialDefense),
                        "spe": member.boost(pokemon_battle_engine::Stat::Speed),
                    },
                    "item": seen_json(member.item(), |item| host.zh_item(names.item(item).unwrap_or("?"))),
                    "ability": seen_json(member.ability(), |ability| host.zh_ability(names.ability(ability).unwrap_or("?"))),
                    "moves": member.moves().revealed().map(|known| {
                        let english = names.move_(known.move_id()).unwrap_or("?");
                        let (pp, max_pp, pp_known) = match known.pp() {
                            Seen::Known((pp, max_pp)) => (Some(pp), Some(max_pp), true),
                            _ => (None, None, false),
                        };
                        json!({
                            "name": host.zh_move(english),
                            "nameEn": english,
                            "pp": pp,
                            "maxPp": max_pp,
                            "ppKnown": pp_known,
                        })
                    }).collect::<Vec<_>>(),
                    "volatiles": member.volatiles().iter().filter_map(log::volatile_label).collect::<Vec<_>>(),
                    "terastallized": member.terastallized().map(|kind| type_zh(kind, host)),
                    "active": active.contains(&Some(member.id().0)),
                })
            })
            .collect();
        sides[side_name(side)] = json!({
            "conditions": side_view.conditions().map(|kind| format!("{kind:?}")).collect::<Vec<_>>(),
            "pokemonLeft": side_view.pokemon_left(),
            "active": active,
            "pokemon": pokemon,
        });
    }
    let field = view.field();
    json!({
        "turn": view.turn(),
        "generation": view.generation().number(),
        "weather": field.weather().map(weather_zh),
        "terrain": field.terrain().map(terrain_zh),
        "conditions": field.conditions().map(|kind| format!("{kind:?}")).collect::<Vec<_>>(),
        "sides": sides,
    })
}

fn gender_name(gender: pokemon_battle_engine::Gender) -> &'static str {
    match gender {
        pokemon_battle_engine::Gender::Male => "M",
        pokemon_battle_engine::Gender::Female => "F",
        pokemon_battle_engine::Gender::Genderless => "N",
    }
}

fn hp_json(hp: Hp) -> Value {
    match hp {
        Hp::Exact { hp, max } => {
            let percent = if max == 0 {
                0
            } else {
                ((u32::from(hp) * 100) / u32::from(max)) as u8
            };
            json!({ "kind": "exact", "current": hp, "max": max, "percent": percent })
        }
        Hp::Percent(percent) => {
            json!({ "kind": "percent", "current": null, "max": null, "percent": percent })
        }
        Hp::Pixels(pixels) => json!({
            "kind": "pixels",
            "current": null,
            "max": null,
            "percent": ((u32::from(pixels) * 100) / 48).min(100) as u8,
        }),
    }
}

fn seen_json<T>(seen: Seen<T>, name: impl FnOnce(T) -> String) -> Value {
    match seen {
        Seen::Known(value) => json!({ "known": true, "value": name(value) }),
        Seen::KnownNone => json!({ "known": true, "value": null }),
        Seen::Unknown => json!({ "known": false, "value": null }),
    }
}
