#!/usr/bin/env python3
"""Remove entries from a wasm module's `target_features` custom section.

wasm-bindgen 0.2.96+ reads that section and, when it sees `+reference-types`,
keeps JS values in a second (externref) table. The prebuilt Rust std for
wasm32-unknown-unknown advertises the feature (Rust 1.82+), so no rustflag can
hide it. Old binaryen (Debian 108, Ubuntu 22.04's 99) mishandles the second
table and the module fails at the first call with "Table.grow(): failed to grow
table". Hiding the feature makes wasm-bindgen use its JS heap array instead.

usage: wasm-strip-features.py IN.wasm OUT.wasm FEATURE [FEATURE...]
"""
import sys


def read_leb(data, i):
    result = shift = 0
    while True:
        b = data[i]
        i += 1
        result |= (b & 0x7F) << shift
        shift += 7
        if not b & 0x80:
            return result, i


def write_leb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def main():
    src, dst, *drop = sys.argv[1:]
    data = open(src, "rb").read()
    assert data[:4] == b"\0asm", "not a wasm module"
    out = bytearray(data[:8])
    i = 8
    removed = []
    while i < len(data):
        sid = data[i]
        size, body_start = read_leb(data, i + 1)
        body = data[body_start : body_start + size]
        i = body_start + size
        if sid == 0:
            name_len, j = read_leb(body, 0)
            name = body[j : j + name_len]
            if name == b"target_features":
                k = j + name_len
                count, k = read_leb(body, k)
                feats = []
                for _ in range(count):
                    prefix = body[k]
                    flen, k2 = read_leb(body, k + 1)
                    fname = body[k2 : k2 + flen]
                    k = k2 + flen
                    if fname.decode() in drop:
                        removed.append(fname.decode())
                    else:
                        feats.append((prefix, fname))
                new_body = bytearray(write_leb(name_len) + name + write_leb(len(feats)))
                for prefix, fname in feats:
                    new_body += bytes([prefix]) + write_leb(len(fname)) + fname
                out += bytes([0]) + write_leb(len(new_body)) + new_body
                continue
        out += bytes([sid]) + write_leb(size) + body
    open(dst, "wb").write(out)
    print(f"target_features: removed {removed or 'nothing'}")


if __name__ == "__main__":
    main()
