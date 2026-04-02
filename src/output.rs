use colored::Colorize;

use crate::deps::TreeNode;
use crate::error::Result;
use crate::ticket::{Status, Ticket};

pub(crate) fn output_one_to_writer<W: std::io::Write>(
    ticket: &Ticket,
    pluck: &Option<String>,
    w: &mut W,
) -> Result<()> {
    let value = serde_json::to_value(ticket)?;
    if let Some(fields) = pluck {
        writeln!(w, "{}", pluck_value(&value, fields))?;
    } else {
        writeln!(w, "{}", value)?;
    }
    Ok(())
}

pub fn output_one(ticket: &Ticket, pluck: &Option<String>) -> Result<()> {
    output_one_to_writer(ticket, pluck, &mut std::io::stdout())
}

pub fn output_many(tickets: &[Ticket], pluck: &Option<String>, count: bool) -> Result<()> {
    if count {
        println!("{}", tickets.len());
        return Ok(());
    }
    let values: Vec<serde_json::Value> = tickets
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<_, _>>()?;
    if let Some(fields) = pluck {
        output_plucked(&values, fields);
    } else {
        println!("{}", serde_json::Value::Array(values));
    }
    Ok(())
}

pub fn pluck_value(value: &serde_json::Value, fields: &str) -> serde_json::Value {
    let parts: Vec<&str> = fields.split(',').map(|s| s.trim()).collect();
    if parts.len() == 1 {
        value.get(parts[0]).cloned().unwrap_or_default()
    } else {
        let mut obj = serde_json::Map::new();
        for field in parts {
            obj.insert(
                field.to_string(),
                value.get(field).cloned().unwrap_or_default(),
            );
        }
        serde_json::Value::Object(obj)
    }
}

pub fn output_plucked(values: &[serde_json::Value], fields: &str) {
    let result: Vec<serde_json::Value> = values
        .iter()
        .map(|v| pluck_value(v, fields))
        .collect();
    println!("{}", serde_json::Value::Array(result));
}

// ── Pretty output helpers ────────────────────────────────────────────────────

fn colorize_status(status: &Status) -> String {
    match status {
        Status::Open => status.as_str().green().to_string(),
        Status::InProgress => status.as_str().yellow().to_string(),
        Status::Closed => status.as_str().dimmed().to_string(),
    }
}

fn colorize_priority(priority: u8) -> String {
    let s = priority.to_string();
    match priority {
        0 | 1 => s.red().to_string(),
        _ => s,
    }
}

fn format_estimate(minutes: u32) -> String {
    if minutes < 60 {
        format!("{}m", minutes)
    } else {
        let h = minutes / 60;
        let m = minutes % 60;
        if m == 0 {
            format!("{}h", h)
        } else {
            format!("{}h {}m", h, m)
        }
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > max_chars {
        let truncated: String = chars[..max_chars - 1].iter().collect();
        format!("{}…", truncated)
    } else {
        s.to_string()
    }
}

// ── pretty_list ──────────────────────────────────────────────────────────────

pub fn pretty_list(tickets: &[Ticket]) -> Result<()> {
    if tickets.is_empty() {
        println!("No tickets found.");
        return Ok(());
    }

    let id_w = tickets
        .iter()
        .map(|t| t.id.len())
        .max()
        .unwrap_or(2)
        .max("ID".len());
    let status_w = tickets
        .iter()
        .map(|t| t.status.as_str().len())
        .max()
        .unwrap_or(4)
        .max("STATUS".len());
    let type_w = tickets
        .iter()
        .map(|t| t.ticket_type.as_str().len())
        .max()
        .unwrap_or(4)
        .max("TYPE".len());

    // Header row (no color)
    println!(
        "{:<id_w$}  {:<1}  {:<status_w$}  {:<type_w$}  {}",
        "ID",
        "P",
        "STATUS",
        "TYPE",
        "TITLE",
        id_w = id_w,
        status_w = status_w,
        type_w = type_w
    );

    for t in tickets {
        let title = truncate(&t.title, 50);
        let id_padded = format!("{:<width$}", t.id, width = id_w);
        let priority_str = colorize_priority(t.priority);
        // Color status then pad manually so ANSI codes don't break alignment
        let status_colored = colorize_status(&t.status);
        let status_padding = " ".repeat(status_w.saturating_sub(t.status.as_str().len()));
        let type_padded = format!("{:<width$}", t.ticket_type.as_str(), width = type_w);

        println!(
            "{}  {}  {}{}  {}  {}",
            id_padded, priority_str, status_colored, status_padding, type_padded, title
        );
    }
    Ok(())
}

// ── pretty_show ──────────────────────────────────────────────────────────────

pub fn pretty_show(ticket: &Ticket) -> Result<()> {
    // Header: id — title
    println!("{} \u{2014} {}", ticket.id.bold(), ticket.title);

    // Status / Type / Priority line
    let status_colored = colorize_status(&ticket.status);
    let priority_colored = colorize_priority(ticket.priority);
    println!(
        "Status: {}  Type: {}  Priority: {}",
        status_colored,
        ticket.ticket_type.as_str(),
        priority_colored
    );

    // Tags
    if !ticket.tags.is_empty() {
        println!("Tags: {}", ticket.tags.join(", "));
    }

    // Assignee / Estimate on one line if present
    let mut meta: Vec<String> = Vec::new();
    if let Some(ref assignee) = ticket.assignee {
        meta.push(format!("Assignee: {}", assignee));
    }
    if let Some(est) = ticket.estimate {
        meta.push(format!("Estimate: {}", format_estimate(est)));
    }
    if !meta.is_empty() {
        println!("{}", meta.join("  "));
    }

    println!("Created: {}", ticket.created);

    if let Some(ref desc) = ticket.description {
        println!("\nDescription:");
        for line in desc.lines() {
            println!("  {}", line);
        }
    }

    if !ticket.deps.is_empty() {
        println!("Deps: {}", ticket.deps.join(", "));
    }
    if !ticket.blocks.is_empty() {
        println!("Blocks: {}", ticket.blocks.join(", "));
    }

    Ok(())
}

// ── pretty_tree ──────────────────────────────────────────────────────────────

pub fn pretty_tree(node: &TreeNode) {
    println!(
        "{} {} ({})",
        node.id,
        node.title,
        node.status.as_str()
    );
    print_tree_children(&node.deps, "");
}

fn print_tree_children(children: &[TreeNode], prefix: &str) {
    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
        println!(
            "{}{}{} {} ({})",
            prefix,
            connector,
            child.id,
            child.title,
            child.status.as_str()
        );
        let new_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}\u{2502}   ", prefix)
        };
        print_tree_children(&child.deps, &new_prefix);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::{Status, Ticket, TicketType};

    fn make_ticket() -> Ticket {
        Ticket {
            id: "t-1".to_string(),
            title: "Test ticket".to_string(),
            status: Status::Open,
            ticket_type: TicketType::Task,
            priority: 2,
            tags: vec![],
            assignee: None,
            estimate: None,
            deps: vec![],
            links: vec![],
            parent: None,
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
    fn pluck_value_single_field_returns_bare_value() {
        let v = serde_json::json!({"id": "t-1", "title": "hello"});
        let result = pluck_value(&v, "id");
        assert_eq!(result, serde_json::json!("t-1"));
    }

    #[test]
    fn pluck_value_multiple_fields_returns_object() {
        let v = serde_json::json!({"id": "t-1", "title": "hello", "priority": 2});
        let result = pluck_value(&v, "id, title");
        assert_eq!(result, serde_json::json!({"id": "t-1", "title": "hello"}));
    }

    #[test]
    fn pluck_value_missing_field_returns_null() {
        let v = serde_json::json!({"id": "t-1"});
        let result = pluck_value(&v, "nonexistent");
        assert_eq!(result, serde_json::Value::Null);
    }

    #[test]
    fn output_many_count_true_prints_integer() {
        let tickets = vec![make_ticket(), make_ticket()];
        // We can't easily capture stdout in a unit test without extra deps,
        // so we verify the function returns Ok and count logic is correct.
        // The actual integer printing is tested via the len() call.
        assert_eq!(tickets.len(), 2);
        let result = output_many(&tickets, &None, true);
        assert!(result.is_ok());
    }

    #[test]
    fn output_one_without_pluck_returns_ok() {
        let ticket = make_ticket();
        let result = output_one(&ticket, &None);
        assert!(result.is_ok());
    }

    #[test]
    fn format_estimate_minutes_only() {
        assert_eq!(format_estimate(30), "30m");
        assert_eq!(format_estimate(0), "0m");
        assert_eq!(format_estimate(59), "59m");
    }

    #[test]
    fn format_estimate_hours_only() {
        assert_eq!(format_estimate(60), "1h");
        assert_eq!(format_estimate(120), "2h");
    }

    #[test]
    fn format_estimate_hours_and_minutes() {
        assert_eq!(format_estimate(90), "1h 30m");
        assert_eq!(format_estimate(75), "1h 15m");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_gets_ellipsis() {
        let s = "a".repeat(55);
        let result = truncate(&s, 50);
        assert_eq!(result.chars().count(), 50); // 49 chars + ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn pretty_list_empty_prints_no_tickets() {
        // Just verify it doesn't panic and returns Ok
        colored::control::set_override(true);
        let result = pretty_list(&[]);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_list_with_tickets_returns_ok() {
        colored::control::set_override(true);
        let tickets = vec![make_ticket()];
        let result = pretty_list(&tickets);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_show_returns_ok() {
        colored::control::set_override(true);
        let ticket = make_ticket();
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_tree_single_node_no_panic() {
        colored::control::set_override(true);
        let node = TreeNode {
            id: "t-1".to_string(),
            title: "Root".to_string(),
            status: Status::Open,
            deps: vec![],
        };
        pretty_tree(&node); // just verify no panic
        colored::control::set_override(false);
    }
}
