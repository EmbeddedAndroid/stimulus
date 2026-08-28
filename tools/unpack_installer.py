#!/usr/bin/env python3
"""Unpack the vendor Windows installer for the USB logic analyzer.

The installer is a SetupBuilder 10 self-extracting PE:

  offset 0x0000        32-bit PE stub (imports LZ32.dll: LZInit/LZCopy)
  offset 0x4200        SZDD (MS LZ) stream -> 15,360-byte loader DLL
  offset 0x66ab        payload archive: a chain of 'LS\\x03\\x04' records

Each archive record is a 62-byte header followed by the member name and the
member's Deflate64-compressed bytes. Deflate64 (PKZip method 9) is NOT
supported by Python's zlib, hence the bundled inflate64 module.

Usage:  python3 unpack_installer.py <installer.exe> <output-dir>
"""
import binascii
import os
import struct
import sys

from inflate64 import inflate64

SZDD_MAGIC = b'SZDD\x88\xf0\x27\x33'
REC_MAGIC = b'LS\x03\x04'
HDR = 62

# --- Record header layout (all little-endian) -------------------------------
#   0  4  'LS\x03\x04'
#   4  4  header version (observed 0x32)
#   8  2  compression method (9 = Deflate64, 8 = Deflate)
#  10  8  FILETIME (creation)
#  18  8  FILETIME (access)
#  26  8  FILETIME (modification)
#  34  4  Win32 file attributes
#  38  4  CRC32 of the uncompressed member
#  42  4  compressed size
#  46  4  uncompressed size
#  50  8  reserved (observed zero)
#  58  4  name length
#  62  n  member name (ASCII)
#         <compressed size> bytes of payload follow


def szdd_decompress(data, off):
    """Decompress the MS SZDD/LZ stream used for the installer's loader DLL."""
    assert data[off:off+8] == SZDD_MAGIC, 'not an SZDD stream'
    outlen = struct.unpack('<I', data[off+10:off+14])[0]
    p = off + 14
    win = bytearray(b'\x20' * 4096)
    wp = 4096 - 16
    out = bytearray()
    while p < len(data) and len(out) < outlen:
        ctrl = data[p]; p += 1
        for bit in range(8):
            if len(out) >= outlen or p >= len(data):
                break
            if ctrl & (1 << bit):
                b = data[p]; p += 1
                out.append(b); win[wp] = b; wp = (wp + 1) & 0xFFF
            else:
                b1, b2 = data[p], data[p+1]; p += 2
                mpos = b1 | ((b2 & 0xF0) << 4)
                for _ in range((b2 & 0x0F) + 3):
                    if len(out) >= outlen:
                        break
                    c = win[mpos & 0xFFF]; mpos += 1
                    out.append(c); win[wp] = c; wp = (wp + 1) & 0xFFF
    return bytes(out)


def find_archive_start(data):
    i = data.find(REC_MAGIC)
    if i < 0:
        raise SystemExit('no LS\\x03\\x04 archive found')
    return i


def unpack(src, outdir):
    data = open(src, 'rb').read()
    os.makedirs(outdir, exist_ok=True)

    szdd = data.find(SZDD_MAGIC)
    if szdd >= 0:
        loader = szdd_decompress(data, szdd)
        open(os.path.join(outdir, '_loader_stub.dll'), 'wb').write(loader)
        print(f'SZDD loader stub @0x{szdd:x} -> {len(loader)} bytes')

    off = find_archive_start(data)
    print(f'archive starts @0x{off:x}')
    n = ok = 0
    while off < len(data) - HDR and data[off:off+HDR-58] == REC_MAGIC:
        meth = struct.unpack('<H', data[off+8:off+10])[0]
        crc, csize, usize = struct.unpack('<III', data[off+38:off+50])
        namelen = struct.unpack('<I', data[off+58:off+62])[0]
        name = data[off+HDR:off+HDR+namelen].decode('latin1')
        ds = off + HDR + namelen
        blob = data[ds:ds+csize]
        n += 1
        try:
            # method 9 = Deflate64; method 8 = classic deflate (inflate64 is a
            # strict superset, so the same decoder handles both).
            out = inflate64(blob, usize)
        except Exception as exc:                       # noqa: BLE001
            print(f'  FAIL {name}: {exc}')
            off = ds + csize
            continue
        safe = name.replace('/', '_').replace('\\', '_')
        path = os.path.join(outdir, safe)
        i = 2
        while os.path.exists(path):
            b, e = os.path.splitext(safe)
            path = os.path.join(outdir, f'{b}__{i}{e}')
            i += 1
        open(path, 'wb').write(out)
        good = (binascii.crc32(out) & 0xFFFFFFFF) == crc
        ok += good
        print(f'  m={meth} {usize:>9}  crc={"OK " if good else "BAD"}  {name}')
        off = ds + csize
    print(f'\n{ok}/{n} members extracted with matching CRC32')


if __name__ == '__main__':
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    unpack(sys.argv[1], sys.argv[2])
