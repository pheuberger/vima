use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::ticket::{Status, Ticket};

#[derive(Serialize, Debug)]
pub struct TreeNode {
    pub id: String,
    pub status: Status,
    pub title: String,
    pub deps: Vec<TreeNode>,
}

/// Build a dependency tree rooted at `root_id`.
/// In dedup mode (full=false): each node appears once at its deepest position.
/// In full mode (full=true): nodes may repeat, but cycles are still marked.
pub fn build_dep_tree(tickets: &[Ticket], root_id: &str, full: bool) -> Result<TreeNode> {
    let ticket_map: HashMap<&str, &Ticket> =
        tickets.iter().map(|t| (t.id.as_str(), t)).collect();

    if !ticket_map.contains_key(root_id) {
        return Err(Error::NotFound(root_id.to_string()));
    }

    let dep_map: HashMap<&str, &[String]> = tickets
        .iter()
        .map(|t| (t.id.as_str(), t.deps.as_slice()))
        .collect();

    if full {
        Ok(build_full_subtree(&ticket_map, &dep_map, root_id, &mut HashSet::new()))
    } else {
        let mut max_depth: HashMap<String, usize> = HashMap::new();
        dfs_max_depth(&dep_map, root_id, 0, &mut HashSet::new(), &mut max_depth);
        Ok(build_dedup_subtree(
            &ticket_map,
            &dep_map,
            root_id,
            0,
            &mut HashSet::new(),
            &max_depth,
            &mut HashSet::new(),
        ))
    }
}

/// Compute the maximum depth at which each reachable node appears (iterative DFS).
fn dfs_max_depth(
    dep_map: &HashMap<&str, &[String]>,
    id: &str,
    depth: usize,
    path: &mut HashSet<String>,
    max_depth: &mut HashMap<String, usize>,
) {
    if path.contains(id) {
        return; // cycle — stop recursion
    }
    let entry = max_depth.entry(id.to_string()).or_insert(0);
    if depth > *entry {
        *entry = depth;
    }
    path.insert(id.to_string());
    if let Some(deps) = dep_map.get(id) {
        for dep in *deps {
            dfs_max_depth(dep_map, dep, depth + 1, path, max_depth);
        }
    }
    path.remove(id);
}

/// Compute the height (depth of deepest descendant) of the subtree rooted at `id`.
fn subtree_height(
    dep_map: &HashMap<&str, &[String]>,
    id: &str,
    path: &mut HashSet<String>,
) -> usize {
    if path.contains(id) {
        return 0; // cycle — treat as leaf
    }
    let deps = match dep_map.get(id) {
        Some(d) => *d,
        None => return 0,
    };
    if deps.is_empty() {
        return 0;
    }
    path.insert(id.to_string());
    let h = deps
        .iter()
        .map(|d| subtree_height(dep_map, d.as_str(), path))
        .max()
        .unwrap_or(0);
    path.remove(id);
    h + 1
}

/// Build full tree (no dedup). Cycles are marked with `[cycle]` suffix.
fn build_full_subtree(
    ticket_map: &HashMap<&str, &Ticket>,
    dep_map: &HashMap<&str, &[String]>,
    id: &str,
    path: &mut HashSet<String>,
) -> TreeNode {
    if path.contains(id) {
        return cycle_node(ticket_map, id);
    }

    let (status, title, dep_ids) = match ticket_map.get(id) {
        Some(t) => (t.status.clone(), t.title.clone(), t.deps.clone()),
        None => return missing_node(id),
    };

    // Compute sort keys (subtree heights) for children using fresh paths
    let mut dep_order: Vec<(String, usize)> = dep_ids
        .iter()
        .map(|d| (d.clone(), subtree_height(dep_map, d, &mut HashSet::new())))
        .collect();
    dep_order.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

    path.insert(id.to_string());
    let deps: Vec<TreeNode> = dep_order
        .iter()
        .map(|(dep_id, _)| build_full_subtree(ticket_map, dep_map, dep_id, path))
        .collect();
    path.remove(id);

    TreeNode {
        id: id.to_string(),
        status,
        title,
        deps,
    }
}

/// Build deduplicated tree. Each node appears once, at its deepest position.
fn build_dedup_subtree(
    ticket_map: &HashMap<&str, &Ticket>,
    dep_map: &HashMap<&str, &[String]>,
    id: &str,
    depth: usize,
    path: &mut HashSet<String>,
    max_depth: &HashMap<String, usize>,
    claimed: &mut HashSet<String>,
) -> TreeNode {
    if path.contains(id) {
        return cycle_node(ticket_map, id);
    }

    let (status, title, dep_ids) = match ticket_map.get(id) {
        Some(t) => (t.status.clone(), t.title.clone(), t.deps.clone()),
        None => return missing_node(id),
    };

    // Include a child Y only if it sits at its maximum depth (== depth+1) and isn't claimed yet.
    let target_child_depth = depth + 1;
    let mut dep_order: Vec<(String, usize)> = dep_ids
        .iter()
        .filter(|dep_id| {
            let dep_max = max_depth
                .get(dep_id.as_str())
                .copied()
                .unwrap_or(target_child_depth);
            dep_max == target_child_depth && !claimed.contains(dep_id.as_str())
        })
        .map(|dep_id| {
            let h = subtree_height(dep_map, dep_id, &mut HashSet::new());
            (dep_id.clone(), h)
        })
        .collect();

    dep_order.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

    // Claim all selected children before recursing so sibling subtrees can't claim them.
    for (dep_id, _) in &dep_order {
        claimed.insert(dep_id.clone());
    }

    path.insert(id.to_string());
    let deps: Vec<TreeNode> = dep_order
        .iter()
        .map(|(dep_id, _)| {
            build_dedup_subtree(ticket_map, dep_map, dep_id, target_child_depth, path, max_depth, claimed)
        })
        .collect();
    path.remove(id);

    TreeNode {
        id: id.to_string(),
        status,
        title,
        deps,
    }
}

/// Return (status, title) for a ticket; placeholder values if ticket is missing.
fn ticket_status_title(ticket_map: &HashMap<&str, &Ticket>, id: &str) -> (Status, String) {
    match ticket_map.get(id) {
        Some(t) => (t.status.clone(), t.title.clone()),
        None => (Status::Open, "<missing>".to_string()),
    }
}

fn cycle_node(ticket_map: &HashMap<&str, &Ticket>, id: &str) -> TreeNode {
    let (status, title) = ticket_status_title(ticket_map, id);
    TreeNode {
        id: id.to_string(),
        status,
        title: format!("{} [cycle]", title),
        deps: vec![],
    }
}

fn missing_node(id: &str) -> TreeNode {
    TreeNode {
        id: id.to_string(),
        status: Status::Open,
        title: "<missing> [missing]".to_string(),
        deps: vec![],
    }
}

/// Check whether adding a dependency from `from` to `to` would create a cycle.
/// Returns `Some(cycle_path)` if a cycle would be created, `None` otherwise.
/// `cycle_path` is a sequence like [from, to, ..., from] showing the cycle.
pub fn would_create_cycle(tickets: &[Ticket], from: &str, to: &str) -> Option<Vec<String>> {
    // Build lookup: ticket id -> its deps
    let dep_map: HashMap<&str, &[String]> = tickets
        .iter()
        .map(|t| (t.id.as_str(), t.deps.as_slice()))
        .collect();

    // parent[node] = the node that first discovered it in DFS
    // Seed: `to` was "discovered" from `from` (the proposed new dep)
    let mut parent: HashMap<String, String> = HashMap::new();
    parent.insert(to.to_string(), from.to_string());

    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack: Vec<String> = vec![to.to_string()];

    while let Some(current) = stack.pop() {
        if current == from {
            // Cycle found. Reconstruct forward path [from, to, ..., from].
            // Trace parent backwards: from → parent[from] → ... stop when parent[x] == from.
            // Collect the middle nodes, then reverse to get forward order.
            let mut middle: Vec<String> = Vec::new();
            let mut node = from.to_string();
            loop {
                let p = match parent.get(&node) {
                    Some(p) => p.clone(),
                    None => break,
                };
                if p == from {
                    break;
                }
                middle.push(p.clone());
                node = p;
            }
            // middle is backward: [parent[from], ..., to]
            // reversed: [to, ..., parent[from]]
            middle.reverse();
            let mut path = vec![from.to_string()];
            path.extend(middle);
            path.push(from.to_string());
            return Some(path);
        }

        if !visited.insert(current.clone()) {
            continue;
        }

        if let Some(deps) = dep_map.get(current.as_str()) {
            for dep in deps.iter() {
                parent.entry(dep.clone()).or_insert_with(|| current.clone());
                stack.push(dep.clone());
            }
        }
    }

    None
}

/// Detect all cycles across open tickets using 3-color DFS.
/// Returns a deduplicated list of cycles, each normalized so the lexicographically
/// smallest ID appears first.
pub fn detect_all_cycles(tickets: &[Ticket]) -> Vec<Vec<String>> {
    let open_ids: HashSet<&str> = tickets
        .iter()
        .filter(|t| t.status != Status::Closed)
        .map(|t| t.id.as_str())
        .collect();

    // Build adjacency list (only edges to other open tickets)
    let dep_map: HashMap<&str, Vec<&str>> = tickets
        .iter()
        .filter(|t| t.status != Status::Closed)
        .map(|t| {
            let deps: Vec<&str> = t
                .deps
                .iter()
                .map(|d| d.as_str())
                .filter(|d| open_ids.contains(d))
                .collect();
            (t.id.as_str(), deps)
        })
        .collect();

    // 3-color DFS: 0=white(unvisited), 1=gray(in-progress), 2=black(done)
    let mut color: HashMap<&str, u8> = HashMap::new();
    let mut raw_cycles: Vec<Vec<String>> = Vec::new();

    for &id in &open_ids {
        if color.get(id).copied().unwrap_or(0) == 0 {
            let mut path: Vec<&str> = Vec::new();
            dfs_collect_cycles(id, &dep_map, &mut color, &mut path, &mut raw_cycles);
        }
    }

    // Normalize and deduplicate
    let mut normalized: Vec<Vec<String>> = raw_cycles
        .into_iter()
        .map(normalize_cycle)
        .collect();

    normalized.sort();
    normalized.dedup();
    normalized
}

fn dfs_collect_cycles<'a>(
    id: &'a str,
    dep_map: &HashMap<&'a str, Vec<&'a str>>,
    color: &mut HashMap<&'a str, u8>,
    path: &mut Vec<&'a str>,
    cycles: &mut Vec<Vec<String>>,
) {
    color.insert(id, 1); // gray
    path.push(id);

    if let Some(deps) = dep_map.get(id) {
        for &dep in deps {
            match color.get(dep).copied().unwrap_or(0) {
                1 => {
                    // Back edge — dep is gray (in current path): cycle found
                    if let Some(start_pos) = path.iter().position(|&x| x == dep) {
                        let cycle: Vec<String> =
                            path[start_pos..].iter().map(|s| s.to_string()).collect();
                        cycles.push(cycle);
                    }
                }
                0 => {
                    // Unvisited — recurse
                    dfs_collect_cycles(dep, dep_map, color, path, cycles);
                }
                _ => {} // black — already fully processed, no cycle via this edge
            }
        }
    }

    path.pop();
    color.insert(id, 2); // black
}

/// Rotate cycle so the lexicographically smallest ID is first.
fn normalize_cycle(cycle: Vec<String>) -> Vec<String> {
    let min_pos = cycle
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| id.as_str())
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut normalized = Vec::with_capacity(cycle.len());
    normalized.extend_from_slice(&cycle[min_pos..]);
    normalized.extend_from_slice(&cycle[..min_pos]);
    normalized
}

pub fn compute_reverse_fields(tickets: &mut [Ticket]) {
    let mut blocks_map: HashMap<String, Vec<String>> = HashMap::new();
    let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

    for ticket in tickets.iter() {
        for dep in &ticket.deps {
            blocks_map
                .entry(dep.clone())
                .or_default()
                .push(ticket.id.clone());
        }
        if let Some(parent) = &ticket.parent {
            children_map
                .entry(parent.clone())
                .or_default()
                .push(ticket.id.clone());
        }
    }

    for ticket in tickets.iter_mut() {
        ticket.blocks = blocks_map.remove(&ticket.id).unwrap_or_default();
        ticket.children = children_map.remove(&ticket.id).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::{Status, Ticket, TicketType};

    fn make_ticket(id: &str, deps: Vec<&str>, parent: Option<&str>) -> Ticket {
        Ticket {
            id: id.to_string(),
            title: format!("Ticket {id}"),
            status: Status::Open,
            ticket_type: TicketType::Task,
            priority: 2,
            tags: vec![],
            assignee: None,
            estimate: None,
            deps: deps.into_iter().map(|s| s.to_string()).collect(),
            links: vec![],
            parent: parent.map(|s| s.to_string()),
            created: "2026-04-02T00:00:00Z".to_string(),
            description: None,
            design: None,
            acceptance: None,
            notes: vec![],
            body: None,
            blocks: vec![],
            children: vec![],
        }
    }

    #[test]
    fn blocks_chain_a_deps_b_deps_c() {
        let mut tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
            make_ticket("C", vec![], None),
        ];
        compute_reverse_fields(&mut tickets);

        let a = tickets.iter().find(|t| t.id == "A").unwrap();
        let b = tickets.iter().find(|t| t.id == "B").unwrap();
        let c = tickets.iter().find(|t| t.id == "C").unwrap();

        assert!(a.blocks.is_empty());
        assert_eq!(b.blocks, vec!["A"]);
        assert_eq!(c.blocks, vec!["B"]);
    }

    #[test]
    fn children_populated_from_parent() {
        let mut tickets = vec![
            make_ticket("parent", vec![], None),
            make_ticket("child", vec![], Some("parent")),
        ];
        compute_reverse_fields(&mut tickets);

        let parent = tickets.iter().find(|t| t.id == "parent").unwrap();
        let child = tickets.iter().find(|t| t.id == "child").unwrap();

        assert_eq!(parent.children, vec!["child"]);
        assert!(child.children.is_empty());
    }

    #[test]
    fn would_create_cycle_detects_cycle_in_chain() {
        // A->B->C already exists; adding C->A would create a cycle C->A->B->C
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
            make_ticket("C", vec![], None),
        ];
        let result = would_create_cycle(&tickets, "C", "A");
        assert!(result.is_some(), "expected cycle to be detected");
        let path = result.unwrap();
        // Path should start and end with "C"
        assert_eq!(path.first().unwrap(), "C");
        assert_eq!(path.last().unwrap(), "C");
        // Path should contain "A" and "B"
        assert!(path.contains(&"A".to_string()));
        assert!(path.contains(&"B".to_string()));
    }

    #[test]
    fn would_create_cycle_no_cycle_for_acyclic_graph() {
        // A->B->C; adding D->A is fine
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
            make_ticket("C", vec![], None),
            make_ticket("D", vec![], None),
        ];
        let result = would_create_cycle(&tickets, "D", "A");
        assert!(result.is_none(), "expected no cycle");
    }

    #[test]
    fn would_create_cycle_self_dep_is_cycle() {
        // Adding A->A would create a self-cycle
        let tickets = vec![make_ticket("A", vec![], None)];
        let result = would_create_cycle(&tickets, "A", "A");
        assert!(result.is_some(), "expected self-cycle to be detected");
    }

    #[test]
    fn empty_deps_and_parent_gives_empty_blocks_children() {
        let mut tickets = vec![
            make_ticket("X", vec![], None),
            make_ticket("Y", vec![], None),
        ];
        compute_reverse_fields(&mut tickets);

        for t in &tickets {
            assert!(t.blocks.is_empty());
            assert!(t.children.is_empty());
        }
    }

    // ── build_dep_tree unit tests ────────────────────────────────────────────

    #[test]
    fn tree_children_sorted_by_subtree_depth_asc_then_id() {
        // A depends on B (leaf) and C (which depends on D, E — subtree height 1)
        // and Z (which depends on M which depends on N — subtree height 2)
        // Expected sort order: B (height 0) < C (height 1) < Z (height 2)
        let tickets = vec![
            make_ticket("A", vec!["Z", "B", "C"], None),
            make_ticket("B", vec![], None),
            make_ticket("C", vec!["D"], None),
            make_ticket("D", vec![], None),
            make_ticket("Z", vec!["M"], None),
            make_ticket("M", vec!["N"], None),
            make_ticket("N", vec![], None),
        ];
        let tree = build_dep_tree(&tickets, "A", true).unwrap();
        let child_ids: Vec<&str> = tree.deps.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(child_ids, vec!["B", "C", "Z"]);
    }

    #[test]
    fn tree_children_sort_by_id_when_same_height() {
        // A depends on C and B, both leaves (height 0). B < C alphabetically.
        let tickets = vec![
            make_ticket("A", vec!["C", "B"], None),
            make_ticket("B", vec![], None),
            make_ticket("C", vec![], None),
        ];
        let tree = build_dep_tree(&tickets, "A", true).unwrap();
        let child_ids: Vec<&str> = tree.deps.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(child_ids, vec!["B", "C"]);
    }

    #[test]
    fn tree_root_not_found_returns_error() {
        let tickets = vec![make_ticket("A", vec![], None)];
        let result = build_dep_tree(&tickets, "nonexistent", false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "not_found");
    }

    #[test]
    fn tree_full_mode_shows_all_occurrences() {
        // Diamond: A→B, A→C, B→D, C→D — full mode should show D twice
        let tickets = vec![
            make_ticket("A", vec!["B", "C"], None),
            make_ticket("B", vec!["D"], None),
            make_ticket("C", vec!["D"], None),
            make_ticket("D", vec![], None),
        ];
        let tree = build_dep_tree(&tickets, "A", true).unwrap();
        fn count_id(node: &TreeNode, id: &str) -> usize {
            let n = if node.id == id { 1 } else { 0 };
            n + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }
        assert_eq!(count_id(&tree, "D"), 2);
    }

    #[test]
    fn tree_dedup_mode_shows_each_node_once() {
        // Diamond: D should appear once
        let tickets = vec![
            make_ticket("A", vec!["B", "C"], None),
            make_ticket("B", vec!["D"], None),
            make_ticket("C", vec!["D"], None),
            make_ticket("D", vec![], None),
        ];
        let tree = build_dep_tree(&tickets, "A", false).unwrap();
        fn count_id(node: &TreeNode, id: &str) -> usize {
            let n = if node.id == id { 1 } else { 0 };
            n + node.deps.iter().map(|c| count_id(c, id)).sum::<usize>()
        }
        assert_eq!(count_id(&tree, "D"), 1);
    }

    #[test]
    fn tree_cycle_in_data_adds_cycle_marker_no_infinite_loop() {
        // Manually create a cycle: A→B→A (bypass would_create_cycle)
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["A"], None),
        ];
        let tree = build_dep_tree(&tickets, "A", true).unwrap();
        fn has_cycle_marker(node: &TreeNode) -> bool {
            node.title.contains("[cycle]") || node.deps.iter().any(has_cycle_marker)
        }
        assert!(has_cycle_marker(&tree));
    }

    #[test]
    fn tree_missing_dep_adds_missing_marker() {
        let tickets = vec![make_ticket("A", vec!["ghost"], None)];
        let tree = build_dep_tree(&tickets, "A", false).unwrap();
        assert_eq!(tree.deps.len(), 1);
        assert_eq!(tree.deps[0].id, "ghost");
        assert!(tree.deps[0].title.contains("[missing]"));
    }

    // ── detect_all_cycles unit tests ─────────────────────────────────────────

    fn make_closed_ticket(id: &str, deps: Vec<&str>) -> Ticket {
        let mut t = make_ticket(id, deps, None);
        t.status = Status::Closed;
        t
    }

    #[test]
    fn detect_all_cycles_no_cycles_returns_empty() {
        // A→B→C — acyclic
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
            make_ticket("C", vec![], None),
        ];
        let cycles = detect_all_cycles(&tickets);
        assert!(cycles.is_empty());
    }

    #[test]
    fn detect_all_cycles_three_node_cycle() {
        // A→B→C→A — one cycle
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
            make_ticket("C", vec!["A"], None),
        ];
        let cycles = detect_all_cycles(&tickets);
        assert_eq!(cycles.len(), 1);
        // Normalized: smallest ID first → [A, B, C]
        assert_eq!(cycles[0], vec!["A", "B", "C"]);
    }

    #[test]
    fn detect_all_cycles_skips_closed_tickets() {
        // A→B→A would be a cycle, but B is closed — no cycle visible
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_closed_ticket("B", vec!["A"]),
        ];
        let cycles = detect_all_cycles(&tickets);
        assert!(cycles.is_empty(), "closed ticket should break the cycle");
    }

    #[test]
    fn detect_all_cycles_normalizes_rotation() {
        // Cycle C→A→B→C — after normalization should be [A, B, C]
        // We set it up so the DFS starts at C, giving [C, A, B]
        let tickets = vec![
            make_ticket("C", vec!["A"], None),
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["C"], None),
        ];
        let cycles = detect_all_cycles(&tickets);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0], vec!["A", "B", "C"]);
    }

    #[test]
    fn detect_all_cycles_deduplicates() {
        // Two-node cycle A→B→A; DFS from both A and B would each find it
        let tickets = vec![
            make_ticket("A", vec!["B"], None),
            make_ticket("B", vec!["A"], None),
        ];
        let cycles = detect_all_cycles(&tickets);
        assert_eq!(cycles.len(), 1, "same cycle found from different starts should be deduped");
        assert_eq!(cycles[0], vec!["A", "B"]);
    }
}
