# safe-node RPC create task examples

This directory contains a dependency-free Python example for creating tasks
through safe-node's shared RPC HTTP service.

The script calls:

```text
POST /rpc/task/create
```

safe-node must already be running with `rpc_http_addr` enabled. By default the
example connects to `http://127.0.0.1:9909`.

## Configuration

Set the RPC URL when the node listens on a different address:

```sh
export SAFE_NODE_RPC_URL=http://127.0.0.1:9909
```

When `rpc_auth_token` is empty or omitted in the safe-node config, no token is
required. When it is configured, set:

```sh
export SAFE_NODE_RPC_AUTH_TOKEN=your-token
```

The script sends `Authorization: Bearer <token>` only when
`SAFE_NODE_RPC_AUTH_TOKEN` is non-empty.

## Withdraw

Create a `withdraw3` task:

```sh
python3 safe-node/example/create_task.py withdraw \
  --destination 0x1111111111111111111111111111111111111111 \
  --amount 1
```

## Transfer into a sub-account

Create a `send_asset` task from the multisig main account to a configured
sub-account:

```sh
python3 safe-node/example/create_task.py sub-account-in \
  --sub-account 0x2222222222222222222222222222222222222222 \
  --amount 1
```

By default this transfers from the multisig spot balance into the sub-account
perp balance:

```json
{
  "accountType": "spot",
  "sourceDex": "spot",
  "destinationDex": "",
  "token": "USDC",
  "fromSubAccount": ""
}
```

`--account-type` selects the source balance and
`--destination-account-type` selects the destination balance. For example, to
transfer perp to perp:

```sh
python3 safe-node/example/create_task.py sub-account-in \
  --sub-account 0x2222222222222222222222222222222222222222 \
  --amount 1 \
  --account-type perp \
  --destination-account-type perp
```

For a specific spot token, keep `--account-type spot` and pass the token value
expected by Hyperliquid's `sendAsset` action:

```sh
python3 safe-node/example/create_task.py sub-account-in \
  --sub-account 0x2222222222222222222222222222222222222222 \
  --amount 1 \
  --account-type spot \
  --destination-account-type spot \
  --token USDC:0x6d1e7cde53ba9467b783cb7c530ce054
```

## Transfer out of a sub-account

Create a `send_asset` task from a configured sub-account back to the configured
multisig account:

```sh
python3 safe-node/example/create_task.py sub-account-out \
  --sub-account 0x2222222222222222222222222222222222222222 \
  --multisig 0x3333333333333333333333333333333333333333 \
  --amount 1
```

This uses the reverse defaults: the sub-account perp balance is the source and
the multisig spot balance is the destination. Override either side with
`--account-type` or `--destination-account-type` when needed.

`--multisig` is sent as `inputs.destination`. The RPC request does not send the
forbidden top-level fields `creator`, `leader`, `multisig`, or `network`.

## Notes

`sub-account-in` and `sub-account-out` use the `send_asset` template. They must
pass the safe-node local template allowlist, input policy, and cached
sub-account registry checks before safe-node submits the task to the gateway.

All commands accept `--expires-in-secs`, which defaults to `3600`.
