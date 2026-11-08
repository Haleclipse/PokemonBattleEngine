export interface StatBlock {
  hp: number
  atk: number
  def: number
  spa: number
  spd: number
  spe: number
}

export interface SetInput {
  name: string
  species: string
  item: string
  ability: string
  moves: string[]
  nature: string
  gender: string
  evs: StatBlock | null
  ivs: StatBlock | null
  level: number
  shiny: boolean
  happiness: number | null
  hpType: string
  gigantamax: boolean
  teraType: string
}

export interface ProblemOut {
  message: string
  set: number | null
  field: string | null
}

export interface TeamCheck {
  ok: boolean
  problems: ProblemOut[]
  team: SetInput[]
}

export interface FormatSummary {
  id: string
  name: string
  generation: number
  gameType: string
}

export interface FormatInfo extends FormatSummary {
  minTeam: number
  maxTeam: number
  picked: number | null
  preview: boolean
  leads: number
  evLimit: number | null
  maxMoves: number
  defaultLevel: number
  tera: boolean
  mega: boolean
  zMove: boolean
  dynamax: boolean
  openTeamSheets: string
}

export interface Seen {
  known: boolean
  value: string | null
}

export interface HpView {
  kind: "exact" | "percent" | "pixels"
  current: number | null
  max: number | null
  percent: number
}

export interface MonView {
  id: number
  speciesId: string
  name: string
  nameEn: string
  dex: number
  form: number
  level: number
  gender: string
  hp: HpView
  fainted: boolean
  status: string | null
  boosts: { atk: number; def: number; spa: number; spd: number; spe: number }
  item: Seen
  ability: Seen
  moves: Array<{ name: string; nameEn: string; pp: number | null; maxPp: number | null; ppKnown: boolean }>
  volatiles: string[]
  terastallized: string | null
  active: boolean
}

export interface SideView {
  conditions: string[]
  pokemonLeft: number
  active: Array<number | null>
  pokemon: MonView[]
}

export interface BattlePayload {
  turn: number
  generation: number
  weather: string | null
  terrain: string | null
  conditions: string[]
  sides: { p1: SideView; p2: SideView }
}

export interface MoveOption {
  slot: number
  name: string
  nameEn: string
  pp: number
  maxPp: number
  disabled: boolean
  targets: Array<{ position: number; pokemon: number | null; label: string }>
}

export interface ActorOption {
  actor: number
  position: number
  trapped: string
  locked: boolean
  tera: string | null
  teraEn: string | null
  mega: boolean
  ultra: boolean
  dynamax: boolean
  moves: MoveOption[]
  switches: Array<{ pokemon: number; name: string; nameEn: string }>
}

export interface BattleRequest {
  kind: "turn" | "forced" | "revive"
  actors?: ActorOption[]
  required?: number
  positions?: number[]
  pokemon?: Array<{ pokemon: number; name: string; nameEn: string }>
}

export interface LogLine {
  seq: number
  text: string
  tone: string
}

export interface Cue {
  seq: number
  pokemon: number
  clip: string
}

export interface PublicState {
  format: string
  seed: string
  mode: "ai" | "hotseat"
  phase: "register" | "preview" | "battle" | "finished"
  observer: "p1" | "p2" | "spectator"
  yourSide: "p1" | "p2" | null
  yourTurn: boolean
  finished: { winner: "p1" | "p2" | "draw" } | null
  preview: {
    yourOrder: number[] | null
    opponent: Array<{ details: string; item: string; teraType: string | null }> | null
    sheet: SetInput[] | null
  } | null
  battle: BattlePayload | null
  request: BattleRequest | null
  log: LogLine[]
  cues: Cue[]
}

export function emptyStats(): StatBlock {
  return { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0 }
}

export function emptySet(): SetInput {
  return {
    name: "",
    species: "",
    item: "",
    ability: "",
    moves: ["", "", "", ""],
    nature: "Serious",
    gender: "",
    evs: emptyStats(),
    ivs: null,
    level: 0,
    shiny: false,
    happiness: null,
    hpType: "",
    gigantamax: false,
    teraType: "",
  }
}

export function emptyTeam(): SetInput[] {
  return Array.from({ length: 6 }, emptySet)
}
