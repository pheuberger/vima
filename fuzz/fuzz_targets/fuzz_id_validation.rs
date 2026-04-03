#![no_main]
use libfuzzer_sys::fuzz_target;

use vima::id;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Fuzz ID validation — should never panic
        let _ = id::validate_id(input);
    }
});
