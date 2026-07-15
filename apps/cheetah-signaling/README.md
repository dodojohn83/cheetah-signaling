# cheetah-signaling

Cheetah Signaling edge/cluster executable entry point.

## 职责

- 加载配置、secret 和角色声明。
- 装配 runtime、storage、messaging、cluster、media 和 protocol 组件。
- 管理进程生命周期、ready/health 探针和优雅关闭。
- 不承载协议状态机、业务规则或持久化逻辑。

## 允许依赖

- workspace 内 `crates/*` 下层 crate。
- `tokio`（进程 runtime）。
- `config`（配置加载）。
- `tracing`（结构化日志）。
- `clap`/`axum`（CLI/HTTP 入口，未来引入）。

## 禁止依赖

- 不得直接依赖具体数据库客户端（`sqlx`）、NATS 客户端（`async-nats`）或协议实现细节。
- 不得直接绑定媒体 RTP/RTCP 端口。

## feature

- `default`：标准 edge/cluster 启动。

## 公共入口

- `src/main.rs`：二进制入口，负责装配与启动。
