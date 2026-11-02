# pokemon-battle-engine

一个用 Rust 2024 实现的宝可梦对战规则引擎，覆盖第 1 到第 9 世代的单打与双打，以 [Pokémon Showdown](https://github.com/smogon/pokemon-showdown) 为差分基准逐帧比对。

A Rust Pokémon battle engine for Generations 1–9, singles and doubles, differentially tested frame by frame against Pokémon Showdown.

## 特点

- **可暂停、可恢复的执行。** `submit → step → Awaiting / Progressed / Finished`；任意时刻 `snapshot()` / `restore()` 或 `Clone` 分叉，连随机数一起回到同一刻，结果与不中断运行完全一致。
- **按世代解析的规则。** 行动顺序、伤害公式与取整、残留顺序、异常状态的生命周期等世代差异都在开战前解析为规则表，热路径上没有 `if generation` 分支；Gen 1、Gen 2 的伤害各是独立策略。
- **统一的招式流程。** 所有招式走同一个 Move Shell；状态修改经过 Request → 解析 → Commit，每一步进入 trace。
- **按观察者过滤的视图。** `view(Observer)` 只给某一方知道的信息（己方完整，对方只有公开过的），与 Showdown 客户端（`@pkmn/client`）的视角逐帧对照。
- **赛制层。** 赛制数据、按 Showdown 校验的队伍、队伍预览与公开队伍表、带出成员的选择（`BattleSession`）。
- **诚实的支持范围。** 解析器对每个条目做保守的支持分类，未实现的行为在选择校验时就被拒绝，不做静默的半实现。

## 支持范围

- **世代：** 第 1 到第 9 世代，各按该世代的规则与数据（Showdown 的 `gen1` … `gen9`）。
- **对战模式：** 单打、双打。三打、轮转对战、多人对战、混战不支持。
- **特殊机制：** 超级进化（第 6–7 世代，第 7 世代含究极爆发）、Z 招式（第 7 世代）、极巨化（第 8 世代）、太晶化（第 9 世代）。
- **赛制：** 自定义对战（单打第 1–9 世代，双打第 3–9 世代——原作与 Showdown 的双打都从第 3 世代开始），以及 `data/formats/formats.json` 收录的 34 个赛制：第 1–9 世代的 OU 与 Ubers、第 3–9 世代的双打 OU、第 5–9 世代的 VGC。队伍按 Showdown 的规则校验。Let's Go、晶灿钻石 / 明亮珍珠、阿尔宙斯等非标准规则不在支持范围内。
- **内容：** 各世代的招式、特性、道具（`cargo run --example support_matrix`）：

<!-- support-summary:begin (generated: tests/docs.rs) -->
| 世代 | 招式 | Damage / Status / Custom | 不支持的招式 | 特性（已实现 / 全部） | 道具（已实现 / 全部） |
|---|---|---|---|---|---|
| Gen 1 | 165 | 109 / 40 / 16 | 0 | — | 9 / 9 |
| Gen 2 | 251 | 157 / 56 / 38 | 0 | — | 62 / 62 |
| Gen 3 | 354 | 215 / 79 / 60 | 0 | 76 / 76 | 106 / 106 |
| Gen 4 | 467 | 296 / 91 / 80 | 0 | 123 / 123 | 210 / 210 |
| Gen 5 | 559 | 363 / 99 / 97 | 0 | 164 / 164 | 239 / 239 |
| Gen 6 | 618 | 392 / 107 / 119 | 0 | 191 / 191 | 284 / 284 |
| Gen 7 | 709 | 468 / 114 / 127 | 0 | 233 / 233 | 339 / 339 |
| Gen 8 | 665 | 445 / 103 / 117 | 0 | 267 / 267 | 354 / 354 |
| Gen 9 | 689 | 474 / 104 / 111 | 0 | 310 / 310 | 249 / 249 |
<!-- support-summary:end -->

解析器对每个条目做保守的支持分类，未实现的行为在选择校验时就被拒绝，不会静默地按错误规则执行；`cargo run --example unsupported_moves -- 9` 列出某一世代仍未支持的招式与原因。

## 稳定 API

默认构建只提供稳定 API；内部模块经 `internals` feature 开放，不承诺兼容。一场对局：

- **赛前**：`FormatLibrary`（`data/` 的赛制数据）→ `SessionFormat` → `BattleSession`：两方登记 `TeamSet`（按 Showdown 校验），看对方的队伍预览，可接受公开队伍表，选出带的成员，会话随后开始 `BattleEngine`。
- **对战**：`step()` 推进到需要输入（`EngineStep::Awaiting(BattleRequest)`）或结束；能选什么由 `choice_options(side)` 列出，选择交给 `submit_turn_choices` / `resolve_forced_switches` / `resolve_revive`，可先 `validate_choice`。
- **观察**：`view(Observer)` 是某个观察者知道的对局（未知为 `Seen::Unknown`；`SideView::previewed()` 是队伍预览与公开队伍表），`omniscient_view()` 给受信任的宿主、研究与 AI；`names()` 把 id 译成 Showdown 的英文名。
- **分叉**：`BattleEngine: Clone`，`snapshot()` / `rewind()` 回到某一刻，`TraceMode::Off` 不记 trace。
- **存储**：会话快照与对局快照都可 `to_json()`（一行，用于存储；压缩由宿主决定）/ `to_json_pretty()`，再 `from_json()` 读回。JSON 外带格式名、布局版本与种类，只有同一布局版本能读回；快照是可信数据，并含双方的全部隐藏信息，不能发给玩家。

`cargo doc --open` 的首页有完整的可运行示例。`stable-api/` 是只用默认 API 的使用者包，三个示例分别是 AI 的推演、服务端的回合循环、规则研究的反事实比较：

```sh
cargo run --release --manifest-path stable-api/Cargo.toml --example rollout
cargo run --manifest-path stable-api/Cargo.toml --example server_loop
cargo run --manifest-path stable-api/Cargo.toml --example research
```

## 构建与测试

```sh
scripts/check.sh        # rustfmt --check、clippy -D warnings、全部测试、稳定 API 包、示例
cargo test
cargo run --example singles_turn
```

- `rust-toolchain.toml` 锁定工具链；`scripts/check.sh` 与 `.github/workflows/ci.yml` 执行相同的检查。
- 数据快照与 oracle 录制都提交在仓库里，构建和测试不需要 Node，也不联网。

## 验证

- **Showdown 逐帧对照。** `tests/fixtures/cases/` 有 2800 多场用 `@pkmn/sim`（版本 pin 在 `tools/oracle-node/package-lock.json`）录制的对局：引擎按同样的选择重放，每个回合边界的语义帧（HP、状态、能力等级、临时状态、场地……）逐项一致，每个随机决策都恰好是引擎请求的；录有各方视角的对局，`BattleView` 还与 `@pkmn/client` 逐帧比对。帧格式见 `tests/fixtures/SCHEMA.md`。
- **伤害与能力值。** `@smogon/calc` 的伤害档位与能力值，Gen 1 到 Gen 9 逐值一致。
- **随机差分。** `scripts/nightly.sh` 在限定时间内生成对局与 Showdown 比对（`tools/nightly.py`），新的分歧自动缩减成最小复现（`tools/minimize.py`）；`.github/workflows/nightly.yml` 每天运行一次。
- **按卡带。** 有卡带证据（反汇编、实机测试）表明 Showdown 与原作不同的地方，引擎按原作实现，oracle 打补丁随之对齐；每一处都记在 `tools/oracle-node/src/cartridge.mjs` 的注释里。

- **数据可复现。** `scripts/check-data.sh` 用 pin 住的导出器重新导出 dex、赛制与合法性数据，与仓库里提交的逐文件比对；云端 Nightly 每次都跑。

重新生成数据或录制需要 Node：`cd tools/oracle-node && npm ci`，然后 `node src/export-dex.mjs ../../data/dex`（见 `data/dex/README.md`、`tools/oracle-node/README.md`）。

## 发布

推送与 `Cargo.toml` 版本号一致的 tag（如 `v0.1.0`）时，`.github/workflows/release.yml` 先跑全部检查，再创建 GitHub Release，附上指纹清单 `fingerprints.json`：各世代 dex 的指纹、自定义对战与 34 个赛制的规则指纹、版本与提交号（`cargo run --example fingerprints` 在本地生成同样的清单）。快照只能在指纹相同的构建之间恢复，所以两个版本的清单一比较，就知道彼此的快照能否互通、哪些规则或数据变了。不发布到 crates.io。

## 许可证

本项目以 [GNU General Public License v3.0 only](LICENSE) 发布。`data/` 与 `tests/fixtures/` 中的数据来自 Pokémon Showdown（MIT）、`@smogon/calc`（MIT）与 PokéAPI（BSD-3-Clause），见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

Pokémon 及宝可梦名称是任天堂、Creatures 与 GAME FREAK 的商标；本项目与它们无关，也未获其认可。
