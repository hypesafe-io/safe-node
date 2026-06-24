#!/usr/bin/env python3
"""Create safe-node tasks through POST /rpc/task/create."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from typing import Any


DEFAULT_RPC_URL = "http://127.0.0.1:9909"
CREATE_TASK_PATH = "/rpc/task/create"
DEFAULT_EXPIRES_IN_SECS = 3600
DEFAULT_SEND_ASSET_ACCOUNT_TYPE = "spot"
DEFAULT_SEND_ASSET_TOKEN = "USDC"


class CliError(Exception):
    """User-facing CLI error."""


def env_value(name: str, default: str = "") -> str:
    value = os.environ.get(name, "").strip()
    return value if value else default


def create_task_url() -> str:
    return env_value("SAFE_NODE_RPC_URL", DEFAULT_RPC_URL).rstrip("/") + CREATE_TASK_PATH


def rpc_auth_token() -> str:
    return env_value("SAFE_NODE_RPC_AUTH_TOKEN")


def send_create_task(payload: dict[str, Any]) -> Any:
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    headers = {
        "Accept": "application/json",
        "Content-Type": "application/json",
    }
    token = rpc_auth_token()
    if token:
        headers["Authorization"] = f"Bearer {token}"

    request = urllib.request.Request(
        create_task_url(),
        data=body,
        headers=headers,
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            response_body = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        error_body = error.read().decode("utf-8", errors="replace")
        raise CliError(f"RPC returned HTTP {error.code}: {error_body}") from error
    except urllib.error.URLError as error:
        raise CliError(f"RPC request failed: {error.reason}") from error

    if not response_body:
        return {}
    try:
        return json.loads(response_body)
    except json.JSONDecodeError:
        return response_body


def build_withdraw_payload(args: argparse.Namespace) -> dict[str, Any]:
    return {
        "templateId": "withdraw3",
        "templateVersion": 1,
        "inputs": {
            "destination": args.destination,
            "amount": args.amount,
        },
        "expiresInSecs": args.expires_in_secs,
    }


def send_asset_token(args: argparse.Namespace) -> str:
    return args.token or DEFAULT_SEND_ASSET_TOKEN


def send_asset_dex_values(account_type: str) -> tuple[str, str]:
    if account_type == "spot":
        return ("spot", "spot")
    return ("", "")


def build_sub_account_in_payload(args: argparse.Namespace) -> dict[str, Any]:
    source_dex, destination_dex = send_asset_dex_values(args.account_type)
    return {
        "templateId": "send_asset",
        "templateVersion": 1,
        "inputs": {
            "destination": args.sub_account,
            "accountType": args.account_type,
            "sourceDex": source_dex,
            "destinationDex": destination_dex,
            "token": send_asset_token(args),
            "amount": args.amount,
            "fromSubAccount": "",
        },
        "expiresInSecs": args.expires_in_secs,
    }


def build_sub_account_out_payload(args: argparse.Namespace) -> dict[str, Any]:
    source_dex, destination_dex = send_asset_dex_values(args.account_type)
    return {
        "templateId": "send_asset",
        "templateVersion": 1,
        "inputs": {
            "destination": args.multisig,
            "accountType": args.account_type,
            "sourceDex": source_dex,
            "destinationDex": destination_dex,
            "token": send_asset_token(args),
            "amount": args.amount,
            "fromSubAccount": args.sub_account,
        },
        "expiresInSecs": args.expires_in_secs,
    }


def add_common_task_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--expires-in-secs",
        type=int,
        default=DEFAULT_EXPIRES_IN_SECS,
        help=f"task expiry in seconds, default: {DEFAULT_EXPIRES_IN_SECS}",
    )


def add_send_asset_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--sub-account", required=True, help="configured sub-account address")
    parser.add_argument("--amount", required=True, help="asset amount")
    parser.add_argument(
        "--account-type",
        choices=("perp", "spot"),
        default=DEFAULT_SEND_ASSET_ACCOUNT_TYPE,
        help=f"source balance type, default: {DEFAULT_SEND_ASSET_ACCOUNT_TYPE}",
    )
    parser.add_argument(
        "--token",
        default=DEFAULT_SEND_ASSET_TOKEN,
        help=(
            "token symbol for perp, or COIN:tokenId for spot; "
            f"default: {DEFAULT_SEND_ASSET_TOKEN}"
        ),
    )
    add_common_task_args(parser)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create safe-node tasks through POST /rpc/task/create.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    withdraw = subparsers.add_parser(
        "withdraw",
        help="create a withdraw3 task",
    )
    withdraw.add_argument("--destination", required=True, help="external Arbitrum address")
    withdraw.add_argument("--amount", required=True, help="USDC amount")
    add_common_task_args(withdraw)
    withdraw.set_defaults(build_payload=build_withdraw_payload)

    sub_account_in = subparsers.add_parser(
        "sub-account-in",
        help="transfer from the multisig main account to a sub-account",
    )
    add_send_asset_args(sub_account_in)
    sub_account_in.set_defaults(build_payload=build_sub_account_in_payload)

    sub_account_out = subparsers.add_parser(
        "sub-account-out",
        help="transfer from a sub-account back to the multisig account",
    )
    add_send_asset_args(sub_account_out)
    sub_account_out.add_argument(
        "--multisig",
        required=True,
        help="configured multisig account address; sent as inputs.destination",
    )
    sub_account_out.set_defaults(build_payload=build_sub_account_out_payload)

    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        payload = args.build_payload(args)
        response = send_create_task(payload)
    except CliError as error:
        print(str(error), file=sys.stderr)
        return 1

    if isinstance(response, str):
        print(response)
    else:
        print(json.dumps(response, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
