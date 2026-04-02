use std::fmt;
use crate::cli::FilterArgs;
use crate::ticket::{Status, Ticket, TicketType};

const MAX_PRIORITY: u8 = 4;

#[derive(Debug)]
pub enum FilterError {
    InvalidField(String),
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterError::InvalidField(msg) => write!(f, "invalid field: {}", msg),
        }
    }
}

impl std::error::Error for FilterError {}

pub struct Filter {
    pub status: Option<Status>,
    pub tags: Vec<String>,
    pub ticket_type: Option<TicketType>,
    pub priority_range: Option<(u8, u8)>,
    pub assignee: Option<String>,
    pub limit: Option<usize>,
}

impl Filter {
    pub fn matches(&self, ticket: &Ticket) -> bool {
        if let Some(ref s) = self.status {
            if &ticket.status != s {
                return false;
            }
        }
        if !self.tags.is_empty() && !self.tags.iter().any(|t| ticket.tags.contains(t)) {
            return false;
        }
        if let Some(ref tt) = self.ticket_type {
            if &ticket.ticket_type != tt {
                return false;
            }
        }
        if let Some((lo, hi)) = self.priority_range {
            if ticket.priority < lo || ticket.priority > hi {
                return false;
            }
        }
        if let Some(ref a) = self.assignee {
            if ticket.assignee.as_deref() != Some(a.as_str()) {
                return false;
            }
        }
        true
    }

    pub fn from_args(args: &FilterArgs) -> Result<Filter, FilterError> {
        let priority_range = args.priority.as_deref().map(parse_priority_range).transpose()?;
        Ok(Filter {
            status: args.status.clone(),
            tags: args.tag.clone(),
            ticket_type: args.ticket_type.clone(),
            priority_range,
            assignee: args.assignee.clone(),
            limit: args.limit,
        })
    }
}

pub fn parse_priority_range(s: &str) -> Result<(u8, u8), FilterError> {
    match s.split_once('-') {
        None => {
            let n: u8 = s
                .trim()
                .parse()
                .map_err(|_| FilterError::InvalidField(format!("invalid priority: {}", s)))?;
            if n > MAX_PRIORITY {
                return Err(FilterError::InvalidField(format!(
                    "priority {} out of range (max {})",
                    n, MAX_PRIORITY
                )));
            }
            Ok((n, n))
        }
        Some((lo_str, hi_str)) => {
            let lo: u8 = lo_str
                .trim()
                .parse()
                .map_err(|_| FilterError::InvalidField(format!("invalid priority range: {}", s)))?;
            let hi: u8 = hi_str
                .trim()
                .parse()
                .map_err(|_| FilterError::InvalidField(format!("invalid priority range: {}", s)))?;
            if lo > hi {
                return Err(FilterError::InvalidField(format!(
                    "priority range lo ({}) > hi ({})",
                    lo, hi
                )));
            }
            if hi > MAX_PRIORITY {
                return Err(FilterError::InvalidField(format!(
                    "priority hi {} out of range (max {})",
                    hi, MAX_PRIORITY
                )));
            }
            Ok((lo, hi))
        }
    }
}

pub fn apply_filters(tickets: Vec<Ticket>, filter: &Filter) -> Vec<Ticket> {
    let mut result = tickets.into_iter().filter(|t| filter.matches(t)).collect::<Vec<_>>();
    result.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
    if let Some(limit) = filter.limit {
        result.truncate(limit);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::{Status, Ticket, TicketType};

    fn make_ticket(id: &str, status: Status, priority: u8) -> Ticket {
        Ticket {
            id: id.to_string(),
            title: format!("Ticket {}", id),
            status,
            ticket_type: TicketType::Task,
            priority,
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
    fn matches_status_filter() {
        let filter = Filter {
            status: Some(Status::Open),
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };
        let open = make_ticket("a", Status::Open, 2);
        let closed = make_ticket("b", Status::Closed, 2);
        assert!(filter.matches(&open));
        assert!(!filter.matches(&closed));
    }

    #[test]
    fn matches_tags_or_semantics() {
        let filter = Filter {
            status: None,
            tags: vec!["foo".to_string(), "bar".to_string()],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };
        let mut t1 = make_ticket("a", Status::Open, 1);
        t1.tags = vec!["bar".to_string()];
        let mut t2 = make_ticket("b", Status::Open, 1);
        t2.tags = vec!["baz".to_string()];
        assert!(filter.matches(&t1));
        assert!(!filter.matches(&t2));
    }

    #[test]
    fn matches_priority_range_0_to_2() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: Some((0, 2)),
            assignee: None,
            limit: None,
        };
        assert!(filter.matches(&make_ticket("a", Status::Open, 0)));
        assert!(filter.matches(&make_ticket("b", Status::Open, 1)));
        assert!(filter.matches(&make_ticket("c", Status::Open, 2)));
        assert!(!filter.matches(&make_ticket("d", Status::Open, 3)));
        assert!(!filter.matches(&make_ticket("e", Status::Open, 4)));
    }

    #[test]
    fn parse_priority_range_dash() {
        assert_eq!(parse_priority_range("0-2").unwrap(), (0, 2));
    }

    #[test]
    fn parse_priority_range_single() {
        assert_eq!(parse_priority_range("3").unwrap(), (3, 3));
    }

    #[test]
    fn parse_priority_range_lo_gt_hi_error() {
        assert!(parse_priority_range("3-1").is_err());
    }

    #[test]
    fn parse_priority_range_hi_out_of_range_error() {
        assert!(parse_priority_range("0-5").is_err());
    }

    #[test]
    fn matches_combined_filters_and_semantics() {
        let filter = Filter {
            status: Some(Status::Open),
            tags: vec!["urgent".to_string()],
            ticket_type: Some(TicketType::Bug),
            priority_range: Some((0, 2)),
            assignee: Some("alice".to_string()),
            limit: None,
        };
        let mut t = make_ticket("a", Status::Open, 1);
        t.ticket_type = TicketType::Bug;
        t.tags = vec!["urgent".to_string()];
        t.assignee = Some("alice".to_string());
        assert!(filter.matches(&t));

        // Wrong status
        let mut t2 = t.clone();
        t2.status = Status::Closed;
        assert!(!filter.matches(&t2));

        // Wrong assignee
        let mut t3 = t.clone();
        t3.assignee = Some("bob".to_string());
        assert!(!filter.matches(&t3));
    }

    #[test]
    fn apply_filters_sorts_by_priority_then_id() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };
        let tickets = vec![
            make_ticket("c", Status::Open, 2),
            make_ticket("a", Status::Open, 1),
            make_ticket("b", Status::Open, 1),
        ];
        let result = apply_filters(tickets, &filter);
        assert_eq!(result[0].id, "a");
        assert_eq!(result[1].id, "b");
        assert_eq!(result[2].id, "c");
    }

    #[test]
    fn apply_filters_respects_limit() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: Some(2),
        };
        let tickets = vec![
            make_ticket("a", Status::Open, 0),
            make_ticket("b", Status::Open, 1),
            make_ticket("c", Status::Open, 2),
        ];
        let result = apply_filters(tickets, &filter);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "a");
        assert_eq!(result[1].id, "b");
    }
}
