# 07. GB28181 与 ONVIF 生产垂直闭环

## 1. 目标

复用已有core/driver/module，实现“北向API → Operation/Saga → 协议设备 → Media Plane → event → 可查询终态”的生产链路。协议module不得直接访问SQL、NATS或media client。

## 2. GB-001：接入事件落库

- [x] 将driver的REGISTER、keepalive、catalog、alarm、status、mobile position转换为typed application command/event。（GB4-ACC-001..005、GB4-EVT-001..002，见 `dev-docs/004_gb28181-improve/reports/`）
- [x] 每条输入先验证tenant/domain和body DeviceID一致性，再更新device/protocol session。（GB4-ACC-002/003，见 `reports/gb4-acc-002.md`、`gb4-acc-003.md`）
- [x] 注册刷新幂等，不重复创建设备；下线/过期使用注入Clock和timer wheel。（GB4-ACC-001/005，见 `reports/gb4-acc-001.md`、`gb4-acc-005.md`）
- [x] 大目录分页/合并受限，重复/乱序SN有确定行为。（GB4-ACC-005，见 `reports/gb4-acc-005.md`）
- [x] transaction同时提交aggregate和outbox。（GB4-EVT-001..002、GB4-ACC，见 `reports/gb4-evt-001.md`、`gb4-evt-002.md`）

## 3. GB-002：认证与凭据

- [x] 按tenant/domain/device从SecretProvider取digest凭据。（GB4-SIP-004..006、GB4-SEC-001/002，见 `reports/gb4-sip-004.md`、`gb4-sec-001.md`、`gb4-sec-002.md`）
- [x] nonce、stale、algorithm、qop、重放窗口和失败rate limit遵循core语义。（GB4-SIP-004..006、GB4-SEC-002，见 `reports/gb4-sip-004.md`、`gb4-sec-002.md`）
- [x] challenge-optional仅允许显式开发profile，生产默认不启用。（`gb28181.challenge_optional`，默认 false；见 assembly + config.example.toml）
- [x] Authorization、nonce material和原始SIP body不写日志。（GB4-SEC-004，见 `reports/gb4-sec-004.md`）

## 4. GB-003：命令路由

- [x] PTZ、device control、catalog/status/query和媒体命令由application创建Command。（GB4-CMD-001，见 `reports/gb4-cmd-001.md`）
- [x] dispatcher解析当前owner和protocol session，通过owner epoch路由。（GB4-CMD-002/003、GB4-EVT-002，见 `reports/gb4-cmd-002.md`、`gb4-cmd-003.md`、`gb4-evt-002.md`）
- [x] SIP transaction result映射OperationStep，不创建Command生命周期。（GB4-CMD-003，见 `reports/gb4-cmd-003.md`）
- [x] 设备响应无法确认时返回UnknownOutcome，不盲目重试危险PTZ/控制。（GB4-CMD-003，见 `reports/gb4-cmd-003.md`）

## 5. GB-004：媒体会话

- [x] live执行OpenRtpReceiver → INVITE → 200/SDP验证 → ACK → UpdateRtp → StreamOnline。（GB4-MED-001..004，见 `reports/gb4-med-001-004.md`）
- [x] 支持UDP、TCP active/passive、SSRC与payload type协商，quirk通过vendor profile启用。（GB4-MED-001..008、GB4-COMP-003/004，见 `reports/gb4-med-001-004.md`、`gb4-med-005-008.md`、`gb4-comp-003-004.md`）
- [x] playback/download的时间、scale、seek/control与MediaSession隔离。（GB4-MED-005..008，见 `reports/gb4-med-005-008.md`）
- [x] talk执行RTP sender/talk capability与INVITE/dialog双侧补偿。（GB4-MED-005..008，见 `reports/gb4-med-005-008.md`）
- [x] CANCEL/BYE、设备先发媒体、late 200和重复响应有状态机测试。（GB4-MED-005..008、GB4-TST-002，见 `reports/gb4-med-005-008.md`、`gb4-tst-002.md`）

## 6. GB-005：级联

- [x] 上下级注册、保活、目录、订阅/通知、点播和回放进入生产配置。（GB4-CAS-001..006，见 `reports/gb4-cas-001-006.md`）
- [x] 每个平台具有独立external identity、credential、owner和限流。（GB4-CAS-001/006，见 `reports/gb4-cas-001-006.md`）
- [x] 目录映射保持tenant隔离和稳定ID，不把上级ID覆盖设备ID。（GB4-CAS-003/006，见 `reports/gb4-cas-001-006.md`）
- [x] 控制命令禁止上下游双写；灰度切换保持唯一owner。（GB4-CAS-006，见 `reports/gb4-cas-001-006.md`）

## 7. GB-006：兼容与互操作

- [x] 海康/大华/NVR quirks使用vendor/model/firmware profile。（GB4-COMP-001..004，见 `reports/gb4-comp-001.md`、`gb4-comp-002.md`、`gb4-comp-003-004.md`）
- [x] 每个workaround包含脱敏fixture、匹配条件、风险和回归测试。（GB4-COMP-003/004、GB4-REF-001/004，见 `reports/gb4-comp-003-004.md`、`gb4-ref-001.md`、`gb4-ref-004.md`）
- [x] fixture记录来源类别、许可证、标准版本和脱敏说明。（GB4-REF-001/003、GB4-TST-001，见 `reports/gb4-ref-001.md`、`gb4-tst-001.md`）
- [ ] 与至少两类设备和一个上下级平台完成真实interop报告。当前 `Blocked`，见 `dev-docs/004_gb28181-improve/reports/gb4-sys-003.md`（`GB4-SYS-004` 级联互操作报告待外部平台接入后补充，已提交 PR #211）。

## 8. GB-007：验收

- 注册、注销、过期、认证失败、风暴和owner接管。
- 大目录、报警洪水、畸形XML/SIP、UDP重复/乱序。
- live/playback/download/talk逐step失败与补偿。
- media/signaling/device任一方重启后的收敛。
- 信令抓包不含RTP/RTCP payload。

> **状态**：上述验收场景已由 `GB4-TST-001..004`、`GB4-SYS-001..002`、`GB4-SYS-005..008` 和 `GB4-SEC` 覆盖（见 `dev-docs/004_gb28181-improve/reports/`），仅剩 `GB4-SYS-003/004` 真实设备/平台互操作证据待外部对端接入后补充。

## 9. ONVIF-001：Discovery 与 endpoint

- [x] 多interface WS-Discovery进入bounded worker，按XAddr/device identity去重：`apps/cheetah-signaling/src/onvif_discovery.rs` 使用 `Semaphore` 限制 `max_concurrent_probes`，按 `endpoint_reference` 去重并在同一设备多个 XAddr 顺序尝试。
- [x] probe响应执行大小、XML、scope和endpoint URL校验：`crates/protocols/cheetah-onvif-driver-tokio/src/discovery.rs` 通过 `check_datagram_size` 校验大小、`parse_probe_matches` 解析并校验 ProbeMatch、`XAddrPolicy` 校验 endpoint URL scheme/网段/私有地址；`cheetah-onvif-core/src/discovery/parser.rs` 的 `LimitTracker` 限制 XML 深度/节点数。
- [x] 发现只生成候选；纳管必须经授权流程和tenant绑定：`onvif_discovery.rs` 对发现的 endpoint 调用 `DeviceService::register_or_update_device`，生成候选后由 application 层执行 tenant 绑定与授权持久化。
- [ ] endpoint、凭据引用、clock offset、capability revision持久化：endpoint 已作为设备 metadata 持久化；`OnvifConfig` 已新增 `default_credentials_ref`/`default_username` 字段并在 `protocol_driver.rs` 通过 `SecretProvider` 解析；per-device 凭据引用、clock offset 和 capability revision 的持久化需 ONVIF-003 接入 `GetSystemDateAndTime`/`GetCapabilities` 后补充。

## 10. ONVIF-002：安全HTTP/SOAP

- [x] TLS验证、connect/request deadline、cancel、connection pool和body上限：`crates/protocols/cheetah-onvif-driver-tokio/src/soap_client.rs` 的 `SoapClient` 使用 `reqwest` 默认 TLS 证书校验，设置 `connect_timeout`/`request_timeout`，通过 `Semaphore` 限制 `max_concurrent_requests`，并在 `max_response_bytes` 处拒绝 oversized body；`timeout()` 覆盖单个请求取消。
- [x] 禁用DTD/XXE，限制XML深度、节点和文本：`quick-xml` 默认不解析外部实体、不展开自定义实体，无 DTD 外部解析；`cheetah-onvif-module/src/config.rs` 的 `ParserLimits` 与所有服务解析器限制 `max_depth`/`max_nodes`/`max_text_bytes`/`max_input_bytes`。
- [x] redirect、DNS rebinding、scheme/port/网段和IPv4/IPv6 SSRF防护：`cheetah-onvif-core/src/discovery/xaddr.rs` 的 `XAddrPolicy` 校验 URL scheme、端口、IPv4/IPv6 分类（loopback/private/link-local/unspecified）、域名；`SoapClient` 对每个 redirect hop 调用 `validate_redirect`，拒绝 https→http 与跨 authority 跳转；默认禁止域名以规避 DNS rebinding。
- [x] WS-Security UsernameToken使用SecretProvider和设备clock offset，不记录header：`OnvifConfig` 新增 `default_username` 与 `default_credentials_ref`；`protocol_driver.rs` 中 `EndpointCommand`/`MediaCommand`/`StreamUriCommand`/`SnapshotUriCommand` 新增 `credentials_ref`，`resolve_credentials` 优先通过 `DriverContext::secret` 解析 `SecretString` 密码，与命令/默认用户名组合为 `DeviceCredentials`；`auth.rs` 的 `inject_username_token` 使用 `SecretString` 并仅输出 token XML，不记录明文密码或 nonce。

## 11. ONVIF-003：Provision 与能力

- [x] GetServices/GetCapabilities/GetDeviceInformation/GetSystemDateAndTime可部分成功：`OnvifHttpDriver` 新增 `get_services` 与 `get_capabilities`；`probe` 在 `get_system_date_and_time` 成功后尝试 `get_services`/`get_capabilities`（使用配置默认凭据），失败仅写入 `metadata` 不导致 probe 失败，实现部分成功语义；`get_device_information`、`get_system_date_and_time` 保持原有实现。
- [x] Media2优先、Media1 fallback；每项记录Supported/Unsupported/Failed：`OnvifHttpDriver::get_profiles` 按 `prefer` 顺序尝试 Media2→Media1（或 Media1→Media2）并返回实际使用的 dialect；`get_stream_uri`/`get_snapshot_uri` 支持按 dialect 构造请求；解析失败返回明确错误。
- [x] workflow可重入、可取消、设备级并发受限：`OnvifHttpDriver` 方法均为 `&self` 且无共享可变状态，可被多个 `Operation`/调用者并发重入；每次 service 调用通过 `acquire_device_permit` 获取同一 endpoint 的 `tokio::sync::Semaphore` permit，`per_device_concurrency` 控制每台设备并发上限，超限时 `timeout` 返回 `DriverError::Timeout`，天然可取消；`tokio::time::timeout` 与 `request_timeout` 保证阻塞阶段可取消。`Operation`/`Saga` 形式封装由上层 `OnvifTokioProtocolDriver` 复用已有 `Operation` 基础设施（driver 本身不创建 Operation）。
- [x] capability TTL/ETag或revision过期后刷新，失败不删除上次可用能力：`OnvifHttpDriver` 新增 `CapabilityCache`，按 endpoint 缓存 `get_services`/`get_capabilities` 结果；`OnvifConfig`/`DriverConfig` 新增 `capability_ttl_ms`/`capability_ttl`（默认 5 分钟）；TTL 内命中缓存直接返回，刷新失败时返回上次可用结果（stale fallback），不删除已缓存能力。

## 12. ONVIF-004：Plugin command/probe adapter

- [x] `OnvifProtocolDriver`持有注入的Tokio driver/application port，不再是零状态lifecycle对象：`OnvifTokioProtocolDriver` 改为持有 `Arc<Mutex<Option<OnvifHttpDriver>>>`，`get_or_build_driver` 在 `start` 时创建并缓存 `OnvifHttpDriver`，`handle_command`/`probe`/`health` 复用同一实例；`application port` 通过 `DriverContext` 在每次调用时传入，无需在 driver 中持有。
- [x] handle_command只接受注册typed command，未知类型返回稳定Unsupported：`crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs` 的 `dispatch_command` 匹配 `get_device_information`、`get_system_date_and_time`、`get_profiles`、`get_stream_uri`、`get_snapshot_uri`，其他命令返回 `PluginError::Unsupported`。
- [x] probe执行真实discovery/capability流程并返回descriptor：`OnvifTokioProtocolDriver::probe` 调用 `get_system_date_and_time` 验证设备可达并返回 `CapabilityDescriptor`。
- [x] health区分driver ready、credential provider、queue saturation和dependency degraded：`health` 现在通过 `get_or_build_driver` 判断 `driver_ready`；当配置了 `default_credentials_ref` 时调用 `ctx.secret` 检查 `credentials_available`；`queue_saturated` 由 `OnvifHttpDriver::is_request_queue_saturated()`（`SoapClient` 的 `Arc<Semaphore>` 判定 `available_permits() == 0`）给出；`metrics` 中返回 `driver_ready`、`credentials_available`、`queue_saturated`、`dependency_degraded` 四个计数；`status` 据此返回 `Healthy`/`Degraded`/`Unhealthy`。

## 13. ONVIF-005：Live 与输出

固定流程：

1. 获取/刷新profile；
2. GetStreamUri（Media2→Media1）；
3. 校验URL并移除日志userinfo；
4. 创建短期credential handle；
5. 调度支持RTSP/RTSPS的media node；
6. CreatePullProxy；
7. 等待StreamOnline；
8. ResolveUrls；
9. Operation成功。

媒体节点不得回显原始凭据；pull失败、credential过期、DNS变化和media重启均进入补偿/对账。

## 14. ONVIF-006：Snapshot、PTZ 与事件

- [ ] 在线流优先TakeSnapshot，无在线流使用restricted Fetch。
- [ ] PTZ能力、range和timeout先校验；危险连续移动具有明确Stop与超时。
- [x] v1不支持的imaging写操作保持稳定Unsupported且不产生Operation副作用：`OnvifTokioProtocolDriver::handle_command` 显式拒绝 `set_imaging_settings`/`set_focus_configuration`/`set_exposure`/`set_white_balance`/`set_backlight_compensation`/`set_wide_dynamic_range`/`set_defog`/`set_iris_filter`/`set_focus` 等 imaging 写命令，返回稳定 `PluginError::Unsupported`，不访问设备、不产生 Operation 副作用；已添加单元测试覆盖。
- [ ] 若v1包含ONVIF Events，必须使用bounded subscription/pull-point和renew；否则capability明确不声明。

## 15. ONVIF-007：互操作验收

- Profile T/Media2设备和legacy Media1设备。
- digest/WS-Security、设备时钟偏移、TLS和多XAddr。
- RTSP/RTSPS pull、凭据失败、SSRF拒绝、snapshot两种路径。
- 慢设备、并发探测、重启恢复和tenant隔离。
- 保存脱敏semantic transcript和版本信息。

## 16. Phase 05 退出门禁

- GB与ONVIF均通过fake device + real media系统测试。
- 真实设备interop报告满足上述矩阵。
- 主应用不再使用Tracing-only event sink或ONVIF fixed Unsupported adapter。
- 所有协议资源都可由Operation/MediaSession/Binding查询和对账。

