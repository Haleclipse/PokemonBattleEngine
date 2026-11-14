export interface FormEntry {
  formId: string
  nationalDex: number
  nameEn: string | null
  nameZh: string | null
  formCode: string | null
  shiny: boolean
  url: string
  bytes: number
  animations: string[]
}

export interface StageManifest {
  schema: string
  id: string
  label: string
  kind: "floor"
  url: string
  scale: number
  anchors: {
    p1: [number, number, number]
    p2: [number, number, number]
  }
  allyOffset: number
  camera: {
    position: [number, number, number]
    lookAt: [number, number, number]
    fov: number
  }
  preserve: string[]
  limitations: string
}

export function assetUrl(path: string): string {
  const env = (import.meta as ImportMeta & { env?: Record<string, string | undefined> }).env
  const base = (env?.VITE_ASSET_BASE_URL ?? "").replace(/\/$/, "")
  if (!path) return ""
  if (/^https?:\/\//.test(path)) return path
  return `${base}${path.startsWith("/") ? path : `/${path}`}`
}

/**
 * Map a battle species to a prepared model.
 * Only the base forme (engine form 0) with a null source formCode and no shiny flag is used.
 * Other form codes are not guessed.
 */
export function baseForm(forms: FormEntry[], dex: number, form: number): FormEntry | undefined {
  if (form !== 0 || dex <= 0) return undefined
  return forms.find((entry) => entry.nationalDex === dex && entry.formCode == null && entry.shiny === false)
}
