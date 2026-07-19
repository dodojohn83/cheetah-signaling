# 09. 系统、互操作、性能与发布验收

## 1. 原则

ignored benchmark、simulator单测和一次本地成功不能证明容量或生产可用。所有报告记录commit、硬件、OS/kernel、配置、数据集、协议比例、持续时间和原始结果位置。

## 2. SYS-001：Edge 系统测试

- SQLite/local bus、单一signaling实例和至少一个真实media进程。
- GB设备注册/目录/live/stop/restart恢复。
- ONVIF纳管/pull/snapshot/stop。
- Operation/MediaSession/Binding查询一致。
- ARM64设备执行安装、启动、基本开流和升级smoke。

## 3. SYS-002：Cluster 系统测试

- 三个signaling、三个media、PostgreSQL和NATS JetStream。
- device均衡、owner接管、media调度/扩缩/drain。
- signaling/media/NATS/PostgreSQL分别kill/restart。
- rolling N→N+1期间持续注册、命令和媒体会话。
- 验证旧epoch、重复消息和晚回调不推进新状态。

## 4. SYS-003：统一 media contract

同一测试二进制/fixture运行：

1. deterministic media simulator；
2. 最新真实`cheetah-media-server-rs` gRPC adapter。

覆盖register/heartbeat/drain、capability、RTP、proxy、snapshot、record、playback、URL、events、idempotency、fencing、deadline和reconciliation。

## 5. SYS-004：协议 simulator

- GB simulator支持UDP/TCP、digest、catalog分页、alarm、INVITE/CANCEL/BYE、playback/talk和fault injection。
- ONVIF simulator支持Discovery、Media1/2、WS-Security、clock skew、RTSP/Snapshot URI和慢/畸形响应。
- fault由seed驱动并在失败输出；不得访问公网或固定公共端口。

## 6. SYS-005：真实互操作

报告至少包含：

- GB：海康、大华、NVR、上级和下级平台；
- ONVIF：Profile T/Media2和legacy Media1；
- firmware/profile、网络拓扑、TLS/认证、codec/transport；
- 成功率、首帧、停止残留、已启用quirk；
- 脱敏transcript/pcap来源与许可证。

任何未具备的设备明确标记`Not Run`，不得用simulator替代。

## 7. SYS-006：Chaos

逐一和组合注入：

- DB连接池耗尽/延迟/短断；
- NATS断线、redelivery、consumer重启；
- media command响应丢失、event gap、instance重启；
- owner lease暂停和网络分区；
- protocol socket queue满、设备风暴；
- webhook/telemetry变慢。

通过条件：资源有界、无双owner副作用、desired state最终收敛、恢复后backlog在目标时间清空。

## 8. SYS-007：容量与风暴

场景必须声明：

- 设备总量与online比例；
- GB/ONVIF比例；
- heartbeat/presence周期；
- TLS比例和报文大小；
- 1%/5%/10% active media操作；
- tenant数量与热点分布。

执行：

- edge ARM建议容量基线；
- cluster 10万、50万、100万连接阶梯；
- 10%/50%抖动窗口重注册；
- 大目录、alarm洪水和慢ONVIF；
- NATS/DB/media降速过载。

报告CPU、RSS、socket/FD、queue、timer lag、DB/NATS、P50/P95/P99、错误和恢复时间。不得仅给“支持100万”结论。

## 9. SYS-008：72 小时 soak

- 固定稳定负载加周期性故障。
- 检查RSS、task、socket、timer、cache、outbox/inbox、dead-letter和media orphan趋势。
- 定义允许斜率和告警阈值。
- 每24小时执行抽样start/stop/reconcile正确性。
- 结束后有界shutdown并验证无残留资源。

## 10. SYS-009：发布

- 生成SBOM、license/advisory、checksum和签名。
- edge与cluster容器/包使用non-root、read-only filesystem和最小capability。
- 配置示例不含secret，migration和rollback/runbook齐全。
- Proto/OpenAPI/SQL/plugin SDK有compatibility说明。
- 发布门禁引用全部测试报告；高成本测试未运行则禁止GA，只能标preview。

## 11. 最终验收

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
buf format --diff --exit-code
buf lint
buf breaking --against <released-baseline>
cargo deny check
```

追加双数据库migration/contract、feature/architecture、x86_64/aarch64、real media、interop、chaos、capacity和soak报告。

