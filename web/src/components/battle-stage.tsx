import { useEffect, useRef, useState } from "react"

import { assetUrl, type FormEntry, type StageManifest } from "@/lib/assets"
import { BattleViewport, type FighterView, type StageDebug } from "@/stage/viewport"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"

export function BattleStage({
  manifest,
  forms,
  fighters,
}: {
  manifest: StageManifest
  forms: FormEntry[]
  fighters: FighterView[]
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const viewRef = useRef<BattleViewport | null>(null)
  const [debug, setDebug] = useState<StageDebug | null>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const view = new BattleViewport(canvas, manifest)
    view.setForms(forms)
    viewRef.current = view
    const timer = window.setInterval(() => setDebug({ ...view.debug, pokemon: [...view.debug.pokemon] }), 400)
    return () => {
      window.clearInterval(timer)
      view.destroy()
      viewRef.current = null
    }
  }, [manifest])

  useEffect(() => {
    viewRef.current?.setForms(forms)
    void viewRef.current?.sync(fighters)
  }, [forms, fighters])

  const floor = debug?.floor
  return (
    <div className="flex min-h-[280px] flex-col gap-2">
      <div className="relative h-[420px] overflow-hidden rounded-lg border bg-black sm:h-[480px]">
        <canvas ref={canvasRef} className="h-full w-full" aria-label="对战场景" />
      </div>
      <div className="flex flex-wrap gap-1.5" aria-live="polite">
        <Badge variant="outline">{manifest.id}</Badge>
        <Badge variant={floor?.status === "ready" ? "secondary" : "destructive"}>
          {floor?.status === "ready" ? "地面已加载" : floor?.status === "missing" ? "地面文件缺失" : "地面状态"}
        </Badge>
        {floor?.unlit ? <Badge variant="outline">KHR_materials_unlit</Badge> : null}
        {floor?.uvScale ? (
          <Badge variant="outline">
            UV {floor.uvScale.map((value) => value.toFixed(2)).join(" × ")}
          </Badge>
        ) : null}
      </div>
      {floor && floor.status !== "ready" ? (
        <Alert>
          <AlertTitle>场景诊断</AlertTitle>
          <AlertDescription>
            {floor.message} 资源地址 {assetUrl(manifest.url)}。天空和看台没有随这个地面提供。
          </AlertDescription>
        </Alert>
      ) : null}
      {debug?.pokemon.some((pokemon) => pokemon.status === "fallback") ? (
        <Alert>
          <AlertTitle>模型占位</AlertTitle>
          <AlertDescription>
            {debug.pokemon
              .filter((pokemon) => pokemon.status !== "ready")
              .map((pokemon) => `${pokemon.name}：${pokemon.message}`)
              .join(" ")}
          </AlertDescription>
        </Alert>
      ) : null}
    </div>
  )
}
