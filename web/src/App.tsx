import { useEffect, useState } from "react"
import { Moon, Sun } from "lucide-react"
import { toast } from "sonner"

import { BattleScreen } from "@/components/battle-screen"
import { TeamBuilder } from "@/components/team-builder"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { api } from "@/lib/api"
import { loadTeam, saveTeam } from "@/lib/storage"
import type { FormatInfo, FormatSummary, SetInput } from "@/lib/types"

export default function App() {
  const saved = loadTeam()
  const [format, setFormat] = useState(saved.format)
  const [team, setTeam] = useState<SetInput[]>(saved.team)
  const [formats, setFormats] = useState<FormatSummary[]>([])
  const [info, setInfo] = useState<FormatInfo | null>(null)
  const [bridge, setBridge] = useState<"loading" | "ready" | "down">("loading")
  const [tab, setTab] = useState("team")
  const [theme, setTheme] = useState<"light" | "dark">("dark")
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark")
  }, [theme])

  useEffect(() => {
    void api.health().then(() => setBridge("ready")).catch(() => setBridge("down"))
    void api.formats().then((body) => setFormats(body.formats)).catch(() => setFormats([]))
  }, [])

  useEffect(() => {
    if (bridge !== "ready") return
    void api.format(format).then(setInfo).catch((error: Error) => toast.error(error.message))
  }, [format, bridge])

  useEffect(() => {
    saveTeam(format, team)
  }, [format, team])

  return (
    <TooltipProvider>
      <div className="mx-auto flex min-h-svh max-w-7xl flex-col gap-4 px-3 py-4 sm:px-6">
        <header className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">对战台</h1>
            <p className="text-sm text-muted-foreground">本地规则引擎。没有匹配、账号或天梯。</p>
          </div>
          <Button variant="outline" size="icon" aria-label={theme === "dark" ? "切换到浅色" : "切换到深色"} onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
            {theme === "dark" ? <Sun /> : <Moon />}
          </Button>
        </header>
        {bridge === "down" ? (
          <Alert variant="destructive">
            <AlertTitle>引擎桥没有启动</AlertTitle>
            <AlertDescription>在仓库根目录运行 cargo run --manifest-path bridge/Cargo.toml，然后刷新。</AlertDescription>
          </Alert>
        ) : null}
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList>
            <TabsTrigger value="team">队伍</TabsTrigger>
            <TabsTrigger value="battle">对战</TabsTrigger>
            <TabsTrigger value="about">说明</TabsTrigger>
          </TabsList>
          <TabsContent value="team">
            <TeamBuilder
              formats={formats}
              format={format}
              info={info}
              team={team}
              onFormat={setFormat}
              onTeam={setTeam}
              onUse={() => setTab("battle")}
            />
          </TabsContent>
          <TabsContent value="battle">
            <BattleScreen format={format} team={team} active={bridge === "ready"} />
          </TabsContent>
          <TabsContent value="about">
            <About />
          </TabsContent>
        </Tabs>
      </div>
      <Toaster />
    </TooltipProvider>
  )
}

function About() {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Card>
        <CardHeader><CardTitle>现在能玩的</CardTitle></CardHeader>
        <CardContent className="grid gap-2 text-sm text-muted-foreground">
          <p>单机对战：对人机，或同一台设备上的热座。招式、换人、目标和太晶都交给引擎。</p>
          <p>队伍编辑、Showdown 文本导入导出、本地保存。校验信息来自引擎，不在页面里重算伤害。</p>
          <p>回合记录、体力、异常和能力等级只显示当前视角知道的内容。</p>
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>没有做的</CardTitle></CardHeader>
        <CardContent className="grid gap-2 text-sm text-muted-foreground">
          <p>没有服务器、账号、天梯、匹配或回放文件下载。重新开始会用同一随机种子再开一局。</p>
          <p>场景是 SM_Battle_zhandou_05_Floor，只有一块战斗地面，没有看台和天空。宝可梦模型默认只自动对应基础形态。</p>
          <p>美术文件不随代码发布。把 GLB 放到 web/public/converted，或设置 VITE_ASSET_BASE_URL。</p>
        </CardContent>
      </Card>
    </div>
  )
}
