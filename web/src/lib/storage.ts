import { emptyTeam, type SetInput } from "@/lib/types"

const KEY = "pbe.team.v1"

export function loadTeam(): { format: string; team: SetInput[] } {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return { format: "gen9ou", team: emptyTeam() }
    const parsed = JSON.parse(raw) as { format?: string; team?: SetInput[] }
    const team = Array.isArray(parsed.team) ? parsed.team.slice(0, 6) : []
    while (team.length < 6) team.push(emptyTeam()[0])
    return { format: parsed.format || "gen9ou", team }
  } catch {
    return { format: "gen9ou", team: emptyTeam() }
  }
}

export function saveTeam(format: string, team: SetInput[]) {
  localStorage.setItem(KEY, JSON.stringify({ format, team }))
}
