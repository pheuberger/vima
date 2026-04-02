use std::collections::HashMap;

use crate::ticket::Ticket;

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
}
