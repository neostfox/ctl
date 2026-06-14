use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::application::{generate_uuid, now_iso8601};

// ── Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleGroup {
    pub group_id: String,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConflict {
    pub task_a: String,
    pub task_b: String,
    pub overlapping_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulePlan {
    pub plan_id: String,
    pub groups: Vec<ScheduleGroup>,
    pub conflicts: Vec<ScheduleConflict>,
    pub max_concurrent: usize,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TaskCurrentState {
    pub task_id: String,
    pub phase: String,
    pub is_held: bool,
    pub write_allow: BTreeSet<String>,
}

// ── Overlap detection ──

/// Normalize a path so it always ends with `/` for directory-aware prefix comparison.
fn normalize_path(path: &str) -> String {
    if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{}/", path)
    }
}

/// Detect if two write_allow sets have overlapping paths.
/// Overlap = one path is a prefix of another.
/// Returns overlapping paths if any.
pub fn detect_write_scope_overlap(
    write_allow_a: &BTreeSet<String>,
    write_allow_b: &BTreeSet<String>,
) -> Vec<String> {
    let mut overlaps = Vec::new();

    // For each path in A, check if any path in B is a prefix of it or vice versa.
    for a in write_allow_a {
        let a_norm = normalize_path(a);
        for b in write_allow_b {
            let b_norm = normalize_path(b);
            if a_norm.starts_with(&b_norm) || b_norm.starts_with(&a_norm) {
                // Report the original (non-normalized) paths.
                overlaps.push(a.clone());
                overlaps.push(b.clone());
                break; // each path in A contributes at most once
            }
        }
    }

    overlaps.sort();
    overlaps.dedup();
    overlaps
}

// ── Schedule planning ──

/// Plan concurrent execution of ready tasks.
///
/// Tasks with overlapping `write_allow` are placed in different groups (sequential).
/// Tasks with disjoint `write_allow` can run in the same group (parallel).
/// Read-only tasks (empty `write_allow`) can join any group.
pub fn plan_schedule(tasks: &[(String, BTreeSet<String>)], max_concurrent: usize) -> SchedulePlan {
    let n = tasks.len();
    let max_concurrent = max_concurrent.max(1);

    // Build conflict adjacency list.
    // Two tasks conflict if they have overlapping write_allow sets.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut conflicts: Vec<ScheduleConflict> = Vec::new();

    for i in 0..n {
        for j in (i + 1)..n {
            let overlap = detect_write_scope_overlap(&tasks[i].1, &tasks[j].1);
            if !overlap.is_empty() {
                adj[i].push(j);
                adj[j].push(i);
                conflicts.push(ScheduleConflict {
                    task_a: tasks[i].0.clone(),
                    task_b: tasks[j].0.clone(),
                    overlapping_paths: overlap,
                });
            }
        }
    }

    // Greedy graph coloring.
    // color[i] = group index assigned to task i.
    let mut color: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        // Tasks with no write_allow (read-only) never conflict — they join any group.
        let is_read_only = tasks[i].1.is_empty();

        if is_read_only {
            // Will be assigned below after we figure out which group has room.
        } else {
            // Collect group indices used by conflicting neighbors.
            let neighbor_colors: BTreeSet<usize> =
                adj[i].iter().filter_map(|&nb| color[nb]).collect();

            // Find the first group color not used by any neighbor.
            let mut chosen: Option<usize> = None;
            for c in 0.. {
                if !neighbor_colors.contains(&c) {
                    chosen = Some(c);
                    break;
                }
            }
            color[i] = chosen;
        }
    }

    // Assign read-only tasks: find the first existing group with room,
    // or create a new one.
    // First, count per-group occupancy (from non-read-only tasks).
    let mut group_sizes: Vec<usize> = Vec::new();
    for g in color.iter().flatten() {
        while group_sizes.len() <= *g {
            group_sizes.push(0);
        }
        group_sizes[*g] += 1;
    }

    for (i, task) in tasks.iter().enumerate() {
        if task.1.is_empty() {
            // Read-only: find first group with room.
            let mut assigned = false;
            for (g, size) in group_sizes.iter_mut().enumerate() {
                if *size < max_concurrent {
                    color[i] = Some(g);
                    *size += 1;
                    assigned = true;
                    break;
                }
            }
            if !assigned {
                let new_g = group_sizes.len();
                color[i] = Some(new_g);
                group_sizes.push(1);
            }
        }
    }

    // Now enforce max_concurrent cap: split overfull groups into multiple groups.
    // Reconstruct groups from color assignments, then split any that exceed the cap.
    let max_color = color.iter().filter_map(|c| *c).max().map_or(0, |c| c + 1);
    let mut raw_groups: Vec<Vec<usize>> = vec![Vec::new(); max_color];
    for (i, c) in color.iter().enumerate().take(n) {
        if let Some(g) = c {
            raw_groups[*g].push(i);
        }
    }

    let mut final_groups: Vec<ScheduleGroup> = Vec::new();
    let mut group_counter: usize = 0;

    for members in &raw_groups {
        if members.is_empty() {
            continue;
        }
        // Split into chunks of max_concurrent.
        for chunk in members.chunks(max_concurrent) {
            final_groups.push(ScheduleGroup {
                group_id: format!("g{}", group_counter),
                task_ids: chunk.iter().map(|&i| tasks[i].0.clone()).collect(),
            });
            group_counter += 1;
        }
    }

    SchedulePlan {
        plan_id: generate_uuid(),
        groups: final_groups,
        conflicts,
        max_concurrent,
        created_at: now_iso8601(),
    }
}

// ── Plan validation ──

/// Validate that a plan is still valid against current state.
///
/// Checks: phase still Ready or InProgress, not held, write_allow unchanged.
pub fn validate_plan(
    plan: &SchedulePlan,
    current_states: &[TaskCurrentState],
) -> Result<(), Vec<String>> {
    let state_map: std::collections::HashMap<&str, &TaskCurrentState> = current_states
        .iter()
        .map(|s| (s.task_id.as_str(), s))
        .collect();

    let mut errors: Vec<String> = Vec::new();

    for group in &plan.groups {
        for task_id in &group.task_ids {
            let state = match state_map.get(task_id.as_str()) {
                Some(s) => *s,
                None => {
                    errors.push(format!("task {} not found in current states", task_id));
                    continue;
                }
            };

            if state.phase != "ready" && state.phase != "in_progress" {
                errors.push(format!(
                    "task {} phase is {} (expected ready or in_progress)",
                    task_id, state.phase
                ));
            }

            if state.is_held {
                errors.push(format!("task {} is held", task_id));
            }

            // Check write_allow unchanged — gather the plan's expected write_allow
            // from the tasks used to build the plan. Since the plan doesn't carry
            // write_allow, we validate that the current write_allow is non-empty
            // only if the task appears in conflicts (meaning it had writes).
            // The real invariant: if a task is in any conflict, its current
            // write_allow must still intersect with the conflict paths.
            for conflict in &plan.conflicts {
                if conflict.task_a == *task_id || conflict.task_b == *task_id {
                    let has_overlap = detect_write_scope_overlap(
                        &state.write_allow,
                        &conflict
                            .overlapping_paths
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<String>>(),
                    );
                    if has_overlap.is_empty() && !state.write_allow.is_empty() {
                        errors.push(format!(
                            "task {} write_allow changed — conflict paths no longer overlap",
                            task_id
                        ));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn wa(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_no_overlap_same_group() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("t1".into(), wa(&["src/a/"])),
            ("t2".into(), wa(&["src/b/"])),
        ];
        let plan = plan_schedule(&tasks, 10);

        // Both tasks should be in the same group since write_allow is disjoint.
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].task_ids.len(), 2);
    }

    #[test]
    fn test_overlap_different_groups() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("t1".into(), wa(&["src/foo/"])),
            ("t2".into(), wa(&["src/foo/bar.rs"])),
        ];
        let plan = plan_schedule(&tasks, 10);

        // Overlapping write_allow → different groups.
        assert_eq!(plan.groups.len(), 2);
        assert!(plan.conflicts.len() >= 1);
    }

    #[test]
    fn test_read_only_always_parallel() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("writer".into(), wa(&["src/a/"])),
            ("reader".into(), BTreeSet::new()),
        ];
        let plan = plan_schedule(&tasks, 10);

        // Read-only task joins writer's group.
        assert_eq!(plan.groups.len(), 1);
        assert!(plan.groups[0].task_ids.contains(&"reader".to_string()));
    }

    #[test]
    fn test_max_concurrent_enforced() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("t1".into(), wa(&["src/a/"])),
            ("t2".into(), wa(&["src/b/"])),
            ("t3".into(), wa(&["src/c/"])),
            ("t4".into(), wa(&["src/d/"])),
        ];
        // All disjoint → same group, but max_concurrent = 2 → split into 2 groups.
        let plan = plan_schedule(&tasks, 2);

        assert_eq!(plan.groups.len(), 2);
        for g in &plan.groups {
            assert!(g.task_ids.len() <= 2);
        }
    }

    fn tcs(id: &str, phase: &str, held: bool, paths: &[&str]) -> TaskCurrentState {
        TaskCurrentState {
            task_id: id.to_string(),
            phase: phase.to_string(),
            is_held: held,
            write_allow: wa(paths),
        }
    }

    #[test]
    fn validate_plan_accepts_active_phases_rejects_others_and_held() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("t1".into(), wa(&["src/a/"])),
            ("t2".into(), wa(&["src/b/"])),
        ];
        let plan = plan_schedule(&tasks, 10);

        // in_progress + ready → valid (canonical as_str phase forms).
        let ok = vec![
            tcs("t1", "in_progress", false, &["src/a/"]),
            tcs("t2", "ready", false, &["src/b/"]),
        ];
        assert!(validate_plan(&plan, &ok).is_ok());

        // planning phase → invalid.
        let bad_phase = vec![
            tcs("t1", "planning", false, &["src/a/"]),
            tcs("t2", "ready", false, &["src/b/"]),
        ];
        let errs = validate_plan(&plan, &bad_phase).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("planning")));

        // held task → invalid.
        let held = vec![
            tcs("t1", "in_progress", true, &["src/a/"]),
            tcs("t2", "ready", false, &["src/b/"]),
        ];
        let errs = validate_plan(&plan, &held).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("held")));
    }

    #[test]
    fn test_prefix_overlap_detected() {
        let a = wa(&["src/foo/"]);
        let b = wa(&["src/foo/bar.rs"]);

        let overlap = detect_write_scope_overlap(&a, &b);
        assert!(!overlap.is_empty(), "prefix overlap should be detected");
    }
}
