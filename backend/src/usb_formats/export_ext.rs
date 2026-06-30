//! `exportExt.pdb` exploratory parser scaffold.

use crate::error::{BackendError, BackendResult};

#[derive(Debug, Clone)]
pub struct ExportExtSummary {
    pub byte_len: usize,
    pub pmng_offsets: Vec<usize>,
    pub ptbl_offsets: Vec<usize>,
}

pub fn summarize_export_ext_bytes(bytes: &[u8]) -> BackendResult<ExportExtSummary> {
    if bytes.is_empty() {
        return Err(BackendError::Validation(
            "exportExt bytes must not be empty".to_string(),
        ));
    }
    Ok(ExportExtSummary {
        byte_len: bytes.len(),
        pmng_offsets: find_ascii_tag_offsets(bytes, b"PMNG"),
        ptbl_offsets: find_ascii_tag_offsets(bytes, b"PTBL"),
    })
}

fn find_ascii_tag_offsets(bytes: &[u8], tag: &[u8]) -> Vec<usize> {
    if tag.is_empty() || bytes.len() < tag.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for idx in 0..=bytes.len() - tag.len() {
        if &bytes[idx..idx + tag.len()] == tag {
            out.push(idx);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_finds_pmng_and_ptbl_tags() {
        let bytes = b"xxxxPMNGyyyyPTBLzzPMNG";
        let s = summarize_export_ext_bytes(bytes).expect("summary");
        assert_eq!(s.byte_len, bytes.len());
        assert_eq!(s.pmng_offsets, vec![4, 18]);
        assert_eq!(s.ptbl_offsets, vec![12]);
    }
}
