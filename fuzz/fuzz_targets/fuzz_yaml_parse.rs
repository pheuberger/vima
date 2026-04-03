#![no_main]
use libfuzzer_sys::fuzz_target;

use gray_matter::engine::YAML;
use gray_matter::Matter;
use vima::ticket::Ticket;

fuzz_target!(|data: &[u8]| {
    if let Ok(content) = std::str::from_utf8(data) {
        // Try to parse arbitrary bytes as YAML frontmatter ticket
        let _ = Matter::<YAML>::new().parse::<Ticket>(content);
    }
});
