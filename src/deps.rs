use std::collections::HashMap;

use crate::ticket::Ticket;

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
