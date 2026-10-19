//! FLAC delivered inside a fragmented MP4 (Amazon's streams), possibly CENC-encrypted (`enca` sample
//! entry, AES-128-CTR). Rewritten losslessly as a native `.flac`: the `dfLa` metadata blocks (STREAMINFO
//! with the real sample count) followed by the decrypted FLAC frames. No ffmpeg needed.

use std::path::{Path, PathBuf};

type R<T> = Result<T, String>;

/// Boxes inside `data[from..to]`: (type, body start, body end).
fn boxes(data: &[u8], from: usize, to: usize) -> Vec<([u8; 4], usize, usize)> {
    let (mut out, mut at) = (vec![], from);
    while at + 8 <= to {
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let typ: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (size, head) = match size32 {
            1 if at + 16 <= to => (u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap()) as usize, 16),
            0 => (to - at, 8),
            n => (n, 8),
        };
        if size < head || at + size > to {
            break;
        }
        out.push((typ, at + head, at + size));
        at += size;
    }
    out
}

fn find(data: &[u8], from: usize, to: usize, path: &[&[u8; 4]]) -> Option<(usize, usize)> {
    let (first, rest) = path.split_first()?;
    let (_, s, e) = boxes(data, from, to).into_iter().find(|(t, ..)| t == *first)?;
    if rest.is_empty() { Some((s, e)) } else { find(data, s, e, rest) }
}

fn u32_at(d: &[u8], i: usize) -> R<u32> {
    d.get(i..i + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "mp4: truncated box".into())
}

/// `true` when `path` is an MP4 whose audio is FLAC (plain `fLaC` or encrypted `enca` over FLAC).
fn is_flac_mp4(data: &[u8]) -> bool {
    let Some((s, e)) = find(data, 0, data.len(), &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"]) else { return false };
    let entry = s + 8; // version/flags + entry count
    entry + 8 <= e && (&data[entry + 4..entry + 8] == b"fLaC" || (&data[entry + 4..entry + 8] == b"enca" && find(data, entry + 36, e, &[b"dfLa"]).is_some()))
}

/// Converts `src` (FLAC in MP4, decrypted with the hex `key` when encrypted) to `src` with a `.flac`
/// extension. `Ok(None)` when `src` is not FLAC in MP4.
pub fn to_flac(src: &Path, key: Option<&str>) -> R<Option<PathBuf>> {
    let data = std::fs::read(src).map_err(|e| e.to_string())?;
    if !is_flac_mp4(&data) {
        return Ok(None);
    }
    let (s, e) = find(&data, 0, data.len(), &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsd"]).unwrap();
    let entry = s + 8;
    let entry_end = entry + u32_at(&data, entry)? as usize;
    let encrypted = &data[entry + 4..entry + 8] == b"enca";
    // audio sample entry fields take 28 bytes after the box header; child boxes follow
    let (fs, fe) = find(&data, entry + 36, entry_end.min(e), &[b"dfLa"]).ok_or("mp4: no dfLa box")?;
    let mut meta = data[fs + 4..fe].to_vec(); // FLAC metadata blocks, after version/flags

    // encryption: scheme must be 'cenc' (AES-CTR); per-sample IV size from tenc
    let mut iv_size = 0usize;
    let cipher_key = if encrypted {
        let (ss, se) = find(&data, entry + 36, entry_end, &[b"sinf"]).ok_or("mp4: no sinf box")?;
        let (ms, _) = find(&data, ss, se, &[b"schm"]).ok_or("mp4: no schm box")?;
        let scheme = &data[ms + 4..ms + 8];
        if scheme != b"cenc" {
            return Err(format!("mp4: unsupported encryption scheme {}", String::from_utf8_lossy(scheme)));
        }
        let (ts, _) = find(&data, ss, se, &[b"schi", b"tenc"]).ok_or("mp4: no tenc box")?;
        iv_size = data[ts + 7] as usize; // version/flags(4) reserved(1) byte-block(1) isProtected(1) ivSize(1)
        let hex = key.ok_or("file terenkripsi tetapi ekstensi tidak memberi kunci dekripsi")?;
        let k: Vec<u8> = (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
        Some(<[u8; 16]>::try_from(k.as_slice()).map_err(|_| "kunci dekripsi tidak valid")?)
    } else {
        None
    };

    // trex default sample duration/size, for fragments that don't say
    let (trex_dur, trex_size) = match find(&data, 0, data.len(), &[b"moov", b"mvex", b"trex"]) {
        Some((t, _)) => (u32_at(&data, t + 12)?, u32_at(&data, t + 16)?),
        None => (0, 0),
    };

    let mut frames: Vec<u8> = Vec::with_capacity(data.len());
    let mut samples = 0u64;
    for (typ, ms, me) in boxes(&data, 0, data.len()) {
        if &typ != b"moof" {
            continue;
        }
        let moof_start = ms - 8;
        let Some((ts, te)) = find(&data, ms, me, &[b"traf"]) else { continue };
        let (hs, _) = find(&data, ts, te, &[b"tfhd"]).ok_or("mp4: no tfhd")?;
        let hflags = u32_at(&data, hs)? & 0xff_ffff;
        let mut at = hs + 8; // version/flags + track id
        let mut base = moof_start;
        if hflags & 0x1 != 0 { base = u64::from_be_bytes(data[at..at + 8].try_into().unwrap()) as usize; at += 8; }
        if hflags & 0x2 != 0 { at += 4; }
        let mut def_dur = trex_dur;
        if hflags & 0x8 != 0 { def_dur = u32_at(&data, at)?; at += 4; }
        let mut def_size = trex_size;
        if hflags & 0x10 != 0 { def_size = u32_at(&data, at)?; }

        let (rs, _) = find(&data, ts, te, &[b"trun"]).ok_or("mp4: no trun")?;
        let rflags = u32_at(&data, rs)? & 0xff_ffff;
        let count = u32_at(&data, rs + 4)? as usize;
        let mut at = rs + 8;
        let mut offset = base;
        if rflags & 0x1 != 0 { offset = (base as i64 + u32_at(&data, at)? as i32 as i64) as usize; at += 4; }
        if rflags & 0x4 != 0 { at += 4; }
        let mut sizes = Vec::with_capacity(count);
        for _ in 0..count {
            let mut dur = def_dur;
            let mut size = def_size;
            if rflags & 0x100 != 0 { dur = u32_at(&data, at)?; at += 4; }
            if rflags & 0x200 != 0 { size = u32_at(&data, at)?; at += 4; }
            if rflags & 0x400 != 0 { at += 4; }
            if rflags & 0x800 != 0 { at += 4; }
            samples += dur as u64;
            sizes.push(size as usize);
        }

        // per-sample IVs and subsample ranges (senc)
        let mut ivs: Vec<(Vec<u8>, Vec<(usize, usize)>)> = vec![];
        if let Some(k) = &cipher_key {
            let (es, _) = find(&data, ts, te, &[b"senc"]).ok_or("mp4: no senc")?;
            let sub = u32_at(&data, es)? & 0x2 != 0;
            let mut at = es + 8;
            for _ in 0..u32_at(&data, es + 4)? {
                let iv = data.get(at..at + iv_size).ok_or("mp4: truncated senc")?.to_vec();
                at += iv_size;
                let mut ranges = vec![];
                if sub {
                    let n = u16::from_be_bytes(data[at..at + 2].try_into().unwrap());
                    at += 2;
                    for _ in 0..n {
                        ranges.push((u16::from_be_bytes(data[at..at + 2].try_into().unwrap()) as usize, u32_at(&data, at + 2)? as usize));
                        at += 6;
                    }
                }
                ivs.push((iv, ranges));
            }
            let _ = k;
        }

        for (i, size) in sizes.into_iter().enumerate() {
            let mut sample = data.get(offset..offset + size).ok_or("mp4: sample outside the file")?.to_vec();
            offset += size;
            if let (Some(k), Some((iv, ranges))) = (&cipher_key, ivs.get(i)) {
                decrypt(k, iv, ranges, &mut sample);
            }
            frames.extend_from_slice(&sample);
        }
    }
    if frames.is_empty() {
        return Err("mp4: no audio frames".into());
    }

    // STREAMINFO is the first block: patch total samples (36 bits at body bytes 13..18)
    if meta.len() >= 4 + 34 {
        let b = 4 + 10;
        let packed = u64::from_be_bytes(meta[b..b + 8].try_into().unwrap()) & !((1u64 << 36) - 1) | (samples & ((1u64 << 36) - 1));
        meta[b..b + 8].copy_from_slice(&packed.to_be_bytes());
    }
    let mut out = Vec::with_capacity(4 + meta.len() + frames.len());
    out.extend_from_slice(b"fLaC");
    out.extend_from_slice(&meta);
    out.extend_from_slice(&frames);
    let dst = src.with_extension("flac");
    let part = src.with_extension("flac.part");
    std::fs::write(&part, out).map_err(|e| e.to_string())?;
    std::fs::rename(&part, &dst).map_err(|e| e.to_string())?;
    Ok(Some(dst))
}

/// CENC 'cenc' scheme: AES-128-CTR over the encrypted ranges of a sample, one counter stream per sample.
fn decrypt(key: &[u8; 16], iv: &[u8], ranges: &[(usize, usize)], sample: &mut [u8]) {
    use aes::cipher::{KeyIvInit, StreamCipher};
    let mut full = [0u8; 16];
    full[..iv.len().min(16)].copy_from_slice(&iv[..iv.len().min(16)]);
    let mut c = ctr::Ctr128BE::<aes::Aes128>::new(key.into(), &full.into());
    if ranges.is_empty() {
        return c.apply_keystream(sample);
    }
    let mut at = 0;
    for &(clear, enc) in ranges {
        at += clear;
        let end = (at + enc).min(sample.len());
        c.apply_keystream(&mut sample[at.min(end)..end]);
        at = end;
    }
}
