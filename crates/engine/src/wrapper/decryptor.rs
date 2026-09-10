//! ISO-BMFF (MP4 / fMP4) parsing and FairPlay SAMPLE-AES decryption.
//!
//! Ported and adapted from `apple-music-downloader/internal/fairplay-rip/runv4.go`.
//!
//! Handles:
//! 1. Transforming the init segment (`ftyp` + `moov`):
//!    - Unprotecting `stsd` audio sample entry (`enca` -> `alac`).
//!    - Removing `sinf` box.
//!    - Stripping `sbgp`/`sgpd` grouping boxes (`seam`, `seig`).
//!    - Sanitizing `stsd` to single entry.
//! 2. Decrypting each media fragment (`moof` + `mdat`):
//!    - Reading subsample patterns from `senc`.
//!    - Reading sample sizes and data offset from `trun`.
//!    - Decrypting 16-byte blocks using `temari::rounds::decrypt`.
//!    - Removing `senc`, `saiz`, `saio`, `pssh` boxes.
//!    - Adjusting `trun.data_offset` and updating box lengths.

use super::client::WrapperError;

#[inline]
fn read_u16_be(buf: &[u8]) -> u16 {
    u16::from_be_bytes(buf[..2].try_into().unwrap())
}

#[inline]
fn read_u24_be(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}

#[inline]
fn read_u32_be(buf: &[u8]) -> u32 {
    u32::from_be_bytes(buf[..4].try_into().unwrap())
}

#[inline]
fn write_u32_be(buf: &mut [u8], val: u32) {
    buf[..4].copy_from_slice(&val.to_be_bytes());
}

#[inline]
fn read_i32_be(buf: &[u8]) -> i32 {
    i32::from_be_bytes(buf[..4].try_into().unwrap())
}

#[inline]
fn write_i32_be(buf: &mut [u8], val: i32) {
    buf[..4].copy_from_slice(&val.to_be_bytes());
}

#[inline]
fn read_u64_be(buf: &[u8]) -> u64 {
    u64::from_be_bytes(buf[..8].try_into().unwrap())
}

#[derive(Debug, Clone)]
pub struct SubSample {
    pub clear_bytes: u16,
    pub protected_bytes: u32,
}

#[derive(Debug, Clone)]
pub struct SampleEncryptionInfo {
    pub subsamples: Vec<SubSample>,
}

/// Read a box header from `data` at `offset`.
/// Returns `(box_size, box_type, header_len)`.
pub fn read_box_header(data: &[u8], offset: usize) -> Option<(usize, [u8; 4], usize)> {
    if offset + 8 > data.len() {
        return None;
    }
    let size32 = read_u32_be(&data[offset..offset + 4]) as usize;
    let mut box_type = [0u8; 4];
    box_type.copy_from_slice(&data[offset + 4..offset + 8]);

    if size32 == 1 {
        // 64-bit large size
        if offset + 16 > data.len() {
            return None;
        }
        let size64 = read_u64_be(&data[offset + 8..offset + 16]) as usize;
        Some((size64, box_type, 16))
    } else if size32 == 0 {
        // Extends to EOF
        Some((data.len() - offset, box_type, 8))
    } else {
        Some((size32, box_type, 8))
    }
}

/// Helper to find a child box of given type inside a parent box.
pub fn find_child_box(
    data: &[u8],
    parent_start: usize,
    parent_end: usize,
    target_type: &[u8; 4],
) -> Option<(usize, usize)> {
    let mut cur = parent_start;
    while cur + 8 <= parent_end && cur + 8 <= data.len() {
        let (box_len, box_type, _) = read_box_header(data, cur)?;
        if box_len < 8 || cur + box_len > parent_end {
            break;
        }
        if &box_type == target_type {
            return Some((cur, box_len));
        }
        cur += box_len;
    }
    None
}

/// Find all child boxes of given type inside a parent box.
pub fn find_all_child_boxes(
    data: &[u8],
    parent_start: usize,
    parent_end: usize,
    target_type: &[u8; 4],
) -> Vec<(usize, usize)> {
    let mut res = Vec::new();
    let mut cur = parent_start;
    while cur + 8 <= parent_end && cur + 8 <= data.len() {
        let Some((box_len, box_type, _)) = read_box_header(data, cur) else {
            break;
        };
        if box_len < 8 || cur + box_len > parent_end {
            break;
        }
        if &box_type == target_type {
            res.push((cur, box_len));
        }
        cur += box_len;
    }
    res
}

/// Transforms the init segment (`ftyp` and `moov` boxes).
/// Changes sample entry `enca` to `alac`, strips `sinf`, strips `sbgp`/`sgpd` if seam/seig,
/// and updates box lengths.
pub fn transform_init_segment(init_data: &[u8]) -> Result<Vec<u8>, WrapperError> {
    let mut out = init_data.to_vec();
    let total_len = out.len();

    // Find moov box
    let (moov_off, moov_len) = find_child_box(&out, 0, total_len, b"moov")
        .ok_or_else(|| WrapperError::Message("Missing moov box in init segment".into()))?;

    // Inside moov -> trak -> mdia -> minf -> stbl
    let (trak_off, trak_len) = find_child_box(&out, moov_off + 8, moov_off + moov_len, b"trak")
        .ok_or_else(|| WrapperError::Message("Missing trak box in moov".into()))?;
    let (mdia_off, mdia_len) = find_child_box(&out, trak_off + 8, trak_off + trak_len, b"mdia")
        .ok_or_else(|| WrapperError::Message("Missing mdia box in trak".into()))?;
    let (minf_off, minf_len) = find_child_box(&out, mdia_off + 8, mdia_off + mdia_len, b"minf")
        .ok_or_else(|| WrapperError::Message("Missing minf box in mdia".into()))?;
    let (stbl_off, stbl_len) = find_child_box(&out, minf_off + 8, minf_off + minf_len, b"stbl")
        .ok_or_else(|| WrapperError::Message("Missing stbl box in minf".into()))?;

    // Inside stbl: find stsd
    let (stsd_off, stsd_len) = find_child_box(&out, stbl_off + 8, stbl_off + stbl_len, b"stsd")
        .ok_or_else(|| WrapperError::Message("Missing stsd box in stbl".into()))?;

    // stsd format:
    // 4 bytes size, 4 bytes 'stsd'
    // 1 byte version, 3 bytes flags
    // 4 bytes entry count
    if stsd_len < 16 {
        return Err(WrapperError::Message("stsd box too small".into()));
    }
    let stsd_entry_count = read_u32_be(&out[stsd_off + 12..stsd_off + 16]) as usize;
    let entry_start = stsd_off + 16;

    // Read first entry
    let (entry_len, entry_type, _) = read_box_header(&out, entry_start)
        .ok_or_else(|| WrapperError::Message("Cannot read first stsd entry".into()))?;

    let sinf_removed = if &entry_type == b"enca" {
        // AudioSampleEntry fixed fields before child boxes = 36 bytes:
        // 4 size, 4 'enca', 6 reserved, 2 data_reference_index,
        // 8 reserved, 2 channel_count, 2 sample_size, 4 predefined/reserved, 4 sample_rate
        let enca_children_start = entry_start + 36;
        let enca_end = entry_start + entry_len;

        let (sinf_off, sinf_len) = find_child_box(&out, enca_children_start, enca_end, b"sinf")
            .ok_or_else(|| WrapperError::Message("Missing sinf in enca entry".into()))?;

        // In sinf, find frma to get original codec (should be 'alac')
        let fmt = if let Some((frma_off, _)) =
            find_child_box(&out, sinf_off + 8, sinf_off + sinf_len, b"frma")
        {
            let mut f = [0u8; 4];
            f.copy_from_slice(&out[frma_off + 8..frma_off + 12]);
            f
        } else {
            *b"alac"
        };

        // 1. Rename entry type from 'enca' to original format (e.g. 'alac')
        out[entry_start + 4..entry_start + 8].copy_from_slice(&fmt);

        // 2. Remove sinf box from enca
        out.drain(sinf_off..sinf_off + sinf_len);

        // 3. Update entry size
        let new_entry_len = entry_len - sinf_len;
        write_u32_be(&mut out[entry_start..entry_start + 4], new_entry_len as u32);

        sinf_len
    } else {
        0
    };

    // Sanitize stsd to 1 entry (matches apple-music-downloader sanitizeInit and Symphonia requirement)
    let mut extra_entries_removed = 0;
    if stsd_entry_count > 1 {
        let first_entry_size = read_u32_be(&out[entry_start..entry_start + 4]) as usize;
        let second_entry_start = entry_start + first_entry_size;
        let cur_stsd_end = stsd_off + stsd_len - sinf_removed;
        if second_entry_start < cur_stsd_end {
            let excess = cur_stsd_end - second_entry_start;
            out.drain(second_entry_start..cur_stsd_end);
            extra_entries_removed = excess;
        }
        write_u32_be(&mut out[stsd_off + 12..stsd_off + 16], 1);
    }

    let total_stsd_removed = sinf_removed + extra_entries_removed;

    // Filter sbgp and sgpd in stbl if grouping type is 'seam' or 'seig'
    let cur_stbl_len = read_u32_be(&out[stbl_off..stbl_off + 4]) as usize - total_stsd_removed;
    let mut sbgp_removed = 0;
    let mut check_pos = stbl_off + 8;
    while check_pos + 8 <= stbl_off + cur_stbl_len - sbgp_removed {
        let Some((b_len, b_type, _)) = read_box_header(&out, check_pos) else {
            break;
        };
        if b_len < 8 || check_pos + b_len > out.len() {
            break;
        }
        if &b_type == b"sbgp" || &b_type == b"sgpd" {
            // Check grouping type at check_pos + 12 (FullBox version 1 + flags 3 + grouping_type 4)
            if check_pos + 16 <= out.len() {
                let grp = &out[check_pos + 12..check_pos + 16];
                if grp == b"seam" || grp == b"seig" {
                    out.drain(check_pos..check_pos + b_len);
                    sbgp_removed += b_len;
                    continue;
                }
            }
        }
        check_pos += b_len;
    }

    let total_removed = total_stsd_removed + sbgp_removed;

    // Update parent box sizes: stsd, stbl, minf, mdia, trak, moov
    if total_stsd_removed > 0 {
        let cur = read_u32_be(&out[stsd_off..stsd_off + 4]);
        write_u32_be(
            &mut out[stsd_off..stsd_off + 4],
            cur - total_stsd_removed as u32,
        );
    }
    if total_removed > 0 {
        let cur_stbl = read_u32_be(&out[stbl_off..stbl_off + 4]);
        write_u32_be(
            &mut out[stbl_off..stbl_off + 4],
            cur_stbl - total_removed as u32,
        );

        let cur_minf = read_u32_be(&out[minf_off..minf_off + 4]);
        write_u32_be(
            &mut out[minf_off..minf_off + 4],
            cur_minf - total_removed as u32,
        );

        let cur_mdia = read_u32_be(&out[mdia_off..mdia_off + 4]);
        write_u32_be(
            &mut out[mdia_off..mdia_off + 4],
            cur_mdia - total_removed as u32,
        );

        let cur_trak = read_u32_be(&out[trak_off..trak_off + 4]);
        write_u32_be(
            &mut out[trak_off..trak_off + 4],
            cur_trak - total_removed as u32,
        );

        let cur_moov = read_u32_be(&out[moov_off..moov_off + 4]);
        write_u32_be(
            &mut out[moov_off..moov_off + 4],
            cur_moov - total_removed as u32,
        );
    }

    Ok(out)
}

/// Parsed `trun` box information.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TrunInfo {
    offset_in_data: usize,
    sample_count: usize,
    data_offset_field_pos: Option<usize>,
    data_offset: i32,
    sample_sizes: Vec<usize>,
}

/// Parse `trun` box inside `traf`.
fn parse_trun(data: &[u8], trun_start: usize, trun_len: usize) -> Option<TrunInfo> {
    if trun_len < 16 || trun_start + trun_len > data.len() {
        return None;
    }
    let flags = read_u24_be(&data[trun_start + 9..trun_start + 12]);
    let sample_count = read_u32_be(&data[trun_start + 12..trun_start + 16]) as usize;
    let mut cur = trun_start + 16;

    let data_offset_field_pos;
    let data_offset;
    if flags & 0x000001 != 0 {
        if cur + 4 > data.len() {
            return None;
        }
        data_offset_field_pos = Some(cur);
        data_offset = read_i32_be(&data[cur..cur + 4]);
        cur += 4;
    } else {
        data_offset_field_pos = None;
        data_offset = 0;
    }

    if flags & 0x000004 != 0 {
        cur += 4; // first_sample_flags
    }

    let has_duration = flags & 0x000100 != 0;
    let has_size = flags & 0x000200 != 0;
    let has_flags = flags & 0x000400 != 0;
    let has_ctts = flags & 0x000800 != 0;

    let mut sample_sizes = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        if has_duration {
            cur += 4;
        }
        if has_size {
            if cur + 4 > data.len() {
                return None;
            }
            sample_sizes.push(read_u32_be(&data[cur..cur + 4]) as usize);
            cur += 4;
        } else {
            // Default sample size fallback
            sample_sizes.push(0);
        }
        if has_flags {
            cur += 4;
        }
        if has_ctts {
            cur += 4;
        }
    }

    Some(TrunInfo {
        offset_in_data: trun_start,
        sample_count,
        data_offset_field_pos,
        data_offset,
        sample_sizes,
    })
}

/// Parse `senc` box inside `traf`.
/// Returns list of subsamples for each sample.
fn parse_senc(
    data: &[u8],
    senc_start: usize,
    senc_len: usize,
) -> Option<Vec<SampleEncryptionInfo>> {
    if senc_len < 16 || senc_start + senc_len > data.len() {
        return None;
    }
    let flags = read_u24_be(&data[senc_start + 9..senc_start + 12]);
    let has_subsamples = flags & 0x000002 != 0;
    let sample_count = read_u32_be(&data[senc_start + 12..senc_start + 16]) as usize;

    let mut cur = senc_start + 16;
    let mut res = Vec::with_capacity(sample_count);

    for _ in 0..sample_count {
        if has_subsamples {
            if cur + 2 > data.len() {
                return None;
            }
            let sub_count = read_u16_be(&data[cur..cur + 2]) as usize;
            cur += 2;
            let mut subs = Vec::with_capacity(sub_count);
            for _ in 0..sub_count {
                if cur + 6 > data.len() {
                    return None;
                }
                let clear_bytes = read_u16_be(&data[cur..cur + 2]);
                let protected_bytes = read_u32_be(&data[cur + 2..cur + 6]);
                cur += 6;
                subs.push(SubSample {
                    clear_bytes,
                    protected_bytes,
                });
            }
            res.push(SampleEncryptionInfo { subsamples: subs });
        } else {
            res.push(SampleEncryptionInfo {
                subsamples: Vec::new(),
            });
        }
    }

    Some(res)
}

/// Decrypt one fragmented MP4 segment (`moof` + `mdat`) using Temari.
/// Modifies audio data in-place and removes encryption boxes (`senc`, `saiz`, `saio`, `pssh`).
pub fn decrypt_fragment(
    fragment_data: &[u8],
    template: &temari::rounds::Template,
) -> Result<Vec<u8>, WrapperError> {
    let mut out = fragment_data.to_vec();
    let total_len = out.len();

    // 1. Locate moof and mdat
    let (moof_off, moof_len) = find_child_box(&out, 0, total_len, b"moof")
        .ok_or_else(|| WrapperError::Message("Missing moof box in fragment".into()))?;
    let (mdat_off, _mdat_len) = find_child_box(&out, 0, total_len, b"mdat")
        .ok_or_else(|| WrapperError::Message("Missing mdat box in fragment".into()))?;

    // 2. Locate traf inside moof
    let (traf_off, traf_len) = find_child_box(&out, moof_off + 8, moof_off + moof_len, b"traf")
        .ok_or_else(|| WrapperError::Message("Missing traf box in moof".into()))?;

    // 3. Parse ALL trun boxes in traf
    let trun_boxes = find_all_child_boxes(&out, traf_off + 8, traf_off + traf_len, b"trun");
    if trun_boxes.is_empty() {
        return Err(WrapperError::Message("No trun boxes in traf".into()));
    }
    let mut truns = Vec::with_capacity(trun_boxes.len());
    for (t_off, t_len) in trun_boxes {
        let trun = parse_trun(&out, t_off, t_len)
            .ok_or_else(|| WrapperError::Message("Failed to parse trun box".into()))?;
        truns.push(trun);
    }

    // 4. Parse senc (or UUID senc)
    let senc_box = find_child_box(&out, traf_off + 8, traf_off + traf_len, b"senc");
    let enc_info = if let Some((s_off, s_len)) = senc_box {
        parse_senc(&out, s_off, s_len)
    } else {
        None
    };

    // 5. Decrypt samples in mdat across all trun boxes
    let mut global_sample_idx = 0;
    let mut prev_sample_end = mdat_off + 8;

    for trun in &truns {
        let mut sample_offset = if trun.data_offset > 0 {
            moof_off + trun.data_offset as usize
        } else {
            prev_sample_end
        };

        for &sample_size in &trun.sample_sizes {
            if sample_offset + sample_size > out.len() {
                return Err(WrapperError::Message(format!(
                    "Sample {} offset {} exceeds fragment boundary {}",
                    global_sample_idx,
                    sample_offset + sample_size,
                    out.len()
                )));
            }

            let sample_slice = &mut out[sample_offset..sample_offset + sample_size];

            if let Some(ref encs) = enc_info {
                if let Some(enc) = encs.get(global_sample_idx) {
                    if !enc.subsamples.is_empty() {
                        let mut pos = 0;
                        for sub in &enc.subsamples {
                            pos += sub.clear_bytes as usize;
                            let prot = sub.protected_bytes as usize;
                            if pos + prot > sample_slice.len() {
                                break;
                            }
                            let head = (prot / 16) * 16;
                            if head > 0 {
                                let ct = &sample_slice[pos..pos + head];
                                let pt = temari::rounds::decrypt(template, ct);
                                sample_slice[pos..pos + head].copy_from_slice(&pt[..head]);
                            }
                            pos += prot;
                        }
                    } else {
                        let head = (sample_slice.len() / 16) * 16;
                        if head > 0 {
                            let pt = temari::rounds::decrypt(template, &sample_slice[..head]);
                            sample_slice[..head].copy_from_slice(&pt[..head]);
                        }
                    }
                }
            } else {
                let head = (sample_slice.len() / 16) * 16;
                if head > 0 {
                    let pt = temari::rounds::decrypt(template, &sample_slice[..head]);
                    sample_slice[..head].copy_from_slice(&pt[..head]);
                }
            }

            sample_offset += sample_size;
            global_sample_idx += 1;
        }
        prev_sample_end = sample_offset;
    }

    // 6. Collect encryption boxes to remove: senc, saiz, saio, uuid (in traf), pssh (in moof)
    let mut boxes_to_remove: Vec<(usize, usize)> = Vec::new();
    let mut bytes_removed_from_traf = 0;

    for box_type in [b"senc", b"saiz", b"saio"] {
        for b in find_all_child_boxes(&out, traf_off + 8, traf_off + traf_len, box_type) {
            bytes_removed_from_traf += b.1;
            boxes_to_remove.push(b);
        }
    }
    for b in find_all_child_boxes(&out, traf_off + 8, traf_off + traf_len, b"uuid") {
        bytes_removed_from_traf += b.1;
        boxes_to_remove.push(b);
    }

    let mut bytes_removed_from_moof = bytes_removed_from_traf;
    for b in find_all_child_boxes(&out, moof_off + 8, moof_off + moof_len, b"pssh") {
        bytes_removed_from_moof += b.1;
        boxes_to_remove.push(b);
    }

    // 7. Adjust trun.data_offset for ALL trun boxes and update parent box sizes BEFORE draining
    if bytes_removed_from_moof > 0 {
        for trun in &truns {
            if let Some(pos) = trun.data_offset_field_pos {
                let new_data_offset = trun.data_offset - bytes_removed_from_moof as i32;
                write_i32_be(&mut out[pos..pos + 4], new_data_offset);
            }
        }

        let cur_traf_size = read_u32_be(&out[traf_off..traf_off + 4]);
        write_u32_be(
            &mut out[traf_off..traf_off + 4],
            cur_traf_size - bytes_removed_from_traf as u32,
        );

        let cur_moof_size = read_u32_be(&out[moof_off..moof_off + 4]);
        write_u32_be(
            &mut out[moof_off..moof_off + 4],
            cur_moof_size - bytes_removed_from_moof as u32,
        );

        // Sort descending by start offset so removing earlier boxes doesn't invalidate subsequent offsets
        boxes_to_remove.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (b_off, b_len) in boxes_to_remove {
            out.drain(b_off..b_off + b_len);
        }
    }

    Ok(out)
}
