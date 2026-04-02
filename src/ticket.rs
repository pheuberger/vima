use std::fmt;
use serde::{Deserialize, Serialize};
use clap::ValueEnum;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    InProgress,
    Closed,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Closed => "closed",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum TicketType {
    Bug,
    Feature,
    Task,
    Epic,
    Chore,
}

impl TicketType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TicketType::Bug => "bug",
            TicketType::Feature => "feature",
            TicketType::Task => "task",
            TicketType::Epic => "epic",
            TicketType::Chore => "chore",
        }
    }
}

impl fmt::Display for TicketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub timestamp: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    pub status: Status,
    #[serde(rename = "type")]
    pub ticket_type: TicketType,
    pub priority: u8,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<u32>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(skip)]
    pub body: Option<String>,
    #[serde(default, skip_deserializing)]
    pub blocks: Vec<String>,
    #[serde(default, skip_deserializing)]
    pub children: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_in_progress_as_str() {
        assert_eq!(Status::InProgress.as_str(), "in_progress");
    }

    #[test]
    fn ticket_type_bug_as_str() {
        assert_eq!(TicketType::Bug.as_str(), "bug");
    }

    #[test]
    fn serialize_ticket_field_names() {
        let ticket = Ticket {
            id: "abc".to_string(),
            title: "Test".to_string(),
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
            body: Some("markdown body".to_string()),
            blocks: vec!["x".to_string()],
            children: vec!["y".to_string()],
        };
        let json = serde_json::to_string(&ticket).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        // "type" appears, not "ticket_type"
        assert!(v.get("type").is_some());
        assert!(v.get("ticket_type").is_none());

        // body is absent (serde(skip))
        assert!(v.get("body").is_none());

        // blocks and children appear as empty arrays (skip_deserializing only affects deserialization)
        // but since blocks/children had values they appear; let's check with empty ones
        // Actually blocks=["x"], children=["y"] were set, they serialize
        assert_eq!(v["blocks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn deserialize_blocks_children_are_empty() {
        let json = r#"{
            "id": "abc",
            "title": "Test",
            "status": "open",
            "type": "task",
            "priority": 2,
            "created": "2026-04-02T00:00:00Z",
            "blocks": ["x"],
            "children": ["y"]
        }"#;
        let ticket: Ticket = serde_json::from_str(json).unwrap();
        // skip_deserializing means these are always Default (empty vec)
        assert!(ticket.blocks.is_empty());
        assert!(ticket.children.is_empty());
        // optional fields missing → None
        assert!(ticket.assignee.is_none());
        assert!(ticket.description.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let original = Ticket {
            id: "xyz".to_string(),
            title: "Round trip".to_string(),
            status: Status::InProgress,
            ticket_type: TicketType::Bug,
            priority: 1,
            tags: vec!["alpha".to_string()],
            assignee: Some("alice".to_string()),
            estimate: Some(60),
            deps: vec!["dep1".to_string()],
            links: vec!["link1".to_string()],
            parent: Some("parent1".to_string()),
            created: "2026-04-02T00:00:00Z".to_string(),
            description: Some("desc".to_string()),
            design: Some("design".to_string()),
            acceptance: Some("acceptance".to_string()),
            notes: vec![Note {
                timestamp: "2026-04-02T00:00:00Z".to_string(),
                text: "a note".to_string(),
            }],
            body: Some("body text".to_string()),
            blocks: vec!["b1".to_string()],
            children: vec!["c1".to_string()],
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Ticket = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, original.id);
        assert_eq!(deserialized.title, original.title);
        assert_eq!(deserialized.status, original.status);
        assert_eq!(deserialized.ticket_type, original.ticket_type);
        assert_eq!(deserialized.priority, original.priority);
        assert_eq!(deserialized.tags, original.tags);
        assert_eq!(deserialized.assignee, original.assignee);
        assert_eq!(deserialized.estimate, original.estimate);
        assert_eq!(deserialized.deps, original.deps);
        assert_eq!(deserialized.links, original.links);
        assert_eq!(deserialized.parent, original.parent);
        assert_eq!(deserialized.created, original.created);
        assert_eq!(deserialized.description, original.description);
        assert_eq!(deserialized.design, original.design);
        assert_eq!(deserialized.acceptance, original.acceptance);
        assert_eq!(deserialized.notes.len(), original.notes.len());
        // body is skipped (always None after deserialization)
        assert!(deserialized.body.is_none());
        // blocks/children are computed (skip_deserializing → empty)
        assert!(deserialized.blocks.is_empty());
        assert!(deserialized.children.is_empty());
    }
}
