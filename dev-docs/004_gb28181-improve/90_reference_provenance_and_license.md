# 90. 参考来源、许可证与 Fixture 策略

## 1. 冻结版本

| 项目 | Commit | 审计时许可证结论 | 使用范围 |
| --- | --- | --- | --- |
| [`escoffier/sipsdk`](https://github.com/escoffier/sipsdk/commit/4d906b32cdc0677be6663249712825eb57d1f870) | `4d906b32cdc0677be6663249712825eb57d1f870` | GitHub 未识别仓库许可证 | 仅行为和测试场景，不复制源码 |
| [`648540858/wvp-GB28181-pro`](https://github.com/648540858/wvp-GB28181-pro/commit/642a9fce82cd22246be28a233c046d696a88f283) | `642a9fce82cd22246be28a233c046d696a88f283` | MIT | 行为、结构和 clean-room fixture；复制前仍检查文件头 |
| [`chatop2020/AKStream`](https://github.com/chatop2020/AKStream/commit/3620ff58316534cce7a1d806f8d31239bc92e2c9) | `3620ff58316534cce7a1d806f8d31239bc92e2c9` | MIT | 行为、结构和 clean-room fixture；不引入 media plane |
| [`GB28181/GB28181.Solution`](https://github.com/GB28181/GB28181.Solution/commit/28f423ab11ac59c3f3d9590eb6a78bc4f8b460d3) | `28f423ab11ac59c3f3d9590eb6a78bc4f8b460d3` | 仓库声明 MIT/BSD，README 警告部分依赖/代码可能为 GPL | 默认仅行为参考；逐文件许可批准后才可引用代码 |
| local `simple-media-server` | `bd68e28745a9863f68d6a496fc077d43b9bf99aa` | Mulan PSL v2 | 信令行为和 clean-room fixture；媒体实现禁止进入本仓库 |

远端项目必须使用固定 commit 链接，不以浮动 master/develop 作为验收依据。升级参考基线时新增审计记录，不静默覆盖本表。

## 2. Clean-room 规则

1. 先记录可观察的请求、响应、状态迁移和故障行为，不复制函数、类型、注释或常量表。
2. 根据 GB/T 28181 设计文档和本仓库类型重新编写实现。
3. regression fixture 优先 synthetic；真实抓包必须有合法来源、授权和脱敏记录。
4. 未明确许可证的项目只能转化为文字场景和从零生成的 synthetic message。
5. 任何 copied snippet 必须经过逐文件许可证审查，并在仓库 notice 中履行要求；默认策略是不复制。

## 3. Fixture Metadata

每个 `testdata/gb28181/**/<name>` 必须配套 `<name>.meta.toml`，至少包含：

```toml
source = "synthetic | real-device | reference-peer"
source_project = "optional project name"
source_commit = "optional immutable commit"
standard = "GB/T 28181-2022 | GB/T 28181-2016"
profile = "generic | profile id"
manufacturer = "optional"
model = "optional"
firmware = "optional"
expected = "semantic expectation"
desensitization = "what was replaced or removed"
license = "SPDX identifier or internal authorization reference"
```

真实 fixture 禁止保留密码、Authorization response、nonce secret、真实公网地址、人员名称、地理坐标或未授权设备 ID。

## 4. 可借鉴与禁止照搬边界

| 主题 | 可借鉴 | 禁止照搬 |
| --- | --- | --- |
| SIP | method/transaction/dialog 场景、错误码、重复/乱序行为 | 第三方 parser 源码、宽松字符串解析、完整报文日志 |
| XML | 命令字段和 vendor shape | 未设限 DOM、外部实体、字符串拼接 encoder |
| session | Call-ID/CSeq/SN/SSRC 关联和超时场景 | static map、每设备线程/timer、无 tenant key |
| media | Open/Update/Close 所需信令顺序 | RTP/RTCP/PS/RTSP/codec/recording 实现 |
| persistence | 需要保存的 binding/session/平台状态 | handler 直接访问数据库或 Redis |
| recovery | re-register、late response、catalog timeout、media status | 无限重试、固定成功、无法确认时伪造完成 |

## 5. 审计门禁

- 所有参考项目必须记录 immutable commit 和许可证结论。
- 所有 fixture metadata 必须通过字段、长度、许可证和脱敏检查。
- PR 描述必须列出受参考行为和 clean-room 实现说明。
- 信令 crate 依赖图不得引入任何参考项目或 media engine 依赖。
