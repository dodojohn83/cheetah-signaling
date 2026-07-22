# GB4-COMP-001：GB28181 兼容性 profile schema、exact match、revision pinning、capability 与配置校验

## 任务目标

实现 GB28181 兼容性 profile 的完整 schema：

- profile 字段（standard_version、manufacturer、model、firmware、evidence_ref、revision）。
- 受控 capability 枚举。
- 基于 selector 的 exact → model → manufacturer → standard → default 优先级匹配。
- 会话创建时把 profile revision 固定到 `ProtocolSession`，避免运行时热更新改变已有 dialog 语义。
- 配置加载校验：id 唯一性、字段长度、capability 数量、listener 引用存在性。

## 实现位置

- `crates/domain/cheetah-domain/src/protocol_session.rs`
  - 新增 `CompatibilityCapability` 枚举与 `FromStr` 解析。
  - 扩展 `CompatibilityProfile`：增加 standard/manufacturer/model/firmware、capabilities、evidence_ref、revision。
  - 新增 `ProfileSelector` 与 `CompatibilityProfile::score` / `has`。
  - 保留 `ProtocolSession` 对 `CompatibilityProfile` 的完整拷贝，实现 revision pinning。

- `crates/foundation/cheetah-signal-types/src/config.rs`
  - 新增 `Gb28181CompatibilityProfileConfig`。
  - `Gb28181Config` 增加 `compatibility_profiles`。
  - `Gb28181ListenerConfig` 增加 `compatibility_profile` 引用。
  - `Gb28181Config::validate` 校验 profile id 唯一、字段长度、capabilities 上限、listener 引用有效。

- `crates/protocols/cheetah-gb28181-module/src/profile.rs`
  - 新增 `ProfileResolver` 与 `ProfileResolveError`。
  - 实现优先级解析与歧义检测。
  - 单元测试覆盖 exact/model/manufacturer/standard/default 与歧义场景。

## 验证

```text
cargo fmt --all -- --check                               # pass
cargo clippy --workspace --all-targets -- -D warnings    # pass
cargo test --workspace --lib --bins                        # pass
python3 scripts/audit_architecture.py                    # no new violations
```

## 依赖与后续

- `GB4-COMP-002` 将在此 schema 上实现 charset/MIME/header/endpoint/catalog 首批受控 override。
- `GB4-COMP-003` 将扩展 SDP/Broadcast/MediaStatus override。
- `GB4-COMP-004` 将要求每个 override 补充 provenance fixture、risk、regression 和 removal criteria。
