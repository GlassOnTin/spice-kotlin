#![allow(non_upper_case_globals)] // DEFmaxclen etc. mirror the spice-common C names

// HAVEN: SPICE QUIC image decoder — a faithful Rust port of spice-common
// common/quic.c + quic_tmpl.c + quic_family_tmpl.c (LGPL-2.1, Red Hat).
// QUIC = SFALIC (Starosolski) / LOCO-I style spatial predictor + adaptive
// Golomb-Rice coding through an MSB bit reader, with MELCODE run lengths.
//
// Decode-only. Implemented image types: RGB24 (3), RGB32 (4) and RGBA (5) —
// all 8bpc. RGB uses 3 interleaved channels on a shared wait-mask state; RGBA
// adds a 4th (alpha) channel decoded as a separate per-row pass with its own
// state. Output is always opaque RGBA (the alpha plane is decoded to consume
// the bitstream but forced to 255, matching the BGRx primary-surface path).
// GRAY (1) and RGB16 (2) are deferred (need the 5bpc family / single-plane).

use std::sync::OnceLock;
use tracing::warn;

// ---- constants (quic.c) ----
const QUIC_MAGIC: u32 = 0x4349_5551; // "QUIC"
const QUIC_VERSION: u32 = 0; // (0<<16)|0

const MAXNUMCODES: usize = 8;
const DEFmaxclen: i32 = 26;
const DEFwmimax: i32 = 6;
const DEFwminext: i32 = 2048;
const MELCSTATES: i32 = 32;

#[allow(clippy::unreadable_literal)]
const BPPMASK: [u32; 33] = [
    0x00000000, 0x00000001, 0x00000003, 0x00000007, 0x0000000f, 0x0000001f, 0x0000003f, 0x0000007f,
    0x000000ff, 0x000001ff, 0x000003ff, 0x000007ff, 0x00000fff, 0x00001fff, 0x00003fff, 0x00007fff,
    0x0000ffff, 0x0001ffff, 0x0003ffff, 0x0007ffff, 0x000fffff, 0x001fffff, 0x003fffff, 0x007fffff,
    0x00ffffff, 0x01ffffff, 0x03ffffff, 0x07ffffff, 0x0fffffff, 0x1fffffff, 0x3fffffff, 0x7fffffff,
    0xffffffff,
];

const LZEROES: [u8; 256] = [
    8, 7, 6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const J: [i32; 32] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13,
    14, 15,
];

// wm_trigger table indexed [DEFevol/2][min(wmidx,10)]; DEFevol=3 -> row 1.
const BESTTRIGTAB: [[u16; 11]; 3] = [
    [550, 900, 800, 700, 500, 350, 300, 200, 180, 180, 160],
    [110, 550, 900, 800, 550, 400, 350, 250, 140, 160, 140],
    [100, 120, 550, 900, 700, 500, 400, 300, 220, 250, 160],
];

#[allow(clippy::unreadable_literal)]
const TABRAND_CHAOS: [u32; 256] = [
    0x02c57542, 0x35427717, 0x2f5a2153, 0x9244f155, 0x7bd26d07, 0x354c6052, 0x57329b28, 0x2993868e,
    0x6cd8808c, 0x147b46e0, 0x99db66af, 0xe32b4cac, 0x1b671264, 0x9d433486, 0x62a4c192, 0x06089a4b,
    0x9e3dce44, 0xdaabee13, 0x222425ea, 0xa46f331d, 0xcd589250, 0x8bb81d7f, 0xc8b736b9, 0x35948d33,
    0xd7ac7fd0, 0x5fbe2803, 0x2cfbc105, 0x013dbc4e, 0x7a37820f, 0x39f88e9e, 0xedd58794, 0xc5076689,
    0xfcada5a4, 0x64c2f46d, 0xb3ba3243, 0x8974b4f9, 0x5a05aebd, 0x20afcd00, 0x39e2b008, 0x88a18a45,
    0x600bde29, 0xf3971ace, 0xf37b0a6b, 0x7041495b, 0x70b707ab, 0x06beffbb, 0x4206051f, 0xe13c4ee3,
    0xc1a78327, 0x91aa067c, 0x8295f72a, 0x732917a6, 0x1d871b4d, 0x4048f136, 0xf1840e7e, 0x6a6048c1,
    0x696cb71a, 0x7ff501c3, 0x0fc6310b, 0x57e0f83d, 0x8cc26e74, 0x11a525a2, 0x946934c7, 0x7cd888f0,
    0x8f9d8604, 0x4f86e73b, 0x04520316, 0xdeeea20c, 0xf1def496, 0x67687288, 0xf540c5b2, 0x22401484,
    0x3478658a, 0xc2385746, 0x01979c2c, 0x5dad73c8, 0x0321f58b, 0xf0fedbee, 0x92826ddf, 0x284bec73,
    0x5b1a1975, 0x03df1e11, 0x20963e01, 0xa17cf12b, 0x740d776e, 0xa7a6bf3c, 0x01b5cce4, 0x1118aa76,
    0xfc6fac0a, 0xce927e9b, 0x00bf2567, 0x806f216c, 0xbca69056, 0x795bd3e9, 0xc9dc4557, 0x8929b6c2,
    0x789d52ec, 0x3f3fbf40, 0xb9197368, 0xa38c15b5, 0xc3b44fa8, 0xca8333b0, 0xb7e8d590, 0xbe807feb,
    0xbf5f8360, 0xd99e2f5c, 0x372928e1, 0x7c757c4c, 0x0db5b154, 0xc01ede02, 0x1fc86e78, 0x1f3985be,
    0xb4805c77, 0x00c880fa, 0x974c1b12, 0x35ab0214, 0xb2dc840d, 0x5b00ae37, 0xd313b026, 0xb260969d,
    0x7f4c8879, 0x1734c4d3, 0x49068631, 0xb9f6a021, 0x6b863e6f, 0xcee5debf, 0x29f8c9fb, 0x53dd6880,
    0x72b61223, 0x1f67a9fd, 0x0a0f6993, 0x13e59119, 0x11cca12e, 0xfe6b6766, 0x16b6effc, 0x97918fc4,
    0xc2b8a563, 0x94f2f741, 0x0bfa8c9a, 0xd1537ae8, 0xc1da349c, 0x873c60ca, 0x95005b85, 0x9b5c080e,
    0xbc8abbd9, 0xe1eab1d2, 0x6dac9070, 0x4ea9ebf1, 0xe0cf30d4, 0x1ef5bd7b, 0xd161043e, 0x5d2fa2e2,
    0xff5d3cae, 0x86ed9f87, 0x2aa1daa1, 0xbd731a34, 0x9e8f4b22, 0xb1c2c67a, 0xc21758c9, 0xa182215d,
    0xccb01948, 0x8d168df7, 0x04238cfe, 0x368c3dbc, 0x0aeadca5, 0xbad21c24, 0x0a71fee5, 0x9fc5d872,
    0x54c152c6, 0xfc329483, 0x6783384a, 0xeddb3e1c, 0x65f90e30, 0x884ad098, 0xce81675a, 0x4b372f7d,
    0x68bf9a39, 0x43445f1e, 0x40f8d8cb, 0x90d5acb6, 0x4cd07282, 0x349eeb06, 0x0c9d5332, 0x520b24ef,
    0x80020447, 0x67976491, 0x2f931ca3, 0xfe9b0535, 0xfcd30220, 0x61a9e6cc, 0xa487d8d7, 0x3f7c5dd1,
    0x7d0127c5, 0x48f51d15, 0x60dea871, 0xc9a91cb7, 0x58b53bb3, 0x9d5e0b2d, 0x624a78b4, 0x30dbee1b,
    0x9bdf22e7, 0x1df5c299, 0x2d5643a7, 0xf4dd35ff, 0x03ca8fd6, 0x53b47ed8, 0x6f2c19aa, 0xfeb0c1f4,
    0x49e54438, 0x2f2577e6, 0xbf876969, 0x72440ea9, 0xfa0bafb8, 0x74f5b3a0, 0x7dd357cd, 0x89ce1358,
    0x6ef2cdda, 0x1e7767f3, 0xa6be9fdb, 0x4f5f88f8, 0xba994a3a, 0x08ca6b65, 0xe0893818, 0x9e00a16a,
    0xf42bfc8f, 0x9972eedc, 0x749c8b51, 0x32c05f5e, 0xd706805f, 0x6bfbb7cf, 0xd9210a10, 0x31a1db97,
    0x923a9559, 0x37a7a1f6, 0x059f8861, 0xca493e62, 0x65157e81, 0x8f6467dd, 0xab85ff9f, 0x9331aff2,
    0x8616b9f5, 0xedbd5695, 0xee7e29b1, 0x313ac44f, 0xb903112f, 0x432ef649, 0xdc0a36c0, 0x61cf2bba,
    0x81474925, 0xa8b6c7ad, 0xee5931de, 0xb2f8158d, 0x59fb7409, 0x2e3dfaed, 0x9af25a3f, 0xe1fed4d5,
];

#[inline]
fn tabrand(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_add(1);
    TABRAND_CHAOS[(*seed & 0xff) as usize]
}

fn ceil_log_2(val: i32) -> i32 {
    if val == 1 {
        return 0;
    }
    let mut r = 1;
    let mut v = val - 1;
    v >>= 1;
    while v != 0 {
        r += 1;
        v >>= 1;
    }
    r
}

// ---- family tables (quic_family_tmpl.c + family_init) ----
struct QuicFamily {
    n_gr_codewords: [u32; MAXNUMCODES],
    not_gr_cwlen: [u32; MAXNUMCODES],
    not_gr_prefixmask: [u32; MAXNUMCODES],
    not_gr_suffixlen: [u32; MAXNUMCODES],
    golomb_code_len: [[u32; MAXNUMCODES]; 256],
    xlat_l2u: [u32; 256],
}

fn family_init(bpc: usize, limit: i32) -> QuicFamily {
    let mut f = QuicFamily {
        n_gr_codewords: [0; MAXNUMCODES],
        not_gr_cwlen: [0; MAXNUMCODES],
        not_gr_prefixmask: [0; MAXNUMCODES],
        not_gr_suffixlen: [0; MAXNUMCODES],
        golomb_code_len: [[0; MAXNUMCODES]; 256],
        xlat_l2u: [0; 256],
    };
    for l in 0..bpc {
        let mut altprefixlen = (limit - bpc as i32) as u32;
        if altprefixlen > BPPMASK[bpc - l] {
            altprefixlen = BPPMASK[bpc - l];
        }
        let altcodewords = BPPMASK[bpc] + 1 - (altprefixlen << l);
        f.n_gr_codewords[l] = altprefixlen << l;
        f.not_gr_suffixlen[l] = ceil_log_2(altcodewords as i32) as u32;
        f.not_gr_cwlen[l] = altprefixlen + f.not_gr_suffixlen[l];
        f.not_gr_prefixmask[l] = BPPMASK[(32 - altprefixlen) as usize];
        for b in 0..256u32 {
            let len = if b < f.n_gr_codewords[l] {
                (b >> l) + l as u32 + 1
            } else {
                f.not_gr_cwlen[l]
            };
            f.golomb_code_len[b as usize][l] = len;
        }
    }
    // correlate_init -> xlatL2U
    let pixelbitmask = BPPMASK[bpc];
    for s in 0..=pixelbitmask {
        f.xlat_l2u[s as usize] = if s & 1 != 0 {
            pixelbitmask - (s >> 1)
        } else {
            s >> 1
        };
    }
    f
}

fn family_8bpc() -> &'static QuicFamily {
    static F: OnceLock<QuicFamily> = OnceLock::new();
    F.get_or_init(|| family_init(8, DEFmaxclen))
}

/// level -> bucket index map + bucket count for the given bpc (DEFevol=3).
fn build_buckets(bpc: usize) -> (Vec<usize>, usize) {
    let levels = 1usize << bpc;
    let mut ptrs = vec![0usize; levels];
    let mut repcntr = 1 + 1; // repfirst+1
    let mut bsize = 1usize; // firstsize
    let mut bnumber = 0usize;
    let mut bend = 0usize;
    loop {
        let bstart = if bnumber == 0 { 0 } else { bend + 1 };
        repcntr -= 1;
        if repcntr == 0 {
            repcntr = 1; // repnext
            bsize *= 2; // mulsize
        }
        bend = bstart + bsize - 1;
        if bend + bsize >= levels {
            bend = levels - 1;
        }
        for p in ptrs.iter_mut().take(bend + 1).skip(bstart) {
            *p = bnumber;
        }
        bnumber += 1;
        if bend >= levels - 1 {
            break;
        }
    }
    (ptrs, bnumber)
}

// ---- decoder state ----
#[derive(Clone)]
struct CommonState {
    waitcnt: i32,
    tabrand_seed: u32,
    wm_trigger: u32,
    wmidx: i32,
    wmileft: i32,
    melcstate: i32,
    melclen: i32,
    melcorder: i32,
}

impl CommonState {
    fn reset() -> Self {
        let mut s = CommonState {
            waitcnt: 0,
            tabrand_seed: 0xff, // stabrand()
            wm_trigger: 0,
            wmidx: 0, // DEFwmistart
            wmileft: DEFwminext,
            melcstate: 0,
            melclen: J[0],
            melcorder: 1 << J[0],
        };
        s.set_wm_trigger();
        s
    }
    fn set_wm_trigger(&mut self) {
        let wm = self.wmidx.clamp(0, 10) as usize;
        self.wm_trigger = BESTTRIGTAB[3 / 2][wm] as u32;
    }
}

#[derive(Clone, Copy, Default)]
struct Bucket {
    counters: [u32; MAXNUMCODES],
    bestcode: u32,
}

struct Channel {
    cr: Vec<u8>,   // correlate_row, len width+1; cr[0]=correlate_row[-1], cr[i+1]=correlate_row[i]
    cur: Vec<u8>,  // decoded component for the current row
    prev: Vec<u8>, // decoded component for the previous row
    buckets: Vec<Bucket>,
}

/// MSB bit reader over LE 32-bit words (quic.c io_word machinery).
struct Io {
    words: Vec<u32>,
    io_now: usize,
    io_word: u32,
    io_next_word: u32,
    io_available_bits: i32,
}

impl Io {
    #[inline]
    fn read_io_word(&mut self) {
        self.io_next_word = self.words.get(self.io_now).copied().unwrap_or(0);
        self.io_now += 1;
    }

    fn init_decode_io(&mut self) {
        self.io_word = self.words.first().copied().unwrap_or(0);
        self.io_next_word = self.io_word;
        self.io_now = 1;
        self.io_available_bits = 0;
    }

    #[inline]
    fn decode_eatbits(&mut self, len: i32) {
        self.io_word = self.io_word.wrapping_shl(len as u32);
        let delta = self.io_available_bits - len;
        if delta >= 0 {
            self.io_available_bits = delta;
            self.io_word |= self.io_next_word >> (delta as u32);
            return;
        }
        let delta = -delta;
        self.io_word |= self.io_next_word.wrapping_shl(delta as u32);
        self.read_io_word();
        self.io_available_bits = 32 - delta;
        self.io_word |= self.io_next_word >> ((32 - delta) as u32);
    }

    #[inline]
    fn decode_eat32bits(&mut self) {
        self.decode_eatbits(16);
        self.decode_eatbits(16);
    }
}

fn golomb_decode(fam: &QuicFamily, l: u32, bits: u32) -> (u32, i32) {
    let l = l as usize;
    if bits > fam.not_gr_prefixmask[l] {
        let zeroprefix = bits.leading_zeros();
        let cwlen = zeroprefix + 1 + l as u32;
        let val = (zeroprefix << l) | ((bits >> (32 - cwlen)) & BPPMASK[l]);
        (val, cwlen as i32)
    } else {
        let cwlen = fam.not_gr_cwlen[l];
        let val = fam.n_gr_codewords[l]
            + ((bits >> (32 - cwlen)) & BPPMASK[fam.not_gr_suffixlen[l] as usize]);
        (val, cwlen as i32)
    }
}

fn update_model(ch: &mut Channel, state: &CommonState, bp: &[usize], fam: &QuicFamily, bpc: usize, index: usize) {
    let bidx = bp[ch.cr[index] as usize]; // find_bucket(correlate_row[index-1])
    let curval = ch.cr[index + 1] as usize; // correlate_row[index]
    let bucket = &mut ch.buckets[bidx];
    let mut bestcode = bpc - 1;
    bucket.counters[bestcode] += fam.golomb_code_len[curval][bestcode];
    let mut bestlen = bucket.counters[bestcode];
    for code in (0..bpc - 1).rev() {
        bucket.counters[code] += fam.golomb_code_len[curval][code];
        if bucket.counters[code] < bestlen {
            bestcode = code;
            bestlen = bucket.counters[code];
        }
    }
    bucket.bestcode = bestcode as u32;
    if bestlen > state.wm_trigger {
        for code in 0..bpc {
            bucket.counters[code] >>= 1;
        }
    }
}

/// Decode one channel component at pixel `i`; writes cur[i] and correlate_row[i].
#[inline]
fn decode_pixel(io: &mut Io, ch: &mut Channel, bp: &[usize], fam: &QuicFamily, bpc_mask: i32, i: usize, predictor: i32) {
    let bestcode = ch.buckets[bp[ch.cr[i] as usize]].bestcode;
    let (corr, cwlen) = golomb_decode(fam, bestcode, io.io_word);
    ch.cr[i + 1] = corr as u8;
    ch.cur[i] = ((fam.xlat_l2u[corr as usize] as i32 + predictor) & bpc_mask) as u8;
    io.decode_eatbits(cwlen);
}

/// MELCODE run-length decode (decode_state_run); returns the run length.
fn decode_state_run(io: &mut Io, state: &mut CommonState) -> i32 {
    let mut runlen: i32 = 0;
    loop {
        let temp = LZEROES[((!(io.io_word >> 24)) & 0xff) as usize] as i32;
        for _ in 0..temp {
            runlen += state.melcorder;
            if state.melcstate < MELCSTATES - 1 {
                state.melcstate += 1;
                state.melclen = J[state.melcstate as usize];
                state.melcorder = 1 << state.melclen;
            }
        }
        if temp != 8 {
            io.decode_eatbits(temp + 1);
            break;
        }
        io.decode_eatbits(8);
    }
    if state.melclen != 0 {
        runlen += (io.io_word >> (32 - state.melclen)) as i32;
        io.decode_eatbits(state.melclen);
    }
    if state.melcstate != 0 {
        state.melcstate -= 1;
        state.melclen = J[state.melcstate as usize];
        state.melcorder = 1 << state.melclen;
    }
    runlen
}

#[allow(clippy::too_many_arguments)]
fn uncompress_row0_seg(
    io: &mut Io,
    state: &mut CommonState,
    channels: &mut [Channel],
    chans: &[usize],
    fam: &QuicFamily,
    bp: &[usize],
    bpc: usize,
    bpc_mask: i32,
    start: i32,
    end: i32,
    waitmask: u32,
) {
    let mut i = start;
    let mut stopidx;
    if i == 0 {
        for &c in chans {
            decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, 0, 0);
        }
        if state.waitcnt != 0 {
            state.waitcnt -= 1;
        } else {
            state.waitcnt = (tabrand(&mut state.tabrand_seed) & waitmask) as i32;
            for &c in chans {
                update_model(&mut channels[c], state, bp, fam, bpc, 0);
            }
        }
        i += 1;
        stopidx = i + state.waitcnt;
    } else {
        stopidx = i + state.waitcnt;
    }
    while stopidx < end {
        while i <= stopidx {
            for &c in chans {
                let p = channels[c].cur[(i - 1) as usize] as i32;
                decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, i as usize, p);
            }
            i += 1;
        }
        for &c in chans {
            update_model(&mut channels[c], state, bp, fam, bpc, stopidx as usize);
        }
        stopidx = i + (tabrand(&mut state.tabrand_seed) & waitmask) as i32;
    }
    while i < end {
        for &c in chans {
            let p = channels[c].cur[(i - 1) as usize] as i32;
            decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, i as usize, p);
        }
        i += 1;
    }
    state.waitcnt = stopidx - end;
}

#[allow(clippy::too_many_arguments)]
fn uncompress_row0(
    io: &mut Io,
    state: &mut CommonState,
    channels: &mut [Channel],
    chans: &[usize],
    fam: &QuicFamily,
    bp: &[usize],
    bpc: usize,
    bpc_mask: i32,
    width: i32,
) {
    let mut pos = 0i32;
    let mut w = width;
    while DEFwmimax > state.wmidx && state.wmileft <= w {
        if state.wmileft > 0 {
            let wm = BPPMASK[state.wmidx as usize];
            let left = state.wmileft;
            uncompress_row0_seg(io, state, channels, chans, fam, bp, bpc, bpc_mask, pos, pos + left, wm);
            pos += left;
            w -= left;
        }
        state.wmidx += 1;
        state.set_wm_trigger();
        state.wmileft = DEFwminext;
    }
    if w > 0 {
        let wm = BPPMASK[state.wmidx as usize];
        uncompress_row0_seg(io, state, channels, chans, fam, bp, bpc, bpc_mask, pos, pos + w, wm);
        if DEFwmimax > state.wmidx {
            state.wmileft -= w;
        }
    }
}

#[inline]
fn rle_pred(channels: &[Channel], chans: &[usize], i: i32, run_index: i32) -> bool {
    let iu = i as usize;
    if i <= 2 || run_index == i {
        return false;
    }
    if !chans.iter().all(|&c| channels[c].prev[iu - 1] == channels[c].prev[iu]) {
        return false;
    }
    chans.iter().all(|&c| channels[c].cur[iu - 1] == channels[c].cur[iu - 2])
}

/// Copy the run of identical pixels; returns the new `i` (run_end).
fn do_run(io: &mut Io, state: &mut CommonState, channels: &mut [Channel], chans: &[usize], i: i32, end: i32) -> i32 {
    let run = decode_state_run(io, state);
    if run < 0 || run > (end - i) {
        warn!("QUIC: bad RLE run {run} (end-i={})", end - i);
        return end;
    }
    let run_end = i + run;
    for k in i..run_end {
        let ku = k as usize;
        for &c in chans {
            channels[c].cur[ku] = channels[c].cur[ku - 1];
        }
    }
    run_end
}

#[allow(clippy::too_many_arguments)]
fn uncompress_row_seg(
    io: &mut Io,
    state: &mut CommonState,
    channels: &mut [Channel],
    chans: &[usize],
    fam: &QuicFamily,
    bp: &[usize],
    bpc: usize,
    bpc_mask: i32,
    start: i32,
    end: i32,
) {
    let waitmask = BPPMASK[state.wmidx as usize];
    let mut i = start;
    let mut stopidx;
    let mut run_index = 0i32;

    if i == 0 {
        for &c in chans {
            let p = channels[c].prev[0] as i32;
            decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, 0, p);
        }
        if state.waitcnt != 0 {
            state.waitcnt -= 1;
        } else {
            state.waitcnt = (tabrand(&mut state.tabrand_seed) & waitmask) as i32;
            for &c in chans {
                update_model(&mut channels[c], state, bp, fam, bpc, 0);
            }
        }
        i += 1;
        stopidx = i + state.waitcnt;
    } else {
        stopidx = i + state.waitcnt;
    }

    'outer: loop {
        while stopidx < end {
            while i <= stopidx {
                if rle_pred(channels, chans, i, run_index) {
                    state.waitcnt = stopidx - i;
                    run_index = i;
                    i = do_run(io, state, channels, chans, i, end);
                    if i == end {
                        return;
                    }
                    stopidx = i + state.waitcnt;
                    continue 'outer;
                }
                for &c in chans {
                    let iu = i as usize;
                    let a = channels[c].cur[iu - 1] as i32;
                    let b = channels[c].prev[iu] as i32;
                    decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, iu, (a + b) >> 1);
                }
                i += 1;
            }
            for &c in chans {
                update_model(&mut channels[c], state, bp, fam, bpc, stopidx as usize);
            }
            stopidx = i + (tabrand(&mut state.tabrand_seed) & waitmask) as i32;
        }
        while i < end {
            if rle_pred(channels, chans, i, run_index) {
                state.waitcnt = stopidx - i;
                run_index = i;
                i = do_run(io, state, channels, chans, i, end);
                if i == end {
                    return;
                }
                stopidx = i + state.waitcnt;
                continue 'outer;
            }
            for &c in chans {
                let iu = i as usize;
                let a = channels[c].cur[iu - 1] as i32;
                let b = channels[c].prev[iu] as i32;
                decode_pixel(io, &mut channels[c], bp, fam, bpc_mask, iu, (a + b) >> 1);
            }
            i += 1;
        }
        state.waitcnt = stopidx - end;
        return;
    }
}

#[allow(clippy::too_many_arguments)]
fn uncompress_row(
    io: &mut Io,
    state: &mut CommonState,
    channels: &mut [Channel],
    chans: &[usize],
    fam: &QuicFamily,
    bp: &[usize],
    bpc: usize,
    bpc_mask: i32,
    width: i32,
) {
    let mut pos = 0i32;
    let mut w = width;
    while DEFwmimax > state.wmidx && state.wmileft <= w {
        if state.wmileft > 0 {
            let left = state.wmileft;
            uncompress_row_seg(io, state, channels, chans, fam, bp, bpc, bpc_mask, pos, pos + left);
            pos += left;
            w -= left;
        }
        state.wmidx += 1;
        state.set_wm_trigger();
        state.wmileft = DEFwminext;
    }
    if w > 0 {
        uncompress_row_seg(io, state, channels, chans, fam, bp, bpc, bpc_mask, pos, pos + w);
        if DEFwmimax > state.wmidx {
            state.wmileft -= w;
        }
    }
}

/// Decode a SPICE QUIC image stream (bytes after SpiceQUICData.data_size).
/// Returns top-down opaque RGBA. Implemented for RGB24/RGB32/RGBA (8bpc).
pub fn quic_decode(stream: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let nwords = stream.len().div_ceil(4);
    let mut words = vec![0u32; nwords];
    for (w, chunk) in words.iter_mut().zip(stream.chunks(4)) {
        let mut b = [0u8; 4];
        b[..chunk.len()].copy_from_slice(chunk);
        *w = u32::from_le_bytes(b);
    }

    let fam = family_8bpc();
    let (bucket_ptrs, n_buckets) = build_buckets(8);
    let bpc = 8usize;
    let bpc_mask = 0xffi32;

    let mut io = Io {
        words,
        io_now: 0,
        io_word: 0,
        io_next_word: 0,
        io_available_bits: 0,
    };
    io.init_decode_io();

    let magic = io.io_word;
    io.decode_eat32bits();
    if magic != QUIC_MAGIC {
        warn!("QUIC: bad magic {magic:#010x}");
        return None;
    }
    let version = io.io_word;
    io.decode_eat32bits();
    if version != QUIC_VERSION {
        warn!("QUIC: bad version {version:#x}");
        return None;
    }
    let qtype = io.io_word;
    io.decode_eat32bits();
    let width = io.io_word as i32;
    io.decode_eat32bits();
    let height = io.io_word as i32;
    io.decode_eat32bits();

    if width <= 0 || height <= 0 || (width as u64 * height as u64) > 64 * 1024 * 1024 {
        warn!("QUIC: implausible size {width}x{height}");
        return None;
    }

    // QUIC_IMAGE_TYPE: GRAY=1, RGB16=2, RGB24=3, RGB32=4, RGBA=5.
    let has_alpha = match qtype {
        3 | 4 => false,
        5 => true,
        other => {
            warn!("QUIC: image type {other} not yet implemented (only RGB24/RGB32/RGBA)");
            return None;
        }
    };

    let w = width as usize;
    let h = height as usize;
    let n_chan = if has_alpha { 4 } else { 3 };

    // encoder_reset_channels: fresh correlate rows + buckets (bestcode=bpc-1).
    let mut channels: Vec<Channel> = (0..n_chan)
        .map(|_| Channel {
            cr: vec![0u8; w + 1],
            cur: vec![0u8; w],
            prev: vec![0u8; w],
            buckets: vec![
                Bucket {
                    counters: [0; MAXNUMCODES],
                    bestcode: (bpc - 1) as u32,
                };
                n_buckets
            ],
        })
        .collect();

    let mut rgb_state = CommonState::reset();
    let mut alpha_state = CommonState::reset();
    let rgb: [usize; 3] = [0, 1, 2];
    let alpha: [usize; 1] = [3];

    let mut rgba = vec![0u8; w * h * 4];

    for row in 0..h {
        // correlate_row[-1] = 0 (row 0) else correlate_row[0]
        for c in 0..n_chan {
            channels[c].cr[0] = if row == 0 { 0 } else { channels[c].cr[1] };
        }
        if row == 0 {
            uncompress_row0(&mut io, &mut rgb_state, &mut channels, &rgb, fam, &bucket_ptrs, bpc, bpc_mask, width);
            if has_alpha {
                uncompress_row0(&mut io, &mut alpha_state, &mut channels, &alpha, fam, &bucket_ptrs, bpc, bpc_mask, width);
            }
        } else {
            uncompress_row(&mut io, &mut rgb_state, &mut channels, &rgb, fam, &bucket_ptrs, bpc, bpc_mask, width);
            if has_alpha {
                uncompress_row(&mut io, &mut alpha_state, &mut channels, &alpha, fam, &bucket_ptrs, bpc, bpc_mask, width);
            }
        }
        let base = row * w * 4;
        for x in 0..w {
            let p = base + x * 4;
            rgba[p] = channels[0].cur[x];
            rgba[p + 1] = channels[1].cur[x];
            rgba[p + 2] = channels[2].cur[x];
            rgba[p + 3] = 255; // primary surface is opaque; alpha plane consumed but unused
        }
        for c in 0..n_chan {
            let ch = &mut channels[c];
            std::mem::swap(&mut ch.cur, &mut ch.prev);
        }
    }

    Some((rgba, width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_and_buckets_sane() {
        let f = family_8bpc();
        assert!(f.golomb_code_len[0][0] >= 1);
        let mut seen = [false; 256];
        for s in 0..256 {
            seen[f.xlat_l2u[s] as usize] = true;
        }
        assert!(seen.iter().all(|&x| x));
        let (ptrs, n) = build_buckets(8);
        assert_eq!(ptrs.len(), 256);
        assert_eq!(n, 8);
        assert_eq!(ptrs[0], 0);
        assert_eq!(ptrs[255], 7);
    }

    #[test]
    fn bad_magic_returns_none() {
        assert!(quic_decode(&[0, 0, 0, 0, 1, 2, 3, 4]).is_none());
    }
}
