# safe-node

[English](README.md) | 中文

`safe-node` 是 HypeSafe 的本地签名与风控节点，不是 Node.js 服务，也不是链节点。

它从 `safe-gateway` 拉取指定 multi-sig 的任务，并按本地规则处理指定 `leader` 下被允许的 task template。普通 signer 节点只协签；当本机 signer 等于 `leader` 时，该节点还会为链上模板提交 Hyperliquid `/exchange` 并把结果写回 gateway。

## 安装

从当前仓库用 `curl` 下载安装脚本，脚本会自动识别 latest release，下载对应的
release archive，校验 SHA-256 后解包到本地：

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | bash
```

安装指定 tag：

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | SAFE_NODE_TAG=v0.1.0 bash
```

指定安装目录：

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | SAFE_NODE_INSTALL_DIR="$HOME/safe-node" bash
```

## 功能

- JSON 配置，默认读取 `config/node.json`。
- 加密 keystore，本地解密后签名。
- keystore 密码可来自环境变量；如果环境变量为空，则启动时手动输入。
- template allow list 风控；对声明了 `amount` 字段的模板额外应用金额限额。
- policy reject 会尽量提交到 gateway 作为正式拒绝投票；不会签名，也不会提交 Hyperliquid。
- 本地状态使用 SQLite，SQL 操作通过 SeaORM。
- 主服务提供只读 debug HTTP。
- `safe-node tui` 在 terminal 中展示最近交易、配置摘要和风控规则。

## 常用命令

```bash
safe-node keystore generate --out config/signer.json
safe-node keystore import --out config/signer.json
safe-node run
safe-node once --dry-run
safe-node tui
```

`--config` 可以缺省；需要指定其他配置文件时使用：

```bash
safe-node run --config config/node.json
```

## 配置

使用 `config/example.node.json` 作为全量配置模板：

```bash
cp config/example.node.json config/node.json
```

启动节点前，修改复制后的配置，填入真实的 `leader`、`multisig`、signer
keystore 路径和风控参数。

`allowed_templates` 是 task template allow list。这里使用 gateway `template_id`
字符串，不是固定枚举。`config/example.node.json` 已列出当前 gateway 暴露的全部签名模板；
如果 node 只应签部分业务，可以收窄这个列表。内置缺省值仍保持保守：

- `withdraw3`
- `sub_account_withdraw3`

## 签名信任边界

签名前，`safe-node` 会校验 gateway 下发的 EIP-712 typed-data 与 task /
signing-payload 响应声明的 digest 一致，避免签下与当前 task digest 不匹配的 payload。
这仍是部分信任模型：node 还没有从 `template + inputs + ctx` 独立重建 typed-data，
在强校验落地前，payload 构造仍以 `safe-gateway` 为来源。

## Debug HTTP

`safe-node run` 运行后，打开 `http://127.0.0.1:9909/` 可以访问只读浏览器 dashboard。
JSON 接口仍保留在 `/debug/status`、`/debug/config`、`/debug/policy` 和
`/debug/transactions`。

## Docker

构建本地镜像：

```bash
./scripts/docker-build.sh
```

默认镜像名是 `hypesafe/safe-node:local`。需要覆盖时使用：

```bash
SAFE_NODE_IMAGE=registry.example.com/hypesafe/safe-node:tag ./scripts/docker-build.sh
```

镜像入口是 `safe-node`，默认命令是 `run`，所以同一个镜像既可以作为常驻服务，也可以作为一次性 executor。

作为一次性 executor 运行：

```bash
./scripts/docker-run-executor.sh --dry-run
```

等价的直接 Docker 命令：

```bash
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --env HOME=/tmp \
  --volume "$PWD:/app/config:ro" \
  --volume "$PWD:/app/data" \
  hypesafe/safe-node:local \
  once --config /app/config/config.json --dry-run
```

作为常驻服务启动：

```bash
./scripts/docker-run-service.sh
docker logs -f safe-node
```

等价的直接 Docker 命令：

```bash
docker run --detach \
  --name safe-node \
  --restart unless-stopped \
  --user "$(id -u):$(id -g)" \
  --env HOME=/tmp \
  --publish 127.0.0.1:9909:9909 \
  --volume "$PWD:/app/config:ro" \
  --volume "$PWD:/app/data" \
  hypesafe/safe-node:local \
  run --config /app/config/config.json
```

脚本会把当前运行目录同时挂载到容器内 `/app/config` 和 `/app/data`：`/app/config` 只读，
`/app/data` 可写。容器用当前宿主机 UID/GID 运行，并在运行目录下的 `.env` 存在时自动加载。
把 `config.json`、SQLite db 文件以及可选的 `signer.json` 等文件放在执行脚本的目录即可。
可以把 `config/example.node.json` 复制到该目录并命名为 `config.json`，修改后再启动容器。
容器内配置路径是 `/app/config/config.json`；配置里的 SQLite 路径通常写
`sqlite:data/node.db`，signer 路径通常写 `config/signer.json`。

如果需要从宿主机访问 debug HTTP，把挂载配置里的 `debug_http_addr` 设置为
`0.0.0.0:9909`；服务脚本默认发布到宿主机 `127.0.0.1:9909`。

## Releases

使用语义化的 `vMAJOR.MINOR.PATCH` tag 发布。辅助脚本会 bump `Cargo.toml`、
更新 `Cargo.lock`、运行测试、提交、创建 tag，并在 push 前再次确认：

```bash
./scripts/release.sh
```

推送 tag 后会触发 GitHub Actions release workflow。workflow 会构建 Linux x86_64
二进制、校验 `safe-node --version`，并上传 release archive 和 SHA-256 校验文件。
