use crate::cli::FilterArgs;
use crate::error::Error;
use crate::ticket::{Status, Ticket, TicketType};

pub const MAX_PRIORITY: u8 = 4;

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

    pub fn from_args(args: &FilterArgs) -> Result<Filter, Error> {
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

pub fn parse_priority_range(s: &str) -> Result<(u8, u8), Error> {
    match s.split_once('-') {
        None => {
            let n: u8 = s
                .trim()
                .parse()
                .map_err(|_| Error::InvalidField(format!("invalid priority: {}", s)))?;
            if n > MAX_PRIORITY {
                return Err(Error::InvalidField(format!(
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
                .map_err(|_| Error::InvalidField(format!("invalid priority range: {}", s)))?;
            let hi: u8 = hi_str
                .trim()
                .parse()
                .map_err(|_| Error::InvalidField(format!("invalid priority range: {}", s)))?;
            if lo > hi {
                return Err(Error::InvalidField(format!(
                    "priority range lo ({}) > hi ({})",
                    lo, hi
                )));
            }
            if hi > MAX_PRIORITY {
                return Err(Error::InvalidField(format!(
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

    #[test]
    fn all_criteria_applied_simultaneously() {
        let filter = Filter {
            status: Some(Status::InProgress),
            tags: vec!["backend".to_string(), "api".to_string()],
            ticket_type: Some(TicketType::Feature),
            priority_range: Some((1, 3)),
            assignee: Some("carol".to_string()),
            limit: None,
        };

        // Ticket that satisfies every criterion
        let mut pass = make_ticket("t1", Status::InProgress, 2);
        pass.ticket_type = TicketType::Feature;
        pass.tags = vec!["backend".to_string()];
        pass.assignee = Some("carol".to_string());
        assert!(filter.matches(&pass));

        // Fails status
        let mut fail_status = pass.clone();
        fail_status.status = Status::Open;
        assert!(!filter.matches(&fail_status));

        // Fails tags
        let mut fail_tags = pass.clone();
        fail_tags.tags = vec!["frontend".to_string()];
        assert!(!filter.matches(&fail_tags));

        // Fails ticket type
        let mut fail_type = pass.clone();
        fail_type.ticket_type = TicketType::Bug;
        assert!(!filter.matches(&fail_type));

        // Fails priority (too high)
        let mut fail_pri = pass.clone();
        fail_pri.priority = 4;
        assert!(!filter.matches(&fail_pri));

        // Fails assignee
        let mut fail_assignee = pass.clone();
        fail_assignee.assignee = Some("dave".to_string());
        assert!(!filter.matches(&fail_assignee));

        // No assignee at all
        let mut fail_no_assignee = pass.clone();
        fail_no_assignee.assignee = None;
        assert!(!filter.matches(&fail_no_assignee));
    }

    #[test]
    fn empty_filter_results() {
        let filter = Filter {
            status: Some(Status::Closed),
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };
        let tickets = vec![
            make_ticket("a", Status::Open, 0),
            make_ticket("b", Status::InProgress, 1),
        ];
        let result = apply_filters(tickets, &filter);
        assert!(result.is_empty());
    }

    #[test]
    fn priority_range_0_0_matches_only_zero() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: Some((0, 0)),
            assignee: None,
            limit: None,
        };
        assert!(filter.matches(&make_ticket("a", Status::Open, 0)));
        assert!(!filter.matches(&make_ticket("b", Status::Open, 1)));
        assert!(!filter.matches(&make_ticket("c", Status::Open, 4)));
    }

    #[test]
    fn priority_range_4_4_matches_only_four() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: Some((4, 4)),
            assignee: None,
            limit: None,
        };
        assert!(!filter.matches(&make_ticket("a", Status::Open, 0)));
        assert!(!filter.matches(&make_ticket("b", Status::Open, 3)));
        assert!(filter.matches(&make_ticket("c", Status::Open, 4)));
    }

    #[test]
    fn sort_stability_same_priority_ordered_by_id() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };
        // All same priority; input order is reverse-alphabetical
        let tickets = vec![
            make_ticket("zz", Status::Open, 2),
            make_ticket("mm", Status::Open, 2),
            make_ticket("aa", Status::Open, 2),
        ];
        let result = apply_filters(tickets, &filter);
        assert_eq!(result[0].id, "aa");
        assert_eq!(result[1].id, "mm");
        assert_eq!(result[2].id, "zz");
    }

    #[test]
    fn limit_larger_than_result_set() {
        let filter = Filter {
            status: None,
            tags: vec![],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: Some(100),
        };
        let tickets = vec![
            make_ticket("a", Status::Open, 0),
            make_ticket("b", Status::Open, 1),
        ];
        let result = apply_filters(tickets, &filter);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn multiple_tags_or_logic() {
        let filter = Filter {
            status: None,
            tags: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            ticket_type: None,
            priority_range: None,
            assignee: None,
            limit: None,
        };

        // Has one of the tags
        let mut t1 = make_ticket("a", Status::Open, 0);
        t1.tags = vec!["y".to_string()];
        assert!(filter.matches(&t1));

        // Has two of the tags
        let mut t2 = make_ticket("b", Status::Open, 0);
        t2.tags = vec!["x".to_string(), "z".to_string()];
        assert!(filter.matches(&t2));

        // Has none of the tags
        let mut t3 = make_ticket("c", Status::Open, 0);
        t3.tags = vec!["w".to_string()];
        assert!(!filter.matches(&t3));

        // Has no tags at all
        let t4 = make_ticket("d", Status::Open, 0);
        assert!(!filter.matches(&t4));
    }
}
