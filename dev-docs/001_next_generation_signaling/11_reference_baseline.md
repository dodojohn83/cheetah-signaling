# 11. 标准与技术参考基线

## 1. 使用规则

本文件记录设计阶段使用的公开基线。实现时应把实际采用的标准版本、crate/tool 版本和兼容偏差写入 release/compatibility matrix。

标准原文优先于开源实现；开源实现用于发现真实设备行为和测试场景，不能替代标准，也不能在未检查许可证时复制代码或 fixture。

## 2. 协议标准

### 2.1 GB28181

- [GB/T 28181-2022 国家标准信息](https://openstd.samr.gov.cn/bzgk/std/newGbInfo?hcno=8BBC2475624A6C31DC34A28052B3923D)：2022-12-30 发布，2023-07-01 实施，当前主设计基线。
- GB/T 28181-2016：作为存量设备兼容基线；任何与 2022 的差异必须进入显式 compat profile。
- [RFC 3261: SIP](https://www.rfc-editor.org/rfc/rfc3261)
- [RFC 4566: SDP](https://www.rfc-editor.org/rfc/rfc4566)
- [RFC 7616: HTTP Digest](https://www.rfc-editor.org/rfc/rfc7616)：GB 设备仍可能只实现历史算法，现代算法要求不得反向假定设备支持。

### 2.2 ONVIF

- [ONVIF Network Interface Specifications](https://www.onvif.org/profiles/specifications/)
- [ONVIF Profiles](https://www.onvif.org/profiles/)
- [Profile T](https://www.onvif.org/profiles/profile-t/)：v1 首选视频 Profile。
- [Profile S deprecation Q&A](https://www.onvif.org/profiles/profile-s/profile-s-deprecation-qna/)：支持将在 2027-03-31 结束，项目只保留受控 legacy 兼容。
- [ONVIF Client Test Specifications](https://www.onvif.org/profiles/conformance/client-test/)：用于建立功能和 conformance 测试清单。
- [OASIS WS-Discovery 1.1](https://docs.oasis-open.org/ws-dd/discovery/1.1/os/wsdd-discovery-1.1-spec-os.html)
- [OASIS Web Services Security UsernameToken Profile](https://docs.oasis-open.org/wss-m/wss/v1.1.1/os/wss-UsernameTokenProfile-v1.1.1-os.html)

## 3. 内部协议与 API

- [Protocol Buffers proto3 language guide](https://protobuf.dev/programming-guides/proto3/)：字段演进、unknown field 和 enum 兼容规则。
- [Buf breaking change detection](https://buf.build/docs/breaking/)：Proto schema CI 门禁。
- [gRPC](https://grpc.io/docs/what-is-grpc/introduction/)：插件与媒体节点跨进程传输。
- [RFC 9457: Problem Details for HTTP APIs](https://www.rfc-editor.org/rfc/rfc9457)：REST 统一错误模型。
- [OpenAPI 3.1 Specification](https://spec.openapis.org/oas/v3.1.1.html)
- [W3C Trace Context](https://www.w3.org/TR/trace-context/)：跨 REST/NATS/gRPC 的 traceparent/tracestate。

## 4. Rust 与基础设施

- [Rust 官方发布记录](https://blog.rust-lang.org/releases/)：设计时最新稳定版为 Rust 1.96.1；工程实际基线由 `rust-toolchain.toml` 固定。
- [Rust 2024 Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
- [Tokio](https://tokio.rs/)：首个异步运行时，只允许出现在 driver/adapter/app 装配边界。
- [SQLx](https://github.com/launchbadge/sqlx)：SQLite/PostgreSQL adapter 候选；设计不依赖 `Any` driver。
- [NATS JetStream](https://docs.nats.io/nats-concepts/jetstream)：持久命令/事件、consumer 和重放。
- [NATS KV](https://docs.nats.io/nats-concepts/jetstream/key-value-store)：节点租约、CAS ownership 和 watcher。
- [OpenTelemetry](https://opentelemetry.io/docs/)：trace/metric/log 语义；具体 Rust SDK 版本需在实现阶段验证成熟度。

## 5. 项目与参考实现

- [`cheetah-media-server-rs` 媒体 API 规划](../../../cheetah-media-server-rs/dev-docs/901_api_plan/README.md)：MediaKey、MediaControlApi、RTP、事件与信令边界的直接上游契约。
- [epimore/gmv](https://github.com/epimore/gmv)：Rust GB28181 实现和真实设备兼容参考。只借鉴测试场景与工程经验，不复制其信令/媒体耦合边界。
- `cheetah-media-server-rs` 现有 GB28181/RTP 代码：用于提取已验证 fixture、SSRC/transport quirk 和迁移回归，不作为新信令服务运行时依赖。

## 6. 版本冻结要求

进入实现 Phase 0 时必须生成机器可读清单：

- Rust toolchain、target 和 MSRV；
- Proto/Buf/protoc 与生成插件；
- Tokio、HTTP、gRPC、XML、SQL、NATS、TLS、crypto crate；
- PostgreSQL、SQLite、NATS server 的最小/测试版本；
- GB/ONVIF 标准与测试规范版本；
- media contract crate/version。

升级清单中的 major/minor 版本必须运行协议 fixture、wire breaking、migration、性能和 ARM 构建测试，不能只依赖 SemVer 或 `cargo update` 成功。
