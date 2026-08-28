#!/usr/bin/env python3
"""Pure-Python Deflate64 (Enhanced Deflate, PKZip method 9) decompressor.

Differences from RFC1951 deflate:
  * length code 285 -> base 3 with 16 extra bits (instead of fixed length 258)
  * distance codes 30 and 31 exist -> bases 32769 / 49153 with 14 extra bits
  * history window is 64 KiB instead of 32 KiB
"""

LENGTH_BASE = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35,
               43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 3]
LENGTH_EXTRA = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
                4, 4, 4, 4, 5, 5, 5, 5, 16]

DIST_BASE = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257,
             385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289,
             16385, 24577, 32769, 49153]
DIST_EXTRA = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8,
              9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14]

CLEN_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]


class Huffman:
    """Canonical Huffman decoder built from a code-length vector."""
    __slots__ = ('counts', 'symbols')

    def __init__(self, lengths):
        maxbits = max(lengths) if lengths else 0
        counts = [0] * (maxbits + 1)
        for l in lengths:
            if l:
                counts[l] += 1
        offs = [0] * (maxbits + 2)
        for i in range(1, maxbits + 1):
            offs[i + 1] = offs[i] + counts[i]
        symbols = [0] * sum(counts[1:])
        for sym, l in enumerate(lengths):
            if l:
                symbols[offs[l]] = sym
                offs[l] += 1
        self.counts = counts
        self.symbols = symbols


class BitReader:
    __slots__ = ('data', 'pos', 'bitbuf', 'bitcnt')

    def __init__(self, data):
        self.data = data
        self.pos = 0
        self.bitbuf = 0
        self.bitcnt = 0

    def bits(self, n):
        while self.bitcnt < n:
            if self.pos >= len(self.data):
                raise EOFError('out of input')
            self.bitbuf |= self.data[self.pos] << self.bitcnt
            self.pos += 1
            self.bitcnt += 8
        v = self.bitbuf & ((1 << n) - 1)
        self.bitbuf >>= n
        self.bitcnt -= n
        return v

    def decode(self, h):
        code = first = index = 0
        counts = h.counts
        for length in range(1, len(counts)):
            code |= self.bits(1)
            count = counts[length]
            if code - first < count:
                return h.symbols[index + (code - first)]
            index += count
            first = (first + count) << 1
            code <<= 1
        raise ValueError('bad huffman code')

    def align_byte(self):
        self.bitbuf = 0
        self.bitcnt = 0


def _fixed_trees(_cache={}):
    if not _cache:
        litlen = [8] * 144 + [9] * 112 + [7] * 24 + [8] * 8
        _cache['lit'] = Huffman(litlen)
        _cache['dist'] = Huffman([5] * 32)
    return _cache['lit'], _cache['dist']


def inflate64(data, expected=None):
    br = BitReader(data)
    out = bytearray()
    while True:
        final = br.bits(1)
        btype = br.bits(2)
        if btype == 0:
            br.align_byte()
            if br.pos + 4 > len(br.data):
                raise EOFError('truncated stored block')
            ln = br.data[br.pos] | (br.data[br.pos + 1] << 8)
            br.pos += 4
            out += br.data[br.pos:br.pos + ln]
            br.pos += ln
        elif btype in (1, 2):
            if btype == 1:
                lit, dist = _fixed_trees()
            else:
                hlit = br.bits(5) + 257
                hdist = br.bits(5) + 1
                hclen = br.bits(4) + 4
                clens = [0] * 19
                for i in range(hclen):
                    clens[CLEN_ORDER[i]] = br.bits(3)
                ctree = Huffman(clens)
                lens = []
                while len(lens) < hlit + hdist:
                    sym = br.decode(ctree)
                    if sym < 16:
                        lens.append(sym)
                    elif sym == 16:
                        lens += [lens[-1]] * (3 + br.bits(2))
                    elif sym == 17:
                        lens += [0] * (3 + br.bits(3))
                    else:
                        lens += [0] * (11 + br.bits(7))
                lit = Huffman(lens[:hlit])
                dist = Huffman(lens[hlit:hlit + hdist])
            while True:
                sym = br.decode(lit)
                if sym < 256:
                    out.append(sym)
                elif sym == 256:
                    break
                else:
                    i = sym - 257
                    if i >= len(LENGTH_BASE):
                        raise ValueError(f'bad length symbol {sym}')
                    length = LENGTH_BASE[i]
                    if LENGTH_EXTRA[i]:
                        length += br.bits(LENGTH_EXTRA[i])
                    dsym = br.decode(dist)
                    if dsym >= len(DIST_BASE):
                        raise ValueError(f'bad distance symbol {dsym}')
                    d = DIST_BASE[dsym]
                    if DIST_EXTRA[dsym]:
                        d += br.bits(DIST_EXTRA[dsym])
                    if d > len(out):
                        raise ValueError(f'distance {d} exceeds output {len(out)}')
                    start = len(out) - d
                    if d >= length:
                        out += out[start:start + length]
                    else:
                        for k in range(length):
                            out.append(out[start + k])
        else:
            raise ValueError('invalid block type 3')
        if final:
            break
        if expected is not None and len(out) >= expected:
            break
    return bytes(out)
