# 验收记录（第 13 节 12 项）

- **版本**：v0.1.2（tag `v0.1.2`，产物见
  <https://github.com/fw6/oxide-fab/releases/tag/v0.1.2>）
- **记录时间**：2026-11-17
- **本机**：macOS/arm64，docker compose v2，nsh 0.5.x

每一行都是实际运行的结果；`artifacts/` 下的日志是运行输出，未被编辑。

> **阶段 2 重跑（v0.2.0，2026-11-17）**：这 12 项已在桌面工作台上重新跑通，
> 见 [acceptance-phase2.md](acceptance-phase2.md) §3。第 2 项的交付目标按
> [ADR 005](decisions/005-phase2-desktop-scope.md) 由 web/TUI 两个包改为
> macOS / Windows / Linux 三个原生包；其余 11 项口径不变。下表是 v0.1.2 的
> 历史记录，保留作对照。

| # | 验收项 | 实际命令 | 结果 | 证据 |
|---|---|---|---|---|
| 1 | fmt / clippy / workspace 测试 | `cargo fmt --all -- --check`；`cargo clippy --all-targets --all-features -- -D warnings`；`cargo test --workspace` | 通过：fmt 干净、clippy 无告警、29 passed / 0 failed（core 19、sim 6、store 3、secs 1） | `artifacts/accept-1-lint-test.log` |
| 2 | `nsh test` 通过；web/tui 双目标构建并运行；ADR 001 含 `llms.txt` 实读证据 | `nsh test crates/oxide-ui --junit artifacts/nsh-tests.xml`；`nsh test crates/oxide-ui --target web`；`nsh test crates/oxide-ui --target tui`；`nsh run --target web` | 通过：12 tests / 12 passed（含控制条两条）；web 静态站 6.7 MB 并已截图；tui 在内存终端渲染并断言面板文本 | `artifacts/nsh-tests.xml`、`artifacts/ui-web*.log`、`artifacts/ui-tui*.log`、`docs/screenshots/dashboard-web.png`、`docs/decisions/001-ui-toolchain.md` |
| 3 | `compose up` 后 smoke：建 Lot → 4 工序 → 逐片 genealogy | `docker compose up -d --wait && ./scripts/smoke.sh` | 通过：lots=6、wafers=60、events=108、published=108、outbox_pending=0、`problems: []` | `artifacts/smoke-final.log` |
| 4 | 拆分/合并/返工/Hold/部分批次/重复 Track-out 单测 | `cargo test -p oxide-core --lib -- --list` | 通过：`split_distributes_wafers_and_ends_the_parent`、`merge_combines_two_lots_into_one`、`rework_moves_the_lot_back_and_clears_completed_steps`、`hold_blocks_dispatch_until_released`、`a_partial_lot_finishes_its_step_in_two_loads`、`a_duplicate_track_out_changes_nothing` 等 19 项 | `artifacts/accept-1-lint-test.log` |
| 5 | 幂等：同一 `event_id` 投两次，点数不增 | `./scripts/idempotency.sh` | 通过：两次重投 `delta=0`，四张表前后计数一致 | `artifacts/idempotency-final.log` |
| 6 | 清空 Greptime → replay → 点数与聚合一致 | `./scripts/replay.sh` | 通过：24/24、24/24、72/72、6/6，`spc_avg` delta=0 | `artifacts/replay-final.log` |
| 7 | 隔离实例 backup + restore，行数与 `max(created_at)` 校验 | `./scripts/backup.sh && ./scripts/restore.sh` | 通过：`ok=true`、lots=6、events=108、Greptime 4 表 24 行、82.7 s | `artifacts/backup-final.log`、`artifacts/restore-final.log`、`artifacts/restore-latest.json` |
| 8 | chaos 场景跑完且含 RPO/RTO 数字 | `./scripts/chaos.sh` | 通过 4 场景：kill-store 48.6 s（积压 60 条）、kill-db 18.9 s、secs-reconnect 5.3 s（spool 36/36）、restore-drill 91.9 s，RPO 全 0 | `docs/chaos-report.md`、`artifacts/chaos-console-final.log` |
| 9 | 两个 workflow 可解析；`v*` 链产出 3 平台二进制 + SHA256SUMS + GHCR 镜像（含 nsh 产物映射，见下） | CI 的 actionlint job；`git push origin v0.1.0`、`v0.1.1`、`v0.1.2` | 通过：actionlint 绿；三次 release 各 7 个资产（3 平台二进制、web、tui、SHA256SUMS、chaos-report），5 个校验和本地 `shasum -c` 全 OK；镜像已推送，manifest 含 linux/amd64 + linux/arm64，本机 `docker pull` 成功并运行 | run `43941341476`、`43952543568`、`43964558701`；`gh release view v0.1.2`；`docker manifest inspect ghcr.io/fw6/oxide-fab:0.1.2` |
| 10 | README 含边界声明、命名由来、真实截图、架构图、RPO/RTO、已知限制 | `grep '^## ' README.md` | 通过：10 节齐全 | `README.md`、`artifacts/accept-10-12-checks.log` |
| 11 | 无硬编码密钥、无 TODO 占位假绿 | `git grep -nE 'TODO\|unimplemented!\|todo!'`；密钥模式 grep | 通过：均为空 | `artifacts/accept-10-12-checks.log` |
| 12 | 全局命名一致性 | `git grep -nE '\bfab-[a-z0-9]\|OWNER\|REPO\b'` | 通过：无输出 | 同上 |

## 第 9 行的 nsh 产物映射（写死，避免歧义）

`release.yml` 的 `nsh-packages` job 只跑在 ubuntu 上，产出两件压缩包：

| 目标 | 实际命令 | 产物 | 验证方式 |
|---|---|---|---|
| `web` | `nsh package crates/oxide-ui --target web --name oxide-ui -o dist/web` | `oxide-ui-<version>-web.tar.gz`（静态站点，`index.html` + wasm + 资源） | CI 每次 push 都构建该包；`nsh test … --target web` 在 headless 浏览器里跑通 |
| `tui` | **不存在** `nsh package --target tui` | `oxide-ui-<version>-tui.tar.gz` = `nsh build` 的 `app.json` + 自包含 crate（`nsh.json`、`theme.json`、`src/`、`tests/`、`Cargo.toml`、`Cargo.lock`、`RUNNING.txt`） | `nsh test crates/oxide-ui --target tui --junit artifacts/nsh-tui-tests.xml`：构建终端宿主、在内存终端渲染同一 `App` 并断言面板文本 |

为什么是替代而不是漏做：`tui` 是 nsh 内置的 **run/test** 目标，CLI 在查外部 target
机制之前就拒绝它，0.5.0、0.5.1 与 nightly 一致：

```text
$ nsh package crates/oxide-ui --target tui
error: invalid value 'tui' for '--target <TARGET>'
  [possible values: ios, ios-simulator, android, linux, macos, windows, web]
```

记录见 `docs/decisions/001-ui-toolchain.md` 的 “The TUI is not an nsh package
(checked, not assumed)” 一节；release notes 的同一条限制也写给下载者。

## §8 控制面：API 与 UI 两侧

| 侧 | 命令 / 测试 | 结果 | 证据 |
|---|---|---|---|
| API | `POST /api/sim/{pause,play,step,speed/5}` | 四个都 200，快照里 `speed` 变为 5 | `artifacts/ui-controls-api.log` |
| UI | `nsh test crates/oxide-ui` 的 `the_control_bar_pauses_steps_and_changes_the_speed`、`the_same_button_plays_a_paused_line` | 通过：按下「暂停」「单步」「−」「+」后，stub 网络记录到 `POST /api/sim/pause`、`/step`、`/speed/5`、`/speed/20`；暂停态下按「运行」记录到 `POST /api/sim/play` | `crates/oxide-ui/tests/dashboard.rs`、`artifacts/nsh-tests-controls.log` |

## 复现

```bash
cp .env.example .env && docker compose up -d --wait && ./scripts/smoke.sh
./scripts/backup.sh && ./scripts/restore.sh
./scripts/chaos.sh
```
