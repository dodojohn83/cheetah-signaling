# Cheetah Signaling 下一轮外部编程执行计划

## 1. 文档定位

本目录是 [002 开发计划](../002_vibe_coding_plan/README.md) 的继任计划。002 保留为历史基线，不再通过补勾复原进度；本目录依据代码、生产装配、自动验证和验收报告重新判定完成度，并把所有未闭环内容转换为可直接交给外部编程执行体的任务。

本轮目标不是继续增加孤立 crate 或测试 skeleton，而是把已经存在的领域、存储、协议、集群和媒体组件装配成可运行的 edge/cluster 产品，完成 GB28181、ONVIF 与最新 `cheetah-media-server-rs` 的真实垂直闭环。

审计冻结点：

- `cheetah-signaling`: `f295c1b`
- `cheetah-media-server-rs-dev`: `d41ecbec4764519939d2b720141f275886a9bd8c`
- 审计日期：2026-07-19

后续执行若基线变化，必须在任务记录中写明新 commit、差异范围和重新验证结果。

## 2. 状态与证据规则

每个任务只能使用以下状态：

| 状态 | 判定 |
| --- | --- |
| `Completed` | 行为进入生产装配，正常与失败路径有测试，要求的验证命令或报告通过 |
| `Partial` | 类型、crate、局部实现或单元测试存在，但生产链路、故障语义或验收缺失 |
| `Not Implemented` | 没有实现，或生产入口明确返回 `NotImplemented`/占位 `Unsupported` |
| `Blocked` | 上游契约、工具链或外部交付未满足，执行体无法安全完成 |
| `Superseded` | 原任务已被新契约替代，必须给出替代任务 ID，不能直接删除 |

文件存在、代码可编译、单元测试存在均不能单独证明 `Completed`。完成证据至少包含：

1. 生产装配入口；
2. 对外行为或后台生命周期；
3. 正常、失败、超时、取消、重复和旧 epoch 测试；
4. 实际执行的命令与结果；
5. 需要真实系统/设备时的可复现报告。

002 中除已明确完成的 3 个 parser property-test 项外，其余未勾选项继续视为开放项；已存在实现的项目标为 `Partial`，不得因代码量较大而自动关闭。

## 3. 执行规则

1. 严格按 Phase 00–08 顺序推进；前一 Phase 的退出门禁未满足时，不得把后续依赖任务标为完成。
2. 每项任务使用本文档定义的稳定 ID，在 PR、commit、测试报告和上游 issue 中引用。
3. 先冻结契约和 contract test，再修改 adapter 与生产装配。
4. 不修改 002 的 checkbox；进度只记录在本目录对应任务后。
5. 发现与 001 冻结决策冲突时停止相关实现，先提交 ADR 和同步文档。
6. 禁止把 simulator、fake、固定成功或稳定 `Unsupported` 当作生产闭环。
7. 外部依赖未完成时可以实现本地 mapper、client、simulator 和失败路径，但对应真实联调任务保持 `Blocked`。
8. 每个任务完成后追加：commit、验证环境、命令、结果摘要和未运行测试。

## 4. 阶段索引

| Phase | 文档 | 退出结果 |
| --- | --- | --- |
| 00 | [01](01_002_completion_audit.md) | 002 所有任务有保守状态和后续归属 |
| 01 | [02](02_baseline_ci_and_quality_gate.md) | 官方工具链、codegen、CI 和质量门禁可复现 |
| 02 | [03](03_production_assembly_and_runtime_closure.md) | edge/cluster 生产装配不再忽略配置或使用占位 provider |
| 03 | [04](04_media_contract_alignment.md)、[05](05_media_registry_scheduler_client_and_events.md) | typed media 契约、节点管理、client 和事件闭环 |
| 04 | [06](06_media_workflows_and_reconciliation.md) | 四模型媒体 Saga、补偿和对账闭环 |
| 05 | [07](07_gb28181_and_onvif_vertical_completion.md) | GB28181/ONVIF 真实协议垂直链路 |
| 06 | [08](08_api_cluster_plugin_security_completion.md) | 北向 API、cluster、plugin、安全和运维闭环 |
| 07 | [09](09_system_interop_performance_and_release.md) | 系统、真实互操作、性能和发布报告 |
| Upstream | [90](90_media_server_upstream_requirements.md) | 可独立转交媒体仓库的 P0/P1/P2 要求 |

## 5. 关键路径

```text
AUD-001
  -> BAS-001..006
  -> MED-C-001..008
  -> [UP-MEDIA-P0] + MED-R-001..008
  -> WF-001..008
  -> GB-001..007 / ONVIF-001..007
  -> PROD-001..008
  -> SYS-001..009
```

媒体上游 P0 与信令 simulator/client 可以并行；真实媒体 contract、GB 开流和 ONVIF 拉流必须等待 P0 完成。

## 6. 全局完成定义

- 002 每个开放要求都已由本目录任务关闭、替代或明确移出 v1，且有证据。
- 主进程不存在 `UnsupportedMediaPort`、开发固定 secret、忽略 NATS/cluster/plugin/ONVIF 配置等生产占位。
- edge SQLite/local bus 与 cluster PostgreSQL/NATS 使用同一 application 行为和 contract suite。
- fake media 与真实媒体节点通过同一 typed gRPC contract suite。
- GB28181 和 ONVIF 从北向请求到设备、媒体、事件、Operation 终态形成真实闭环。
- 所有 public API 无未登记 `NotImplemented`；v1 外能力通过 capability 返回稳定 `Unsupported`。
- 格式化、clippy、nextest、Buf、breaking、migration、deny、架构检查全部通过。
- ARM smoke、真实互操作、故障注入、百万设备和 72 小时 soak 具有可复现报告。
- 信令进程不绑定或处理 RTP/RTCP/PS/TS/ES 媒体负载。

