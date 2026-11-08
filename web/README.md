# 对战台

本地可玩的宝可梦对战界面。伤害、合法性、换人和回合顺序都来自仓库里的 `pokemon-battle-engine`，页面只提交选择并显示引擎给当前视角的状态。

## 版本

- Vite 8.3.6
- React 19.3.1
- TypeScript 7.0.2
- PlayCanvas 2.23.3
- `@playcanvas/react` 0.11.7（与引擎一起安装；场景生命周期由 React `useEffect` 创建和销毁 PlayCanvas `Application`）
- Tailwind CSS 4.3.5
- shadcn 4.23.0，`info --json`：style `radix-nova`，base `radix`，别名 `@/components/ui`

## 主题

使用 tweakcn 官方预设 **Graphite**（注册表名 `graphite`）。

- 来源：<https://tweakcn.com/r/themes/graphite.json>
- 编辑器：<https://tweakcn.com/editor/theme?theme=graphite>
- 已核对的注册表副本：`src/theme/graphite.tweakcn.json`
- 安装：`npx shadcn@4.23.0 add --overwrite https://tweakcn.com/r/themes/graphite.json`
- 实际变量：`src/index.css` 的 `:root` 与 `.dark`
- 选择原因：中性灰、0.35rem 圆角、浅色用 Montserrat、深色用 Inter。界面本身不抢属性色和体力色；属性与体力条用单独的颜色，并始终带文字。

## 场景

`public/assets/stage.json` 指定 **SM_Battle_zhandou_05_Floor**。这是一块已有的战斗土地面，不是生成的占位，也不是带看台的体育场。`SM_Battle_sc_001` 没有用，因为锚点下没有地面。

- 父节点缩放 0.1
- 锚点 `(0, 0.02, ±2.5)`，宝可梦不挂在缩放后的地面下
- 双打第二只用横向 `allyOffset` 1.6
- 相机 `(8, 6, 10)` 看向 `(0, 0.8, 0)`，视野 40
- 加载器保留 `KHR_materials_unlit` 和 `KHR_texture_transform`。只有在 PlayCanvas 没有应用 10 倍 UV 或仍在受光时，才按扩展语义补回，不替换贴图，也不改宝可梦材质（包括喷火龙火焰贴图）

## 模型

`public/assets/forms.index.json` 是 2787 个形态的清单，不含二进制。自动对应规则只有：引擎形态序号为 0、清单 `formCode` 为 null、且不是异色。其他 `formCode` 不会猜测。

当前随开发环境提供、但不提交的样本：

- `/converted/forms/00025/00025.glb` 皮卡丘
- `/converted/forms/00006/00006.glb` 喷火龙
- `/converted/forms/SM_Battle_zhandou_05_Floor/SM_Battle_zhandou_05_Floor.glb`

把文件放到 `public/converted/`（见该目录 README），或设置 `VITE_ASSET_BASE_URL`。文件缺失时显示标注为「诊断占位」的胶囊，不会假装模型已加载。

这些是既有游戏资源，再分发许可未核实，因此不进入公开提交。

## 运行

```sh
# 终端 1，仓库根目录
cargo run --manifest-path bridge/Cargo.toml

# 终端 2
cd web
npm install
npm run dev
```

桥只监听 `127.0.0.1:8787`。Vite 把 `/api` 代理过去。

对人机使用引擎列出的合法行动做随机选择，不计算伤害。热座在同一屏幕切换 P1 / P2，每次只拉当前视角。

「皮卡丘对喷火龙」使用第九世代 Ubers，这样两只样本模型可以合法上场。OU 样本是引擎示例里的合法队伍，模型多半会落到占位。

## 检查

```sh
cargo test --manifest-path bridge/Cargo.toml
cd web && npx tsc -b --noEmit && npm run build
node --experimental-strip-types --test tests/assets.test.ts
```
