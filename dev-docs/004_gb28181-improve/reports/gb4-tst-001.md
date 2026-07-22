# GB4-TST-001：SIP/XML/SDP/MANSRTSP golden corpus 扩充

- 任务：`GB4-TST-001`
- 状态：`Completed`
- 日期：2026-07-21

## 1. 新增 fixture 清单

### SIP

| fixture | 内容 | 标准 |
| --- | --- | --- |
| `bye` | BYE 请求，含 dialog tag | GB/T 28181-2022 |
| `ack` | ACK 请求 | GB/T 28181-2022 |
| `cancel` | CANCEL 请求 | GB/T 28181-2022 |
| `options` | OPTIONS 请求 | GB/T 28181-2022 |
| `subscribe` | SUBSCRIBE 请求，含 Expires | GB/T 28181-2022 |

### MANSCDP/MANSRTSP XML

| fixture | 内容 | 标准 |
| --- | --- | --- |
| `device_info` | DeviceInfo 响应 | GB/T 28181-2022 |
| `device_status` | DeviceStatus 状态通知 | GB/T 28181-2022 |
| `alarm` | 报警通知 | GB/T 28181-2022 |
| `mobile_position` | 移动位置通知 | GB/T 28181-2022 |
| `record_info` | 录像查询响应 | GB/T 28181-2022 |

## 2. 验证结果

```bash
UPDATE_GOLDEN=1 cargo test -p cheetah-gb28181-core --test golden
UPDATE_GOLDEN=1 cargo test -p cheetah-gb28181-module --test golden_xml
python3 scripts/verify_gb4_fixtures.py
```

全部通过：`OK: 34 fixture data files and 17 metadata files validated.`

## 3. 规范

- 所有 fixture 来源均为 `synthetic`；
- 每个 fixture 均包含 `.meta.toml` provenance 文件；
- 所有 device ID、坐标、时间戳、厂商型号均为虚构；
- 许可证 MIT-0。

## 4. 备注

MANSRTSP  SDP 样本与 fuzz corpus 的扩充将在 `GB4-TST-004` simulator 重构与 `GB4-MED` 阶段继续补充。
