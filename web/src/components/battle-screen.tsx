import { useEffect, useMemo, useState } from "react"
import { toast } from "sonner"

import { BattleStage } from "@/components/battle-stage"
import { api } from "@/lib/api"
import type { FormEntry, StageManifest } from "@/lib/assets"
import type { ActorOption, MoveOption, PublicState, SetInput, SideView } from "@/lib/types"
import { hpBand } from "@/lib/type-colors"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"

type Draft = { kind: string; slot?: number; target?: number; mechanic?: string; pokemon?: number }

export function BattleScreen({
  format,
  team,
  active,
}: {
  format: string
  team: SetInput[]
  active: boolean
}) {
  const [stage, setStage] = useState<StageManifest | null>(null)
  const [forms, setForms] = useState<FormEntry[]>([])
  const [battleId, setBattleId] = useState<string | null>(null)
  const [state, setState] = useState<PublicState | null>(null)
  const [observer, setObserver] = useState<"p1" | "p2">("p1")
  const [mode, setMode] = useState<"ai" | "hotseat">("ai")
  const [busy, setBusy] = useState(false)
  const [order, setOrder] = useState<number[]>([0, 1, 2, 3, 4, 5])
  const [drafts, setDrafts] = useState<Record<number, Draft>>({})
  const [pendingMove, setPendingMove] = useState<{ actor: ActorOption; move: MoveOption } | null>(null)
  const [tera, setTera] = useState(false)

  useEffect(() => {
    void fetch("/assets/stage.json").then((response) => response.json()).then(setStage)
    void fetch("/assets/forms.index.json").then((response) => response.json()).then((body: { pokemon: FormEntry[] }) => setForms(body.pokemon))
  }, [])

  useEffect(() => {
    if (!battleId) return
    void api.state(battleId, observer).then(setState).catch((error: Error) => toast.error(error.message))
  }, [battleId, observer])

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (!state?.yourTurn || state.request?.kind !== "turn") return
      const target = event.target as HTMLElement | null
      if (target && ["INPUT", "TEXTAREA"].includes(target.tagName)) return
      const actor = state.request.actors?.[0]
      if (!actor) return
      if (event.key >= "1" && event.key <= "4") {
        const move = actor.moves[Number(event.key) - 1]
        if (move && !move.disabled) pickMove(actor, move)
      }
      if (event.key === "t" && actor.tera) setTera((value) => !value)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  })

  const fighters = useMemo(() => {
    if (!state?.battle) return []
    return (["p1", "p2"] as const).flatMap((side) => {
      const view = state.battle?.sides[side]
      if (!view) return []
      return view.active.map((id, slot) => {
        const mon = view.pokemon.find((entry) => entry.id === id)
        if (!mon || id === null) return null
        const cue = [...(state.cues ?? [])].reverse().find((entry) => entry.pokemon === mon.id)
        return {
          key: `${side}-${mon.id}-${mon.dex}-${mon.form}`,
          side,
          slot,
          dex: mon.dex,
          form: mon.form,
          name: mon.name,
          fainted: mon.fainted,
          cueSeq: cue?.seq ?? -1,
          cueClip: cue?.clip ?? null,
        }
      }).filter((entry) => entry !== null)
    })
  }, [state])

  async function start(kind: "team" | "ou" | "showcase") {
    setBusy(true)
    try {
      let battleFormat = format
      let p1 = team.filter((set) => set.species)
      let p2 = p1
      if (kind === "ou") {
        battleFormat = "gen9ou"
        const presets = await api.presets("gen9ou")
        p1 = presets.presets.find((preset) => preset.id === "sample")?.team ?? p1
        p2 = presets.presets.find((preset) => preset.id === "mirror")?.team ?? p1
      } else if (kind === "showcase") {
        battleFormat = "gen9ubers"
        const presets = await api.presets("gen9ubers")
        p1 = presets.presets.find((preset) => preset.id === "pikachu")?.team ?? p1
        p2 = presets.presets.find((preset) => preset.id === "charizard")?.team ?? p1
      } else {
        const presets = await api.presets(format)
        p2 = presets.presets.find((preset) => preset.id === "mirror")?.team ?? presets.presets[0]?.team ?? p1
      }
      const created = await api.createBattle({ format: battleFormat, mode, p1, p2, seed: 20261108 })
      setBattleId(created.id)
      setObserver("p1")
      setState(created.state)
      setOrder([0, 1, 2, 3, 4, 5])
      toast.success(mode === "ai" ? "本地对战已开始，对手由引擎的合法选择驱动" : "热座对战已开始")
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "无法开始")
    } finally {
      setBusy(false)
    }
  }

  async function run(action: () => Promise<PublicState>) {
    setBusy(true)
    try {
      setState(await action())
      setDrafts({})
      setPendingMove(null)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "行动被拒绝")
    } finally {
      setBusy(false)
    }
  }

  function pickMove(actor: ActorOption, move: MoveOption) {
    if (move.targets.length > 1) {
      setPendingMove({ actor, move })
      return
    }
    const action = { actor: actor.actor, kind: actor.locked ? "locked" : "move", slot: move.slot, mechanic: tera ? "tera" : undefined }
    const actors = state?.request?.actors ?? []
    if (actors.length === 1) {
      if (!battleId || !state) return
      void run(() => api.act(battleId, state.observer, [action]))
      return
    }
    setDrafts((current) => ({ ...current, [actor.actor]: action }))
  }

  const you = state?.yourSide === "p2" ? state.battle?.sides.p2 : state?.battle?.sides.p1
  const foe = state?.yourSide === "p2" ? state.battle?.sides.p1 : state?.battle?.sides.p2

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1.3fr)_minmax(320px,0.7fr)]">
      <div className="grid gap-3">
        {stage ? <BattleStage manifest={stage} forms={forms} fighters={fighters} /> : <Skeleton className="h-[320px] w-full" />}
        <Card>
          <CardHeader>
            <CardTitle>开始</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-2">
            <Tabs value={mode} onValueChange={(value) => setMode(value as "ai" | "hotseat")}>
              <TabsList>
                <TabsTrigger value="ai">对人机</TabsTrigger>
                <TabsTrigger value="hotseat">热座</TabsTrigger>
              </TabsList>
            </Tabs>
            <Button disabled={busy || !active} onClick={() => void start("team")}>当前队伍</Button>
            <Button variant="outline" disabled={busy} onClick={() => void start("ou")}>第九世代 OU 样本</Button>
            <Button variant="outline" disabled={busy} onClick={() => void start("showcase")}>皮卡丘对喷火龙</Button>
            {battleId && state ? (
              <Button variant="secondary" disabled={busy} onClick={() => void run(() => api.restart(battleId, observer))}>重新开始</Button>
            ) : null}
            {state?.mode === "hotseat" && battleId ? (
              <Button variant="ghost" onClick={() => setObserver((value) => (value === "p1" ? "p2" : "p1"))}>
                切换到{observer === "p1" ? "二" : "一"}号视角
              </Button>
            ) : null}
          </CardContent>
        </Card>
        {state?.phase === "preview" ? (
          <Card>
            <CardHeader>
              <CardTitle>队伍预览</CardTitle>
            </CardHeader>
            <CardContent className="grid gap-3">
              <p className="text-sm text-muted-foreground">只能看到对方公开的预览，看不到对方的出战顺序。</p>
              <div className="flex flex-wrap gap-2">
                {(state.preview?.opponent ?? []).map((entry) => (
                  <Badge key={entry.details} variant="outline">{entry.details}</Badge>
                ))}
              </div>
              <div className="flex flex-wrap gap-2">
                {order.map((slot, index) => (
                  <Button key={slot} variant="outline" onClick={() => setOrder(moveSlot(order, index, -1))}>
                    {index + 1}. {team[slot]?.species || state.preview?.yourOrder?.[slot] || `位置 ${slot + 1}`}
                  </Button>
                ))}
              </div>
              <Button disabled={!state.yourTurn || busy} onClick={() => battleId && void run(() => api.choose(battleId, state.observer, order))}>
                确认出战顺序
              </Button>
            </CardContent>
          </Card>
        ) : null}
      </div>
      <div className="grid gap-3">
        {!state ? (
          <Empty className="border">
            <EmptyHeader>
              <EmptyTitle>还没有对战</EmptyTitle>
              <EmptyDescription>这是本机对战。没有匹配、账号或天梯。</EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <>
            <Hud side={you} title={state.mode === "ai" ? "我方" : state.yourSide === "p2" ? "玩家二" : "玩家一"} />
            <Hud side={foe} title={state.mode === "ai" ? "对手" : "对方"} />
            {state.finished ? (
              <Alert>
                <AlertTitle>对战结束</AlertTitle>
                <AlertDescription>{winnerText(state)}</AlertDescription>
              </Alert>
            ) : null}
            {state.yourTurn && state.request?.kind === "turn" ? (
              <Card>
                <CardHeader>
                  <CardTitle>选择</CardTitle>
                </CardHeader>
                <CardContent className="grid gap-3">
                  <p className="text-sm text-muted-foreground">数字键 1–4 选择第一只的招式，T 切换太晶。</p>
                  {state.request.actors?.map((actor) => (
                    <div key={actor.actor} className="grid gap-2">
                      <div className="flex flex-wrap gap-2">
                        {actor.moves.map((move) => (
                          <Button key={move.slot} variant={move.disabled ? "ghost" : "outline"} disabled={move.disabled || busy} onClick={() => pickMove(actor, move)}>
                            {move.name} {move.pp}/{move.maxPp}
                          </Button>
                        ))}
                      </div>
                      {actor.tera ? (
                        <Button variant={tera ? "default" : "secondary"} onClick={() => setTera((value) => !value)}>
                          太晶{actor.tera}{tera ? "（已选）" : ""}
                        </Button>
                      ) : null}
                      {actor.trapped === "yes" ? <Badge variant="destructive">无法替换</Badge> : null}
                      <div className="flex flex-wrap gap-2">
                        {actor.switches.map((member) => (
                          <Button key={member.pokemon} variant="outline" disabled={busy} onClick={() => {
                            if (!battleId || !state) return
                            void run(() => api.act(battleId, state.observer, [{ actor: actor.actor, kind: "switch", pokemon: member.pokemon }]))
                          }}>
                            换上 {member.name}
                          </Button>
                        ))}
                      </div>
                    </div>
                  ))}
                  {pendingMove ? (
                    <div className="flex flex-wrap gap-2">
                      {pendingMove.move.targets.map((target) => (
                        <Button key={target.position} onClick={() => {
                          if (!battleId || !state) return
                          void run(() => api.act(battleId, state.observer, [{
                            actor: pendingMove.actor.actor,
                            kind: "move",
                            slot: pendingMove.move.slot,
                            target: target.position,
                            mechanic: tera ? "tera" : undefined,
                          }]))
                        }}>
                          目标 {target.label}
                        </Button>
                      ))}
                    </div>
                  ) : null}
                  {Object.keys(drafts).length > 0 && battleId ? (
                    <Button onClick={() => void run(() => api.act(battleId, state.observer, Object.entries(drafts).map(([actor, action]) => ({ actor: Number(actor), ...action }))))}>
                      确认本回合
                    </Button>
                  ) : null}
                </CardContent>
              </Card>
            ) : null}
            {state.yourTurn && state.request?.kind === "forced" ? (
              <Card>
                <CardHeader><CardTitle>换上宝可梦</CardTitle></CardHeader>
                <CardContent className="flex flex-wrap gap-2">
                  {state.request.pokemon?.map((member) => (
                    <Button key={member.pokemon} disabled={busy} onClick={() => {
                      if (!battleId) return
                      const position = state.request?.positions?.[0] ?? 0
                      void run(() => api.force(battleId, state.observer, [{ position, pokemon: member.pokemon }]))
                    }}>
                      {member.name}
                    </Button>
                  ))}
                </CardContent>
              </Card>
            ) : null}
            {state.yourTurn && state.request?.kind === "revive" ? (
              <Card>
                <CardHeader><CardTitle>选择复活</CardTitle></CardHeader>
                <CardContent className="flex flex-wrap gap-2">
                  {state.request.pokemon?.map((member) => (
                    <Button key={member.pokemon} disabled={busy} onClick={() => battleId && void run(() => api.revive(battleId, state.observer, member.pokemon))}>
                      {member.name}
                    </Button>
                  ))}
                </CardContent>
              </Card>
            ) : null}
            <Card>
              <CardHeader>
                <CardTitle>战斗记录</CardTitle>
              </CardHeader>
              <CardContent>
                <ScrollArea className="h-64">
                  <div className="grid gap-1 pr-3 text-sm">
                    {state.log.map((line) => (
                      <p key={line.seq}>{line.text}</p>
                    ))}
                  </div>
                </ScrollArea>
              </CardContent>
            </Card>
          </>
        )}
      </div>
    </div>
  )
}

function Hud({ side, title }: { side: SideView | undefined; title: string }) {
  if (!side) return null
  const active = side.pokemon.filter((mon) => mon.active)
  const bench = side.pokemon.filter((mon) => !mon.active)
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center justify-between">
          {title}
          <span className="text-sm font-normal text-muted-foreground">剩余 {side.pokemonLeft}</span>
        </CardTitle>
      </CardHeader>
      <CardContent className="grid gap-3">
        {side.conditions.length > 0 ? <p className="text-sm">{side.conditions.join(" · ")}</p> : null}
        {active.map((mon) => (
          <div key={mon.id} className="grid gap-1">
            <div className="flex flex-wrap items-center gap-2">
              <strong>{mon.name}</strong>
              <span className="text-sm text-muted-foreground">Lv.{mon.level}</span>
              {mon.status ? <Badge variant="destructive">{mon.status}</Badge> : null}
              {mon.terastallized ? <Badge>太晶 {mon.terastallized}</Badge> : null}
              {mon.item.known ? <Badge variant="outline">{mon.item.value ?? "无道具"}</Badge> : <Badge variant="outline">道具未知</Badge>}
              {mon.ability.known ? <Badge variant="outline">{mon.ability.value ?? "无特性"}</Badge> : <Badge variant="outline">特性未知</Badge>}
            </div>
            <div className="hp-bar" data-band={hpBand(mon.hp.percent)}>
              <Progress value={mon.hp.percent} aria-label={`${mon.name} 体力 ${mon.hp.kind === "exact" ? `${mon.hp.current}/${mon.hp.max}` : `${mon.hp.percent}%`}`} />
            </div>
            <p className="text-sm">
              {mon.hp.kind === "exact" ? `${mon.hp.current}/${mon.hp.max}` : mon.hp.kind === "pixels" ? `体力像素 ${mon.hp.percent}%` : `${mon.hp.percent}%`}
              {boostText(mon.boosts)}
            </p>
            <div className="flex flex-wrap gap-1">
              {mon.moves.map((move) => (
                <Badge key={move.name} variant="secondary">
                  {move.name}{move.ppKnown ? ` ${move.pp}/${move.maxPp}` : ""}
                </Badge>
              ))}
              {mon.volatiles.map((name) => (
                <Badge key={name} variant="outline">{name}</Badge>
              ))}
            </div>
          </div>
        ))}
        {bench.length > 0 ? <Separator /> : null}
        <div className="flex flex-wrap gap-1">
          {bench.map((mon) => (
            <Badge key={mon.id} variant={mon.fainted ? "destructive" : "outline"}>
              {mon.name}{mon.fainted ? " 濒死" : ""}
            </Badge>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

function boostText(boosts: { atk: number; def: number; spa: number; spd: number; spe: number }) {
  const parts = [
    ["攻", boosts.atk],
    ["防", boosts.def],
    ["特攻", boosts.spa],
    ["特防", boosts.spd],
    ["速", boosts.spe],
  ].filter(([, value]) => value !== 0).map(([label, value]) => `${label}${Number(value) > 0 ? "+" : ""}${value}`)
  return parts.length ? ` · ${parts.join(" ")}` : ""
}

function winnerText(state: PublicState) {
  if (state.finished?.winner === "draw") return "平局"
  if (state.mode === "ai") return state.finished?.winner === state.yourSide ? "你赢了" : "你输了"
  return state.finished?.winner === "p1" ? "玩家一获胜" : "玩家二获胜"
}

function moveSlot(order: number[], index: number, delta: number) {
  const next = order.slice()
  const target = index + delta
  if (target < 0 || target >= next.length) return next
  const swap = next[index]
  next[index] = next[target]
  next[target] = swap
  return next
}
