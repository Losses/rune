#!/usr/bin/env python3
"""Minimal Dart VM service client (no external deps).

Usage:
  vm_eval.py <ws_base> getvm
  vm_eval.py <ws_base> libs <isolateId>
  vm_eval.py <ws_base> eval <isolateId> <targetId> <expression>

<ws_base> like 127.0.0.1:32939/dq3CXKCT-Jg=
"""
import base64
import hashlib
import json
import os
import socket
import sys
import uuid


def ws_connect(base):
    hostport, _, path = base.partition("/")
    host, _, port = hostport.partition(":")
    s = socket.create_connection((host, int(port)))
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET /{path}/ws HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    s.sendall(req.encode())
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += s.recv(4096)
    assert b"101" in resp.split(b"\r\n")[0], resp
    return s


def ws_send(s, payload: bytes):
    header = bytearray([0x81])
    n = len(payload)
    if n < 126:
        header.append(0x80 | n)
    elif n < 65536:
        header.append(0x80 | 126)
        header += n.to_bytes(2, "big")
    else:
        header.append(0x80 | 127)
        header += n.to_bytes(8, "big")
    mask = os.urandom(4)
    header += mask
    s.sendall(bytes(header) + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))


def _recv_exact(s, n):
    buf = b""
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("eof")
        buf += chunk
    return buf


def ws_recv(s):
    data = b""
    while True:
        b1, b2 = _recv_exact(s, 2)
        fin = b1 & 0x80
        opcode = b1 & 0x0F
        n = b2 & 0x7F
        if n == 126:
            n = int.from_bytes(_recv_exact(s, 2), "big")
        elif n == 127:
            n = int.from_bytes(_recv_exact(s, 8), "big")
        if b2 & 0x80:
            mask = _recv_exact(s, 4)
            payload = bytes(x ^ mask[i % 4] for i, x in enumerate(_recv_exact(s, n)))
        else:
            payload = _recv_exact(s, n)
        if opcode == 9:  # ping -> pong
            s.sendall(bytes([0x8A, 0x80]) + os.urandom(4))
            continue
        if opcode == 8:
            raise ConnectionError("closed")
        data += payload
        if fin:
            return data


def rpc(s, method, params):
    rid = uuid.uuid4().hex[:8]
    ws_send(s, json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}).encode())
    while True:
        msg = json.loads(ws_recv(s))
        if msg.get("id") == rid:
            return msg


def main():
    base = sys.argv[1].removeprefix("http://").removesuffix("/")
    cmd = sys.argv[2]
    s = ws_connect(base)
    if cmd == "getvm":
        out = rpc(s, "getVM", {})
        print(json.dumps(out))
    elif cmd == "libs":
        out = rpc(s, "getIsolate", {"isolateId": sys.argv[3]})
        iso = out.get("result", {})
        for lib in iso.get("libraries", []):
            print(lib.get("id"), lib.get("uri"))
    elif cmd == "eval":
        out = rpc(s, "evaluate", {
            "isolateId": sys.argv[3],
            "targetId": sys.argv[4],
            "expression": sys.argv[5],
        })
        print(json.dumps(out))
    elif cmd == "evalfile":
        with open(sys.argv[5]) as f:
            expr = f.read()
        out = rpc(s, "evaluate", {
            "isolateId": sys.argv[3],
            "targetId": sys.argv[4],
            "expression": expr,
        })
        print(json.dumps(out))
    else:
        raise SystemExit(f"unknown cmd {cmd}")


if __name__ == "__main__":
    main()
