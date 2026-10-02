//! Small helpers over lopdf's object model: reference resolution, typed
//! lookups, stream filters, loading and saving with the settings the planner
//! wants.

use lopdf::{Dictionary, Document, Object, ObjectId, SaveOptions, Stream};

use crate::codec::{self, Predictor};
use crate::PdfError;

/// Follow indirect references (bounded) to the underlying object.
pub fn resolve<'a>(doc: &'a Document, mut obj: &'a Object) -> &'a Object {
    for _ in 0..32 {
        match obj {
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(o) => obj = o,
                Err(_) => return &Object::Null,
            },
            _ => return obj,
        }
    }
    &Object::Null
}

/// Dictionary entry, dereferenced.
pub fn entry<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    dict.get(key).ok().map(|o| resolve(doc, o))
}

pub fn entry_i64(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match entry(doc, dict, key)? {
        Object::Integer(i) => Some(*i),
        Object::Real(r) => Some(*r as i64),
        _ => None,
    }
}

pub fn entry_name<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    entry(doc, dict, key).and_then(|o| o.as_name().ok())
}

/// Filter names of a stream, in decoding order, with references resolved.
pub fn filters(doc: &Document, stream: &Stream) -> Vec<Vec<u8>> {
    match entry(doc, &stream.dict, b"Filter") {
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|o| resolve(doc, o).as_name().ok().map(|n| n.to_vec()))
            .collect(),
        _ => Vec::new(),
    }
}

/// `/DecodeParms` for the first (or only) filter, as a predictor description.
/// Absent parameters mean no predictor.
pub fn first_decode_parms(doc: &Document, stream: &Stream) -> Option<Predictor> {
    let parms = entry(doc, &stream.dict, b"DecodeParms")?;
    let dict = match parms {
        Object::Dictionary(d) => d,
        Object::Array(items) => match items.first().map(|o| resolve(doc, o)) {
            Some(Object::Dictionary(d)) => d,
            _ => return None,
        },
        _ => return None,
    };
    Some(Predictor {
        predictor: entry_i64(doc, dict, b"Predictor").unwrap_or(1),
        colors: entry_i64(doc, dict, b"Colors").unwrap_or(1).max(0) as usize,
        bits_per_component: entry_i64(doc, dict, b"BitsPerComponent")
            .unwrap_or(8)
            .max(0) as usize,
        columns: entry_i64(doc, dict, b"Columns").unwrap_or(1).max(0) as usize,
    })
}

/// Raw bytes of a stream after undoing a single Flate layer (or none), with
/// strict error handling. Other filters are an error.
pub fn plain_bytes(doc: &Document, stream: &Stream) -> Result<Vec<u8>, String> {
    let f = filters(doc, stream);
    match f.as_slice() {
        [] => Ok(stream.content.clone()),
        [name] if name == b"FlateDecode" => codec::inflate(&stream.content, codec::INFLATE_LIMIT),
        other => Err(format!(
            "unsupported filter chain {:?}",
            other
                .iter()
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .collect::<Vec<_>>()
        )),
    }
}

pub fn is_image_xobject(dict: &Dictionary) -> bool {
    dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image")
}

/// Load with the lenient parser but a per-stream inflate bound, mapping any
/// parse failure to [`PdfError::Damaged`].
pub fn load(bytes: &[u8]) -> Result<Document, PdfError> {
    let opts = lopdf::LoadOptions {
        max_decompressed_size: Some(codec::INFLATE_LIMIT),
        ..Default::default()
    };
    let doc = Document::load_mem_with_options(bytes, opts)
        .map_err(|e| PdfError::Damaged(e.to_string()))?;
    if doc.get_pages().is_empty() {
        return Err(PdfError::Damaged("no pages found".into()));
    }
    Ok(doc)
}

/// Serialise with object streams and a cross-reference stream (PDF 1.5
/// features; lopdf bumps the version header if needed).
pub fn save(doc: &mut Document) -> Result<Vec<u8>, PdfError> {
    // Keys that only describe the cross-reference stream we loaded from;
    // lopdf writes fresh ones for the stream it produces.
    for key in [
        b"Prev".as_slice(),
        b"XRefStm",
        b"Filter",
        b"DecodeParms",
        b"Length",
        b"W",
        b"Index",
        b"Type",
    ] {
        doc.trailer.remove(key);
    }
    let options = SaveOptions::builder()
        .use_object_streams(true)
        .use_xref_streams(true)
        .compression_level(9)
        .build();
    let mut out = Vec::new();
    doc.save_with_options(&mut out, options)
        .map_err(|e| PdfError::Internal(format!("serialising PDF: {e}")))?;
    Ok(out)
}

/// All object ids whose object is a stream, in id order.
pub fn stream_ids(doc: &Document) -> Vec<ObjectId> {
    doc.objects
        .iter()
        .filter(|(_, o)| matches!(o, Object::Stream(_)))
        .map(|(id, _)| *id)
        .collect()
}
