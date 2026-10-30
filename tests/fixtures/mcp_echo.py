#!/usr/bin/env python3
"""Minimal stdio MCP server used by lx manager integration tests.

Supports initialize, tools/list and tools/call. Two tools:
- echo(text): returns "echo:<text>"
- sleep(seconds): sleeps before returning (used for cancellation tests)
Writes its pid to MCP_PID_FILE when set.
"""
import json
import os
import sys
import time

PID_FILE = os.environ.get("MCP_PID_FILE")
TOOLS_FILE = os.environ.get("MCP_TOOLS_FILE")
if PID_FILE:
    with open(PID_FILE, "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))

TOOLS = [
    {
        "name": "echo",
        "description": "Echo text back",
        "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
    },
    {
        "name": "sleep",
        "description": "Sleep then return",
        "inputSchema": {"type": "object", "properties": {"seconds": {"type": "number"}}},
    },
]


def current_tools():
    if TOOLS_FILE and os.path.exists(TOOLS_FILE):
        with open(TOOLS_FILE, "r", encoding="utf-8") as handle:
            return json.load(handle)
    return TOOLS


def respond(request_id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}) + "\n")
    sys.stdout.flush()


def respond_error(request_id, code, message):
    sys.stdout.write(
        json.dumps({"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}})
        + "\n"
    )
    sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = message.get("method")
        if method == "initialize":
            respond(
                message.get("id"),
                {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "echo", "version": "0.0.1"},
                },
            )
        elif method == "tools/list":
            respond(message.get("id"), {"tools": current_tools()})
        elif method == "resources/list":
            respond(message.get("id"), {"resources": [{"uri": "echo://doc", "name": "doc", "description": "fixture resource", "mimeType": "text/plain"}]})
        elif method == "resources/templates/list":
            respond(message.get("id"), {"resourceTemplates": [{"uriTemplate": "echo://{name}", "name": "echo"}]})
        elif method == "resources/read":
            params = message.get("params") or {}
            if params.get("uri") == "echo://slow":
                time.sleep(10)
            respond(message.get("id"), {"contents": [{"uri": "echo://doc", "mimeType": "text/plain", "text": "resource-body"}]})
        elif method == "tools/call":
            params = message.get("params") or {}
            name = params.get("name")
            arguments = params.get("arguments") or {}
            if name == "sleep":
                time.sleep(float(arguments.get("seconds", 30)))
                respond(message.get("id"), {"content": [{"type": "text", "text": "slept"}]})
            elif arguments.get("text") == "__rpc_error__":
                respond_error(message.get("id"), -32602, "invalid params")
            else:
                text = str(arguments.get("text", ""))
                respond(
                    message.get("id"),
                    {"content": [{"type": "text", "text": "echo:" + text}]},
                )
        elif method == "notifications/initialized":
            continue
        elif message.get("id") is not None:
            respond(message.get("id"), {"content": [{"type": "text", "text": "ok"}]})


if __name__ == "__main__":
    main()
