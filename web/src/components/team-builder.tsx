import { useEffect, useState } from "react"
import { toast } from "sonner"

import { api, type SpeciesDetail } from "@/lib/api"
import { saveTeam } from "@/lib/storage"
import { emptySet, type FormatInfo, type FormatSummary, type ProblemOut, type SetInput } from "@/lib/types"
import { typeStyle } from "@/lib/type-colors"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"
import { Field, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Textarea } from "@/components/ui/textarea"

const STATS = [
  ["hp", "HP"],
  ["atk", "攻击"],
  ["def", "防御"],
  ["spa", "特攻"],
  ["spd", "特防"],
  ["spe", "速度"],
] as const

export function TeamBuilder({
  formats,
  format,
  info,
  team,
  onFormat,
  onTeam,
  onUse,
}: {
  formats: FormatSummary[]
  format: string
  info: FormatInfo | null
  team: SetInput[]
  onFormat: (id: string) => void
  onTeam: (team: SetInput[]) => void
  onUse: () => void
}) {
  const [slot, setSlot] = useState(0)
  const [problems, setProblems] = useState<ProblemOut[]>([])
  const [speciesOpen, setSpeciesOpen] = useState(false)
  const [moveOpen, setMoveOpen] = useState<number | null>(null)
  const [pasteOpen, setPasteOpen] = useState(false)
  const [query, setQuery] = useState("")
  const [results, setResults] = useState<Array<{ id: string; name: string; nameZh: string; types?: string[]; typeZh?: string[] }>>([])
  const [detail, setDetail] = useState<SpeciesDetail | null>(null)
  const [natures, setNatures] = useState<Array<{ name: string; nameZh: string }>>([])
  const [paste, setPaste] = useState("")
  const [checking, setChecking] = useState(false)
  const current = team[slot] ?? emptySet()

  useEffect(() => {
    void api.search(format, "nature", "").then((body) => {
      setNatures(body.results as Array<{ name: string; nameZh: string }>)
    }).catch(() => setNatures([]))
  }, [format])

  useEffect(() => {
    if (!speciesOpen || query.trim().length === 0) return
    const handle = window.setTimeout(() => {
      void api.search(format, "species", query).then((body) => {
        setResults(body.results as typeof results)
      }).catch((error: Error) => toast.error(error.message))
    }, 180)
    return () => window.clearTimeout(handle)
  }, [query, speciesOpen, format])

  useEffect(() => {
    if (!current.species) {
      setDetail(null)
      return
    }
    void api.species(format, current.species).then(setDetail).catch(() => setDetail(null))
  }, [format, current.species])

  function update(next: SetInput) {
    const copy = team.slice()
    copy[slot] = next
    onTeam(copy)
  }

  async function validate() {
    setChecking(true)
    try {
      const check = await api.validate(format, team.filter((set) => set.species))
      setProblems(check.problems)
      if (check.ok) {
        const filled = team.slice()
        let cursor = 0
        for (let index = 0; index < filled.length; index += 1) {
          if (filled[index].species) {
            filled[index] = check.team[cursor]
            cursor += 1
          }
        }
        onTeam(filled)
        saveTeam(format, filled)
        toast.success("引擎已接受这支队伍")
      } else {
        toast.error("队伍没有通过引擎校验")
      }
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "校验失败")
    } finally {
      setChecking(false)
    }
  }

  async function doImport() {
    try {
      const result = await api.importTeam(format, paste)
      const next = result.team.slice(0, 6)
      while (next.length < 6) next.push(emptySet())
      onTeam(next)
      setProblems(result.check.problems)
      setPasteOpen(false)
      toast.success(result.check.ok ? "已导入并通过校验" : "已导入，但仍有校验问题")
    } catch (error) {
      toast.error(error instanceof Error ? error.message : "导入失败")
    }
  }

  async function doExport() {
    const result = await api.exportTeam(team.filter((set) => set.species))
    await navigator.clipboard.writeText(result.text)
    toast.success("已复制 Showdown 文本")
  }

  async function loadPreset(which: string) {
    const body = await api.presets(format)
    const found = body.presets.find((preset) => preset.id === which) ?? body.presets[0]
    if (!found) {
      toast.error("这个赛制没有内置样本")
      return
    }
    const next = found.team.slice(0, 6)
    while (next.length < 6) next.push(emptySet())
    onTeam(next)
    saveTeam(format, next)
    toast.success("已载入样本队伍")
  }

  const slotProblems = problems.filter((problem) => problem.set === slot || problem.set === null)
  const evSum = STATS.reduce((sum, [key]) => sum + (current.evs?.[key] ?? 0), 0)

  return (
    <div className="grid gap-4 lg:grid-cols-[220px_1fr]">
      <Card>
        <CardHeader>
          <CardTitle>六个位置</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-2">
          {team.map((set, index) => (
            <Button key={index} variant={index === slot ? "default" : "outline"} className="justify-start" onClick={() => setSlot(index)}>
              {index + 1}. {set.species || "空位"}
            </Button>
          ))}
        </CardContent>
      </Card>
      <div className="grid gap-4">
        <Card>
          <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
            <div>
              <CardTitle>队伍编辑</CardTitle>
              <p className="text-sm text-muted-foreground">合法性只由对战引擎判定。</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Select value={format} onValueChange={onFormat}>
                <SelectTrigger className="w-56" aria-label="赛制">
                  <SelectValue placeholder="赛制" />
                </SelectTrigger>
                <SelectContent>
                  {formats.map((entry) => (
                    <SelectItem key={entry.id} value={entry.id}>
                      {entry.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button variant="outline" onClick={() => void loadPreset("sample")}>样本</Button>
              <Button variant="outline" onClick={() => setPasteOpen(true)}>导入</Button>
              <Button variant="outline" onClick={() => void doExport()}>导出</Button>
              <Button onClick={() => void validate()} disabled={checking}>{checking ? "校验中" : "校验"}</Button>
              <Button onClick={() => { saveTeam(format, team); onUse() }}>用这支队伍对战</Button>
            </div>
          </CardHeader>
          <CardContent>
            {team.every((set) => !set.species) ? (
              <Empty className="border">
                <EmptyHeader>
                  <EmptyTitle>还没有宝可梦</EmptyTitle>
                  <EmptyDescription>从一个位置开始选择物种，或载入样本队伍。</EmptyDescription>
                </EmptyHeader>
              </Empty>
            ) : null}
            <FieldGroup className="mt-4">
              <Field data-invalid={slotProblems.some((problem) => problem.field === "species") || undefined}>
                <FieldLabel>物种</FieldLabel>
                <Button variant="outline" onClick={() => { setQuery(""); setSpeciesOpen(true) }}>
                  {current.species || "选择物种"}
                </Button>
                <FieldError errors={slotProblems.filter((problem) => problem.field === "species").map((problem) => ({ message: problem.message }))} />
              </Field>
              {detail ? (
                <div className="flex flex-wrap gap-1">
                  {detail.typeZh.map((label, index) => (
                    <Badge key={label} style={typeStyle(detail.types[index] ?? "")}>{label}</Badge>
                  ))}
                  {detail.baseStats ? (
                    <span className="text-sm text-muted-foreground">
                      种族值 {detail.baseStats.hp}/{detail.baseStats.atk}/{detail.baseStats.def}/{detail.baseStats.spa}/{detail.baseStats.spd}/{detail.baseStats.spe}
                    </span>
                  ) : null}
                </div>
              ) : null}
              <div className="grid gap-3 sm:grid-cols-2">
                <Field>
                  <FieldLabel>特性</FieldLabel>
                  <Select value={current.ability || "none"} onValueChange={(value) => update({ ...current, ability: value === "none" ? "" : value })}>
                    <SelectTrigger aria-label="特性"><SelectValue placeholder="特性" /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">未指定</SelectItem>
                      {(detail?.abilities ?? []).map((ability) => (
                        <SelectItem key={ability.name} value={ability.name}>{ability.nameZh}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel>道具</FieldLabel>
                  <Input aria-label="道具" value={current.item} onChange={(event) => update({ ...current, item: event.target.value })} placeholder="Leftovers" />
                </Field>
                <Field>
                  <FieldLabel>性格</FieldLabel>
                  <Select value={current.nature || "Serious"} onValueChange={(value) => update({ ...current, nature: value })}>
                    <SelectTrigger aria-label="性格"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      {natures.map((nature) => (
                        <SelectItem key={nature.name} value={nature.name}>{nature.nameZh}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel>太晶</FieldLabel>
                  <Input aria-label="太晶属性" value={current.teraType} onChange={(event) => update({ ...current, teraType: event.target.value })} placeholder={info?.tera ? "Fire" : "本赛制可能没有太晶"} />
                </Field>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                {current.moves.map((move, index) => (
                  <Field key={index}>
                    <FieldLabel>招式 {index + 1}</FieldLabel>
                    <Button variant="outline" onClick={() => setMoveOpen(index)}>{move || "选择招式"}</Button>
                  </Field>
                ))}
              </div>
              <div className="grid grid-cols-3 gap-2 sm:grid-cols-6">
                {STATS.map(([key, label]) => (
                  <Field key={key}>
                    <FieldLabel>{label}</FieldLabel>
                    <Input
                      aria-label={`${label}努力值`}
                      inputMode="numeric"
                      value={current.evs?.[key] ?? 0}
                      onChange={(event) => update({
                        ...current,
                        evs: { ...(current.evs ?? { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0 }), [key]: Number(event.target.value) || 0 },
                      })}
                    />
                  </Field>
                ))}
              </div>
              <p className="text-sm text-muted-foreground">努力值合计 {evSum}{info?.evLimit ? ` / ${info.evLimit}` : ""}。最终是否超限由引擎判断。</p>
              {slotProblems.length > 0 ? (
                <Alert variant="destructive">
                  <AlertTitle>引擎校验</AlertTitle>
                  <AlertDescription>{slotProblems.map((problem) => problem.message).join(" ")}</AlertDescription>
                </Alert>
              ) : null}
            </FieldGroup>
          </CardContent>
        </Card>
      </div>

      <Dialog open={speciesOpen} onOpenChange={setSpeciesOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>选择物种</DialogTitle>
            <DialogDescription>按中文、英文或图鉴编号搜索。这里只是候选列表。</DialogDescription>
          </DialogHeader>
          <Input autoFocus aria-label="搜索物种" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="皮卡丘 / Pikachu / 25" />
          <ScrollArea className="h-72">
            <div className="grid gap-1">
              {results.map((row) => (
                <Button
                  key={row.id}
                  variant="ghost"
                  className="justify-start"
                  onClick={() => {
                    update({ ...current, species: row.name, ability: "" })
                    setSpeciesOpen(false)
                  }}
                >
                  {row.nameZh} <span className="text-muted-foreground">{row.name}</span>
                </Button>
              ))}
            </div>
          </ScrollArea>
        </DialogContent>
      </Dialog>

      <Sheet open={moveOpen !== null} onOpenChange={(open) => { if (!open) setMoveOpen(null) }}>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>选择招式</SheetTitle>
            <SheetDescription>{detail?.note ?? "学习集只是候选。"}</SheetDescription>
          </SheetHeader>
          <ScrollArea className="h-[70vh] pr-3">
            <div className="grid gap-1">
              {(detail?.moves ?? []).map((move) => (
                <Button
                  key={move.id}
                  variant="ghost"
                  className="justify-start"
                  onClick={() => {
                    if (moveOpen === null) return
                    const moves = current.moves.slice()
                    moves[moveOpen] = move.name
                    update({ ...current, moves })
                    setMoveOpen(null)
                  }}
                >
                  {move.nameZh} <span className="text-muted-foreground">{move.name}</span>
                </Button>
              ))}
            </div>
          </ScrollArea>
        </SheetContent>
      </Sheet>

      <Dialog open={pasteOpen} onOpenChange={setPasteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>导入 Showdown 队伍</DialogTitle>
            <DialogDescription>粘贴导出文本。引擎会再校验一次。</DialogDescription>
          </DialogHeader>
          <Textarea aria-label="队伍文本" value={paste} onChange={(event) => setPaste(event.target.value)} className="min-h-48" />
          <Button onClick={() => void doImport()}>导入</Button>
        </DialogContent>
      </Dialog>
    </div>
  )
}
