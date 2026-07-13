# 12 GB28181 SIP 核心

## 1. 范围

实现独立、无业务状态的 SIP 栈子集，满足 GB/T 28181 接入和级联需要。参考 `gmv` 的行为与测试思路，但不得复制其架构耦合；协议依据以项目配置的标准版本和互通样本为准。

首版支持 UDP/TCP，预留 TLS。支持 REGISTER、MESSAGE、INVITE、ACK、BYE、CANCEL、OPTIONS 及必要响应；不实现与目标无关的完整 RFC 生态。

## 2. 模块

```text
crates/protocol-gb28181/src/sip/
  message.rs
  parser.rs
  encoder.rs
  headers.rs
  uri.rs
  transaction.rs
  dialog.rs
  transport.rs
  digest.rs
```

Parser 与 encoder 不依赖设备仓储或应用服务。事务层通过 trait 发出请求/响应事件。

## 3. 解析与编码任务

### GB-SIP-001：消息模型

- [ ] 请求行、状态行和常用 header 使用强类型。
- [ ] 未识别 header 按原值保留，header 名比较不区分大小写。
- [ ] 支持重复 Via/Route/Record-Route/Contact。
- [ ] Content-Length 必须与字节长度一致，不能用字符数。
- [ ] 配置最大首行、header 数、header 总字节和 body 字节。

### GB-SIP-002：流式解析

- [ ] UDP datagram 必须完整解析，多余字节判错。
- [ ] TCP 支持粘包、半包和连续消息，按 Content-Length 分帧。
- [ ] 拒绝冲突 Content-Length、非法换行、header 注入和超限消息。
- [ ] 错误包含安全的 offset/kind，不回显凭据或完整 body。

### GB-SIP-003：规范编码

- [ ] 编码器稳定输出 CRLF 和正确 Content-Length。
- [ ] 允许保留互通所需 header 顺序，但签名/事务字段由类型生成。
- [ ] parse → encode → parse 的语义保持一致。

## 4. 事务与对话

### GB-SIP-004：事务标识

事务键使用 top Via branch、CSeq method、Call-ID 及方向生成。对不合规设备缺失 magic cookie 的情况提供显式兼容策略，不得默认降低所有请求校验。

### GB-SIP-005：客户端/服务端事务

- [ ] UDP 实现 INVITE 和 non-INVITE 定时器、重传和终止状态。
- [ ] TCP 不做应用层重传，但保持超时与响应匹配。
- [ ] 重复请求重发缓存响应，不重复触发业务。
- [ ] CANCEL 与原 INVITE 正确关联；ACK 分 2xx/non-2xx 路径处理。

### GB-SIP-006：Dialog

- [ ] dialog key 使用 Call-ID、local tag、remote tag。
- [ ] 保存 route set、remote target、本地/远端 CSeq。
- [ ] BYE、重 INVITE 和超时清理遵守 dialog 状态。
- [ ] dialog 只持有必要协议状态，媒体会话仍由领域层管理。

## 5. Digest 认证

- [ ] 支持 MD5（兼容必需）和可配置的更强算法；策略决定是否允许 MD5。
- [ ] nonce 包含时间和服务器签名，支持过期与 stale 响应。
- [ ] 校验 realm、URI、method、qop、nc/cnonce；常量时间比较摘要。
- [ ] 防重放缓存按设备和 nonce 限界。
- [ ] 日志仅记录认证结果与原因码，不记录 Authorization 内容。

## 6. 传输与防护

- [ ] UDP 监听可配置地址、socket buffer 和最大 datagram。
- [ ] TCP 每连接有读取超时、空闲超时、消息速率和缓存上限。
- [ ] 响应优先使用事务收到的源端点，设备上报地址仅作兼容候选。
- [ ] 提供 allow/deny CIDR 和未认证来源速率限制。
- [ ] 传输关闭生成明确事件，使上层处理设备离线与事务失败。

## 7. 测试与验收

- [ ] RFC/GB 样本黄金测试和畸形消息 corpus。
- [ ] parser 使用 cargo-fuzz，任意输入不得 panic 或无界分配。
- [ ] UDP 丢包、重复、乱序测试覆盖所有事务计时器。
- [ ] TCP 半包/粘包每个字节切分位置均测试。
- [ ] Wireshark 可正确解析编码结果。
- SIP 核心可在不启动数据库和 runtime 的情况下单独测试。
