use std::collections::{BTreeSet, HashMap, VecDeque};

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
    /// M-d: declared prerequisite task IDs.
    pub depends_on: BTreeSet<String>,
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

/// Detect a dependency cycle among the in-plan edges (M-d).
///
/// `prereqs[t]` is the set of in-plan task IDs that `t` depends on. Returns the
/// task IDs that take part in (or are blocked behind) a cycle, sorted — empty
/// when the dependency subgraph is acyclic. Uses Kahn's algorithm: any node
/// never reaching in-degree zero is on or downstream of a cycle.
fn dependency_cycle(
    ids: &BTreeSet<String>,
    prereqs: &HashMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut indeg: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0usize)).collect();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for id in ids {
        for p in &prereqs[id] {
            *indeg.get_mut(id.as_str()).unwrap() += 1;
            dependents.entry(p.as_str()).or_default().push(id.as_str());
        }
    }
    let mut queue: VecDeque<&str> = indeg
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&n, _)| n)
        .collect();
    let mut settled = 0usize;
    while let Some(n) = queue.pop_front() {
        settled += 1;
        if let Some(ds) = dependents.get(n) {
            for &d in ds {
                let e = indeg.get_mut(d).unwrap();
                *e -= 1;
                if *e == 0 {
                    queue.push_back(d);
                }
            }
        }
    }
    if settled == ids.len() {
        return Vec::new();
    }
    let mut stuck: Vec<String> = indeg
        .iter()
        .filter(|(_, &d)| d > 0)
        .map(|(&n, _)| n.to_string())
        .collect();
    stuck.sort();
    stuck
}

/// Plan concurrent execution of tasks under two constraints (M-c overlap + M-d
/// dependencies):
///
/// - **Write isolation**: tasks with overlapping `write_allow` never share a
///   group; disjoint (and read-only, empty-scope) tasks may run in parallel.
/// - **Dependencies**: if `B` depends on `A` (both in the plan), `B` is placed
///   in a strictly later group than `A`.
///
/// `deps` maps a task ID to its declared prerequisites; edges to tasks outside
/// this plan are ignored (assumed already satisfied). Returns `Err` listing the
/// tasks involved if the in-plan dependency graph contains a cycle. With no
/// dependencies this degrades to greedy earliest-fit write-isolation grouping.
pub fn plan_schedule(
    tasks: &[(String, BTreeSet<String>)],
    deps: &HashMap<String, BTreeSet<String>>,
    max_concurrent: usize,
) -> Result<SchedulePlan, Vec<String>> {
    let max_concurrent = max_concurrent.max(1);
    let ids: BTreeSet<String> = tasks.iter().map(|(id, _)| id.clone()).collect();
    let scope_of: HashMap<&str, &BTreeSet<String>> =
        tasks.iter().map(|(id, wa)| (id.as_str(), wa)).collect();

    // In-plan prerequisite edges only (external deps assumed satisfied).
    let prereqs: HashMap<String, BTreeSet<String>> = ids
        .iter()
        .map(|id| {
            let p = deps
                .get(id)
                .map(|d| d.iter().filter(|x| ids.contains(*x)).cloned().collect())
                .unwrap_or_default();
            (id.clone(), p)
        })
        .collect();

    let cycle = dependency_cycle(&ids, &prereqs);
    if !cycle.is_empty() {
        return Err(vec![format!(
            "dependency cycle among tasks: {}",
            cycle.join(", ")
        )]);
    }

    // Dependency levels (longest path) order tasks so prereqs are placed first.
    let mut level: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0usize)).collect();
    {
        let mut indeg: HashMap<&str, usize> = ids.iter().map(|id| (id.as_str(), 0usize)).collect();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        for id in &ids {
            for p in &prereqs[id] {
                *indeg.get_mut(id.as_str()).unwrap() += 1;
                dependents.entry(p.as_str()).or_default().push(id.as_str());
            }
        }
        let mut queue: VecDeque<&str> = indeg
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&n, _)| n)
            .collect();
        while let Some(n) = queue.pop_front() {
            let ln = level[n];
            if let Some(ds) = dependents.get(n) {
                for &d in ds {
                    if ln + 1 > level[d] {
                        *level.get_mut(d).unwrap() = ln + 1;
                    }
                    let e = indeg.get_mut(d).unwrap();
                    *e -= 1;
                    if *e == 0 {
                        queue.push_back(d);
                    }
                }
            }
        }
    }

    // Report all write-overlap pairs (informational, like the previous planner).
    let mut conflicts: Vec<ScheduleConflict> = Vec::new();
    for i in 0..tasks.len() {
        for j in (i + 1)..tasks.len() {
            let overlap = detect_write_scope_overlap(&tasks[i].1, &tasks[j].1);
            if !overlap.is_empty() {
                conflicts.push(ScheduleConflict {
                    task_a: tasks[i].0.clone(),
                    task_b: tasks[j].0.clone(),
                    overlapping_paths: overlap,
                });
            }
        }
    }

    // Greedy earliest-fit assignment in (level, id) order. A task goes into the
    // earliest group that is (a) after all its prerequisites' groups, (b) free of
    // write-overlap with current members, and (c) below the concurrency cap.
    let mut order: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    order.sort_by(|a, b| level[a].cmp(&level[b]).then_with(|| a.cmp(b)));

    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut group_of: HashMap<&str, usize> = HashMap::new();
    for &t in &order {
        let min_g = prereqs[t]
            .iter()
            .map(|p| group_of[p.as_str()] + 1)
            .max()
            .unwrap_or(0);
        let scope_t = scope_of[t];
        let mut g = min_g;
        loop {
            if g >= groups.len() {
                groups.push(Vec::new());
            }
            let full = groups[g].len() >= max_concurrent;
            let overlaps = groups[g]
                .iter()
                .any(|&m| !detect_write_scope_overlap(scope_t, scope_of[m]).is_empty());
            if !full && !overlaps {
                groups[g].push(t);
                group_of.insert(t, g);
                break;
            }
            g += 1;
        }
    }

    let final_groups: Vec<ScheduleGroup> = groups
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_empty())
        .map(|(i, members)| ScheduleGroup {
            group_id: format!("g{}", i),
            task_ids: members.iter().map(|&s| s.to_string()).collect(),
        })
        .collect();

    Ok(SchedulePlan {
        plan_id: generate_uuid(),
        groups: final_groups,
        conflicts,
        max_concurrent,
        created_at: now_iso8601(),
    })
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

    // M-d: dependency order — every in-plan prerequisite must sit in an EARLIER
    // group than the task that depends on it. Deps to tasks outside the plan are
    // skipped (assumed already satisfied), matching plan_schedule.
    let group_of: std::collections::HashMap<&str, usize> = plan
        .groups
        .iter()
        .enumerate()
        .flat_map(|(gi, g)| g.task_ids.iter().map(move |t| (t.as_str(), gi)))
        .collect();
    for (gi, group) in plan.groups.iter().enumerate() {
        for task_id in &group.task_ids {
            if let Some(state) = state_map.get(task_id.as_str()) {
                for dep in &state.depends_on {
                    if let Some(&dg) = group_of.get(dep.as_str()) {
                        if dg >= gi {
                            errors.push(format!(
                                "task {} depends on {} but is not scheduled after it (group {} vs {})",
                                task_id, dep, gi, dg
                            ));
                        }
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
        let plan = plan_schedule(&tasks, &HashMap::new(), 10).unwrap();

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
        let plan = plan_schedule(&tasks, &HashMap::new(), 10).unwrap();

        // Overlapping write_allow → different groups.
        assert_eq!(plan.groups.len(), 2);
        assert!(!plan.conflicts.is_empty());
    }

    #[test]
    fn test_read_only_always_parallel() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("writer".into(), wa(&["src/a/"])),
            ("reader".into(), BTreeSet::new()),
        ];
        let plan = plan_schedule(&tasks, &HashMap::new(), 10).unwrap();

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
        let plan = plan_schedule(&tasks, &HashMap::new(), 2).unwrap();

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
            depends_on: BTreeSet::new(),
        }
    }

    #[test]
    fn validate_plan_accepts_active_phases_rejects_others_and_held() {
        let tasks: Vec<(String, BTreeSet<String>)> = vec![
            ("t1".into(), wa(&["src/a/"])),
            ("t2".into(), wa(&["src/b/"])),
        ];
        let plan = plan_schedule(&tasks, &HashMap::new(), 10).unwrap();

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
    fn deps_place_dependent_in_later_group() {
        // Disjoint scopes would share group 0, but B depends on A → B goes later.
        let tasks: Vec<(String, BTreeSet<String>)> =
            vec![("a".into(), wa(&["src/a/"])), ("b".into(), wa(&["src/b/"]))];
        let mut deps = HashMap::new();
        deps.insert("b".to_string(), wa(&["a"]));
        let plan = plan_schedule(&tasks, &deps, 10).unwrap();
        let group_of = |t: &str| {
            plan.groups
                .iter()
                .position(|g| g.task_ids.iter().any(|x| x == t))
                .unwrap()
        };
        assert!(
            group_of("b") > group_of("a"),
            "dependent 'b' must be scheduled in a later group than 'a'"
        );
    }

    #[test]
    fn dependency_cycle_is_rejected() {
        let tasks: Vec<(String, BTreeSet<String>)> =
            vec![("a".into(), wa(&["src/a/"])), ("b".into(), wa(&["src/b/"]))];
        let mut deps = HashMap::new();
        deps.insert("a".to_string(), wa(&["b"]));
        deps.insert("b".to_string(), wa(&["a"]));
        let err = plan_schedule(&tasks, &deps, 10).unwrap_err();
        assert!(err.iter().any(|e| e.contains("cycle")), "got: {:?}", err);
    }

    #[test]
    fn external_dependency_is_ignored_in_planning() {
        // 'b' depends on 'ext' which is not in the plan → no ordering / no cycle.
        let tasks: Vec<(String, BTreeSet<String>)> =
            vec![("a".into(), wa(&["src/a/"])), ("b".into(), wa(&["src/b/"]))];
        let mut deps = HashMap::new();
        deps.insert("b".to_string(), wa(&["ext"]));
        let plan = plan_schedule(&tasks, &deps, 10).unwrap();
        assert_eq!(
            plan.groups.len(),
            1,
            "disjoint + no in-plan deps → one group"
        );
    }

    #[test]
    fn validate_plan_flags_dependency_order_violation() {
        // Plan puts a and b in the SAME group, but b depends on a.
        let plan = SchedulePlan {
            plan_id: "p".into(),
            groups: vec![ScheduleGroup {
                group_id: "g0".into(),
                task_ids: vec!["a".into(), "b".into()],
            }],
            conflicts: vec![],
            max_concurrent: 10,
            created_at: "t".into(),
        };
        let states = vec![
            tcs("a", "ready", false, &["src/a/"]),
            TaskCurrentState {
                task_id: "b".into(),
                phase: "ready".into(),
                is_held: false,
                write_allow: wa(&["src/b/"]),
                depends_on: wa(&["a"]),
            },
        ];
        let errs = validate_plan(&plan, &states).unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("depends on a")),
            "got: {:?}",
            errs
        );
    }

    #[test]
    fn test_prefix_overlap_detected() {
        let a = wa(&["src/foo/"]);
        let b = wa(&["src/foo/bar.rs"]);

        let overlap = detect_write_scope_overlap(&a, &b);
        assert!(!overlap.is_empty(), "prefix overlap should be detected");
    }
}
