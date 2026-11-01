//! M8 differential campaign runner (ROADMAP §12.2). Not part of the regular suite: a campaign
//! is a directory of generated `<id>.json` / `<id>.showdown.json` pairs (`tools/campaign.py`),
//! replayed here exactly like the pinned scenarios.
//!
//!     CAMPAIGN_DIR=<dir> cargo test --release --test campaign -- --ignored --nocapture
//!
//! `CAMPAIGN_ONLY=<substring>` replays only the matching battles (with `REPLAY_DEBUG=1`, the
//! engine trace of a plan the engine did not use up). Every pair is replayed in isolation: a panic in one battle is recorded as that battle's
//! result and the campaign goes on. `<dir>/results.json` lists each battle's outcome and its
//! divergences; each diverging battle gets a self-contained failure bundle under
//! `<dir>/failures/<id>/` (TESTING.md §9).

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{Scenario, load_json, replay_against_oracle, write_failure_bundle};

fn battles(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("campaign dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".json")
                && !name.ends_with(".showdown.json")
                && name != "results.json"
                && path
                    .with_file_name(name.replace(".json", ".showdown.json"))
                    .exists()
        })
        .collect();
    paths.sort();
    paths
}

#[test]
#[ignore = "a differential campaign: CAMPAIGN_DIR=<dir> (tools/campaign.py)"]
fn campaign() {
    let Some(dir) = std::env::var_os("CAMPAIGN_DIR").map(PathBuf::from) else {
        panic!("set CAMPAIGN_DIR to a directory of generated battles");
    };
    // A battle that panics is a result, not the end of the campaign; keep its message only
    // (a filtered run keeps the whole report, for debugging).
    let only = std::env::var("CAMPAIGN_ONLY").ok();
    if only.is_none() {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let mut results = Vec::new();
    let (mut passed, mut diverged, mut panicked) = (0, 0, 0);
    for path in battles(&dir) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if only.as_deref().is_some_and(|only| !name.contains(only)) {
            continue;
        }
        let oracle_path = path.with_file_name(name.replace(".json", ".showdown.json"));
        let scenario: Scenario = load_json(&path);
        let oracle: Value = load_json(&oracle_path);
        support::REPLAY_CONTEXT.with(|c| c.borrow_mut().clear());
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            replay_against_oracle(&scenario, &oracle)
        }));
        let entry = match outcome {
            Ok(report) if report.divergences.is_empty() => {
                passed += 1;
                json!({
                    "id": scenario.id,
                    "status": "pass",
                    "frames": report.frames_compared,
                    "actions": report.actions_replayed,
                })
            }
            Ok(report) => {
                diverged += 1;
                let bundle = dir.join("failures").join(&scenario.id);
                let reproduce = format!(
                    "CAMPAIGN_DIR={} cargo test --release --test campaign -- --ignored --nocapture   # battle {}",
                    dir.display(),
                    scenario.id
                );
                write_failure_bundle(&bundle, &path, &oracle, &report, &reproduce)
                    .expect("write failure bundle");
                json!({
                    "id": scenario.id,
                    "status": "diverged",
                    "frames": report.frames_compared,
                    "actions": report.actions_replayed,
                    "first_divergence": report.divergences.first(),
                    "divergences": report.divergences.len(),
                })
            }
            Err(payload) => {
                panicked += 1;
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_else(|| "panic".into());
                let context = support::REPLAY_CONTEXT.with(|c| c.borrow().clone());
                let message = format!("{context}: {message}");
                json!({"id": scenario.id, "status": "panicked", "message": message})
            }
        };
        results.push(entry);
    }
    if only.is_none() {
        let _ = std::panic::take_hook();
    }
    let summary = json!({
        "battles": results.len(),
        "passed": passed,
        "diverged": diverged,
        "panicked": panicked,
        "results": results,
    });
    if only.is_none() {
        std::fs::write(
            dir.join("results.json"),
            serde_json::to_string_pretty(&summary).expect("json") + "\n",
        )
        .expect("write results");
    } else {
        // A targeted replay leaves results.json alone and says what went wrong here instead.
        for entry in &results {
            if entry["status"] != "pass" {
                let detail = entry
                    .get("first_divergence")
                    .or_else(|| entry.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                println!("{} {}: {detail}", entry["id"], entry["status"]);
            }
        }
    }
    println!(
        "campaign: {} battles, {passed} passed, {diverged} diverged, {panicked} panicked",
        summary["battles"]
    );
}
