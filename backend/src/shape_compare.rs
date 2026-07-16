use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::edb::open_edb;
use crate::service::usb_vendor_compat::{USB_VENDOR_DB_DIR, USB_VENDOR_ROOT_DIR};
use crate::utils::{read_u16_le_at as read_u16_le, read_u32_le_at as read_u32_le};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PdbTablePointer {
    pub table_type: u32,
    pub first_page: u32,
    pub last_page: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ShapeSnapshot {
    pub edb_schema: BTreeMap<String, Vec<String>>,
    pub pdb_len_page: u32,
    pub pdb_num_tables: u32,
    pub pdb_table_pointers: Vec<PdbTablePointer>,
    pub pdb_table_shapes: Vec<PdbTableShape>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PdbTableShape {
    pub table_type: u32,
    pub pages: u32,
    pub rows_pages: u32,
    pub empty_pages: u32,
    pub sentinel_8191_pages: u32,
    pub sentinel_flag_pages: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PdbPointerDiff {
    pub table_type: u32,
    pub expected_first_page: u32,
    pub expected_last_page: u32,
    pub actual_first_page: u32,
    pub actual_last_page: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EdbSchemaTableDiff {
    pub table: String,
    pub missing_columns: Vec<String>,
    pub extra_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PdbTableShapeDiff {
    pub table_type: u32,
    pub expected: PdbTableShape,
    pub actual: PdbTableShape,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ShapeDiff {
    pub strict_match: bool,
    pub expected: ShapeSnapshot,
    pub actual: ShapeSnapshot,
    pub pdb_len_page_match: bool,
    pub pdb_num_tables_match: bool,
    pub missing_pdb_table_ids: Vec<u32>,
    pub extra_pdb_table_ids: Vec<u32>,
    pub pdb_pointer_diffs: Vec<PdbPointerDiff>,
    pub pdb_table_shape_diffs: Vec<PdbTableShapeDiff>,
    pub missing_edb_tables: Vec<String>,
    pub extra_edb_tables: Vec<String>,
    pub edb_schema_table_diffs: Vec<EdbSchemaTableDiff>,
}

fn vendor_db_dir(root: &Path) -> PathBuf {
    root.join(USB_VENDOR_ROOT_DIR).join(USB_VENDOR_DB_DIR)
}

fn edb_path(root: &Path) -> PathBuf {
    vendor_db_dir(root).join("exportLibrary.db")
}

fn pdb_path(root: &Path) -> PathBuf {
    vendor_db_dir(root).join("export.pdb")
}

fn load_edb_schema(path: &Path) -> BTreeMap<String, Vec<String>> {
    let mut warnings = Vec::new();
    let conn = open_edb(path, &mut warnings).expect("open eDB");
    let mut schema = BTreeMap::<String, Vec<String>>::new();
    let mut tables_stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare table list");
    let table_rows = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query table list");
    for table in table_rows {
        let table = table.expect("table name");
        let mut cols_stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .expect("prepare table_info");
        let cols = cols_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query table_info")
            .map(|row| row.expect("column name"))
            .collect::<Vec<_>>();
        schema.insert(table, cols);
    }
    schema
}

fn load_pdb_table_pointers(path: &Path) -> (u32, u32, Vec<PdbTablePointer>) {
    let bytes = std::fs::read(path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).expect("len_page");
    let num_tables = read_u32_le(&bytes, 8).expect("num_tables");
    let mut pointers = Vec::<PdbTablePointer>::with_capacity(num_tables as usize);
    let mut cursor = 28usize;
    for _ in 0..num_tables {
        let table_type = read_u32_le(&bytes, cursor).expect("table_type");
        let first_page = read_u32_le(&bytes, cursor + 8).expect("first_page");
        let last_page = read_u32_le(&bytes, cursor + 12).expect("last_page");
        pointers.push(PdbTablePointer {
            table_type,
            first_page,
            last_page,
        });
        cursor += 16;
    }
    (len_page, num_tables, pointers)
}

fn load_pdb_table_shapes(path: &Path) -> Vec<PdbTableShape> {
    let bytes = std::fs::read(path).expect("read PDB");
    let len_page = read_u32_le(&bytes, 4).unwrap_or(4096) as usize;
    if len_page == 0 || bytes.len() < len_page {
        return Vec::new();
    }
    let total_pages = bytes.len() / len_page;
    let mut by_table = BTreeMap::<u32, PdbTableShape>::new();

    // Skip page 0 (file header)
    for page_idx in 1..total_pages {
        let page_off = page_idx * len_page;
        let tt = read_u32_le(&bytes, page_off + 0x08).unwrap_or(u32::MAX);
        if tt == u32::MAX {
            continue;
        }
        let packed_lo = bytes.get(page_off + 0x18).copied().unwrap_or(0) as u32;
        let packed_mid = bytes.get(page_off + 0x19).copied().unwrap_or(0) as u32;
        let packed_hi = bytes.get(page_off + 0x1a).copied().unwrap_or(0) as u32;
        let packed = packed_lo | (packed_mid << 8) | (packed_hi << 16);
        let num_rl = (packed & 0x1FFF) as u16;
        let used_size = read_u16_le(&bytes, page_off + 0x1e).unwrap_or(0);

        let entry = by_table.entry(tt).or_insert(PdbTableShape {
            table_type: tt,
            pages: 0,
            rows_pages: 0,
            empty_pages: 0,
            sentinel_8191_pages: 0,
            sentinel_flag_pages: 0,
        });
        entry.pages += 1;
        let page_flags = bytes.get(page_off + 0x1b).copied().unwrap_or(0);
        if page_flags == 0x64 {
            entry.sentinel_flag_pages += 1;
        }
        if num_rl == 8191 {
            entry.sentinel_8191_pages += 1;
        }
        if used_size == 0 {
            entry.empty_pages += 1;
        } else {
            entry.rows_pages += 1;
        }
    }

    by_table.into_values().collect()
}

fn snapshot_usb_shape(root: &Path) -> ShapeSnapshot {
    let edb_schema = load_edb_schema(&edb_path(root));
    let (pdb_len_page, pdb_num_tables, pdb_table_pointers) =
        load_pdb_table_pointers(&pdb_path(root));
    let pdb_table_shapes = load_pdb_table_shapes(&pdb_path(root));
    ShapeSnapshot {
        edb_schema,
        pdb_len_page,
        pdb_num_tables,
        pdb_table_pointers,
        pdb_table_shapes,
    }
}

pub fn compare_usb_shape(expected_root: &Path, actual_root: &Path) -> ShapeDiff {
    let expected = snapshot_usb_shape(expected_root);
    let actual = snapshot_usb_shape(actual_root);

    let pdb_len_page_match = expected.pdb_len_page == actual.pdb_len_page;
    let pdb_num_tables_match = expected.pdb_num_tables == actual.pdb_num_tables;

    let expected_ids = expected
        .pdb_table_pointers
        .iter()
        .map(|p| p.table_type)
        .collect::<BTreeSet<_>>();
    let actual_ids = actual
        .pdb_table_pointers
        .iter()
        .map(|p| p.table_type)
        .collect::<BTreeSet<_>>();

    let missing_pdb_table_ids = expected_ids
        .difference(&actual_ids)
        .copied()
        .collect::<Vec<_>>();
    let extra_pdb_table_ids = actual_ids
        .difference(&expected_ids)
        .copied()
        .collect::<Vec<_>>();

    let expected_ptrs = expected
        .pdb_table_pointers
        .iter()
        .map(|p| (p.table_type, p))
        .collect::<BTreeMap<_, _>>();
    let actual_ptrs = actual
        .pdb_table_pointers
        .iter()
        .map(|p| (p.table_type, p))
        .collect::<BTreeMap<_, _>>();
    let mut pdb_pointer_diffs = Vec::<PdbPointerDiff>::new();
    for table_type in expected_ids.intersection(&actual_ids) {
        let e = expected_ptrs.get(table_type).expect("expected ptr");
        let a = actual_ptrs.get(table_type).expect("actual ptr");
        if e.first_page != a.first_page || e.last_page != a.last_page {
            pdb_pointer_diffs.push(PdbPointerDiff {
                table_type: *table_type,
                expected_first_page: e.first_page,
                expected_last_page: e.last_page,
                actual_first_page: a.first_page,
                actual_last_page: a.last_page,
            });
        }
    }

    let expected_shapes = expected
        .pdb_table_shapes
        .iter()
        .map(|s| (s.table_type, s))
        .collect::<BTreeMap<_, _>>();
    let actual_shapes = actual
        .pdb_table_shapes
        .iter()
        .map(|s| (s.table_type, s))
        .collect::<BTreeMap<_, _>>();
    let mut pdb_table_shape_diffs = Vec::<PdbTableShapeDiff>::new();
    for table_type in expected_ids.intersection(&actual_ids) {
        if let (Some(e), Some(a)) = (
            expected_shapes.get(table_type),
            actual_shapes.get(table_type),
        )
            && *e != *a {
                pdb_table_shape_diffs.push(PdbTableShapeDiff {
                    table_type: *table_type,
                    expected: (*e).clone(),
                    actual: (*a).clone(),
                });
            }
    }

    let expected_tables = expected.edb_schema.keys().cloned().collect::<BTreeSet<_>>();
    let actual_tables = actual.edb_schema.keys().cloned().collect::<BTreeSet<_>>();
    let missing_edb_tables = expected_tables
        .difference(&actual_tables)
        .cloned()
        .collect::<Vec<_>>();
    let extra_edb_tables = actual_tables
        .difference(&expected_tables)
        .cloned()
        .collect::<Vec<_>>();

    let mut edb_schema_table_diffs = Vec::<EdbSchemaTableDiff>::new();
    for table in expected_tables.intersection(&actual_tables) {
        let e_cols = expected
            .edb_schema
            .get(table)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let a_cols = actual
            .edb_schema
            .get(table)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let missing_columns = e_cols.difference(&a_cols).cloned().collect::<Vec<_>>();
        let extra_columns = a_cols.difference(&e_cols).cloned().collect::<Vec<_>>();
        if !missing_columns.is_empty() || !extra_columns.is_empty() {
            edb_schema_table_diffs.push(EdbSchemaTableDiff {
                table: table.clone(),
                missing_columns,
                extra_columns,
            });
        }
    }

    let strict_match = pdb_len_page_match
        && pdb_num_tables_match
        && missing_pdb_table_ids.is_empty()
        && extra_pdb_table_ids.is_empty()
        && pdb_pointer_diffs.is_empty()
        && pdb_table_shape_diffs.is_empty()
        && missing_edb_tables.is_empty()
        && extra_edb_tables.is_empty()
        && edb_schema_table_diffs.is_empty();

    ShapeDiff {
        strict_match,
        expected,
        actual,
        pdb_len_page_match,
        pdb_num_tables_match,
        missing_pdb_table_ids,
        extra_pdb_table_ids,
        pdb_pointer_diffs,
        pdb_table_shape_diffs,
        missing_edb_tables,
        extra_edb_tables,
        edb_schema_table_diffs,
    }
}
