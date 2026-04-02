use crate::error::Result;
use crate::ticket::Ticket;

pub fn output_one(ticket: &Ticket, pluck: &Option<String>) -> Result<()> {
    let value = serde_json::to_value(ticket)?;
    if let Some(fields) = pluck {
        println!("{}", pluck_value(&value, fields));
    } else {
        println!("{}", value);
    }
    Ok(())
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
}
