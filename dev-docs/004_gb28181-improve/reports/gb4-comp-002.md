# GB4-COMP-002：GB28181 兼容性 profile 的首批受控 override（charset/MIME/header/endpoint/catalog）

## 任务目标

在 `CompatibilityProfile` / `ProfileResolver` 基础上实现首批受控 override，所有 workaround 均通过 `profile.has(...)` 显式启用，禁止在通用 parser 中直接硬编码宽松分支。

本次覆盖：

- `CharsetFallback`：XML 声明与实际编码不一致时的 charset fallback（UTF-8 声明但 GBK/GB18030 编码）。
- `MimeAlias`：将厂商私有 `Content-Type` 别名（`application/kslp+xml` 等）映射到标准 `MANSCDP`/`MANSRTSP`。
- `ContactRportRoute`：允许仅依赖 `Via received=` 进行 endpoint 路由，即使 `rport` 缺失。
- `HeaderNormalization`：规范化非歧义 SIP 头部，包括请求方法大小写、`CSeq` 方法大小写以及跳过头部块中的空行。
- `CatalogCountFragment`：接受 `Num`/`SumNum` 与实际 item 数量不一致的目录分片。
- `CatalogNotify`：接受根元素为 `<Notify>` 的目录变更通知。

## 实现位置

### core 层（`crates/protocols/cheetah-gb28181-core`）

- `src/lib.rs`：re-export `CompatibilityProfile` / `CompatibilityCapability`，供 driver 使用而不直接依赖 domain。
- `Cargo.toml`：增加 `cheetah-domain` 依赖。
- `src/sip/endpoint.rs`：
  - `ViaRouteParams::resolved_endpoint_with_policy`：在 `ContactRportRoute` 启用时，`rport` 缺失也可使用 `received`。
  - `EndpointRoute::from_registration_with_profile`：按 profile 决策 `received` 优先权。
- `src/sip/parser.rs`：
  - `SipParser` 增加 `profile` 字段，`parse_datagram_with_profile` 入口。
  - `HeaderNormalization` 时把请求行方法转为大写，规范化 `CSeq` 方法名，跳过头部块中的非歧义空行。

### driver 层（`crates/protocols/cheetah-gb28181-driver-tokio`）

- `src/config.rs`：`DriverConfig` 增加 `compatibility_profile` 与 `with_compatibility_profile`。
- `src/shared.rs`：`Shared` 保存 profile 并提供 `compatibility_profile()` 访问。
- `src/lib.rs`、`src/udp.rs`、`src/tcp.rs`：将 profile 传入 `SipParser`。

### module 层（`crates/protocols/cheetah-gb28181-module`）

- `src/config.rs`：`Gb28181DomainConfig` 增加 `compatibility` 字段与 builder。
- `src/assembly.rs`：`GbAccessSettings` 增加 `compatibility_profile` 与 builder；`build_domain_config` 应用 profile。
- `src/error.rs`：新增 `UnsupportedContentType(String)`。
- `src/lib.rs`：新增 `pub(crate) mod mime`。
- `src/mime.rs`：新增 `ContentType` 枚举与 `resolve_vendor_content_type`；默认只接受标准 MIME，启用 `MimeAlias` 后识别厂商别名。
- `src/xml/reader.rs`：`parse_xml_with_profile` 与 `decode_body` 实现 `CharsetFallback` 到 GBK/GB18030/UTF-8 的回退。
- `src/xml/catalog.rs`：`extract_catalog_with_profile`：
  - `CatalogNotify` 允许 `<Notify>` 根。
  - `CatalogCountFragment` 跳过 `Num`/`SumNum` 与 items 数量的严格校验。
- `src/access.rs`：REGISTER 路由与 MESSAGE body 解析均使用 `compatibility()` profile；拒绝 MANSRTSP 通过 MESSAGE 送达。
- `src/xml/mod.rs`：re-export `parse_xml_with_profile` 与 `extract_catalog_with_profile`。

### 应用装配（`apps/cheetah-signaling`）

- `src/assembly.rs`：
  - 新增 `build_compatibility_profile`，把 listener 配置中的 `compatibility_profile` 名称解析为 `CompatibilityProfile`。
  - 装配 `GbAccessSettings` 与 `GbDriverConfig` 时注入 profile。
  - 当 `HeaderNormalization` 启用时，将 `ManagerConfig.branch_policy` 设为 `BranchPolicy::Permissive`。
- `Cargo.toml`：新增 `cheetah-gb28181-core` 依赖。

## Provenance fixture

每个 override 在 `testdata/gb28181/profiles/` 下提供一对 `meta.toml` + 样本文件，已通过 `scripts/verify_gb4_fixtures.py`：

| Capability | 样本文件 | meta 文件 |
|------------|----------|-----------|
| `CharsetFallback` | `charset-fallback.xml` | `charset-fallback.meta.toml` |
| `MimeAlias` | `mime-alias.txt` | `mime-alias.meta.toml` |
| `HeaderNormalization` | `header-normalization.txt` | `header-normalization.meta.toml` |
| `ContactRportRoute` | `contact-rport-route.txt` | `contact-rport-route.meta.toml` |
| `CatalogCountFragment` | `catalog-count-fragment.xml` | `catalog-count-fragment.meta.toml` |
| `CatalogNotify` | `catalog-notify.xml` | `catalog-notify.meta.toml` |

所有 fixture 均为 synthetic，使用脱敏的 dummy device ID、IP 与 Call-ID。

## 风险与移除条件

| Capability | 风险 | 移除条件 |
|------------|------|----------|
| `CharsetFallback` | 放宽 XML 编码校验，可能让非预期编码通过并导致乱码解析 | 当目标设备固件修复为 XML 声明与实际编码一致，或弃用该设备支持后移除 |
| `MimeAlias` | 把非标准 `Content-Type` 映射为 `MANSCDP`/`MANSRTSP`，可能误识别未知消息 | 厂商固件支持标准 `application/manscdp+xml` 后，针对该厂商 profile 禁用 |
| `ContactRportRoute` | 使用 `Via received=` 覆盖 Contact，可能把消息发到非对称 NAT 外错误地址 | 设备正确携带 `rport` 参数或部署在可保证 Contact 可达网络时移除 |
| `HeaderNormalization` | 放宽 SIP 语法检查，Permissive branch policy 可能接受非 RFC 3261 branch | 设备修正大小写、补齐 magic cookie 并去掉头部空行后移除 |
| `CatalogCountFragment` | 目录计数不可靠，导致上层依赖 `SumNum` 做完整性判断出错 | 设备分片时 `Num`/`SumNum` 准确后移除 |
| `CatalogNotify` | 将无订阅的 catalog `<Notify>` 当作合法变更，可能引入通知风暴 | 设备按规范先建立 `SUBSCRIBE/NOTIFY` 订阅后移除 |

## 测试

- 单元测试覆盖默认严格拒绝与 profile 启用后接受两种行为。
- `cargo fmt --all -- --check`：pass
- `cargo clippy --workspace --all-targets -- -D warnings`：pass
- `cargo test --workspace --lib --bins`：pass
- `python3 scripts/verify_gb4_fixtures.py`：pass

## 后续

- `GB4-COMP-003`：扩展 SDP/Broadcast/MediaStatus override，保持 MediaPort 网络边界。
- `GB4-COMP-004`：持续补充每个新增 override 的 provenance fixture、风险与移除条件。
