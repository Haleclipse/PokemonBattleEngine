import type { FormatInfo, FormatSummary, PublicState, SetInput, TeamCheck } from "@/lib/types"

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  })
  const body = (await response.json()) as T & { error?: string }
  if (!response.ok) {
    throw new Error(body.error || `请求失败 ${response.status}`)
  }
  return body
}

export const api = {
  health: () => request<{ ok: boolean }>("/api/health"),
  formats: () => request<{ formats: FormatSummary[] }>("/api/formats"),
  format: (id: string) => request<FormatInfo>(`/api/formats/${id}`),
  search: (format: string, kind: string, q: string) =>
    request<{ results: Array<Record<string, unknown>> }>(
      `/api/search?format=${encodeURIComponent(format)}&kind=${kind}&q=${encodeURIComponent(q)}&limit=24`,
    ),
  species: (format: string, name: string) =>
    request<SpeciesDetail>(`/api/species?format=${encodeURIComponent(format)}&name=${encodeURIComponent(name)}`),
  presets: (format: string) =>
    request<{ presets: Array<{ id: string; team: SetInput[] }> }>(`/api/presets?format=${encodeURIComponent(format)}`),
  validate: (format: string, team: SetInput[]) =>
    request<TeamCheck>("/api/teams/validate", { method: "POST", body: JSON.stringify({ format, team }) }),
  importTeam: (format: string, text: string) =>
    request<{ team: SetInput[]; check: TeamCheck }>("/api/teams/import", {
      method: "POST",
      body: JSON.stringify({ format, text }),
    }),
  exportTeam: (team: SetInput[]) =>
    request<{ text: string }>("/api/teams/export", { method: "POST", body: JSON.stringify({ team }) }),
  createBattle: (body: { format: string; mode: string; seed?: number; p1: SetInput[]; p2: SetInput[] }) =>
    request<{ id: string; state: PublicState }>("/api/battles", { method: "POST", body: JSON.stringify(body) }),
  state: (id: string, observer: string) => request<PublicState>(`/api/battles/${id}?observer=${observer}`),
  choose: (id: string, observer: string, order: number[]) =>
    request<PublicState>(`/api/battles/${id}/choose`, {
      method: "POST",
      body: JSON.stringify({ observer, order }),
    }),
  act: (id: string, observer: string, actions: unknown[]) =>
    request<PublicState>(`/api/battles/${id}/act`, {
      method: "POST",
      body: JSON.stringify({ observer, actions }),
    }),
  force: (id: string, observer: string, replacements: unknown[]) =>
    request<PublicState>(`/api/battles/${id}/switch`, {
      method: "POST",
      body: JSON.stringify({ observer, replacements }),
    }),
  revive: (id: string, observer: string, pokemon: number) =>
    request<PublicState>(`/api/battles/${id}/revive`, {
      method: "POST",
      body: JSON.stringify({ observer, pokemon }),
    }),
  restart: (id: string, observer: string) =>
    request<PublicState>(`/api/battles/${id}/restart`, {
      method: "POST",
      body: JSON.stringify({ observer }),
    }),
}

export interface SpeciesDetail {
  id: string
  name: string
  nameZh: string
  num: number
  types: string[]
  typeZh: string[]
  abilities: Array<{ name: string; nameZh: string }>
  baseStats: { hp: number; atk: number; def: number; spa: number; spd: number; spe: number } | null
  moves: Array<{ id: string; name: string; nameZh: string }>
  note: string
}
