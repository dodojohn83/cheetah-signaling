# Cheetah Signaling 外部编程执行计划

## 1. 文档定位

本目录把 [001 架构设计](../001_next_generation_signaling/README.md) 转换为可直接执行的开发任务。执行体不能依赖未写入本目录的隐含决策，也不能以参考仓库现状替代本计划定义的接口、错误和验收语义。

本文档集只规定 v1 实施顺序：Rust/Edition 2024、edge + cluster、原生多租户、GB28181 设备接入与级联、ONVIF client/controller、独立 Media Plane、REST/SSE/Webhook、进程插件、百万在线设备。

## 2. 执行规则

1. 严格按章节依赖顺序实施；某阶段完成条件未满足时不得把下游占位标记为完成。
2. 每项 `[ ]` 对应可独立评审的代码/测试交付。完成后改成 `[x]`，在条目后追加 commit、测试命令和结果摘要。
3. 禁止空 provider、`todo!()`、`unimplemented!()`、HTTP 200 假成功或吞掉错误。未实现能力返回稳定 `Unsupported`。
4. 公共契约先于 adapter；先写 contract test，再实现 SQLite/PostgreSQL、local/NATS、fake/real media。
5. 所有队列、缓存、批次、分页、解析、定时器、重试和并发均需上限与过载测试。
6. 任何改变 crate、Proto、REST、表结构、NATS subject 或 001 架构边界的实现，必须先更新本计划和对应设计文档。
7. 执行体不可复制 `vendor-ref` 或媒体仓库业务代码；可提取已验证 fixture，但必须记录来源、许可证和脱敏方式。

## 3. 文档索引与阶段

| Phase | 文档 | 交付 |
| --- | --- | --- |
| 0 | [01](01_execution_contract_and_baseline.md)、[02](02_workspace_bootstrap_and_ci.md)、[03](03_crate_graph_and_dependency_rules.md) | 工程、CI、crate 图 |
| 1 | [04](04_foundation_types_errors_config.md)、[05](05_proto_contracts_and_codegen.md)、[06](06_domain_and_application_services.md) | 公共契约和领域内核 |
| 2 | [07](07_runtime_sharding_and_timers.md)、[08](08_storage_sqlite_postgres.md)、[09](09_messaging_outbox_and_ownership.md) | edge 内核与持久性 |
| 3 | [10](10_northbound_http_and_events.md)、[11](11_media_registry_scheduler_and_client.md) | 北向 API 与媒体面 |
| 4 | [12](12_gb28181_sip_core.md)、[13](13_gb28181_device_access.md)、[14](14_gb28181_media_operations.md)、[15](15_gb28181_cascade_and_compatibility.md) | GB28181 完整闭环 |
| 5 | [16](16_onvif_core_discovery_and_security.md)、[17](17_onvif_services_and_workflows.md) | ONVIF 完整闭环 |
| 6 | [18](18_cluster_ha_and_reconciliation.md) | cluster/HA |
| 7 | [19](19_plugin_sdk_and_host.md)、[20](20_security_observability_and_operations.md) | 插件与生产加固 |
| 8 | [21](21_testing_simulators_and_performance.md)、[22](22_packaging_migration_and_release.md) | 百万设备与发布 |

## 4. 全局完成定义

- [ ] 01–22 的任务与验收全部完成，无未登记 TODO。
- [ ] `cargo fmt --check`、workspace clippy/test、Proto/OpenAPI breaking、migration、license/advisory 全通过。
- [ ] edge SQLite/local bus 与 cluster PostgreSQL/NATS 通过同一 contract suite。
- [ ] fake media 与真实 `cheetah-media-server-rs` 通过同一媒体 contract suite。
- [ ] GB 海康/大华设备、NVR、上级和下级平台完成真实互操作。
- [ ] ONVIF Profile T/Media2 与 legacy Media1 设备完成真实互操作。
- [ ] gateway/media/workflow/NATS/PostgreSQL 故障验证 fencing、补偿和 15/30 秒目标。
- [ ] 100 万在线容量、重注册风暴和 72 小时 soak 报告可复现。
- [ ] 信令进程未引入任何 RTP/RTCP/PS/ES 媒体负载处理。

## 5. 需求覆盖矩阵

| 001 要求 | 002 归属 |
| --- | --- |
| edge/cluster 双部署 | 02、07–09、18、22 |
| 百万在线与分片 worker | 07、21 |
| 多租户领域模型 | 04、06、08、10 |
| 统一 Protobuf/插件 | 05、19 |
| SQLite/PostgreSQL | 08 |
| NATS/ownership/fencing | 09、18 |
| Media Plane 解耦 | 11、14、17、22 |
| GB 设备接入/级联 | 12–15 |
| ONVIF Profile T/Media2 | 16–17 |
| REST/SSE/Webhook | 10 |
| 安全/可观测/十年演进 | 20、22 |
| 互操作/chaos/百万验收 | 21 |
