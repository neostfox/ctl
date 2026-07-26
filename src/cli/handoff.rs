use super::*;

pub(super) fn cmd_handoff(command: &HandoffCommands) -> Result<()> {
    match command {
        HandoffCommands::Export { id, json } => {
            let app = app_open(false)?;
            let h = app.handoff_export(id)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&h)?);
            } else {
                render_handoff_human(&h);
            }
            Ok(())
        }
        HandoffCommands::Capture { id, file, json } => {
            let app = app_open(false)?;
            let capture = app.capture_handoff(id, Path::new(file))?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&capture)?);
            } else {
                println!(
                    "Captured handoff judgment for '{}' at .ctl/handoffs/{}.json",
                    id, id
                );
                println!("Next safe action: {}", capture["next_safe_action"]);
            }
            Ok(())
        }
    }
}

/// Human-readable digest of a `control.handoff.v1` artifact.
pub(super) fn render_handoff_human(h: &serde_json::Value) {
    let s = |k: &str| h.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let arr_join = |v: Option<&serde_json::Value>| {
        v.and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };

    println!("── Handoff: {} ──", s("task_id"));
    println!(
        "Phase: {}{}",
        s("phase"),
        if h.get("is_held").and_then(|v| v.as_bool()) == Some(true) {
            " (HELD)"
        } else {
            ""
        }
    );
    if let Some(obj) = h.get("objective").and_then(|v| v.as_str()) {
        println!("Objective: {obj}");
    }
    if let Some(b) = h.get("boundary") {
        println!("Write scope: {}", arr_join(b.get("write_allow")));
        let deny = arr_join(b.get("write_deny"));
        if !deny.is_empty() {
            println!("Write deny: {deny}");
        }
        println!("Read scope: {}", arr_join(b.get("read_scope")));
    }

    if let Some(gates) = h.get("gate_status").and_then(|v| v.as_array()) {
        println!("Gates:");
        for g in gates {
            println!(
                "  {} {}",
                g.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
                g.get("gate").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
    }

    if let Some(verdict) = h
        .get("interlock")
        .and_then(|i| i.get("verdict"))
        .and_then(|v| v.as_str())
    {
        println!("Completion interlock: {verdict}");
    }

    if let Some(na) = h.get("next_action").filter(|v| !v.is_null()) {
        println!(
            "Next action: {} — {}",
            na.get("action").and_then(|v| v.as_str()).unwrap_or("?"),
            na.get("rationale").and_then(|v| v.as_str()).unwrap_or("")
        );
        if let Some(cmd) = na.get("suggested_command").and_then(|v| v.as_str()) {
            if !cmd.is_empty() {
                println!("  -> {cmd}");
            }
        }
    }

    match h.get("uncommitted_in_scope") {
        Some(serde_json::Value::Array(files)) if !files.is_empty() => {
            println!("Uncommitted in scope ({}):", files.len());
            for f in files {
                if let Some(f) = f.as_str() {
                    println!("  {f}");
                }
            }
        }
        Some(serde_json::Value::Array(_)) => println!("Uncommitted in scope: none (clean)"),
        _ => println!("Uncommitted in scope: unverifiable (non-git)"),
    }

    if let Some(events) = h.get("recent_events").and_then(|v| v.as_array()) {
        println!("Recent events:");
        for e in events {
            println!(
                "  seq {} {} ({})",
                e.get("seq").and_then(|v| v.as_i64()).unwrap_or(0),
                e.get("type").and_then(|v| v.as_str()).unwrap_or("?"),
                e.get("at").and_then(|v| v.as_str()).unwrap_or("")
            );
        }
    }
}
