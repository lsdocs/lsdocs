"""An lsdocs parser plugin written from the protocol spec alone.

It uses only Python's standard library: no lsdocs bindings and no
MessagePack package (a small codec is below). It parses `.txt` files into
paragraphs, one per run of non-blank lines, and shows everything a plugin
in another language needs: framing, the handshake, requests, and shutdown.
See spec/wire.md.

Run by lsdocs from `lsdocs.toml`:

    [[plugins]]
    name = "txt"
    command = ["python3", "txt_parser.py"]
"""

import struct
import sys

PROTOCOL_VERSION = "1.0-rc.0"

# -- MessagePack ---------------------------------------------------------------


def pack(value) -> bytes:
    """Encodes `value` (None, bool, int, float, str, bytes, list, dict)."""
    if value is None:
        return b"\xc0"
    if value is True:
        return b"\xc3"
    if value is False:
        return b"\xc2"
    if isinstance(value, int):
        if 0 <= value < 0x80:
            return bytes([value])
        if -32 <= value < 0:
            return struct.pack("b", value)
        if value >= 0:
            return b"\xcf" + struct.pack(">Q", value)
        return b"\xd3" + struct.pack(">q", value)
    if isinstance(value, float):
        return b"\xcb" + struct.pack(">d", value)
    if isinstance(value, str):
        data = value.encode("utf-8")
        n = len(data)
        if n < 32:
            return bytes([0xA0 | n]) + data
        return b"\xdb" + struct.pack(">I", n) + data
    if isinstance(value, bytes):
        return b"\xc6" + struct.pack(">I", len(value)) + value
    if isinstance(value, (list, tuple)):
        head = bytes([0x90 | len(value)]) if len(value) < 16 else b"\xdd" + struct.pack(">I", len(value))
        return head + b"".join(pack(v) for v in value)
    if isinstance(value, dict):
        head = bytes([0x80 | len(value)]) if len(value) < 16 else b"\xdf" + struct.pack(">I", len(value))
        return head + b"".join(pack(k) + pack(v) for k, v in value.items())
    raise TypeError(f"cannot encode {type(value).__name__}")


def unpack(data: bytes):
    """Decodes one MessagePack value from `data`."""
    value, end = _unpack(data, 0)
    if end != len(data):
        raise ValueError("trailing bytes")
    return value


def _unpack(d: bytes, i: int):
    b = d[i]
    i += 1
    if b < 0x80:
        return b, i
    if b >= 0xE0:
        return b - 0x100, i
    if 0x80 <= b <= 0x8F:
        return _map(d, i, b & 0x0F)
    if 0x90 <= b <= 0x9F:
        return _array(d, i, b & 0x0F)
    if 0xA0 <= b <= 0xBF:
        n = b & 0x1F
        return d[i:i + n].decode("utf-8"), i + n
    fixed = {
        0xC0: None, 0xC2: False, 0xC3: True,
    }
    if b in fixed:
        return fixed[b], i
    # Sized types: (struct format, size of the length or value).
    ints = {0xCC: ">B", 0xCD: ">H", 0xCE: ">I", 0xCF: ">Q",
            0xD0: ">b", 0xD1: ">h", 0xD2: ">i", 0xD3: ">q",
            0xCA: ">f", 0xCB: ">d"}
    if b in ints:
        fmt = ints[b]
        size = struct.calcsize(fmt)
        return struct.unpack(fmt, d[i:i + size])[0], i + size
    lengths = {0xD9: ">B", 0xDA: ">H", 0xDB: ">I",   # str
               0xC4: ">B", 0xC5: ">H", 0xC6: ">I",   # bin
               0xDC: ">H", 0xDD: ">I",               # array
               0xDE: ">H", 0xDF: ">I"}               # map
    if b in lengths:
        fmt = lengths[b]
        size = struct.calcsize(fmt)
        n = struct.unpack(fmt, d[i:i + size])[0]
        i += size
        if b in (0xD9, 0xDA, 0xDB):
            return d[i:i + n].decode("utf-8"), i + n
        if b in (0xC4, 0xC5, 0xC6):
            return d[i:i + n], i + n
        if b in (0xDC, 0xDD):
            return _array(d, i, n)
        return _map(d, i, n)
    raise ValueError(f"unsupported MessagePack type 0x{b:02x}")


def _array(d: bytes, i: int, n: int):
    out = []
    for _ in range(n):
        v, i = _unpack(d, i)
        out.append(v)
    return out, i


def _map(d: bytes, i: int, n: int):
    out = {}
    for _ in range(n):
        k, i = _unpack(d, i)
        v, i = _unpack(d, i)
        out[k] = v
    return out, i


# -- Framing -------------------------------------------------------------------


def read_message(stream):
    """The next message, or None at the end of the stream."""
    header = stream.read(4)
    if len(header) < 4:
        return None
    (length,) = struct.unpack(">I", header)
    return unpack(stream.read(length))


def write_message(stream, message) -> None:
    payload = pack(message)
    stream.write(struct.pack(">I", len(payload)) + payload)
    stream.flush()


# -- The parser ------------------------------------------------------------------


def parse(source: str, text: str) -> dict:
    """Paragraphs of `text`, with byte spans."""
    children = []
    offset = 0
    lines = []  # (start, end, text) of the current paragraph's lines
    for line in text.splitlines(keepends=True):
        size = len(line.encode("utf-8"))
        content = line.rstrip("\r\n")
        if content.strip():
            lines.append((offset, offset + len(content.encode("utf-8")), content))
        elif lines:
            children.append(paragraph(lines))
            lines = []
        offset += size
    if lines:
        children.append(paragraph(lines))
    return {"source": source, "children": children}


def paragraph(lines) -> dict:
    span = {"start": lines[0][0], "end": lines[-1][1]}
    value = "\n".join(text for _, _, text in lines)
    return {"type": "paragraph", "span": span,
            "children": [{"type": "text", "span": span, "value": value}]}


def handle(method: str, params: list):
    """The result of a request; raises KeyError for unknown methods."""
    if method == "initialize":
        return {
            "protocol_version": PROTOCOL_VERSION,
            "plugin": {"name": "txt-parser", "version": "1.0.0"},
            "capabilities": {"parse": {"extensions": ["txt"]}, "cache": ""},
        }
    if method == "parse":
        p = params[0]
        return {"document": parse(p["source"], p["text"])}
    if method == "shutdown":
        return None
    raise KeyError(method)


def main() -> None:
    stdin, stdout = sys.stdin.buffer, sys.stdout.buffer
    # Stray prints must not reach the protocol stream.
    sys.stdout = sys.stderr
    while (message := read_message(stdin)) is not None:
        if message[0] == 0:  # request: [0, id, method, params]
            _, msgid, method, params = message
            try:
                write_message(stdout, [1, msgid, None, handle(method, params)])
            except KeyError:
                error = {"code": -32601, "message": f"no handler for `{method}`"}
                write_message(stdout, [1, msgid, error, None])
        elif message[0] == 2 and message[1] == "exit":  # notification
            break


if __name__ == "__main__":
    main()
