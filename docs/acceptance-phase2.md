# 阶段 2 验收记录（桌面 wgpu 3D 工作台）

- **版本**：0.2.0（本次工作区，未打 tag）
- **记录时间**：2026-11-17（2026-11-18 补入 CI 上的 Windows 证据与提速后的 CI 数据）
- **本机**：macOS / arm64（Darwin 27，Apple M2，Metal）、Docker/OrbStack、nsh 0.5.0
- **功能规格**：[docs/design/client-features.md](design/client-features.md)
- **范围决定**：[ADR 005](decisions/005-phase2-desktop-scope.md)（交付目标由 web/TUI 改为三个桌面包）

每一行都是实际运行的结果；`artifacts/` 下的日志是运行输出，未被编辑。
**未在本机执行的项目在「结果」列写明原因**，不以文档代替运行。

| 平台 | 构建 | 打包 | 运行 + 窗口截图 | 渲染适配器 |
|---|---|---|---|---|
| macOS arm64 | ✅ | ✅ `oxide-fab.app` | ✅ 本机 | Metal（Apple M2；无软件 Metal 实现） |
| Linux arm64 | ✅ 容器内 | ✅ `oxide-fab.tar.gz` | ✅ 容器 + Xvfb | **lavapipe**（`software: true`） |
| Windows x86_64 | ✅ 本机 xwin + CI | ✅ CI `oxide-fab-0.2.0-windows-x86_64.zip` / `-setup.exe` | ✅ CI（`windows-latest`） | **WARP**（`Microsoft Basic Render Driver`） |

A-01 因此 **三平台全部通过**：macOS 与 Linux 在本机（Linux 在容器内）执行，
Windows 由 CI 的 `desktop` 矩阵在 `windows-latest` 上执行，run `44278832829`
的 `desktop-windows` artifact 内含 zip、Inno Setup 安装包与窗口截图。

## 1. 三平台与视觉

| # | 验收点 | 实际命令 / 证据 | 结果 |
|---|---|---|---|
| A-01 | 三平台各自构建、打包、运行并产出窗口截图 | macOS：`./scripts/desktop-package.sh dist` → `dist/macos/oxide-fab.app`；运行 `dist/macos/oxide-fab.app/Contents/MacOS/oxidefab`，`artifacts/desktop-macos-packaged.png` | **macOS 通过** |
| | | Linux：`./scripts/linux-container-evidence.sh`（Ubuntu 24.04 arm64：cargo test 83 项、资产校验、lavapipe 四层 + 正交渲染、`nsh package --target linux`），`artifacts/linux-evidence.log` 末行 `LINUX-EVIDENCE-DONE`，渲染报告 `artifacts/render-linux/render-report.json`（`software: true`、llvmpipe）；打包目录在 Xvfb 中启动并连真实后端截图：`artifacts/desktop-linux.png`、`docs/screenshots/workbench-linux-l3.png`；包为 `artifacts/oxide-fab-linux.tar.gz` | **Linux 通过** |
| | | Windows：`cargo xwin check --target x86_64-pc-windows-msvc` 在本机做完整类型检查（真实 MSVC 头/库，`artifacts/windows-check.log`）；CI 的 `desktop` 矩阵在 `windows-latest` 上 `nsh package --target windows`，产出 `dist/windows/oxide-fab/oxide-fab.exe`（注意 Windows 的可执行文件按 `--name` 命名，与 macOS/Linux 的 `oxidefab` 不同）与 Inno Setup 安装包，然后以 `OXIDE_DEMO=1` 启动 exe、等 20 s、按 DWM 边框截取窗口（`DwmGetWindowAttribute` + `CopyFromScreen`），run `44278832829` 的 `desktop-windows` artifact 内含 zip、setup.exe 与 `desktop-windows.png`；日志记录 `desktop 1024x768; window 1120x720; capturing 1122x752` | **Windows 通过（CI）** |
| A-02 | 四域映射可核对，模块名与官方来源一致 | 功能规格第 1 节表；`docs/decisions/005-phase2-desktop-scope.md` 记录 nsh 文档实读；README 的边界声明 | 通过 |
| A-03 | L0→L1→L2→L3 可进入/退回/按 ID 直达 | `crates/oxide-scene` 27 项测试；面包屑 L0–L3 + 退回 + 复位；截图 `docs/screenshots/workbench-dark-l1-exploded.png`、`...-dark-l3-event.png`；`nsh test` 的 `pressing_a_lot_expands_its_wafers_and_genealogy` | 通过 |
| A-04 | L3 显示并复制 `event_id`，能在 GreptimeDB 找到数据点 | 打包应用截图 `docs/screenshots/workbench-dark-l3-event.png`（event_id + 复制按钮 + outbox 已投递 + 派生行计数）；`GET /api/events/{id}` 实测 `track_out … outbox published yes; greptime: e116_perf:1,equipment_telemetry:1,gem_event:1,spc_sample:1`（`artifacts/accept-api-phase2.log`） | 通过 |
| A-05 | 拆分/合并/返工/Hold/Release 与逐片 genealogy | `cargo test -p oxide-core` 的 `split_distributes_wafers_and_ends_the_parent`、`merge_combines_two_lots_into_one`、`rework_moves_the_lot_back_and_clears_completed_steps`、`hold_blocks_dispatch_until_released` 等；UI 的「演示触发」按钮走 `/api/lots/...`；逐片履历测试与截图 | 通过 |
| A-06 | Q-time 定位 wafer 与两个工序；SPC 超限定机台 | `oxide-core::analytics` 的 `a_late_track_in_is_a_qtime_violation_tied_to_its_event`、`a_mismatched_checksum_flags_the_recipe`、`utilisation_is_busy_time_over_the_event_window`；`nsh test` 的质量面板测试；本机 smoke 数据无 Q-time 违规（面板如实显示「无违规」） | 通过（检测与定位路径有单测；运行数据无违规） |
| A-07 | 每条派工显示规则与依据 | 实测快照 `dispatch`：`{"lot_id":"LOT-0005","step_no":4,"equipment_id":"MET-01","queue_position":0,"queue_len":1,"qualified_tools":["MET-01"],"recipe_id":"RCP-METRO","rule":"优先级 → FIFO"}`；`artifacts/accept-api-phase2.log`；效率面板截图 | 通过 |
| A-08 | what-if 产出前后对比，运行实例未改写 | 单测 `what_if_runs_from_the_snapshot_without_touching_the_live_sim`；`POST /api/what-if` 实测：加一台 etch，周期 38083→30583 ms、产出 317.2→406.8/h（`artifacts/accept-api-phase2.log`）；截图 `docs/screenshots/workbench-dark-whatif.png`（含「从 t0 重放」标注） | 通过 |
| A-09 | 巡查覆盖 PG / Greptime / MinIO / API / 投递者 | 实测 `monitor: postgres=up greptimedb=up object-store=up outbox-publisher=up oxide-api=up secs-host=up`；横切面板截图 | 通过 |
| A-10 | chaos 三场景行为符合第 5 节，503 不产生脏数据，报告含 RPO/RTO | `./scripts/chaos.sh` 四场景全 PASS：kill-store RTO 48.6 s（60 条积压）、kill-db 18.9 s、secs-reconnect 5.3 s（spool 36/36）、restore-drill 91.9 s，RPO 全 0；`docs/chaos-report.md`、`artifacts/accept-chaos.log`。UI 现场：停 Greptime 后 footer 显示「事件 108 · 已投递 0 · outbox 积压 108 · Greptime 断流（积压在 PG）」（`workbench-dark-chaos-backlog.png`），恢复后 `已投递 108 · 积压 0 · 在线`（`workbench-dark-chaos-drained.png`） | 通过 |
| A-11 | 减少动效后功能不减；三层下钻不依赖指针 | 「减少动效」只关闭 FOUP 插值；L0–L3/退回/复位均为按钮，可用键盘激活；`nsh test` 的演示与控制条测试不依赖坐标点击 | 通过（代码路径；未做逐键手动录屏） |
| A-12 | 每个可见数值可追到来源并标口径 | 卡片右上角来源行 + `/api/analytics` 的 8 条 `notes`；KPI 卡逐条渲染公式；无合成指标 | 通过 |

## 2. 功能清单（F-01…F-24）

| 编号 | 实现位置 | 证据 |
|---|---|---|
| F-01 | `tools/blender/build_fab.py`；`oxide-render/glb.rs`；`oxide-scene/graph.rs`；`oxide-ui/viewport.rs` | `scripts/validate-assets.sh`：46 manifest 节点 = 46 GLB mesh；`artifacts/render/l0-factory.png`；应用截图 |
| F-02 | `panels/mes.rs` 工序卡；`graph.rs` 状态灯 | `nsh test` 的 MES 面板断言；截图 |
| F-03 | `panels/mes.rs` 批次/晶圆/履历；`client::{hold,release,rework,split,merge,scrap}` | UI 测试；`oxide-core` 状态机测试 |
| F-04 | `graph.rs` 三态灯 + `panels/mes.rs` E116 行 | 截图；`analytics` 的利用率测试 |
| F-05 | `oxide-scene/layout.rs` `foup_placement` + `step_since` | `a_recent_step_change_puts_the_foup_on_the_rail` 等 5 项布局测试 |
| F-06 | `lib.rs` `search_results` + `client::load_trace` | 搜索 lot/wafer/equipment/event_id；无结果不改选中 |
| F-07 | `panels/quality.rs` SPC | 截图；`nsh test` 质量断言 |
| F-08 | `panels/quality.rs` 告警 → L3 | 告警行由 `equipment_down/up` 生成并带 event_id；本机 smoke 无告警（`alarms=0`），UI 测试用 stub 覆盖该列表 |
| F-09 | `panels/quality.rs` 配方 | 实测四个配方 `verified`；单测 checksum mismatch |
| F-10 | `panels/quality.rs` R2R（只读） | 截图：建议调整 `-0.20 nm（只读展示）` |
| F-11 | `analytics.rs` Q-time + `panels/quality.rs` | 单测；上限 S1→S2/S2→S3/S3→S4 ≤ 60 s |
| F-12 | `panels/mes.rs` 25 槽 + `panels/quality.rs` 良率 | 单测缺陷分类；截图 |
| F-13 | `oxide-sim` `DispatchDecision` + `panels/productivity.rs` | 实测 dispatch JSON |
| F-14 | `panels/productivity.rs` 自动/单步 + 空窗 | 面板显示运行/暂停与空窗合计 |
| F-15 | `panels/productivity.rs` 气泡/瓶颈/利用率 | 截图 |
| F-16 | `oxide-sim/whatif.rs` + `POST /api/what-if` + `panels/productivity.rs` | 单测 + 实测 200 |
| F-17 | `WhatIfDelta` | 面板逐项差值 |
| F-18 | `lib.rs` 控制条 | `nsh test` POST 断言 |
| F-19 | `panels/supply.rs` | 计划/实际、交付偏差 |
| F-20 | `GET /api/monitor` + `panels/cross.rs` | 实测六个组件 up |
| F-21 | outbox 计数 + chaos 曲线 | 截图；chaos 报告 |
| F-22 | `analytics` KPI + `notes` | 截图；8 条口径 |
| F-23 | `demo.rs` 八步演示 | `nsh test` 的启动/停止测试；演示截图 |
| F-24 | 导出（场景/批次/chaos）、减少动效、标签密度、启动 compose | 设置卡；`save` 使用系统保存对话框 |

## 3. 阶段 1 的 12 项（重新跑通）

| # | 命令 | 结果 | 证据 |
|---|---|---|---|
| 1 | `cargo fmt --all -- --check`；`cargo clippy --all-targets --all-features -- -D warnings`；`cargo test --workspace` | 通过：83 passed / 0 failed（18 个测试二进制） | `artifacts/accept-tests-phase2.log` |
| 2 | `nsh test crates/oxide-ui --junit artifacts/nsh-tests.xml` | 通过：**13 tests / 13 passed**（5 个数据折叠 + 8 个 UI；应用无 TSX 入口，nsh test 只构建 Rust 测试宿主） | `artifacts/nsh-test-phase2.log`、`artifacts/nsh-tests.xml` |
| 3 | `docker compose up -d --wait && ./scripts/smoke.sh` | 通过：lots=6 wafers=60 events=108 published=108 pending=0 | `artifacts/accept-smoke.log` |
| 4 | `cargo test -p oxide-core --lib` | 通过（状态机 8×11 矩阵等） | `artifacts/accept-tests-phase2.log` |
| 5 | `./scripts/idempotency.sh` | 通过：两次重投 delta=0 | `artifacts/accept-idempotency.log` |
| 6 | `./scripts/replay.sh` | 通过：四表行数与 SPC 均值 delta=0 | `artifacts/accept-replay.log` |
| 7 | `./scripts/backup.sh && ./scripts/restore.sh` | 通过：隔离 PITR，lots=6 events=106，82.4 s | `artifacts/accept-backup.log`、`artifacts/accept-restore.log` |
| 8 | `./scripts/chaos.sh` | 通过：四场景 RPO 0；RTO 48.6/18.9/5.3/91.9 s | `artifacts/accept-chaos.log`、`docs/chaos-report.md` |
| 9 | actionlint（`rhysd/actionlint` 容器）解析两个 workflow | 通过（SC2016 已按意图加 disable 注释） | 本记录 §1 的运行命令 |
| 10 | README 章节与截图 | 通过：边界声明、命名、截图、架构、RPO/RTO、已知限制均在 | `README.md` |
| 11 | `git grep -nE 'TODO\|unimplemented!\|todo!'`、密钥模式 | 通过：均为空 | `artifacts/accept-10-12-checks.log` |
| 12 | 全局命名一致性 grep | 通过：无 `fab-`/`OWNER`/`REPO` 残留 | 同上 |

## 4. 复现

```bash
# 后端
cp .env.example .env && docker compose up -d --wait && ./scripts/smoke.sh

# 桌面工作台（本机）
cd crates/oxide-ui && nsh run

# 无窗口渲染证据（Linux/Windows 可用软件适配器）
cargo run -p oxide-render --bin oxide-render-evidence -- --out artifacts/render
./scripts/validate-assets.sh

# 打包（在目标操作系统上）
./scripts/desktop-package.sh dist

# Linux 容器内全流程（macOS 主机有 Docker 时；host-ip 让容器内应用连到
# 本机后端，不给则只跑演示数据）
OXIDE_API_URL=http://<host-ip>:8080 ./scripts/linux-container-evidence.sh
```

## 5. 已知未在本机执行

- **Windows 的打包与运行只在 CI**：本机没有 Windows，`cargo xwin` 只做类型检查；
  打包、启动与窗口截图在 `windows-latest` 上执行（run `44278832829`）。
- **Windows 截图的取景**：runner 的桌面是 1024x768，比应用声明的最小窗口
  1120x720 还窄，且该镜像没有 `Set-DisplayResolution`，所以截图会丢掉窗口最右侧
  约 100 px（空白）；日志里的 `desktop ... window ... capturing ...` 一行记录了实际
  取景。窗口本身是完整的（1122x752），右侧面板的完整视觉以 macOS/Linux 截图和
  `artifacts/render/` 的 Windows 软件渲染帧为准。
- **Linux 原生运行**：在 Ubuntu 24.04 arm64 容器内以 Xvfb + lavapipe 执行；
  这是真实 Linux 用户态，但不是 CI runner 的 x86_64 环境。
- **CI 在本机之外**：workflow 通过 actionlint，三平台矩阵已在 PR #14 上跑通
  （墙钟约 10–18 分钟，详见 `docs/architecture.md` 的 CI 小节）。
