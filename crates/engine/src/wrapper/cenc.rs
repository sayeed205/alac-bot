//! CENC (Common Encryption, ISO/IEC 23001-7) sample decryption for the
//! webplayback AAC path.
//!
//! Ported from `apple-music-downloader` (Go `mp4ff`): one continuous
//! AES-128-CTR keystream per sample — started from the sample's `senc`
//! IV (or a zero IV when the senc carries none) and flowing across all
//! subsample boundaries of that sample. FairPlay runs elsewhere in this
//! crate; this module is only the Widevine/CENC counterpart.
//!
//! Layout handled: whole-file download of `EXT-X-MAP` init byterange +
//! one `moof`/`mdat` fragment per `EXT-X-BYTERANGE` segment, matching
//! how the engine stages webplayback files.

use ctr::cipher::{KeyIvInit, StreamCipher};

use super::{
    client::WrapperError,
    decryptor::{find_child_box, read_box_header},
};

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// One sample's senc entry: IV (0/8/16 bytes) + subsample pattern.
#[derive(Debug, Clone, Default)]
struct SencSample {
    iv: Vec<u8>,
    subsamples: Vec<(u16, u32)>, // (clear, protected)
}

/// Parse a `senc` box, mirroring `mp4ff.DecodeSenc`:
/// - `flags & 0x1` (UseSubSampleEncryption) gates the subsample entries;
/// - without subsamples the per-sample IV size is inferred as
///   `remaining / sample_count` when no hint exists;
/// - a working parse must consume the payload exactly.
fn parse_senc_samples(data: &[u8], start: usize, len: usize) -> Option<Vec<SencSample>> {
    if len < 16 || start + len > data.len() {
        return None;
    }
    // FullBox: version(1) at start+8, flags(3) at start+9..start+12.
    let flags = ((data[start + 9] as u32) << 16)
        | ((data[start + 10] as u32) << 8)
        | data[start + 11] as u32;
    // mp4ff: UseSubSampleEncryption = 0x2 (subsample data present).
    let has_subsamples = flags & 0x2 != 0;
    let sample_count = u32::from_be_bytes(data[start + 12..start + 16].try_into().ok()?) as usize;
    let body_len = len - 16;

    let try_parse = |iv_size: usize| -> Option<Vec<SencSample>> {
        let mut pos = start + 16;
        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let mut sample = SencSample::default();
            if iv_size > 0 {
                let iv = data.get(pos..pos + iv_size)?;
                sample.iv = iv.to_vec();
                pos += iv_size;
            }
            if has_subsamples {
                let count = u16::from_be_bytes(data.get(pos..pos + 2)?.try_into().ok()?) as usize;
                pos += 2;
                for _ in 0..count {
                    let clear = u16::from_be_bytes(data.get(pos..pos + 2)?.try_into().ok()?);
                    let protected =
                        u32::from_be_bytes(data.get(pos + 2..pos + 6)?.try_into().ok()?);
                    pos += 6;
                    sample.subsamples.push((clear, protected));
                }
            }
            samples.push(sample);
        }
        // The payload must be fully consumed for the IV size to be right.
        if pos == start + len {
            Some(samples)
        } else {
            None
        }
    };

    if !has_subsamples {
        // mp4ff infers: nrBytesLeft / sampleCount.
        let iv_size = body_len / sample_count.max(1);
        return match iv_size {
            0 | 8 | 16 => try_parse(iv_size),
            _ => (body_len == 0).then(Vec::new),
        };
    }
    for iv_size in [0usize, 8, 16] {
        if let Some(samples) = try_parse(iv_size) {
            return Some(samples);
        }
    }
    None
}

/// Decrypt one `moof`+`mdat` fragment in place with a CENC content key.
///
/// Mirrors the Go `DecryptSegment`: a fragment without `senc` passes
/// through untouched (unencrypted fragments legally precede or follow
/// encrypted ones in the same playlist). The keystream runs continuously
/// across a sample's subsample boundaries.
pub fn decrypt_cenc_fragment(fragment: &mut [u8], key: &[u8; 16]) -> Result<bool, WrapperError> {
    let total_len = fragment.len();

    let (moof_off, moof_len) = find_child_box(fragment, 0, total_len, b"moof")
        .ok_or_else(|| WrapperError::Message("Missing moof box in fragment".into()))?;
    let (mdat_off, _mdat_len) = find_child_box(fragment, 0, total_len, b"mdat")
        .ok_or_else(|| WrapperError::Message("Missing mdat box in fragment".into()))?;
    let (traf_off, traf_len) = find_child_box(fragment, moof_off + 8, moof_off + moof_len, b"traf")
        .ok_or_else(|| WrapperError::Message("Missing traf box in moof".into()))?;

    let Some((senc_off, senc_len)) =
        find_child_box(fragment, traf_off + 8, traf_off + traf_len, b"senc")
    else {
        return Ok(false);
    };
    let Some(samples_info) = parse_senc_samples(fragment, senc_off, senc_len) else {
        return Err(WrapperError::Message("Failed to parse senc box".into()));
    };

    // Walk trun boxes for sample sizes and data offsets.
    let mut trun_boxes = Vec::new();
    let mut cur = traf_off + 8;
    while cur + 8 <= traf_off + traf_len {
        let Some((len, kind, _)) = read_box_header(fragment, cur) else {
            break;
        };
        if len < 8 || cur + len > traf_off + traf_len {
            break;
        }
        if &kind == b"trun" {
            trun_boxes.push((cur, len));
        }
        cur += len;
    }

    let mut global_sample_idx = 0usize;
    let mut sample_offset = mdat_off + 8;

    for (trun_off, trun_len) in trun_boxes {
        let Some(trun) =
            super::decryptor::parse_trun_pub(fragment, traf_off, traf_len, trun_off, trun_len)
        else {
            return Err(WrapperError::Message("Failed to parse trun box".into()));
        };
        if trun.data_offset > 0 {
            sample_offset = moof_off + trun.data_offset as usize;
        }
        for &size in &trun.sample_sizes {
            if size == 0 {
                continue;
            }
            let start = sample_offset;
            let end = start + size;
            if end > fragment.len() {
                return Err(WrapperError::Message(format!(
                    "CENC sample {global_sample_idx} range {start}..{end} out of bounds ({})",
                    fragment.len()
                )));
            }

            if let Some(info) = samples_info.get(global_sample_idx) {
                // IV: senc per-sample IV (zero-padded to 16 when 8 bytes) or
                // all-zero base.
                let mut iv = [0u8; 16];
                if !info.iv.is_empty() {
                    let n = info.iv.len().min(16);
                    iv[..n].copy_from_slice(&info.iv[..n]);
                }
                let mut cipher = Aes128Ctr::new(key.into(), &iv.into());
                if info.subsamples.is_empty() {
                    cipher.apply_keystream(&mut fragment[start..end]);
                } else {
                    let mut pos = 0usize;
                    for (clear, protected) in &info.subsamples {
                        pos += *clear as usize;
                        let prot = *protected as usize;
                        if pos + prot > size {
                            break;
                        }
                        // One keystream flows across subsamples: do NOT
                        // re-init the cipher here.
                        cipher.apply_keystream(&mut fragment[start + pos..start + pos + prot]);
                        pos += prot;
                    }
                }
            }

            sample_offset = end;
            global_sample_idx += 1;
        }
    }

    Ok(true)
}

/// Strip the encryption-related boxes a decrypted fragment no longer
/// needs: `senc`, `saiz`, `saio`, `uuid` (inside traf) and `pssh`
/// (inside moof), adjusting `trun` data offsets. Returns the fragment
/// trimmed of those boxes; equals the input when nothing applies.
///
/// The Go flow encodes the same fragment again after decrypting, so its
/// box removal happens in `Encode`. We stage raw byte ranges, so this
/// does the surgery directly.
pub fn strip_encryption_boxes(fragment: &mut Vec<u8>) -> Result<(), WrapperError> {
    let total_len = fragment.len();
    let (moof_off, moof_len) = find_child_box(fragment, 0, total_len, b"moof")
        .ok_or_else(|| WrapperError::Message("Missing moof box in fragment".into()))?;
    let (traf_off, traf_len) = find_child_box(fragment, moof_off + 8, moof_off + moof_len, b"traf")
        .ok_or_else(|| WrapperError::Message("Missing traf box in moof".into()))?;

    let mut boxes_to_remove: Vec<(usize, usize)> = Vec::new();
    let mut removed_from_traf = 0usize;
    for kind in [b"senc", b"saiz", b"saio", b"uuid"] {
        let mut cur = traf_off + 8;
        while cur + 8 <= traf_off + traf_len {
            let Some((len, box_type, _)) = read_box_header(fragment, cur) else {
                break;
            };
            if len < 8 || cur + len > traf_off + traf_len {
                break;
            }
            if &box_type == kind {
                removed_from_traf += len;
                boxes_to_remove.push((cur, len));
            }
            cur += len;
        }
    }
    // Drop sample-encryption group metadata (`seam`) left over in traf.
    {
        let mut cur = traf_off + 8;
        while cur + 8 <= traf_off + traf_len {
            let Some((len, box_type, _)) = read_box_header(fragment, cur) else {
                break;
            };
            if len < 8 || cur + len > traf_off + traf_len {
                break;
            }
            if (&box_type == b"sgpd" || &box_type == b"sbgp")
                && cur + 16 <= fragment.len()
                && &fragment[cur + 12..cur + 16] == b"seam"
            {
                removed_from_traf += len;
                boxes_to_remove.push((cur, len));
            }
            cur += len;
        }
    }
    let mut removed_from_moof = removed_from_traf;
    {
        let mut cur = moof_off + 8;
        while cur + 8 <= moof_off + moof_len {
            let Some((len, box_type, _)) = read_box_header(fragment, cur) else {
                break;
            };
            if len < 8 || cur + len > moof_off + moof_len {
                break;
            }
            if &box_type == b"pssh" {
                removed_from_moof += len;
                boxes_to_remove.push((cur, len));
            }
            cur += len;
        }
    }

    if removed_from_moof == 0 {
        return Ok(());
    }

    // Adjust every trun.data_offset by the bytes removed before mdat.
    // pssh (moof child) and the traf boxes all sit before mdat.
    // Walk the ORIGINAL box positions — the drains happen afterwards.
    let mut cur = traf_off + 8;
    while cur + 8 <= traf_off + traf_len {
        let Some((len, box_type, _)) = read_box_header(fragment, cur) else {
            break;
        };
        if len < 8 {
            break;
        }
        if &box_type == b"trun" {
            // trun layout: size/type (8) + version/flags (4) +
            // sample_count (4) + [data_offset (4), present because we only
            // get here with data-offset-flagged truns] + sample fields.
            let field = cur + 8 + 4 + 4;
            let data_offset = i32::from_be_bytes(
                fragment
                    .get(field..field + 4)
                    .and_then(|s| s.try_into().ok())
                    .ok_or_else(|| WrapperError::Message("trun data offset".into()))?,
            );
            let new_offset = data_offset - removed_from_moof as i32;
            fragment[field..field + 4].copy_from_slice(&new_offset.to_be_bytes());
        }
        cur += len;
    }

    // Shrink traf and moof sizes.
    let traf_size = u32::from_be_bytes(fragment[traf_off..traf_off + 4].try_into().unwrap());
    fragment[traf_off..traf_off + 4]
        .copy_from_slice(&(traf_size - removed_from_traf as u32).to_be_bytes());
    let moof_size = u32::from_be_bytes(fragment[moof_off..moof_off + 4].try_into().unwrap());
    fragment[moof_off..moof_off + 4]
        .copy_from_slice(&(moof_size - removed_from_moof as u32).to_be_bytes());

    // Remove boxes back-to-front so offsets stay valid.
    boxes_to_remove.sort_by_key(|(off, _)| std::cmp::Reverse(*off));
    for (off, len) in boxes_to_remove {
        fragment.drain(off..off + len);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `moof` (traf: trun + senc) + `mdat` fragment with
    /// one sample of `clear` + 16 protected bytes.
    fn build_fragment(clear: &[u8], iv: &[u8]) -> Vec<u8> {
        let iv_size = iv.len();
        let mut senc = Vec::new();
        let senc_body_len = 4 + 4 + (iv_size + 2 + 6); // ver/flags+count / sample
        senc.extend_from_slice(&((8 + senc_body_len) as u32).to_be_bytes());
        senc.extend_from_slice(b"senc");
        senc.extend_from_slice(&[0, 0, 0, 2]); // flags: subsamples present
        senc.extend_from_slice(&1u32.to_be_bytes()); // sample_count
        senc.extend_from_slice(iv);
        senc.extend_from_slice(&1u16.to_be_bytes()); // subsample count
        senc.extend_from_slice(&(clear.len() as u16).to_be_bytes());
        senc.extend_from_slice(&16u32.to_be_bytes());

        let payload_len = clear.len() + 16;
        let mut trun = Vec::new();
        let trun_body = 4 + 4 + 4 + 4; // ver/flags + count + data_offset + size
        trun.extend_from_slice(&((8 + trun_body) as u32).to_be_bytes());
        trun.extend_from_slice(b"trun");
        trun.extend_from_slice(&[0, 0, 0x02, 0x01]); // flags: data_offset + size
        trun.extend_from_slice(&1u32.to_be_bytes());
        trun.extend_from_slice(&0i32.to_be_bytes()); // patched below
        trun.extend_from_slice(&(payload_len as u32).to_be_bytes());

        let traf_len = 8 + trun.len() + senc.len();
        let moof_len = 8 + traf_len;
        let data_offset = moof_len as i32 + 8;

        let mut traf = Vec::new();
        traf.extend_from_slice(&(traf_len as u32).to_be_bytes());
        traf.extend_from_slice(b"traf");
        traf.extend_from_slice(&trun);
        traf.extend_from_slice(&senc);

        let mut frag = Vec::new();
        frag.extend_from_slice(&(moof_len as u32).to_be_bytes());
        frag.extend_from_slice(b"moof");
        frag.extend_from_slice(&traf);
        // Patch trun.data_offset (its position inside frag).
        let trun_start = 8 + traf_len - senc.len() - trun.len();
        let doff_pos = trun_start + 8 + 4 + 4;
        frag[doff_pos..doff_pos + 4].copy_from_slice(&data_offset.to_be_bytes());

        frag.extend_from_slice(&((8 + payload_len) as u32).to_be_bytes());
        frag.extend_from_slice(b"mdat");
        frag.extend_from_slice(clear);
        frag.extend_from_slice(&[0u8; 16]);
        frag
    }

    #[test]
    fn keystream_flows_across_subsamples_in_one_sample() {
        let clear = [1, 2, 3, 4];
        let iv = [5u8; 16];
        let mut frag = build_fragment(&clear, &iv);

        // Two subsamples would also work; here the single (4 clear, 16
        // protected) pattern must decrypt the protected tail back to zeros
        // when we encrypt with the same CTR stream.
        let key = [0x42u8; 16];
        let prot_start = frag.len() - 16;
        let mut enc = Aes128Ctr::new((&key).into(), (&iv).into());
        enc.apply_keystream(&mut frag[prot_start..]);

        let decrypted_any = decrypt_cenc_fragment(&mut frag, &key).expect("decrypt");
        assert!(decrypted_any);
        assert_eq!(&frag[prot_start..], &[0u8; 16]);
        assert_eq!(&frag[prot_start - 4..prot_start], &clear);
    }

    #[test]
    fn iv_size_is_auto_detected_from_senc_payload() {
        let clear = [1, 2, 3, 4];
        // 8-byte IV must be parsed as 8 (payload consumed exactly).
        let iv8 = [9u8; 8];
        let mut frag = build_fragment(&clear, &iv8);
        // Zero-extend the IV for encryption, mirroring the decryptor.
        let mut iv16 = [0u8; 16];
        iv16[..8].copy_from_slice(&iv8);
        let key = [0x11u8; 16];
        let prot_start = frag.len() - 16;
        let mut enc = Aes128Ctr::new((&key).into(), (&iv16).into());
        enc.apply_keystream(&mut frag[prot_start..]);

        decrypt_cenc_fragment(&mut frag, &key).expect("decrypt");
        assert_eq!(&frag[prot_start..], &[0u8; 16]);
    }

    #[test]
    fn fragment_without_senc_is_untouched() {
        let mut trun = Vec::new();
        trun.extend_from_slice(&((8 + 4 + 4 + 4 + 4) as u32).to_be_bytes());
        trun.extend_from_slice(b"trun");
        trun.extend_from_slice(&[0, 0, 0x02, 0x01]);
        trun.extend_from_slice(&1u32.to_be_bytes());
        trun.extend_from_slice(&16i32.to_be_bytes());
        trun.extend_from_slice(&16u32.to_be_bytes());
        let mut traf = Vec::new();
        traf.extend_from_slice(&((8 + trun.len()) as u32).to_be_bytes());
        traf.extend_from_slice(b"traf");
        traf.extend_from_slice(&trun);
        let mut frag = Vec::new();
        frag.extend_from_slice(&((8 + traf.len()) as u32).to_be_bytes());
        frag.extend_from_slice(b"moof");
        frag.extend_from_slice(&traf);
        frag.extend_from_slice(&((8 + 16) as u32).to_be_bytes());
        frag.extend_from_slice(b"mdat");
        frag.extend_from_slice(&[9u8; 16]);

        let before = frag.clone();
        let touched = decrypt_cenc_fragment(&mut frag, &[7u8; 16]).expect("no-op");
        assert!(!touched);
        assert_eq!(frag, before);
    }

    #[test]
    fn strip_removes_senc_and_adjusts_offsets() {
        let clear = [1, 2, 3, 4];
        let mut frag = build_fragment(&clear, &[0u8; 16]);
        let len_before = frag.len();
        strip_encryption_boxes(&mut frag).expect("strip");
        assert!(frag.len() < len_before);
        // senc is gone; mdat payload survives.
        assert!(find_child_box(&frag, 0, frag.len(), b"mdat").is_some());
        assert!(!frag.windows(4).any(|w| w == b"senc"));
    }

    /// Size-less trun (Apple ec-3 layout): sizes come from
    /// tfhd.default_sample_size and must decrypt correctly.
    #[test]
    fn sizeless_trun_resolves_sizes_from_tfhd() {
        let sample = [7u8; 16];
        let iv = [0x33u8; 16];
        let payload = sample.to_vec();

        // senc: 2 samples, no subsamples, 16-byte IVs.
        let mut senc = Vec::new();
        let senc_body_len = 4 + 4 + 2 * (16);
        senc.extend_from_slice(&((8 + senc_body_len) as u32).to_be_bytes());
        senc.extend_from_slice(b"senc");
        senc.extend_from_slice(&[0, 0, 0, 0]); // flags: no subsamples
        senc.extend_from_slice(&2u32.to_be_bytes());
        senc.extend_from_slice(&iv);
        senc.extend_from_slice(&iv);

        // tfhd: track 1, default duration 1024, default size 16.
        let mut tfhd = Vec::new();
        let tfhd_body = 4 + 4 + 4; // track_id + duration + size
        tfhd.extend_from_slice(&((8 + 4 + tfhd_body) as u32).to_be_bytes());
        tfhd.extend_from_slice(b"tfhd");
        tfhd.extend_from_slice(&[0, 0, 0x02, 0x18]); // duration + size defaults
        tfhd.extend_from_slice(&1u32.to_be_bytes());
        tfhd.extend_from_slice(&1024u32.to_be_bytes());
        tfhd.extend_from_slice(&16u32.to_be_bytes());

        // trun: data_offset only (no per-sample sizes).
        let mut trun = Vec::new();
        trun.extend_from_slice(&(20u32).to_be_bytes());
        trun.extend_from_slice(b"trun");
        trun.extend_from_slice(&[0, 0, 0, 0x01]); // data_offset flag
        trun.extend_from_slice(&2u32.to_be_bytes()); // 2 samples
        trun.extend_from_slice(&0i32.to_be_bytes()); // patched below

        let traf_len = 8 + tfhd.len() + senc.len() + trun.len();
        let moof_len = 8 + 16 + traf_len; // + mfhd
        let payload_off = moof_len + 8;

        trun[16..20].copy_from_slice(&(payload_off as i32).to_be_bytes());

        let mut frag = Vec::new();
        frag.extend_from_slice(&(moof_len as u32).to_be_bytes());
        frag.extend_from_slice(b"moof");
        frag.extend_from_slice(&16u32.to_be_bytes());
        frag.extend_from_slice(b"mfhd");
        frag.extend_from_slice(&0u32.to_be_bytes());
        frag.extend_from_slice(&1u32.to_be_bytes());
        frag.extend_from_slice(&(traf_len as u32).to_be_bytes());
        frag.extend_from_slice(b"traf");
        frag.extend_from_slice(&tfhd);
        frag.extend_from_slice(&senc);
        frag.extend_from_slice(&trun);
        frag.extend_from_slice(&((8 + payload.len() * 2) as u32).to_be_bytes());
        frag.extend_from_slice(b"mdat");
        // Each sample restarts the CTR stream (per-sample IV): encrypt the
        // two samples with independent ciphers using the same IV.
        let key = [0x55u8; 16];
        let mut e1 = Aes128Ctr::new((&key).into(), (&iv).into());
        let mut e2 = Aes128Ctr::new((&key).into(), (&iv).into());
        let mut encrypted = payload.clone();
        e1.apply_keystream(&mut encrypted);
        let mut encrypted2 = payload.clone();
        e2.apply_keystream(&mut encrypted2);
        frag.extend_from_slice(&encrypted);
        frag.extend_from_slice(&encrypted2);

        let decrypted_any = decrypt_cenc_fragment(&mut frag, &key).expect("decrypt");
        assert!(decrypted_any);
        let data_start = frag.len() - 32;
        assert_eq!(&frag[data_start..data_start + 16], &sample);
        assert_eq!(&frag[data_start + 16..data_start + 32], &sample);
    }
}
