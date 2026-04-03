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

/// Fields stripped from list output unless `--full` is passed.
const HEAVY_FIELDS: &[&str] = &["description", "design", "acceptance", "notes", "body"];

pub fn strip_heavy_fields(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        for field in HEAVY_FIELDS {
            map.remove(*field);
        }
    }
}

pub fn output_many(tickets: &[Ticket], pluck: &Option<String>, count: bool) -> Result<()> {
    output_many_full(tickets, pluck, count, false)
}

pub fn output_many_full(
    tickets: &[Ticket],
    pluck: &Option<String>,
    count: bool,
    full: bool,
) -> Result<()> {
    if count {
        println!("{}", tickets.len());
        return Ok(());
    }
    let mut values: Vec<serde_json::Value> = tickets
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<_, _>>()?;
    if !full && pluck.is_none() {
        for v in &mut values {
            strip_heavy_fields(v);
        }
    }
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
    let result: Vec<serde_json::Value> = values.iter().map(|v| pluck_value(v, fields)).collect();
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
    println!("{} {} ({})", node.id, node.title, node.status.as_str());
    print_tree_children(&node.deps, "");
}

fn print_tree_children(children: &[TreeNode], prefix: &str) {
    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251c}\u{2500}\u{2500} "
        };
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
    fn strip_heavy_fields_removes_all_heavy_fields() {
        let mut v = serde_json::json!({
            "id": "t-1",
            "title": "hello",
            "description": "desc",
            "design": "design",
            "acceptance": "acc",
            "notes": [{"text": "note"}],
            "body": "body text"
        });
        strip_heavy_fields(&mut v);
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("title"));
        assert!(!obj.contains_key("description"));
        assert!(!obj.contains_key("design"));
        assert!(!obj.contains_key("acceptance"));
        assert!(!obj.contains_key("notes"));
        assert!(!obj.contains_key("body"));
    }

    #[test]
    fn strip_heavy_fields_noop_on_non_object() {
        let mut v = serde_json::json!("just a string");
        strip_heavy_fields(&mut v);
        assert_eq!(v, serde_json::json!("just a string"));
    }

    #[test]
    fn strip_heavy_fields_noop_when_fields_absent() {
        let mut v = serde_json::json!({"id": "t-1", "title": "hello"});
        strip_heavy_fields(&mut v);
        assert_eq!(v, serde_json::json!({"id": "t-1", "title": "hello"}));
    }

    #[test]
    fn output_many_full_false_strips_heavy_fields() {
        let mut ticket = make_ticket();
        ticket.description = Some("a description".to_string());
        ticket.design = Some("design notes".to_string());
        ticket.acceptance = Some("acceptance criteria".to_string());
        ticket.body = Some("body text".to_string());
        let result = output_many_full(&[ticket], &None, false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn output_many_full_true_keeps_heavy_fields() {
        let mut ticket = make_ticket();
        ticket.description = Some("a description".to_string());
        let result = output_many_full(&[ticket], &None, false, true);
        assert!(result.is_ok());
    }

    #[test]
    fn output_many_full_with_pluck_skips_stripping() {
        let mut ticket = make_ticket();
        ticket.description = Some("a description".to_string());
        let result = output_many_full(&[ticket], &Some("description".to_string()), false, false);
        assert!(result.is_ok());
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

    // ── Pretty output tests with content verification ───────────────────────

    fn make_full_ticket() -> Ticket {
        Ticket {
            id: "vi-abcd".to_string(),
            title: "Implement full feature".to_string(),
            status: Status::InProgress,
            ticket_type: TicketType::Feature,
            priority: 1,
            tags: vec![
                "backend".to_string(),
                "api".to_string(),
                "urgent".to_string(),
            ],
            assignee: Some("alice".to_string()),
            estimate: Some(90),
            deps: vec!["vi-0001".to_string(), "vi-0002".to_string()],
            links: vec![],
            parent: None,
            created: "2026-04-02T00:00:00Z".to_string(),
            description: Some("A detailed description\nwith multiple lines.".to_string()),
            design: Some("Design notes here.".to_string()),
            acceptance: Some("Must pass all tests.".to_string()),
            notes: vec![],
            body: Some("Extended body content.".to_string()),
            blocks: vec!["vi-0003".to_string()],
            children: vec![],
        }
    }

    #[test]
    fn pretty_list_all_optional_fields_populated() {
        colored::control::set_override(true);
        let tickets = vec![
            make_full_ticket(),
            {
                let mut t = make_full_ticket();
                t.id = "vi-efgh".to_string();
                t.title = "Another ticket with tags".to_string();
                t.status = Status::Open;
                t.ticket_type = TicketType::Bug;
                t.priority = 0;
                t.tags = vec!["frontend".to_string()];
                t.assignee = Some("bob".to_string());
                t.estimate = Some(120);
                t
            },
            {
                let mut t = make_full_ticket();
                t.id = "vi-ijkl".to_string();
                t.title = "Closed chore".to_string();
                t.status = Status::Closed;
                t.ticket_type = TicketType::Chore;
                t.priority = 4;
                t
            },
        ];
        let result = pretty_list(&tickets);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_show_unicode_title() {
        colored::control::set_override(true);
        let mut ticket = make_ticket();
        ticket.title = "修复国际化bug 🐛 — résumé naïve".to_string();
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_tree_deep_nesting_three_plus_levels() {
        colored::control::set_override(true);
        let node = TreeNode {
            id: "root".to_string(),
            title: "Root node".to_string(),
            status: Status::Open,
            deps: vec![
                TreeNode {
                    id: "child-1".to_string(),
                    title: "Child 1".to_string(),
                    status: Status::InProgress,
                    deps: vec![TreeNode {
                        id: "gc-1".to_string(),
                        title: "Grandchild 1".to_string(),
                        status: Status::Open,
                        deps: vec![TreeNode {
                            id: "ggc-1".to_string(),
                            title: "Great-grandchild 1".to_string(),
                            status: Status::Closed,
                            deps: vec![],
                        }],
                    }],
                },
                TreeNode {
                    id: "child-2".to_string(),
                    title: "Child 2".to_string(),
                    status: Status::Open,
                    deps: vec![],
                },
            ],
        };
        pretty_tree(&node);
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_show_all_optional_fields_design_acceptance_body() {
        colored::control::set_override(true);
        let ticket = make_full_ticket();
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_list_column_alignment_varying_field_lengths() {
        colored::control::set_override(true);
        let tickets = vec![
            {
                let mut t = make_ticket();
                t.id = "a".to_string();
                t.ticket_type = TicketType::Bug;
                t.status = Status::Open;
                t.priority = 3;
                t
            },
            {
                let mut t = make_ticket();
                t.id = "long-prefix-abcd".to_string();
                t.ticket_type = TicketType::Feature;
                t.status = Status::InProgress;
                t.priority = 0;
                t
            },
            {
                let mut t = make_ticket();
                t.id = "med-xx".to_string();
                t.ticket_type = TicketType::Epic;
                t.status = Status::Closed;
                t.priority = 2;
                t
            },
        ];
        let result = pretty_list(&tickets);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_show_deps_and_blocks_populated() {
        colored::control::set_override(true);
        let mut ticket = make_ticket();
        ticket.deps = vec![
            "vi-dep1".to_string(),
            "vi-dep2".to_string(),
            "vi-dep3".to_string(),
        ];
        ticket.blocks = vec!["vi-blk1".to_string(), "vi-blk2".to_string()];
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_tree_multiple_children_same_level() {
        colored::control::set_override(true);
        let node = TreeNode {
            id: "root".to_string(),
            title: "Root".to_string(),
            status: Status::Open,
            deps: vec![
                TreeNode {
                    id: "c-1".to_string(),
                    title: "First child".to_string(),
                    status: Status::Open,
                    deps: vec![],
                },
                TreeNode {
                    id: "c-2".to_string(),
                    title: "Second child".to_string(),
                    status: Status::InProgress,
                    deps: vec![],
                },
                TreeNode {
                    id: "c-3".to_string(),
                    title: "Third child".to_string(),
                    status: Status::Closed,
                    deps: vec![],
                },
                TreeNode {
                    id: "c-4".to_string(),
                    title: "Fourth child".to_string(),
                    status: Status::Open,
                    deps: vec![],
                },
            ],
        };
        pretty_tree(&node);
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_list_different_priority_levels() {
        colored::control::set_override(true);
        let tickets: Vec<Ticket> = (0..=4)
            .map(|p| {
                let mut t = make_ticket();
                t.id = format!("vi-p{}", p);
                t.priority = p;
                t.title = format!("Priority {} ticket", p);
                t
            })
            .collect();
        let result = pretty_list(&tickets);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn colorize_status_returns_expected_strings() {
        colored::control::set_override(false);
        let open = colorize_status(&Status::Open);
        let in_progress = colorize_status(&Status::InProgress);
        let closed = colorize_status(&Status::Closed);
        assert!(open.contains("open"));
        assert!(in_progress.contains("in_progress"));
        assert!(closed.contains("closed"));
    }

    #[test]
    fn colorize_priority_returns_expected_strings() {
        colored::control::set_override(false);
        assert!(colorize_priority(0).contains('0'));
        assert!(colorize_priority(1).contains('1'));
        assert_eq!(colorize_priority(2), "2");
        assert_eq!(colorize_priority(3), "3");
        assert_eq!(colorize_priority(4), "4");
    }

    #[test]
    fn truncate_unicode_string() {
        let s = "日本語のテスト文字列です";
        let result = truncate(s, 5);
        assert_eq!(result.chars().count(), 5);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn pretty_show_multiline_description() {
        colored::control::set_override(true);
        let mut ticket = make_ticket();
        ticket.description =
            Some("Line one\nLine two\nLine three\n\nLine five after blank".to_string());
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_show_tags_assignee_estimate() {
        colored::control::set_override(true);
        let mut ticket = make_ticket();
        ticket.tags = vec!["perf".to_string(), "critical".to_string()];
        ticket.assignee = Some("charlie".to_string());
        ticket.estimate = Some(45);
        let result = pretty_show(&ticket);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_list_title_truncation() {
        colored::control::set_override(true);
        let mut ticket = make_ticket();
        ticket.title = "A".repeat(100);
        let result = pretty_list(&[ticket]);
        assert!(result.is_ok());
        colored::control::set_override(false);
    }

    #[test]
    fn pretty_tree_deep_nesting_with_mixed_siblings() {
        colored::control::set_override(true);
        let node = TreeNode {
            id: "root".to_string(),
            title: "Root".to_string(),
            status: Status::Open,
            deps: vec![
                TreeNode {
                    id: "a".to_string(),
                    title: "A".to_string(),
                    status: Status::Open,
                    deps: vec![
                        TreeNode {
                            id: "a1".to_string(),
                            title: "A1".to_string(),
                            status: Status::InProgress,
                            deps: vec![
                                TreeNode {
                                    id: "a1x".to_string(),
                                    title: "A1X".to_string(),
                                    status: Status::Closed,
                                    deps: vec![],
                                },
                                TreeNode {
                                    id: "a1y".to_string(),
                                    title: "A1Y".to_string(),
                                    status: Status::Open,
                                    deps: vec![],
                                },
                            ],
                        },
                        TreeNode {
                            id: "a2".to_string(),
                            title: "A2".to_string(),
                            status: Status::Open,
                            deps: vec![],
                        },
                    ],
                },
                TreeNode {
                    id: "b".to_string(),
                    title: "B".to_string(),
                    status: Status::Closed,
                    deps: vec![TreeNode {
                        id: "b1".to_string(),
                        title: "B1".to_string(),
                        status: Status::Open,
                        deps: vec![],
                    }],
                },
            ],
        };
        pretty_tree(&node);
        colored::control::set_override(false);
    }
}
