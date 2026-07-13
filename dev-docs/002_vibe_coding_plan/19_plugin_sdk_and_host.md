# 19 插件 SDK、宿主与协议扩展

## 1. 目标

为 MQTT、SIP、RTSP Control、HomeKit、Matter、ISUP、大华 SDK、JT808 等后续协议建立长期稳定扩展面。首版 GB28181/ONVIF 可内置编译，但必须使用与插件一致的 `ProtocolDriver` 端口。

插件分两类：

- Rust built-in：随主程序编译，性能高，适用于首版核心协议。
- 隔离插件：默认进程外 gRPC；未来可增加 WASI 组件。厂商 C/C++ SDK 必须进程外隔离。

不承诺 Rust 动态库 ABI 稳定，不使用 `.so/.dll` 直接加载任意 Rust trait object。

## 2. SDK 契约

`crates/plugin-sdk` 定义：

- `ProtocolDriverFactory`
- `ProtocolDriver`
- `DriverContext`
- `DeviceSink` / `CommandSource`
- `CapabilityDescriptor`
- `HealthReport`
- `PluginManifest`

核心方法：`start`、`drain`、`shutdown`、`handle_command`、`probe`、`health`。所有方法带 deadline/cancellation，错误使用稳定分类。

Driver 只接收受限能力：发布规范事件、查询必要设备配置、读取凭据引用、申请媒体会话、注册协议 endpoint。不得直接获得数据库连接池、NATS 客户端或全局配置对象。

## 3. 实现任务

### PLG-001：Manifest 与版本协商

- [ ] manifest 包含 plugin ID、版本、SDK 范围、协议、入口、权限、配置 schema 和资源预算。
- [ ] 启动前验证签名/校验和和兼容版本。
- [ ] plugin ID 与配置实例 ID 分离，同插件可运行多个实例。
- [ ] 不兼容插件保持禁用并报告原因，不使主进程崩溃。

### PLG-002：内置驱动注册

- [ ] 通过静态 registry 注册 GB28181/ONVIF factory。
- [ ] driver 生命周期统一受 plugin host 管理。
- [ ] 配置更新采用 validate → prepare → activate，失败保留旧实例。
- [ ] health 聚合到 readiness，但非关键插件可降级而不下线整节点。

### PLG-003：进程外插件协议

- [ ] 使用版本化 gRPC/Proto，复用统一 envelope 与强类型 ID。
- [ ] 宿主启动子进程时设置最小环境、工作目录、用户权限和资源限制。
- [ ] heartbeat、启动超时、优雅停止和崩溃退避有明确状态机。
- [ ] stdout/stderr 限速并结构化接入日志。
- [ ] 不自动无限重启永久配置错误。

### PLG-004：厂商 SDK 隔离

- [ ] ISUP/大华 SDK adapter 单独进程和镜像/包。
- [ ] FFI unsafe 只存在 adapter 内，边界做长度、线程和回调生命周期校验。
- [ ] SDK 崩溃只影响对应插件实例。
- [ ] 明确供应商库授权、架构支持与升级流程。

### PLG-005：开发者模板

- [ ] 提供 `examples/protocol-plugin`，实现模拟注册、事件和命令。
- [ ] 提供配置 JSON Schema、契约测试工具和 mock host。
- [ ] 文档说明能力权限、错误分类、幂等、背压与安全要求。

## 4. 兼容策略

SDK 使用语义版本；同一 major 只允许向后兼容增加 optional 字段/能力。Proto 字段号永不复用。插件持久状态必须自行声明 schema 和迁移，核心数据库只保存其有限 metadata。

每个正式版本维护插件兼容矩阵，并至少支持 N-1 SDK major 的进程外协商或给出离线迁移工具。

## 5. 测试与验收

- [ ] 驱动契约测试同样运行 built-in 和 out-of-process 示例。
- [ ] 插件崩溃、卡死、输出洪水、超内存、错误 schema 和版本不兼容测试。
- [ ] 插件不得访问未授予租户或凭据。
- [ ] 替换插件实例不会重复发布注册或事件副作用。
- 新协议无需修改 domain 核心状态机即可接入；若确需新增通用能力，先走契约演进评审。
