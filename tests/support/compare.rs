use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiff {
    pub path: String,
    pub expected: String,
    pub actual: String,
}

pub fn compare_json<T: Serialize>(
    expected: &T,
    actual: &T,
    ignored_paths: &[&str],
) -> Vec<SemanticDiff> {
    let expected = serde_json::to_value(expected).expect("serialize expected");
    let actual = serde_json::to_value(actual).expect("serialize actual");
    let mut diffs = Vec::new();
    compare_value("$", &expected, &actual, ignored_paths, &mut diffs);
    diffs
}

fn compare_value(
    path: &str,
    expected: &Value,
    actual: &Value,
    ignored_paths: &[&str],
    diffs: &mut Vec<SemanticDiff>,
) {
    if ignored_paths.contains(&path) {
        return;
    }

    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys = a.keys().chain(b.keys()).collect::<Vec<_>>();
            keys.sort();
            keys.dedup();
            for key in keys {
                let next = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(left), Some(right)) => {
                        compare_value(&next, left, right, ignored_paths, diffs)
                    }
                    (left, right) => diffs.push(SemanticDiff {
                        path: next,
                        expected: left
                            .map_or_else(|| "<missing>".into(), |value| value.to_string()),
                        actual: right.map_or_else(|| "<missing>".into(), |value| value.to_string()),
                    }),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let len = a.len().max(b.len());
            for index in 0..len {
                let next = format!("{path}[{index}]");
                match (a.get(index), b.get(index)) {
                    (Some(left), Some(right)) => {
                        compare_value(&next, left, right, ignored_paths, diffs)
                    }
                    (left, right) => diffs.push(SemanticDiff {
                        path: next,
                        expected: left
                            .map_or_else(|| "<missing>".into(), |value| value.to_string()),
                        actual: right.map_or_else(|| "<missing>".into(), |value| value.to_string()),
                    }),
                }
            }
        }
        _ if expected != actual => diffs.push(SemanticDiff {
            path: path.into(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        }),
        _ => {}
    }
}

pub fn assert_semantic_eq<T: Serialize>(expected: &T, actual: &T, ignored_paths: &[&str]) {
    let diffs = compare_json(expected, actual, ignored_paths);
    assert!(
        diffs.is_empty(),
        "semantic mismatch (first {}):\n{}",
        diffs.len().min(10),
        diffs
            .iter()
            .take(10)
            .map(|diff| format!(
                "{}\n  expected: {}\n  actual:   {}",
                diff.path, diff.expected, diff.actual
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
