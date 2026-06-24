# safe-node

[English](README.md) | 中文

`safe-node` 是 HypeSafe 的自托管签名与风控节点，不是 Node.js 服务，也不是链节点。

节点使用本地加密 keystore 登录 `safe-gateway`，跟踪一个配置好的 Hyperliquid
multi-sig 账户，按本地策略判断任务并为允许的任务签名。当本机 signer 在
`allowed_leaders` 中时，节点还会处理可执行任务；只有任务的 leader 是本机 signer
时才会提交到 Hyperliquid。

## 安装

安装最新 release：

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | bash
```

可选环境变量：

- `SAFE_NODE_TAG`：安装指定 release tag。
- `SAFE_NODE_INSTALL_DIR`：指定安装目录。

本地构建：

```bash
cargo build --release --locked
```

## 配置

`safe-node` 默认读取 `config/node.json`，配置格式兼容 JSON5：

```bash
cp config/example.node.json config/node.json
safe-node keystore generate --out config/signer.json
```

启动前修改 `config/node.json`：

- 设置 `leader`、`multisig` 和 `signer.keystore_path`。
- 如果希望从环境变量读取 keystore 密码，设置 `signer.password_env`。
- 收窄 `allowed_templates`、`allowed_creators`、`allowed_leaders` 到本节点信任的
  地址和任务类型。
- 使用 `template_input_policies` 配置模板级金额上限和目标地址 allow list。
  `withdraw_limit` 仍作为带 `amount` 输入、且没有模板级金额规则时的 fallback。

`allowed_creators` 或 `allowed_leaders` 为空时会默认使用 `leader`。
省略 `allowed_templates` 时默认只允许 `withdraw3` 和 `sub_account_withdraw3`。

## 常用命令

```bash
safe-node run
safe-node run --config config/node.json
safe-node once --dry-run
safe-node tui
safe-node keystore import --out config/signer.json
```

`run` 启动轮询和本地 RPC HTTP 服务。`once` 只处理一轮。`tui` 读取正在运行节点的
RPC HTTP 接口。

## RPC HTTP

`safe-node run` 默认监听 `http://127.0.0.1:9909/`：

- `/`：浏览器 dashboard。
- `/debug/status`、`/debug/config`、`/debug/policy`、`/debug/transactions`：只读
  JSON 接口。
- `POST /rpc/task/create`：使用 node signer 创建 task。

配置 `rpc_auth_token` 后，写 RPC 需要 `Authorization: Bearer <token>`。如果把
`rpc_http_addr` 绑定到非 localhost 地址，应同时配置 token。

## Docker

构建镜像并作为服务运行：

```bash
./scripts/docker-build.sh
./scripts/docker-run-service.sh
```

只运行一轮轮询：

```bash
SAFE_NODE_DRY_RUN=1 ./scripts/docker-run-executor.sh
```

Docker helper 默认使用 `hypesafe/safe-node:local`，把 `SAFE_NODE_RUN_DIR` 或当前目录挂载到
容器内 `/app/config` 和 `/app/data`，并自动加载该目录下的 `.env`。容器内默认配置路径是
`/app/config/config.json`。
