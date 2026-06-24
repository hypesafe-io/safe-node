# safe-node

English | [中文](README.zh-CN.md)

`safe-node` is a self-hosted HypeSafe signer and policy node. It is not a
Node.js service and not a blockchain node.

The node signs in to `safe-gateway` with a local encrypted keystore, tracks one
configured Hyperliquid multi-sig account, evaluates local policy, and signs
allowed tasks. When the loaded signer is in `allowed_leaders`, the node also
handles executable tasks and submits to Hyperliquid only when the task leader is
the local signer.

## Install

Install the latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | bash
```

Optional environment variables:

- `SAFE_NODE_TAG`: install a specific release tag.
- `SAFE_NODE_INSTALL_DIR`: install into a specific directory.

Build locally:

```bash
cargo build --release --locked
```

## Setup

`safe-node` reads JSON5 config from `config/node.json` by default:

```bash
cp config/example.node.json config/node.json
safe-node keystore generate --out config/signer.json
```

Edit `config/node.json` before running:

- Set `leader`, `multisig`, and `signer.keystore_path`.
- Set `signer.password_env` if the keystore password should come from an
  environment variable.
- Narrow `allowed_templates`, `allowed_creators`, and `allowed_leaders` to the
  addresses and task types this node should trust.
- Use `template_input_policies` for per-template amount limits and destination
  allow lists. `withdraw_limit` remains the fallback for templates with an
  `amount` input and no template-specific amount rule.

If `allowed_creators` or `allowed_leaders` is omitted, it defaults to `leader`.
If `allowed_templates` is omitted, it defaults to `withdraw3` and
`sub_account_withdraw3`.

## Commands

```bash
safe-node run
safe-node run --config config/node.json
safe-node once --dry-run
safe-node tui
safe-node keystore import --out config/signer.json
```

`run` starts the polling loop and local RPC HTTP service. `once` processes a
single polling cycle. `tui` reads from the running node's RPC HTTP endpoint.

## RPC HTTP

By default, `safe-node run` serves `http://127.0.0.1:9909/`:

- `/`: browser dashboard.
- `/debug/status`, `/debug/config`, `/debug/policy`, `/debug/transactions`:
  read-only JSON endpoints.
- `POST /rpc/task/create`: creates a task using the node signer.

Set `rpc_auth_token` to require `Authorization: Bearer <token>` for write RPC
endpoints. If `rpc_http_addr` is bound to a non-localhost address, configure a
token.

## Docker

Build the image and run it as a service:

```bash
./scripts/docker-build.sh
./scripts/docker-run-service.sh
```

Run one polling cycle:

```bash
SAFE_NODE_DRY_RUN=1 ./scripts/docker-run-executor.sh
```

Docker helper scripts use `hypesafe/safe-node:local` by default, mount
`SAFE_NODE_RUN_DIR` or the current directory into `/app/config` and `/app/data`,
and load `.env` from that directory when it exists. Inside the container, the
default config path is `/app/config/config.json`.
