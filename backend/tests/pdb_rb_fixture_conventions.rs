//! Env-gated golden-fixture test: verifies our documented per-table
//! `(u5, num_rl)` convention table (see `docs/PDB.md` and
//! `pdb_writer::data_page_footer_fields`) matches a real
//! reference-exported PDB.
//!
//! Set `REFERENCE_FIXTURE_PDB=<path>` to run; otherwise the test silently
//! returns. This mirrors the pattern for large reference export fixtures:
//! large fixtures don't belong in the repo, but local
//! development still gets coverage.

use backend::pdb_reader::{collect_pdb_data_page_seqs, validate_pdb_page_conventions};

#[test]
fn reference_exported_pdb_matches_documented_conventions() {
    let path = match std::env::var("REFERENCE_FIXTURE_PDB") {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "skipping reference_exported_pdb_matches_documented_conventions: \
                 set REFERENCE_FIXTURE_PDB=<path> to run"
            );
            return;
        }
    };

    let bytes = std::fs::read(&path).unwrap_or_else(|err| {
        panic!("failed to read REFERENCE_FIXTURE_PDB={path}: {err}");
    });

    let mismatches = validate_pdb_page_conventions(&bytes);
    assert!(
        mismatches.is_empty(),
        "reference export fixture {} violates page-header conventions on {} pages:\n{}",
        path,
        mismatches.len(),
        mismatches
            .iter()
            .take(20)
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );

    let seqs = collect_pdb_data_page_seqs(&bytes);
    assert!(
        seqs.len() >= 5,
        "expected reference fixture to have many data pages, found {}",
        seqs.len()
    );
    let unique: std::collections::HashSet<u32> = seqs.iter().copied().collect();
    assert!(
        unique.len() > 1,
        "reference fixture {} has all {} data-page seq values collapsed to {:?} — \
         the conventions module misidentified a reference-exported file",
        path,
        seqs.len(),
        seqs.first()
    );
}
