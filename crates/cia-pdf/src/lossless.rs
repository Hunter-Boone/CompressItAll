//! Step 1 of the PDF planner: everything that shrinks the file without
//! changing what it renders. Metadata and thumbnails go, unreferenced
//! objects go, duplicate streams merge, and every Flate stream is squeezed
//! harder than its producer bothered to.

use std::collections::HashMap;

use lopdf::{Document, Object, ObjectId};
use sha2::{Digest, Sha256};

use crate::codec;
use crate::doc::{filters, stream_ids};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LosslessStats {
    pub pruned: usize,
    pub deduplicated: usize,
    pub redeflated: usize,
    pub newly_deflated: usize,
    pub keys_removed: usize,
}

/// Run the whole lossless pass in place. Saving is the caller's job.
pub fn lossless_pass(doc: &mut Document, keep_document_details: bool) -> LosslessStats {
    let keys_removed = strip_metadata(doc, keep_document_details);
    let pruned = doc.prune_objects().len();
    let (redeflated, newly_deflated) = redeflate_all(doc);
    let deduplicated = dedupe_streams(doc);
    LosslessStats {
        pruned,
        deduplicated,
        redeflated,
        newly_deflated,
        keys_removed,
    }
}

/// Drop `/Thumb`, `/PieceInfo` and (unless kept) `/Metadata` from every
/// dictionary in the file. The objects they pointed at become unreferenced
/// and fall to `prune_objects`.
fn strip_metadata(doc: &mut Document, keep_document_details: bool) -> usize {
    let mut removed = 0;
    let mut keys: Vec<&[u8]> = vec![b"Thumb", b"PieceInfo"];
    if !keep_document_details {
        keys.push(b"Metadata");
    }
    for object in doc.objects.values_mut() {
        let dict = match object {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        for key in &keys {
            if dict.remove(key).is_some() {
                removed += 1;
            }
        }
    }
    removed
}

/// Re-deflate every stream whose first filter is Flate, and deflate every
/// unfiltered stream, keeping the result only when it is smaller.
fn redeflate_all(doc: &mut Document) -> (usize, usize) {
    let mut redeflated = 0;
    let mut newly = 0;
    for id in stream_ids(doc) {
        let (f, is_container) = {
            let stream = match doc.get_object(id) {
                Ok(Object::Stream(s)) => s,
                _ => continue,
            };
            let ty = stream.dict.get(b"Type").ok().and_then(|o| o.as_name().ok());
            (
                filters(doc, stream),
                matches!(ty, Some(b"ObjStm") | Some(b"XRef")),
            )
        };
        if is_container {
            continue;
        }
        let stream = match doc.get_object_mut(id) {
            Ok(Object::Stream(s)) => s,
            _ => continue,
        };
        match f.first().map(|n| n.as_slice()) {
            None => {
                if stream.content.is_empty() {
                    continue;
                }
                let z = codec::deflate_best(&stream.content);
                // "/Filter /FlateDecode" costs about 20 bytes of dictionary.
                if z.len() + 20 < stream.content.len() {
                    stream
                        .dict
                        .set("Filter", Object::Name(b"FlateDecode".to_vec()));
                    stream.set_content(z);
                    newly += 1;
                }
            }
            Some(b"FlateDecode") => {
                let plain = match codec::inflate(&stream.content, codec::INFLATE_LIMIT) {
                    Ok(p) => p,
                    Err(e) => {
                        log::debug!("object {id:?}: leaving Flate stream alone: {e}");
                        continue;
                    }
                };
                let z = codec::deflate_best(&plain);
                if z.len() < stream.content.len() {
                    stream.set_content(z);
                    redeflated += 1;
                }
            }
            Some(_) => {}
        }
    }
    (redeflated, newly)
}

/// Merge streams whose dictionary and bytes are identical. Returns how many
/// objects were folded away.
pub fn dedupe_streams(doc: &mut Document) -> usize {
    let mut seen: HashMap<[u8; 32], ObjectId> = HashMap::new();
    let mut replace: HashMap<ObjectId, ObjectId> = HashMap::new();
    for id in stream_ids(doc) {
        let stream = match doc.get_object(id) {
            Ok(Object::Stream(s)) => s,
            _ => continue,
        };
        let ty = stream.dict.get(b"Type").ok().and_then(|o| o.as_name().ok());
        if matches!(ty, Some(b"ObjStm") | Some(b"XRef")) {
            continue;
        }
        let mut hasher = Sha256::new();
        // The Debug form of a dictionary is deterministic for equal insertion
        // order; a differing order merely misses a merge, never makes one.
        hasher.update(format!("{:?}", stream.dict).as_bytes());
        hasher.update([0u8]);
        hasher.update(&stream.content);
        let digest: [u8; 32] = hasher.finalize().into();
        match seen.get(&digest) {
            Some(&canonical) => {
                replace.insert(id, canonical);
            }
            None => {
                seen.insert(digest, id);
            }
        }
    }
    if replace.is_empty() {
        return 0;
    }
    doc.traverse_objects(|obj| {
        if let Object::Reference(id) = obj {
            if let Some(new) = replace.get(id) {
                *id = *new;
            }
        }
    });
    for id in replace.keys() {
        doc.objects.remove(id);
    }
    replace.len()
}
