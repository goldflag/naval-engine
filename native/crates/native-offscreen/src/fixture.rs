//! Recorded expectations for native tests. A test computes what it checks as JSON and compares it with a committed
//! file; with `UPDATE_FIXTURES=1` it writes that file instead. A deliberate change is re-recorded with
//! `UPDATE_FIXTURES=1 cargo test -p <crate> --test <test>`, and the diff of the file is reviewed and committed with it.
//!
//! The files are written one value per line where a line would run long, so a re-recorded diff shows what moved.

use std::path::Path;

use serde_json::Value;

/// Whether this run rewrites its fixtures (`UPDATE_FIXTURES=1`).
pub fn updating() -> bool {
    std::env::var("UPDATE_FIXTURES").is_ok_and(|v| v == "1")
}

/// Check `actual` against the fixture at `path` (numbers within `tolerance`, relative: `|a - b| <= t * (1 + |b|)`;
/// integers, strings, booleans and the shape exactly), or write `actual` there when [`updating`].
pub fn check_or_record(path: &Path, actual: &Value, tolerance: f64) {
    if updating() {
        std::fs::write(path, to_lines(actual))
            .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        eprintln!("re-recorded {}", path.display());
        return;
    }
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e} (record it with UPDATE_FIXTURES=1)",
            path.display()
        )
    });
    let expected: Value = serde_json::from_str(&text).unwrap();
    if let Err(at) = compare(actual, &expected, tolerance, "") {
        panic!(
            "{}: {at}\nIf the change is deliberate, re-record with UPDATE_FIXTURES=1 and review the diff.",
            path.display()
        );
    }
}

/// The first place `actual` leaves `expected`, as a JSON path and both values.
pub fn compare(actual: &Value, expected: &Value, tolerance: f64, at: &str) -> Result<(), String> {
    let differs = || Err(format!("{at}: now {actual}, recorded {expected}"));
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
                return if a == b { Ok(()) } else { differs() };
            }
            if let (Some(a), Some(b)) = (a.as_u64(), b.as_u64()) {
                return if a == b { Ok(()) } else { differs() };
            }
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if (a - b).abs() <= tolerance * (1.0 + b.abs()) {
                Ok(())
            } else {
                differs()
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!("{at}: {} entries, recorded {}", a.len(), b.len()));
            }
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, tolerance, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => compare(x, y, tolerance, &format!("{at}.{key}"))?,
                    (Some(_), None) => return Err(format!("{at}.{key}: new, not recorded")),
                    (None, _) => return Err(format!("{at}.{key}: recorded, now missing")),
                }
            }
            Ok(())
        }
        (a, b) if a == b => Ok(()),
        _ => differs(),
    }
}

/// JSON with a value on one line when it fits in 120 columns, expanded one entry per line otherwise.
pub fn to_lines(value: &Value) -> String {
    let mut out = String::new();
    write(value, 0, &mut out);
    out.push('\n');
    out
}

fn write(value: &Value, depth: usize, out: &mut String) {
    let compact = value.to_string();
    if compact.len() + depth * 2 <= 120 {
        out.push_str(&compact);
        return;
    }
    let pad = "  ".repeat(depth + 1);
    let entries: Vec<(Option<&String>, &Value)> = match value {
        Value::Array(a) => a.iter().map(|v| (None, v)).collect(),
        Value::Object(o) => o.iter().map(|(k, v)| (Some(k), v)).collect(),
        _ => {
            out.push_str(&compact);
            return;
        }
    };
    let (open, close) = if value.is_array() {
        ('[', ']')
    } else {
        ('{', '}')
    };
    out.push(open);
    for (i, (key, v)) in entries.iter().enumerate() {
        out.push('\n');
        out.push_str(&pad);
        if let Some(key) = key {
            out.push_str(&Value::String((*key).clone()).to_string());
            out.push_str(": ");
        }
        write(v, depth + 1, out);
        if i + 1 < entries.len() {
            out.push(',');
        }
    }
    out.push('\n');
    out.push_str(&"  ".repeat(depth));
    out.push(close);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_compare_within_the_tolerance_and_integers_exactly() {
        assert!(compare(&json!(1.0 + 1e-10), &json!(1.0), 1e-9, "").is_ok());
        assert!(compare(&json!(1.1), &json!(1.0), 1e-9, "").is_err());
        assert!(compare(&json!(4_000_000_001u64), &json!(4_000_000_000u64), 1e-3, "").is_err());
        let at = compare(
            &json!({"a": [1, {"b": true}]}),
            &json!({"a": [1, {"b": false}]}),
            0.0,
            "",
        );
        assert_eq!(at.unwrap_err(), ".a[1].b: now true, recorded false");
    }

    #[test]
    fn the_written_file_reads_back_the_same() {
        let value = json!({"long": (0..60).collect::<Vec<_>>(), "short": [1.5, -2.25], "s": "x"});
        let text = to_lines(&value);
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), value);
        assert!(text.lines().count() > 3);
    }
}
