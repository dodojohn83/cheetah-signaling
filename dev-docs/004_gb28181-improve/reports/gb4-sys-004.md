# GB4-SYS-004：上级/下级平台级联互操作报告

- 任务：`GB4-SYS-004`
- 状态：`Blocked`
- 日期：2026-07-21

## 1. 目标

完成至少一组上级/下级 GB28181 平台级联互操作验证，覆盖 GB/T 28181-2022 第 9 章级联场景：

- 下级平台向上级注册/注销、心跳保活；
- 上级向下级发起目录查询、订阅/通知（catalog/alarm/status/mobile position）；
- 上下级之间直播/回放/下载/对讲的 SIP/SDP 桥接协商；
- 多上级隔离、ACL、loop/hop 检测、唯一 control owner；
- NAT/endpoint 路由、TCP/UDP 传输、Digest 认证和跨平台兼容性 profile。

## 2. 测试矩阵

| 维度 | 上级平台 | 下级平台 |
| --- | --- | --- |
| 标准 | GB/T 28181-2022 | GB/T 28181-2022 或 2016（MD5 profile） |
| 方向 | 向上级注册、接收查询/订阅/控制 | 向下级下发查询/订阅/控制 |
| 传输 | UDP / TCP | UDP / TCP |
| 认证 | Digest SHA-256 | Digest MD5 / SHA-256 |
| 网络 | 公网 NAT / 专网 | 专网 / NAT |

## 3. 已准备的验收 checklist

- [ ] 下级平台成功向上级 REGISTER，并周期 keepalive；
- [ ] 上级目录查询（`Catalog`）下级设备/通道，分片完整聚合；
- [ ] 上级订阅下级 alarm、status、mobile position，NOTIFY 正常到达并解析；
- [ ] 上级发起桥接 live/playback/download/talk INVITE，下级 200 OK SDP 协商成功；
- [ ] 桥接媒体由 `MediaPort` 控制，信令层不处理 RTP/RTCP/PS/TS/ES；
- [ ] CANCEL/BYE 后桥接会话正确终止；
- [ ] 多上级同时注册时互不影响，目录/control owner 隔离；
- [ ] loop/hop 检测拒绝自环或超 hop 路由；
- [ ] ACL 正确拒绝未授权 catalog/bridge/control；
- [ ] 断网/重启/重复响应后状态恢复。

## 4. 记录模板

每项验证记录：
- 上级/下级平台厂商、版本、标准声明；
- 平台 ID、domain、realm、本地 SIP endpoint；
- 网络拓扑（source/observed/Contact/Via received-rport、NAT 类型）；
- 标准版本与 compatibility profile；
- 脱敏 semantic transcript（无 Authorization/密码/完整 body）；
- 不支持能力列表与对应 fallback 行为；
- 跨平台差异和 profile 启用记录。

## 5. 当前已实现控制面基线

`GB4-CAS-001..006` 已在本仓库实现级联控制面模型与单元/contract 覆盖，为真实平台互操作提供可验证基线：

- `GbPlatformLink` 聚合、`PlatformLinkRepository` 与 SQLite/PostgreSQL 双后端 contract；
- `CascadeManager` 多上级隔离、下级接入、平台身份校验、ACL、`detect_loop`、`MAX_CASCADE_HOPS`、唯一 control owner；
- 单链路 `Gb28181Cascade` 状态机支持 REGISTER/keepalive/deregister、目录查询/通知、SUBSCRIBE/NOTIFY、桥接 saga；
- `tools/gb28181-simulator` 的 `platform` 模块可模拟上下级注册、目录和桥接握手，作为本地 pre-interop 冒烟 harness。

这些实现通过 `cargo test`、`cargo clippy`、架构审查和 fixture 校验；真实上下级平台互操作证据仍需外部平台接入后补充。

## 6. 验收 checklist 与本地预验证映射

以下映射说明 checklist 中每项已在本地控制面通过哪些 `GB4-*` 任务、系统测试或 simulator 预验证；真实上下级平台证据仍需在获得外部对端后补充。

| Checklist | 本地预验证 | 证据位置 |
| --- | --- | --- |
| 下级平台成功向上级 REGISTER，并周期 keepalive | `GB4-CAS-001..006` 级联平台模型；`CascadeManager` 多上级隔离、下级接入、平台身份校验；`cheetah-state-machine-tests/tests/cascade.rs` 状态机单测 | `crates/protocols/cheetah-gb28181-module/src/cascade/`、`src/cascade/tests.rs`；`crates/testing/cheetah-state-machine-tests/tests/cascade.rs` |
| 上级目录查询（`Catalog`）下级设备/通道，分片完整聚合 | `GB4-ACC-005` bounded catalog aggregation；`GB4-CAS-001..006` catalog filter/whitelist/tenant 隔离；`cheetah-state-machine-tests/tests/catalog.rs` | `apps/cheetah-signaling/src/gb_catalog_buffer.rs`；`crates/protocols/cheetah-gb28181-module/src/cascade/catalog.rs`、`src/cascade/tests/catalog.rs`；`crates/testing/cheetah-state-machine-tests/tests/catalog.rs` |
| 上级订阅下级 alarm、status、mobile position，NOTIFY 正常到达并解析 | `GB4-CAS-001..006` subscription/notify 状态机；`GB4-EVT-001` 事件落库；`cheetah-gb28181-module/src/cascade/tests/subscription.rs` | `crates/protocols/cheetah-gb28181-module/src/cascade/subscription/mod.rs`、`src/cascade/tests/subscription.rs` |
| 上级发起桥接 live/playback/download/talk INVITE，下级 200 OK SDP 协商成功 | `GB4-MED-001..008`、`GB4-WF-001..004` workflow；`GB4-CAS-001..006` 桥接 saga；`cheetah-state-machine-tests/tests/media.rs`、`tests/cascade.rs` | `crates/protocols/cheetah-gb28181-module/src/cascade/tests/bridge.rs`；`crates/testing/cheetah-state-machine-tests/tests/media.rs`、`tests/cascade.rs`；`crates/testing/cheetah-gb-system-tests/tests/gb4_sys_002_cluster.rs` |
| 桥接媒体由 `MediaPort` 控制，信令层不处理 RTP/RTCP/PS/TS/ES | `GB4-MED-001..008` `MediaPort` contract；`MediaSession`/`MediaBinding`/`Operation` 四模型；`GB4-SYS-001/002` edge/cluster 全纵向测试 | `crates/media/cheetah-media-scheduler/`；`crates/protocols/cheetah-gb28181-module/src/media/session.rs`；`crates/testing/cheetah-gb-system-tests/tests/gb4_sys_002_cluster.rs` |
| CANCEL/BYE 后桥接会话正确终止 | `GB4-CAS-001..006` 桥接状态机；`GB4-MED-001..008` stop/release；`GB4-WF-004` stop saga | `crates/protocols/cheetah-gb28181-module/src/cascade/tests/bridge.rs`；`crates/protocols/cheetah-gb28181-module/src/media/tests/bye_tests.rs`；`crates/testing/cheetah-state-machine-tests/tests/media.rs` |
| 多上级同时注册时互不影响，目录/control owner 隔离 | `GB4-CAS-001..006` `PlatformLink` tenant/owner/ACL；`CascadeManager` 多上级隔离；`cheetah-state-machine-tests/tests/cascade.rs` | `crates/protocols/cheetah-gb28181-module/src/cascade/tests.rs`；`crates/testing/cheetah-state-machine-tests/tests/cascade.rs` |
| loop/hop 检测拒绝自环或超 hop 路由 | `GB4-CAS-001..006` `detect_loop`、`MAX_CASCADE_HOPS`；`EndpointPolicy` zone 校验 | `crates/protocols/cheetah-gb28181-module/src/cascade/tests.rs` |
| ACL 正确拒绝未授权 catalog/bridge/control | `GB4-CAS-001..006` `CatalogFilter` tenant/whitelist/tag/org-prefix；`PlatformLink` 平台 ACL | `crates/protocols/cheetah-gb28181-module/src/cascade/tests/catalog_security.rs`、`src/cascade/tests/catalog.rs` |
| 断网/重启/重复响应后状态恢复 | `GB4-SYS-006` chaos/rolling upgrade；`GB4-TST-004` deterministic fault DSL；`owner epoch`、`link generation`、`revision` 与 reconcile | `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_006_chaos.rs`、`gb4_sys_002_cluster.rs`；`tools/gb28181-simulator` |

## 7. 阻塞原因

当前环境未接入真实的 GB28181 上级或下级平台；报告将在获得真实平台对端后补充。
