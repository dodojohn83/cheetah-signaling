# AUD-001：002 Checkbox Registry Completion Report

## 目标

为 `dev-docs/002_vibe_coding_plan` 中的每个 checkbox 生成稳定引用 `002-<chapter>-<ordinal>`，并建立可追溯到 003 任务组的登记表。

## 执行

- 更新 `scripts/generate_002_registry.py`：
  - 除章节汇总外，新增 **Checkbox Registry** 表格。
  - 每行包含 `002-<chapter>-<ordinal>`、章节、行号、状态、003 主归属、003 上下文说明和原文。
- 运行脚本生成：
  - `dev-docs/003_next_round_vibe_coding_plan/91_002_checkbox_registry.md`
  - `target/002_checkbox_registry.json`
- 运行 `scripts/audit_002_registry.py` 验证：
  - 文件数：23
  - 总数：577
  - 已完成：3
  - 未完成：574
  - 结果：`AUDIT PASSED`

## 003 归属原则

每个 002 checkbox 的 `003 Primary` 列来自 `PHASE_003` 映射，按章节统一归属：

| 002 章节 | 003 主归属 |
| --- | --- |
| 01 | BAS |
| 02 | BAS |
| 03 | BAS |
| 04 | BAS, PROD |
| 05 | MED-C |
| 06 | WF |
| 07 | ASM, SYS |
| 08 | BAS, SYS |
| 09 | ASM, PROD |
| 10 | PROD |
| 11 | MED-C, MED-R, UP |
| 12 | GB, BAS |
| 13 | GB |
| 14 | WF, GB, UP |
| 15 | GB, SYS |
| 16 | ONVIF |
| 17 | ONVIF, WF, UP |
| 18 | ASM, PROD, SYS |
| 19 | ASM, PROD |
| 20 | PROD |
| 21 | SYS |
| 22 | SYS |
| README | N/A |

## 完成条件

- 所有 577 个 checkbox 均有唯一稳定 ID。
- 登记表总数与源文件一致。
- `audit_002_registry.py` 通过。

## 引用

- `dev-docs/003_next_round_vibe_coding_plan/91_002_checkbox_registry.md`
- `scripts/generate_002_registry.py`
- `scripts/audit_002_registry.py`
