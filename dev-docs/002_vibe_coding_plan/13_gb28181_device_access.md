# 13 GB28181 设备接入与业务消息

## 1. 目标

在 SIP 核心之上实现 GB28181 南向接入：注册、保活、目录、设备信息、状态、报警、移动位置、PTZ 与录像查询。所有 XML 先转换为协议 DTO，再由适配器转换为统一领域命令/事件。

## 2. 接入配置

每个 GB 域配置：`domain_id`、`realm`、监听端点、标准版本、字符集策略、认证策略、心跳超时、目录分页上限、设备 ID 校验规则和兼容 profile。配置支持多租户域，但同一监听端点的 realm 路由必须无歧义。

## 3. 实现任务

### GB-ACC-001：REGISTER

- [ ] 解析设备 ID、Contact、Expires、源地址和 User-Agent。
- [ ] 未认证请求返回 challenge；认证成功后 upsert 设备 endpoint。
- [ ] Expires=0 执行注销，重复注销幂等。
- [ ] 注册刷新更新 protocol session 和 owner lease，不重复创建设备。
- [ ] 同设备从新地址注册时按 epoch 替换旧会话。

### GB-ACC-002：Keepalive

- [ ] 处理 `CmdType=Keepalive`，校验 SN/DeviceID/Status。
- [ ] 更新 last_seen 使用节流写策略，避免每次心跳写数据库。
- [ ] 心跳丢失由 timer wheel 判定离线并生成一次事件。
- [ ] 离线后收到合法心跳可恢复在线，但不能绕过注册策略。

### GB-ACC-003：XML 编解码

- [ ] 使用 quick-xml 流式解析并禁用外部实体类行为。
- [ ] 支持 XML declaration 指定编码及项目配置的 GB2312/GBK 兼容。
- [ ] 限制元素深度、文本长度、列表项数和总 body 大小。
- [ ] 保留未知扩展字段到有限制的 extension map。
- [ ] 编码时正确转义文本，禁止字符串拼接 XML。

### GB-ACC-004：Catalog/DeviceInfo/DeviceStatus

- [ ] 主动发送查询并关联 SN 与事务 deadline。
- [ ] Catalog 分片响应按 `SumNum` 聚合，允许乱序和重复项。
- [ ] 达到 deadline 时返回部分结果及完整性标记。
- [ ] 目录转换为统一 Channel，并以 revision-safe 方式替换快照。
- [ ] 大目录分批提交，避免单事务和单事件过大。

### GB-ACC-005：Alarm/MobilePosition

- [ ] 解析报警优先级、方法、类型、时间和坐标。
- [ ] 基于协议键和时间窗口去重，原始扩展字段限长保留。
- [ ] 转换为统一事件并通过 outbox 发布。
- [ ] 无效坐标/时间记录协议错误，不得导致设备会话退出。

### GB-ACC-006：设备控制

- [ ] 统一 PTZ 命令转换为 GB 控制字节并生成校验码。
- [ ] 支持设备配置/重启等首版选定 DeviceControl 子集。
- [ ] 命令响应与应用层 command ID 关联。
- [ ] 超时、设备拒绝与格式错误映射为不同错误码。

### GB-ACC-007：RecordInfo

- [ ] 请求按通道、时间范围和类型构造。
- [ ] 分页/分片结果流式聚合，限制最大条目。
- [ ] 对重复录像项使用稳定业务键去重。
- [ ] 返回部分结果、设备声明总数与实际条数。

## 4. 兼容 Profile

兼容项必须以设备厂商/型号/固件匹配的 profile 明确启用，例如：错误 charset 声明、非标准 tag、Contact 私网地址、固定 SN、缺失字段。每个 workaround 包含：匹配条件、行为、风险、测试样本和计划复核版本。

禁止在通用解析路径加入不可追踪的宽松分支。

## 5. 测试与验收

- [ ] 模拟设备完成注册、认证、心跳、目录和注销全流程。
- [ ] 10 万通道目录输入保持内存有界并可超时取消。
- [ ] XML fuzz 不 panic、不执行外部资源、不突破限制。
- [ ] 重复 MESSAGE 不产生重复领域事件。
- [ ] GMV/真实设备脱敏报文形成黄金样本，标明预期输出。
- 设备业务 handler 不直接执行 SQL 或媒体 RPC。
