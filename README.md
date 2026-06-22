# safe-node

English | [中文](README.zh-CN.md)

`safe-node` is a local signing and risk-control node for HypeSafe. It is not a Node.js service and not a blockchain node.

It pulls tasks for one configured multi-sig account from `safe-gateway` and handles the task templates allowed by local policy for the configured `leader`. A normal signer node only co-signs. If the local signer equals the configured `leader`, the node also submits Hyperliquid `/exchange` requests for on-chain templates and writes results back to the gateway.

## Features

- JSON config, default path: `config/node.json`.
- Encrypted local keystore for signer keys.
- Keystore password can come from an environment variable; otherwise it is entered interactively at startup.
- Template allow-list policy with an amount limit for templates that declare an `amount` field.
- Policy rejects are submitted to the gateway as reject votes when possible; no signing or Hyperliquid submission is performed.
- Local SQLite state, with SQL access through SeaORM.
- Read-only debug HTTP endpoint in the main service.
- `safe-node tui` shows recent transactions, config summary, and policy state in the terminal.

## Commands

```bash
safe-node keystore generate --out config/signer.json
safe-node keystore import --out config/signer.json
safe-node run
safe-node once --dry-run
safe-node tui
```

`--config` is optional. Use it only when a non-default config path is needed:

```bash
safe-node run --config config/node.json
```

## Config

Use `config/example.node.json` as the full config template:

```bash
cp config/example.node.json config/node.json
```

Before running a node, edit the copied config and set the real `leader`,
`multisig`, signer keystore path, and policy values.

`allowed_templates` is the task template allow list. Values are plain gateway
`template_id` strings, not a closed enum. `config/example.node.json` lists every
current signing template exposed by the gateway; keep the list narrower if the
node should only sign selected business actions. Built-in defaults remain
conservative:

- `withdraw3`
- `sub_account_withdraw3`

## Signing Trust Boundary

Before signing, `safe-node` now verifies that gateway-provided EIP-712 typed-data
matches the digest declared by the task/signing-payload response. This prevents
signing a payload that does not match the task digest returned by `safe-gateway`.
It is still a partial trust model: the node does not yet independently rebuild
the typed-data from `template + inputs + ctx`, so `safe-gateway` remains the
source of payload construction until that stronger validation lands.

## Debug HTTP

When `safe-node run` is active, open `http://127.0.0.1:9909/` for the read-only
browser dashboard. The JSON endpoints remain available under `/debug/status`,
`/debug/config`, `/debug/policy`, and `/debug/transactions`.

## Docker

Build the local image:

```bash
./scripts/docker-build.sh
```

The default image name is `hypesafe/safe-node:local`. Override it when needed:

```bash
SAFE_NODE_IMAGE=registry.example.com/hypesafe/safe-node:tag ./scripts/docker-build.sh
```

The image entrypoint is `safe-node` and the default command is `run`, so the
same image supports both service and one-shot executor modes.

Use the image as a one-shot executor:

```bash
./scripts/docker-run-executor.sh --dry-run
```

Equivalent direct Docker command:

```bash
docker run --rm \
  --user "$(id -u):$(id -g)" \
  --env HOME=/tmp \
  --volume "$PWD:/app/config:ro" \
  --volume "$PWD:/app/data" \
  hypesafe/safe-node:local \
  once --config /app/config/config.json --dry-run
```

Start the image as a long-running service:

```bash
./scripts/docker-run-service.sh
docker logs -f safe-node
```

Equivalent direct Docker command:

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

The scripts mount the current working directory into both `/app/config`
read-only and `/app/data` read-write, run the container with the current host
UID/GID, and load `.env` from that directory automatically if it exists. Keep
`config.json`, the SQLite database file, and optional files such as
`signer.json` in the directory where you run the script. You can copy
`config/example.node.json` to that directory as `config.json` and edit it before
starting the container. Inside the container, the config path is
`/app/config/config.json`; a typical SQLite path in the
config is `sqlite:data/node.db`, and a typical signer path is
`config/signer.json`.

For host access to the debug HTTP endpoint, set `debug_http_addr` in the mounted
config to `0.0.0.0:9909`; the service script publishes it on host
`127.0.0.1:9909` by default.

## Releases

Create releases with a semantic `vMAJOR.MINOR.PATCH` tag. The helper script
bumps `Cargo.toml`, updates `Cargo.lock`, runs tests, commits, creates the tag,
and asks before pushing:

```bash
./scripts/release.sh
```

Pushing the tag starts the GitHub Actions release workflow. The workflow builds
the Linux x86_64 binary, verifies `safe-node --version`, and uploads the release
archive plus its SHA-256 checksum.

Install the latest release by downloading the installer script from this
repository. The script then downloads the matching release archive, verifies its
SHA-256 checksum, and unpacks it locally:

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | bash
```

Install a specific release tag:

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | SAFE_NODE_TAG=v0.1.0 bash
```

Choose the install directory:

```bash
curl -fsSL https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh | SAFE_NODE_INSTALL_DIR="$HOME/safe-node" bash
```

The public repository does not require a token. For a private fork or GitHub API
rate limits, pass a token to both the script download and release download:

```bash
curl -fsSL \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/hypesafe-io/safe-node/main/scripts/install.sh \
  | GITHUB_TOKEN="$GITHUB_TOKEN" bash
```
