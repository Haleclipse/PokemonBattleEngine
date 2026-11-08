import {
  Application,
  Asset,
  Color,
  Entity,
  FILLMODE_NONE,
  FOG_LINEAR,
  RESOLUTION_AUTO,
  StandardMaterial,
  Vec2,
  type AnimTrack,
  type StandardMaterial as StandardMaterialType,
} from "playcanvas"

import { assetUrl, baseForm, type FormEntry, type StageManifest } from "@/lib/assets"
import type { ContainerResource } from "playcanvas"

export interface FighterView {
  key: string
  side: "p1" | "p2"
  slot: number
  dex: number
  form: number
  name: string
  fainted: boolean
  cueSeq: number
  cueClip: string | null
}

export interface StageDebug {
  sceneId: string
  label: string
  limitations: string
  floor: {
    status: "loading" | "ready" | "missing" | "error"
    url: string
    unlit: boolean | null
    uvScale: [number, number] | null
    uvOffset: [number, number] | null
    patched: boolean
    message: string
  }
  pokemon: Array<{
    key: string
    name: string
    status: "loading" | "ready" | "fallback" | "error"
    message: string
  }>
}

const CLIP_RANK: Record<string, number> = {
  Dead: 6,
  Appear: 5,
  Attack: 4,
  Cast: 3,
  Wound: 2,
  Buff: 1,
  Idle: 0,
}

type RenderLike = {
  meshInstances: ReadonlyArray<{ aabb: { getMin: () => { y: number } }; material: StandardMaterialType }>
}

export class BattleViewport {
  readonly debug: StageDebug
  private readonly app: Application
  private readonly manifest: StageManifest
  private forms: FormEntry[] = []
  private actors = new Map<string, { entity: Entity; model: Entity | null; clip: string; cueSeq: number; status: StageDebug["pokemon"][number] }>()
  private destroyed = false

  constructor(canvas: HTMLCanvasElement, manifest: StageManifest) {
    this.manifest = manifest
    this.debug = {
      sceneId: manifest.id,
      label: manifest.label,
      limitations: manifest.limitations,
      floor: {
        status: "loading",
        url: assetUrl(manifest.url),
        unlit: null,
        uvScale: null,
        uvOffset: null,
        patched: false,
        message: "正在加载战斗地面",
      },
      pokemon: [],
    }
    this.app = new Application(canvas, {
      graphicsDeviceOptions: { antialias: true, alpha: false },
    })
    this.app.setCanvasFillMode(FILLMODE_NONE)
    this.app.setCanvasResolution(RESOLUTION_AUTO)
    this.app.start()
    const sky = new Color(0.07, 0.09, 0.14)
    this.app.scene.ambientLight = new Color(0.55, 0.58, 0.64)
    const fog = this.app.scene.fog
    fog.type = FOG_LINEAR
    fog.color = sky
    fog.start = 14
    fog.end = 32
    const camera = new Entity("camera")
    camera.addComponent("camera", {
      clearColor: sky,
      fov: manifest.camera.fov,
      nearClip: 0.05,
      farClip: 80,
    })
    const [cx, cy, cz] = manifest.camera.position
    const [lx, ly, lz] = manifest.camera.lookAt
    camera.setPosition(cx, cy, cz)
    camera.lookAt(lx, ly, lz)
    this.app.root.addChild(camera)
    const sun = new Entity("sun")
    sun.addComponent("light", {
      type: "directional",
      color: new Color(1, 0.97, 0.92),
      intensity: 1.15,
      castShadows: false,
    })
    sun.setEulerAngles(48, 28, 0)
    this.app.root.addChild(sun)
    this.resize()
    window.addEventListener("resize", this.resize)
    void this.loadFloor()
  }

  setForms(forms: FormEntry[]) {
    this.forms = forms
  }

  resize = () => {
    const canvas = this.app.graphicsDevice.canvas
    const parent = canvas.parentElement
    if (!parent) return
    this.app.resizeCanvas(parent.clientWidth, Math.max(parent.clientHeight, 240))
  }

  destroy() {
    this.destroyed = true
    window.removeEventListener("resize", this.resize)
    this.app.destroy()
  }

  async sync(fighters: FighterView[]) {
    const alive = new Set(fighters.map((fighter) => fighter.key))
    for (const [key, actor] of this.actors) {
      if (!alive.has(key)) {
        actor.entity.destroy()
        this.actors.delete(key)
      }
    }
    await Promise.all(fighters.map((fighter) => this.ensure(fighter)))
    this.debug.pokemon = [...this.actors.values()].map((actor) => actor.status)
    publish(this.debug)
  }

  private async loadFloor() {
    const url = assetUrl(this.manifest.url)
    try {
      const bytes = await fetchGlb(url)
      if (!bytes) {
        this.debug.floor.status = "missing"
        this.debug.floor.message = `战斗地面文件不在 ${url}。这不是生成的占位地面，文件需要按清单放到资源地址。`
        publish(this.debug)
        return
      }
      const asset = await loadContainer(this.app, blobUrl(bytes), this.manifest.id)
      if (this.destroyed) return
      const resource = containerOf(asset)
      const model = resource.instantiateRenderEntity()
      const root = new Entity("arena")
      root.setLocalScale(this.manifest.scale, this.manifest.scale, this.manifest.scale)
      root.addChild(model)
      this.app.root.addChild(root)
      this.noteFloor(model)
      publish(this.debug)
    } catch (error) {
      this.debug.floor.status = "error"
      this.debug.floor.message = error instanceof Error ? error.message : "地面加载失败"
      publish(this.debug)
    }
  }

  private noteFloor(model: Entity) {
    const material = firstMaterial(model)
    if (!material) {
      this.debug.floor.status = "error"
      this.debug.floor.message = "地面没有材质"
      return
    }
    const tiling = material.emissiveMap ? material.emissiveMapTiling : material.diffuseMapTiling
    let patched = false
    const scaleX = tiling?.x ?? 1
    const scaleY = tiling?.y ?? 1
    if (Math.abs(scaleX - 10) > 0.01 || Math.abs(scaleY - 10) > 0.01) {
      const target = material.emissiveMap ? "emissive" : "diffuse"
      material[`${target}MapTiling` as "emissiveMapTiling"] = new Vec2(10, 10)
      material[`${target}MapOffset` as "emissiveMapOffset"] = new Vec2(0, 0)
      patched = true
    }
    if (material.useLighting) {
      material.useLighting = false
      material.useSkybox = false
      patched = true
    }
    if (patched) material.update()
    const afterTiling = material.emissiveMap ? material.emissiveMapTiling : material.diffuseMapTiling
    const afterOffset = material.emissiveMap ? material.emissiveMapOffset : material.diffuseMapOffset
    this.debug.floor = {
      ...this.debug.floor,
      status: "ready",
      unlit: material.useLighting === false,
      uvScale: [afterTiling?.x ?? 0, afterTiling?.y ?? 0],
      uvOffset: [afterOffset?.x ?? 0, afterOffset?.y ?? 0],
      patched,
      message: patched
        ? "已按 KHR_materials_unlit / KHR_texture_transform 补回地面材质语义，没有替换贴图。"
        : "PlayCanvas 已保留地面的无光照材质和 10 倍 UV。",
    }
  }

  private async ensure(fighter: FighterView) {
    let actor = this.actors.get(fighter.key)
    if (!actor) {
      const entity = new Entity(fighter.key)
      this.place(entity, fighter)
      this.app.root.addChild(entity)
      actor = {
        entity,
        model: null,
        clip: "",
        cueSeq: -1,
        status: { key: fighter.key, name: fighter.name, status: "loading", message: "正在解析模型" },
      }
      this.actors.set(fighter.key, actor)
      await this.attachModel(actor, fighter)
    }
    this.place(actor.entity, fighter)
    if (fighter.fainted && actor.clip !== "Dead") {
      this.play(actor, "Dead")
    } else if (fighter.cueClip && fighter.cueSeq !== actor.cueSeq) {
      actor.cueSeq = fighter.cueSeq
      this.play(actor, fighter.cueClip)
    }
  }

  private place(entity: Entity, fighter: FighterView) {
    const anchor = this.manifest.anchors[fighter.side]
    const side = fighter.side === "p1" ? -1 : 1
    const x = anchor[0] + (fighter.slot === 0 ? 0 : side * this.manifest.allyOffset)
    entity.setPosition(x, anchor[1], anchor[2])
    entity.setEulerAngles(0, fighter.side === "p1" ? 180 : 0, 0)
  }

  private async attachModel(actor: NonNullable<ReturnType<typeof this.actors.get>>, fighter: FighterView) {
    const entry = baseForm(this.forms, fighter.dex, fighter.form)
    if (!entry) {
      this.fallback(actor, fighter, fighter.form === 0 ? "清单里没有这只宝可梦的基础形态。" : "引擎形态不是基础形态，不会猜测 formCode。")
      return
    }
    const url = assetUrl(entry.url)
    try {
      const bytes = await fetchGlb(url)
      if (!bytes) {
        this.fallback(actor, fighter, `清单有 ${entry.formId}，但 ${url} 没有可加载的 GLB。`)
        return
      }
      const asset = await loadContainer(this.app, blobUrl(bytes), entry.formId)
      if (this.destroyed) return
      const resource = containerOf(asset)
      const model = resource.instantiateRenderEntity()
      actor.entity.addChild(model)
      actor.model = model
      this.alignFeet(actor.entity, this.manifest.anchors[fighter.side][1])
      this.bindClips(model, resource)
      this.play(actor, "Idle")
      actor.status = {
        key: fighter.key,
        name: fighter.name,
        status: "ready",
        message: `${entry.formId} 已加载，材质保持原文件。`,
      }
    } catch (error) {
      this.fallback(actor, fighter, error instanceof Error ? error.message : "模型加载失败")
    }
  }

  private fallback(actor: NonNullable<ReturnType<typeof this.actors.get>>, fighter: FighterView, message: string) {
    const body = new Entity("fallback")
    body.addComponent("render", { type: "capsule" })
    const material = new StandardMaterial()
    material.diffuse = new Color(0.45, 0.48, 0.52)
    material.emissive = new Color(0.08, 0.09, 0.1)
    material.update()
    const render = body.render
    if (render) {
      for (const instance of render.meshInstances) instance.material = material
    }
    body.setLocalScale(0.55, 0.7, 0.55)
    actor.entity.addChild(body)
    actor.model = body
    actor.status = {
      key: fighter.key,
      name: fighter.name,
      status: "fallback",
      message: `诊断占位：${message}`,
    }
  }

  private alignFeet(entity: Entity, anchorY: number) {
    const renders = entity.findComponents("render") as unknown as RenderLike[]
    let minY = Number.POSITIVE_INFINITY
    for (const render of renders) {
      for (const instance of render.meshInstances) {
        minY = Math.min(minY, instance.aabb.getMin().y)
      }
    }
    if (!Number.isFinite(minY)) return
    const position = entity.getPosition()
    entity.setPosition(position.x, position.y + (anchorY - minY), position.z)
  }

  private bindClips(model: Entity, resource: ContainerResource) {
    model.addComponent("anim", { activate: true })
    for (const animation of resource.animations ?? []) {
      const track = animation.resource
      if (!track) continue
      const loop = animation.name === "Idle" || animation.name === "Run"
      model.anim?.assignAnimation(animation.name, track as AnimTrack, undefined, 1, loop)
    }
  }

  private play(actor: NonNullable<ReturnType<typeof this.actors.get>>, clip: string) {
    const model = actor.model
    if (!model?.anim) {
      actor.clip = clip
      return
    }
    const current = CLIP_RANK[actor.clip] ?? 0
    const next = CLIP_RANK[clip] ?? 0
    if (actor.clip === "Dead" && next < current) return
    try {
      model.anim.baseLayer?.play(clip)
      actor.clip = clip
      if (clip !== "Idle" && clip !== "Dead") {
        window.setTimeout(() => {
          if (actor.clip === clip && !actor.entity.findByName("fallback")) model.anim?.baseLayer?.play("Idle")
          if (actor.clip === clip) actor.clip = "Idle"
        }, clip === "Appear" ? 900 : 700)
      }
    } catch {
      actor.clip = clip
    }
  }
}

function firstMaterial(entity: Entity): StandardMaterialType | null {
  const renders = entity.findComponents("render") as unknown as RenderLike[]
  return renders[0]?.meshInstances[0]?.material ?? null
}

async function fetchGlb(url: string): Promise<Uint8Array | null> {
  const response = await fetch(url)
  if (!response.ok) return null
  const bytes = new Uint8Array(await response.arrayBuffer())
  const glb = bytes.length > 12 && bytes[0] === 0x67 && bytes[1] === 0x6c && bytes[2] === 0x54 && bytes[3] === 0x46
  return glb ? bytes : null
}

function blobUrl(bytes: Uint8Array): string {
  const copy = bytes.slice()
  return URL.createObjectURL(new Blob([copy.buffer], { type: "model/gltf-binary" }))
}

function loadContainer(app: Application, url: string, name: string): Promise<Asset<"container">> {
  return new Promise((resolve, reject) => {
    const asset = new Asset<"container">(name, "container", { url })
    asset.once("load", () => resolve(asset))
    asset.once("error", (error: unknown) => reject(error instanceof Error ? error : new Error(String(error))))
    app.assets.add(asset)
    app.assets.load(asset)
  })
}

function containerOf(asset: Asset<"container">) {
  const resource = asset.resource
  if (!resource) throw new Error("模型容器是空的")
  return resource
}

function publish(debug: StageDebug) {
  ;(window as unknown as { __stageDebug?: StageDebug }).__stageDebug = debug
}
