# 22 打包、部署、迁移与发布

## 1. 交付形态

### 边缘单机

- 单一主二进制，内置 GB28181/ONVIF 和 in-process bus。
- SQLite、本地配置/secret 文件，可选同机媒体服务。
- systemd 服务、最小权限用户、数据/日志目录和资源限制。
- ARM64 与 x86_64 包；尽量提供 glibc 基线明确的静态/半静态产物。

### 集群

- OCI 镜像、Kubernetes manifests/Helm chart。
- PostgreSQL、NATS JetStream、外部 secret、mTLS。
- 信令节点和媒体节点独立扩缩，zone/affinity 可配置。

## 2. 实现任务

### REL-001：配置与启动

- [ ] 提供 `config.example.toml`，每项说明默认、范围、是否可热更新和安全影响。
- [ ] 支持文件 + 环境变量覆盖；启动打印脱敏后的有效配置摘要。
- [ ] `check-config` 同时验证语法、跨字段约束、监听冲突和 secret 可用性。
- [ ] 未知配置字段默认报错，防止拼写错误被忽略。

### REL-002：边缘包

- [ ] systemd unit 使用 `NoNewPrivileges`、合理的文件/网络权限和 restart policy。
- [ ] 安装、升级、卸载脚本不删除用户数据。
- [ ] SQLite 数据目录启动前检查空间、权限和备份策略。
- [ ] 提供离线安装包、校验和、SBOM 和签名验证说明。

### REL-003：容器与 Kubernetes

- [ ] 非 root、只读 rootfs、固定 digest 基础镜像、最小运行依赖。
- [ ] readiness/liveness/startup probes 分离。
- [ ] PodDisruptionBudget、anti-affinity、graceful termination 和 topology spread。
- [ ] NetworkPolicy 只开放 API、协议、DB、NATS、媒体控制所需流量。
- [ ] chart 支持 existingSecret，不把密码放入 values 默认值。

### REL-004：数据库升级

- [ ] 采用 expand → migrate/backfill → switch → contract。
- [ ] N 与 N+1 运行期只使用兼容 schema。
- [ ] 大表 backfill 可暂停、限速、断点续跑并暴露进度。
- [ ] destructive contract 迁移延迟至少一个发布窗口。
- [ ] 每版提供备份、升级验证和回滚限制说明。

### REL-005：从旧系统迁移

建立独立 `migration-tool`：

- 导入租户、设备、通道、GB 平台、ONVIF endpoint 和 secret reference。
- dry-run 输出数量、冲突、无效 ID 和缺失凭据，不写目标库。
- 批量写入有 checkpoint，可重复运行且幂等。
- 导入后执行校验摘要，并支持小批灰度设备切换。
- 不假设旧系统明文密码可直接迁移；无法迁移的凭据形成操作清单。

### REL-006：发布流水线

- [ ] 锁定 toolchain 和依赖，验证 `Cargo.lock`。
- [ ] 构建 x86_64/ARM64，运行测试、审计、许可证和镜像扫描。
- [ ] 生成 SBOM、provenance、签名、校验和、迁移说明和 changelog。
- [ ] release candidate 通过边缘耐久和集群故障演练后才可正式发布。
- [ ] 产物可从源码和标签复现，构建过程不下载未锁定 schema。

## 3. 版本与兼容承诺

- API、Proto、数据库、配置和插件 SDK 分别版本化。
- 正式版本遵循语义版本；安全修复可以补丁发布。
- 至少维护滚动升级相邻版本兼容，跨版本升级路径在 release notes 明示。
- 删除字段前先弃用并提供遥测/迁移工具；Proto field number 永不复用。
- 制定十年维护策略：LTS 频率、支持窗口、Rust MSRV 升级节奏、依赖替换和漏洞响应责任。

## 4. 灰度、回滚与灾备

- [ ] 节点按 canary → 单 zone → 全集群升级。
- [ ] 回滚前检查数据库是否仍兼容旧版本；不可逆迁移禁止盲目回滚二进制。
- [ ] 配置、数据库、证书和媒体节点分别有恢复手册。
- [ ] PostgreSQL 定期恢复演练；SQLite 备份在另一设备验证。
- [ ] 定义 RPO/RTO，演练结果记录真实恢复时间与数据差异。

## 5. 最终完成定义

首版发布必须同时满足：

- GB28181 与 ONVIF 章节的必选功能和互通测试通过。
- 单机 SQLite/进程内消息与集群 PostgreSQL/NATS 两套路径通过系统测试。
- 信令与媒体控制契约完成双项目联调，媒体负载不经过信令节点。
- HA、对账、安全、可观测、备份恢复和升级回滚演练有报告。
- 无未归属的占位实现；所有已知限制进入 release notes 并有稳定错误行为。
