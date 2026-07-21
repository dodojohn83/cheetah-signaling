# 004 任务完成证据规范

本目录只保存已经实际执行的验证与验收报告，不预先创建空报告或用 ignored test 代替结果。

每份报告文件名使用小写任务 ID，例如 `gb4-sip-001-udp-tcp-contract.md`，至少包含：

1. 任务 ID、结论和完成日期；
2. signaling、media server、参考 peer 或 simulator 的 commit；
3. OS、架构、Rust、数据库、NATS、设备/固件和网络拓扑；
4. 实际命令及退出码；
5. 正常与失败场景结果、关键指标和脱敏 transcript/产物位置；
6. 未运行的测试、原因、风险和后续责任人；
7. 对公开契约、migration、配置兼容和安全边界的检查；
8. 明确说明信令进程没有接收或处理媒体 payload。

报告不得包含密码、Authorization、nonce material、完整 SIP/XML/SDP body、真实公网地址、个人信息或未脱敏抓包。

