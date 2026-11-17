# oxide-fab 阶段 2 · 客户端功能规格（对标 Applied SmartFactory 功能域）

适用范围：全量 wgpu 真 3D 桌面客户端（macOS / Windows / Linux）。本文件是精简 goal 的**配套功能规格**：goal 只写核心目标，本文件只定义「要做哪些功能、每个功能的数据从哪来、怎么算做完」。

**界面布局、窗口形态、视觉、配色、动效、快捷键、面板排布均不在此定义**，由 coding agent 自行决定；视觉方案另出候选提案（浅色与深色各不少于一套）交用户选定后落地。配套线框图不作为交付依据。

---

## 0. 定位与对标边界

功能按 **Applied SmartFactory（应用材料公司自动化软件产品组合）的四大领域**组织：制造执行 MES、工艺质量 Process Quality、生产效率 Productivity、供应链 Supply Chain，外加横切能力（监控、消息总线、KPI 量化）。产品名与模块名以第 7 节官方来源为准，不得凭记忆编造。

**这是功能语义对标，不是产品复刻。** oxide-fab 是仿真产线 / 面试作品，只实现能用阶段 1 已有数据模型验证的功能切片；商业套件的 AI 模型、求解器、多租户消息总线、多厂集成与设备软件捆绑不在实现范围内（见第 6 节）。README 与界面不得宣称与 Applied SmartFactory 等价、兼容、替代或由其认证。

三条功能原则：

1. **每个数值都有出处**：界面上任何状态值都能追到来源表与采集时刻，不出现无来源的合成指标。
2. **场景即导航**：3D 物体本身承载下钻，不另建一套重复索引同一批对象的导航结构。
3. **状态与动作分离**：工作台是 PostgreSQL 的只读投影；所有写操作统一标注「演示触发」，走 `oxide-api`，绝不绕过 `lot_event` + outbox。

阶段 1 的后端容器、`docker-compose.yml`、迁移、outbox 契约、运维脚本、GHCR 链一行不动；只砍 UI 交付目标，交付三个桌面原生安装包。

---

## 1. 四域对标映射

| SmartFactory 领域 | 代表性模块 | oxide-fab 实现切片 | 阶段 1 依托 |
|---|---|---|---|
| 制造执行 MES | SmartFactory MES 300Works Full-Auto、WIP / genealogy、carrier 与 substrate tracking | Lot/Wafer 全流程执行与逐片追溯、载具与基板语义、Hold/Rework/拆分合并 | `lot` / `wafer` / `lot_event` / `equipment` |
| 工艺质量 Process Quality | Applied E3：SPC、Fault Detection、Run-to-Run、RMS、Yield/Defect Management、UPC、OCAP | SPC 与告警、配方版本校验、Q-time 违规、缺陷与良率下钻、跨事件根因 | `recipe` / `spc_sample` / `gem_event` / `wafer.defect_info` |
| 生产效率 Productivity | APF：RTD、Dispatching & Reporting、Activity Manager、FullAuto、Production Control、AutoSched、Fusion、Material Control | 可解释派工规则、事件驱动自动流转、WIP 气泡与瓶颈、离线 what-if 重放与规则影响对比 | `oxide-sim` / `route_step` / `outbox` / `lot_event` |
| 供应链 Supply Chain | Advanced Planning & Scheduling、计划与执行连接、早期预警 | 计划与实际偏差对照、交付偏差预警（轻量） | `lot_event.occurred_at` / `route_step.duration_ms` |
| 横切 | SmartFactory Monitor、Message Bus、SmartFactory Rx、SmartFactory AI | 组件巡查、outbox 积压追平、KPI 口径化 | health 接口 / `outbox` / 演练产物 |

应用材料官方将 SmartFactory 定义为覆盖制造执行、工艺质量、生产效率、供应链四大能力的组合，且 AI 能力横跨四域；生产效率侧 APF 框架含排程、派工与报表、模拟与预测，并集成企业规划、设备自动化、维护管理与先进过程控制；工艺质量侧 Applied E3 框架含故障检测、run-to-run、SPC、设备自动化、配方管理、良率与缺陷管理 。

---

## 2. 功能清单

### 2.1 制造执行 MES

| 编号 | 对标模块 | 功能 | 数据来源 | 输出 / 约束 |
|---|---|---|---|---|
| F-01 | MES / WIP | 加载 GLB，渲染厂区、四道工序带、AMHS 路径、机台、FOUP；自由旋转、缩放、平移与正交/透视切换 | manifest + `config/scenario.json` | 场景图由数据构造，不硬编码摆放 |
| F-02 | MES / WIP | 每工序队列长度、busy/total 机台、当前 Lot | `route_step` + `lot` | 与场景浮标同源，筛选只影响显示 |
| F-03 | MES / genealogy | Lot 拆分、合并、返工、Hold / Release；载具与基板追踪语义（E87/E90 概念）；逐片 Wafer 全流程履历 | `lot` / `wafer` / `lot_event` | 履历按事件序完整，不跳步 |
| F-04 | MES / equipment | 运行 / 预警 / 停机三态表达，绑定 `equipment.state`；E116 状态时长统计 | `equipment` / `e116_perf` | 三态不依赖单一视觉通道区分 |
| F-05 | Material Control / AMHS | FOUP 沿 AMHS 路径与工序间移动 | `lot.current_step` + sim tick | 位置随仿真 `speed` 推进 |
| F-06 | MES / traceability | 按 `lot_id` / `wafer_id` / `equipment_id` / `event_id` 搜索并定位，自动展开对应层级 | 各表索引 | 无结果时不改变当前选中项 |

### 2.2 工艺质量 Process Quality（Applied E3）

| 编号 | 对标模块 | 功能 | 数据来源 | 输出 / 约束 |
|---|---|---|---|---|
| F-07 | SPC | 控制图、UCL/LCL 超限标记、异常检测 | `spc_sample` 降采样 | 超限项关联目标机台与工序 |
| F-08 | Fault Detection / OCAP | 设备与过程告警列表、告警处置路径展示、告警与目标机台联动定位 | `gem_event` / `spc_sample` | 告警可追到触发事件 |
| F-09 | RMS | recipe 版本、checksum 校验、配方执行一致性展示 | `recipe` / `lot_event` | 版本不匹配时明确标记，不静默通过 |
| F-10 | Run-to-Run (R2R) | 展示 run-to-run 参数调整建议与前后值 | `spc_sample` / `lot_event` | **只读展示**，不做闭环写回设备 |
| F-11 | UPC / Q-time | 连续工序间 Q-time 上限监控与违规告警；跨班次同类告警的重复模式识别 | `lot_event.occurred_at` / `route_step` | 违规可追到具体 wafer 与两个工序  |
| F-12 | Defect / Yield Management | 25 槽 Wafer 网格、缺陷分类标记、良率下钻、缺陷来源层定位（轻量） | `wafer.defect_info` / `lot_event` | 逐片可追溯，不引外部 YMS |

### 2.3 生产效率 Productivity（APF）

| 编号 | 对标模块 | 功能 | 数据来源 | 输出 / 约束 |
|---|---|---|---|---|
| F-13 | RTD / What-Next / Where-Next | 派工规则可解释展示：批次优先级、设备资格矩阵、队列时间、交期 | `oxide-sim` 派工规则 + `route_step` | 每条派工决策显示命中的规则与依据 |
| F-14 | FullAuto / Activity Manager | 事件驱动自动流转，无人工干预；业务规则变更即时生效 | `lot_event` / `oxide-sim` | 演示可切换「自动 / 单步」并对比空窗时间 |
| F-15 | Dispatching & Reporting | 派工报表、WIP 气泡分析、瓶颈工序识别、设备利用率 | `lot` / `route_step` / `lot_event` | 聚合值标注计算口径 |
| F-16 | Production Control / AutoSched | 离线 what-if 重放：改派工规则或机台数后从 snapshot + `lot_event` 重跑，输出周期时间与产出对比 | `snapshot` / `lot_event` / `config/scenario.json` | **不打断当前运行实例**，对照表标口径  |
| F-17 | Fusion | 规则变更影响量化：列出改动前后关键指标差值 | F-16 产物 | 只呈现差值，不自动改生产规则 |
| F-18 | 仿真控制 | 播放 / 暂停 / 单步 / 速度 | `oxide-sim` | 客户端控制，不改写仿真逻辑 |

### 2.4 供应链 Supply Chain

| 编号 | 对标模块 | 功能 | 数据来源 | 输出 / 约束 |
|---|---|---|---|---|
| F-19 | APS / 计划与执行连接 | 计划节拍与实际节拍对照、交付偏差早期预警 | `route_step.duration_ms` / `lot_event.occurred_at` | 预警基于事件时间，不引外部求解器 |

### 2.5 横切能力

| 编号 | 对标模块 | 功能 | 数据来源 | 输出 / 约束 |
|---|---|---|---|---|
| F-20 | SmartFactory Monitor | 巡查全部 CIM 组件：PG / GreptimeDB / MinIO / `oxide-api` / outbox 投递者，显示连接与就绪状态 | health 接口 + 组件自检 | 组件异常直接定位，不弹窗中断演示  |
| F-21 | Message Bus | outbox 积压条数、追平进度、零丢失与故障恢复、事件可重放；断流与追平状态自动呈现 | `outbox WHERE published_at IS NULL` / `published_event` | 对应商业消息总线的持久性与故障恢复语义，实现为阶段 1 outbox + 幂等  |
| F-22 | SmartFactory Rx / KPI | 周期时间、设备利用率、停机时长、缺陷数、RPO/RTO 等指标量化 | 第 4 节来源表 + 演练产物 | 每个指标标注计算口径，禁止无来源合成值 |
| F-23 | 演示模式 | 一键按固定顺序串起「MES 全厂 → 工艺质量 → 生产效率 what-if → 消息总线 chaos → 恢复 → KPI」 | 组合上述能力 | 任一时刻可中断并接管，中断后不自动跳回 |
| F-24 | 导出 / 后端就绪 / 可配置开关 | chaos 报告、当前场景快照、选中 Lot 追溯 JSON；首次启动拉起本地 compose；减少动效与标签密度开关 | 本地文件系统 / health / 本地配置 | 未就绪时禁用写操作；减少动效后功能不减少，仅取消插值 |

---

## 3. 层级与能力关系

L0 → L1 → L2 → L3 是**同一份 SceneGraph 的下钻层级**，不是四个独立页面，也不是四个产品模块：

- L0 全厂聚合（MES 域）、L1 机台拆解（工艺质量 + 设备状态）、L2 批次拆解（genealogy + 缺陷）、L3 事件拆解（`event_id` → outbox → GreptimeDB 数据点）。
- 每一层都可进入、可退回、可按 ID 直达；退回后选中项不丢。
- 层级切换不重载资产、不重建场景图；wgpu 拾取结果映射回 `SceneGraph` 的 `NodeId`，与全局搜索、上下文数据指向同一对象。
- 四大域能力以横切方式接入任意层级，不另建独立页面；场景图只读，下钻只改变查看粒度，不改变业务状态。

---

## 4. 数据契约

| 可见元素 | 唯一来源 |
|---|---|
| 机台灯号 / E116 时长 | `equipment.state` / `e116_perf` |
| 队列长度 / busy·total | `route_step` + `lot` |
| FOUP 位置 | `lot.current_step` |
| recipe 与 checksum | `recipe.version` / `recipe.checksum` |
| genealogy / Wafer slot / 缺陷 | `lot_event` / `wafer.defect_info` |
| SPC / FD / R2R / GEM | GreptimeDB `spc_sample` / `gem_event` / `e116_perf` |
| 派工决策 | `oxide-sim` 规则 + `route_step` 资格矩阵 |
| Q-time 违规 | `lot_event.occurred_at` + `route_step` 上限定义 |
| outbox 积压与追平 | `outbox WHERE published_at IS NULL` |
| 时间轴 / 回看 / what-if | `lot_event.seq` / `occurred_at` + `snapshot` |
| 备份状态 / RPO·RTO | `backup.sh` / `restore.sh` / `chaos.sh` 产物 |
| 连接灯 | `oxide-api` health 接口 |

约束：

- **不新增业务表**。what-if、Q-time、KPI、WIP 气泡全部由现有表与事件日志计算，计算口径写进 `docs/decisions/`。
- 工作台**只读**；写操作按钮统一带「演示触发」标签并走 `oxide-api`，不得直接写库或绕过 `lot_event` + outbox。
- 断线重连后拉一次增量对齐场景图，禁止重放整段历史动画。
- 界面不展示任何无来源的合成指标；聚合值须标注计算口径。
- PG 为唯一事实来源，GreptimeDB 为可重建派生数据，客户端不写任何一库。

---

## 5. 状态与异常（功能性要求）

| 状态 | 功能行为 |
|---|---|
| 首次启动 / 后端未就绪 | 写操作禁用；只读引导可用 |
| GreptimeDB 断流（kill-store） | 仿真继续；事件只积压在 PG outbox；流转展示暂停；受影响机台标记预警 |
| 追平中 | 积压按序下降；恢复后流转续跑；最终点数与关键聚合一致 |
| PG 不可用（kill-db） | 写操作全部拒绝且不产生脏数据；保留最后场景 |
| 重启恢复中 | 由 snapshot + `lot_event` 重放重建场景图，Lot 状态不丢 |
| what-if 重放中 | 在隔离上下文运行，当前运行实例状态不变 |
| Q-time / SPC / FD 告警 | 关联到具体 wafer、工序与事件，不中断仿真 |
| 无数据 / 空场景 | 提供投料入口（演示触发） |
| 搜索无结果 / 下钻关闭 | 不改变或清空当前选中项 |

异常态不中断演示流程；导出失败与后端启动失败允许使用系统提示。

---

## 6. 明确不做

不做声光报警、用户权限体系、多租户、报表设计器、组态编辑器、实时视频融合、移动端适配、VR/AR、自动派工算法调参界面。

**对标但不实现的商业模块**：Predictive Metrology、Knowledge Advisor、Advanced Recipe Tuning、真机器学习与强化学习（minds.ai 相关能力）、多租户 Message Bus Pulsar 集群与跨区复制、Advanced Planning & Scheduling 数学求解器、CMMS 备件库存与工单、多厂数据集成、SEMI 合规认证、设备与软件捆绑交付。这些只在第 1 节做语义映射，不进入功能清单与验收。

**本规格不定义界面布局、窗口形态、视觉、配色、动效、快捷键与面板排布**；这些由 coding agent 决定，视觉方案另行提案交用户选定。

---

## 7. 参考依据（官方产品页，模块命名以此为准）

- `https://www.appliedmaterials.com/zh-hans/automation-software/products/industries`
- `https://appliedsmartfactory.com/zh-hans/semiconductor-blog/manufacturing-execution-zh-hans/cim-solution`
- `https://appliedsmartfactory.com/?p=473`
- `https://appliedsmartfactory.com/process-quality-solutions/`
- `https://appliedsmartfactory.com/ja/semiconductor/productivity-solutions`
- `https://appliedsmartfactory.com/semiconductor/productivity-solutions/production-control/`
- `https://appliedsmartfactory.com/zh-hans/process-quality-solutions/maintenance-management`
- `https://appliedsmartfactory.com/semiconductor-blog/quality/improve-yield-learning/`

SmartFactory CIM 交钥匙方案覆盖制造执行、工艺质量、生产效率、供应链四域；MES 300Works Full-Auto 整合物料控制、工厂事件、计划排程与设备维护；APF 与 Applied E3 分别承载生产效率与工艺质量；监控方案巡查所有 CIM 组件 。工艺质量模块清单含 Run-to-Run、SPC、Fault Detection、Recipe Management System、Equipment Automation、Asset Trace、Maintenance Management、Defect Classification / Defect Source，并叠加 AI 类模块 。生产效率模块清单含 RTD、FullAuto、Advanced Scheduling、Production Control、Activity Manager、Fusion、Simulation AutoSched、Material Control、Real Time Scheduling & Reporting 。Applied E3 产品族在应用材料招聘资料中明确列出 FDC、R2R、SPC、OCAP、RMS、EAP、Integration 。应用材料通过收购 Consilium（FAB300 / WorkStream）与 Brooks Software（PROMIS / FactoryWorks）构建 MES 产品线 ；这一段只作背景，不进入功能实现。

---

## 8. 功能验收点

| 编号 | 验收点 |
|---|---|
| A-01 | 三平台各自构建、打包、运行并产出运行证据 |
| A-02 | 第 1 节四域映射表逐项可在实现中核对，模块名与第 7 节官方来源一致 |
| A-03 | L0→L1→L2→L3 全链路可进入、可退回、可按 ID 直达，退回后选中项不丢 |
| A-04 | L3 能显示并复制 `event_id`，且能在 GreptimeDB 找到对应数据点 |
| A-05 | F-03 拆分 / 合并 / 返工 / Hold / Release 与逐片 genealogy 完整 |
| A-06 | F-11 Q-time 违规能定位到具体 wafer 与两个工序；F-07 SPC 超限能定位机台 |
| A-07 | F-13 每条派工决策显示命中的 What-Next / Where-Next 规则与依据 |
| A-08 | F-16 what-if 重放产出改动前后对比，且当前运行实例状态未被改写 |
| A-09 | F-20 巡查覆盖 PG / GreptimeDB / MinIO / `oxide-api` / outbox 投递者全部组件 |
| A-10 | chaos 三场景功能行为符合第 5 节，503 态不产生脏数据，报告仍含 RPO/RTO 数字 |
| A-11 | 减少动效开启后功能不减少，仅取消插值；三层下钻不依赖指针设备即可完成 |
| A-12 | 界面上每个可见数值均能追到第 4 节来源表并标注口径，无合成指标；无第 6 节商业模块被误实现 |
