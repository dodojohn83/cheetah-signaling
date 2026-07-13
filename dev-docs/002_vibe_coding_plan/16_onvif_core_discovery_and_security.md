# 16 ONVIF 核心、发现与安全

## 1. 范围

实现 ONVIF 设备发现、能力探测和安全 SOAP 客户端基础。首版以 Device、Media/Media2、PTZ、Events 常用能力为目标，所有扩展由能力探测驱动，不以厂商名称硬编码服务支持。

## 2. 模块

```text
crates/protocol-onvif/src/
  discovery/
  soap/{envelope,client,fault,action}.rs
  security/{wsse,clock,tls}.rs
  xsd/
  services/
  compat/
```

生成的 XML 类型与手写领域适配器分离。生成代码不得扩散到 application/domain crate。

## 3. WS-Discovery

### ONVIF-CORE-001：Probe/Resolve

- [ ] 在配置的网卡加入 IPv4 multicast，发送 Probe 并接收 ProbeMatches。
- [ ] 解析 EndpointReference、Types、Scopes、XAddrs、MetadataVersion。
- [ ] 对相同 EPR/XAddr 去重，保留发现来源和时间。
- [ ] 支持主动 Probe、被动 Hello/Bye 和指定 EPR Resolve。
- [ ] 多网卡、无默认路由和容器环境通过显式接口配置处理。

发现只产生候选设备，不自动信任或持久化凭据。管理员策略决定自动纳管或待审批。

### ONVIF-CORE-002：发现防护

- [ ] 限制 datagram、XML 深度、匹配数量和每来源速率。
- [ ] XAddr 访问前执行 SSRF 策略：协议、端口、目标网段和重定向校验。
- [ ] 不访问 multicast 响应中指向本机管理端点或云元数据的地址。

## 4. SOAP 与 XML

### ONVIF-CORE-003：SOAP Client

- [ ] 支持 SOAP 1.2 action、HTTP keepalive、超时、代理禁用/显式配置。
- [ ] HTTP 非 2xx 与 SOAP Fault 分别解析为结构化错误。
- [ ] 每服务 endpoint 建连接池和并发限制。
- [ ] 响应体流式读取并设置压缩后上限，防止压缩炸弹。

### ONVIF-CORE-004：Schema 类型

- [ ] 固定 WSDL/XSD 来源、版本和校验和，不在构建时联网下载。
- [ ] 生成代码通过脚本可重现，生成结果纳入版本控制或 CI 严格比对。
- [ ] 对常见 `xs:dateTime`、duration、any element 编写人工适配。
- [ ] 未知扩展有限保留，解析错误指出 QName 路径。

## 5. WS-Security 与 TLS

### ONVIF-CORE-005：UsernameToken

- [ ] 支持 PasswordDigest 和必要的 PasswordText 兼容策略。
- [ ] digest 使用 nonce + created + password 的规范字节顺序。
- [ ] created 来自可校准设备时钟偏移，nonce 使用安全随机源。
- [ ] 凭据仅通过 `SecretProvider` 获取并尽快 zeroize 临时缓冲。

### ONVIF-CORE-006：时钟偏移

- [ ] 未认证时先尝试 GetSystemDateAndTime，计算设备偏移。
- [ ] 偏移按设备缓存并设置过期；认证失败可受限重校准一次。
- [ ] 大幅漂移生成运维事件，但不修改主机系统时间。

### ONVIF-CORE-007：TLS 策略

- [ ] 默认验证证书；私有 CA 可配置加入 trust store。
- [ ] 不安全跳过验证仅允许设备级显式策略，并产生告警与审计。
- [ ] 禁止全局 accept-invalid-certs 开关默默进入生产。

## 6. 能力探测

按 GetServices/GetCapabilities → DeviceInformation → service-specific capabilities 顺序探测。结果映射为统一 `DeviceCapabilities`，同时保留 endpoint、namespace 和版本用于后续路由。

探测可部分成功；每项记录 `Supported | Unsupported | Failed(reason, retryable)`，不能因为 Events 失败丢弃 Media 能力。

## 7. 测试与验收

- [ ] discovery 多网卡、重复、畸形和恶意 XAddr 测试。
- [ ] SOAP 1.2、Fault、chunked、gzip、超限 body 测试。
- [ ] WSSE 使用公开向量和冻结时间测试。
- [ ] XML parser fuzz 与 SSRF 重定向测试。
- [ ] 所有网络请求都有 connect/request/operation deadline。
- 日志与 tracing span 不含密码、nonce 原文或完整 Security header。
