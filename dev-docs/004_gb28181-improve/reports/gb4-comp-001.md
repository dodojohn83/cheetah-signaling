# GB4-COMP-001 / GB4-COMP-004 完成报告

- 任务 ID：`GB4-COMP-001`（profile schema、exact match、revision pinning、capability、配置校验）与 `GB4-COMP-004`（每个 workaround 的 provenance fixture、risk、regression 与 removal criteria）
- 结论：完成（schema、校验、能力协商、revision pinning 与 provenance 元数据）
- 日期：2026-07-21
- 仓库 commit：`dodojohn83/cheetah-signaling`（分支 `devin/gb4-comp`，PR HEAD；stacked on PR #161 / `devin/gb4-arc-001`）

## 1. 环境

- OS/架构：Ubuntu Linux x86_64
- Rust：1.96.1（`rust-toolchain.toml`）
- 数据库/NATS/设备：本任务为纯 Sans-I/O 协议核心类型，未接触数据库、消息系统或真实设备
- media server / reference peer：无需运行；fixture 为 clean-room synthetic + 行为参考

## 2. 变更摘要

1. 在 `cheetah-gb28181-module` 新增 `compat` 模块（`compat/mod.rs`、`overrides.rs`、`profile.rs`、`registry.rs`、`tests.rs`），实现显式、可审计的 `CompatibilityProfile`：
   - `StandardVersion`（2016/2022）、`DigestAlgorithmPreference`（MD5/SHA-256，映射到 core `DigestAlgorithm`）、`CharsetPreference`（映射到 `CharsetPolicy`）、`EndpointBehavior`（`rport`、source route 策略）、`CatalogOverrides`（fragment size，硬上限 1000）、`SdpOverrides`（SSRC、TCP setup、先发媒体）与 `MediaStatusOverrides`（NotifyType=121、Broadcast handshake）。
   - `CompatibilityCapability` 能力枚举与 `negotiate`（与声明能力取交集，绝不授予未声明能力）。
   - `ProfileMatchKey` 强制 manufacturer → model → firmware 层级；`ProfileId`、`ProfileRevision` 受校验 newtype。
   - `CompatibilityProfileConfig`（serde `deny_unknown_fields`）→ `CompatibilityProfile::from_config` 完成配置反序列化与校验；`CompatibilityProfile::builder` 供程序化构造。
   - `CompatibilityRegistry` 固定优先级选择：exact firmware → model → manufacturer → standard generic；重复 id / 重复 `(standard, match_key)` 在构造期拒绝；同优先级多匹配返回 `AmbiguousMatch`。
   - `PinnedProfile::pin()` 将 revision 固化为快照，`is_superseded_by` 判定热更新是否产生新 revision，运行中的会话不因热更新改变语义。
2. 校验的不可违反边界：schema 不暴露任何放开 tenant/identity/Digest/owner epoch、CRLF/DTD/XXE、body/深度/队列上限或改变公共 Operation 成功语义的字段；catalog fragment size 有硬上限；`broadcast_handshake` / `media_status_accept_121` 必须声明对应能力才能启用。
3. 更新 `cheetah-gb28181-module` README 与 `lib.rs` 公共导出。
4. 新增 provenance fixture：`testdata/gb28181/compat/{akstream,wvp}-gb2016.profile.toml` 及配套 `.meta.toml`，并新增回归测试 `tests/compat_profiles.rs`。

## 3. Fixture Provenance、风险、回归与移除条件

下表覆盖本 PR 引入的受控 override / workaround。每项均为 backlog capability，默认不启用，必须由具体 profile 显式开启。

| workaround / override | provenance fixture | risk | regression test | removal criteria |
| --- | --- | --- | --- | --- |
| MD5 digest 偏好（GB2016 设备只支持 MD5） | `akstream-gb2016.profile.toml` + `.meta.toml`（AKStream，MIT，commit `3620ff5…`） | 弱于 SHA-256，仅在设备不支持 SHA-256 时启用；仍需域配置 `allow_md5` | `compat::tests::from_config_builds_generic_profile_with_safe_defaults`（默认 SHA-256）、`akstream_profile_loads_with_expected_capabilities` | 目标设备固件升级支持 SHA-256 后删除该 profile 的 `digest_algorithm = "md5"` |
| GBK/GB2312 charset 兼容 | 同上 | 转码错误可能导致解析异常；限定在 `CharsetPolicy::GbkCompatible` | `from_config_builds_generic_profile_with_safe_defaults`（默认 UTF-8）、fixture 测试 | 设备统一输出 UTF-8 后移除 |
| catalog fragment size 覆盖 | `akstream-gb2016.profile.toml`（`catalog_fragment_size = 64`） | 过大分片可能超出对端缓冲；已加硬上限 1000 与非零校验 | `compat::tests::catalog_fragment_size_is_bounded` | 对端稳定支持标准默认分片后移除覆盖 |
| Broadcast handshake | `akstream-gb2016.profile.toml`（capability `broadcast`） | 多步握手增加状态；必须声明 `broadcast` 能力 | `compat::tests::override_requires_declared_capability`、`akstream_profile_loads_with_expected_capabilities` | Broadcast 成为标准默认能力后从 profile 移除 |
| MediaStatus NotifyType=121 接受 | `akstream-gb2016.profile.toml`（capability `media_status`） | 误判终止事件风险；必须声明 `media_status_report` 能力，且只影响匹配 bridge generation | `compat::tests::override_requires_declared_capability` | 对端统一使用标准 MediaStatus 语义后移除 |
| Catalog Notify / Alarm / MobilePosition 订阅、多上级、virtual directory、custom external ID | `wvp-gb2016.profile.toml` + `.meta.toml`（WVP，MIT，commit `642a9fc…`） | 能力仅通过声明暴露，不复用万能 payload | `wvp_profile_loads_with_expected_capabilities`、`fixtures_form_a_registry_and_select_by_manufacturer` | 相关能力进入 v1 默认后从 profile 精简 |

fixture metadata 字段（`source`、`source_project`、`source_commit`、`standard`、`profile`、`expected`、`desensitization`、`license`）符合 `90_reference_provenance_and_license.md` 第 3 节；均为 synthetic / 行为参考，无源码照搬、无密码/Authorization/nonce/真实地址/设备 ID/抓包。

## 4. 测试：strict-default rejection 与 profile-enabled acceptance

- strict-default：`from_config_builds_generic_profile_with_safe_defaults` 断言 generic profile 的 override 全部为标准默认（SHA-256、UTF-8、`rport=honor`、`source_route=dialog_target`、无 broadcast / 121 / 先发媒体）。
- profile-enabled acceptance：`override_requires_declared_capability` 断言启用 override 且声明能力后校验通过；未声明能力则拒绝。
- 选择优先级：`selection_prefers_most_specific_match`、`selection_no_match_for_other_standard_version`。
- 配置错误：`registry_rejects_duplicate_id`、`registry_rejects_duplicate_match_key`、`from_config_rejects_unknown_tokens`、`catalog_fragment_size_is_bounded`、`match_key_enforces_hierarchy`、`revision_zero_is_rejected`。
- revision pinning：`revision_pinning_survives_hot_reload`。
- fixture 回归：`tests/compat_profiles.rs`（加载、校验、provenance 元数据存在、注册表选择）。

## 5. 实际命令与结果

```text
cargo fmt --all -- --check                                   # pass
cargo clippy --workspace --all-targets -- -D warnings        # pass
cargo test --workspace                                       # pass
python3 scripts/audit_architecture.py                        # GB28181 路径无新增 violation/placeholder
```

（`cargo nextest` 若环境未安装则以 `cargo test --workspace` 覆盖；`buf`/`cargo deny` 见 PR 说明，本 PR 未修改 `.proto`。）

## 6. 公共契约 / migration / 配置兼容 / 安全边界

- 新增的 `compat` 公共类型为纯新增能力，未改变既有 API/Proto/数据库 schema，无 migration。
- 配置形态为新增可选 `[[gb28181.compatibility_profiles]]`；本 PR 仅提供 schema 与校验，实际装配读取由后续 assembly 任务接入。
- 安全边界：schema 无法放开身份/鉴权/owner/解析上限，`serde(deny_unknown_fields)` 拒绝未知配置项，能力协商只做交集。

## 7. 未运行 / 后续

- 实际 override 行为接入（charset/MIME/header/endpoint/catalog 与 SDP/Broadcast/MediaStatus 生产链路）属于 `GB4-COMP-002` / `GB4-COMP-003`，本 PR 只落地 schema、校验、能力协商与 revision pinning。
- 与真实上级/下级平台的互操作报告在专用 interop 流水线产出，不在本 PR。

## 8. 媒体面边界声明

本 PR 仅新增控制面协议核心的兼容 profile 数据类型与校验逻辑，未接收、转发、解析或存储任何 RTP/RTCP/PS/TS/ES 媒体负载，未绑定媒体端口，未访问媒体引擎实现。SDP override 字段仅描述信令协商偏好，媒体网络边界仍由 MediaPort 契约负责。
