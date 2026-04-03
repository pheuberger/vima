#![no_main]
use libfuzzer_sys::fuzz_target;

use std::io::Cursor;
use vima::batch;
use vima::store::Store;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Set up a temporary store for batch parsing
        let tmp = tempfile::tempdir().unwrap();
        let vima_dir = tmp.path().join(".vima");
        let tickets_dir = vima_dir.join("tickets");
        std::fs::create_dir_all(&tickets_dir).unwrap();
        std::fs::write(vima_dir.join("config.yml"), "prefix: fz\n").unwrap();

        // Safety: fuzz targets run single-threaded
        unsafe { std::env::set_var("VIMA_DIR", &vima_dir) };
        if let Ok(store) = Store::open() {
            // Try to batch-create from arbitrary JSON — should never panic
            let _ = batch::batch_create_reader(&store, Cursor::new(input.as_bytes()), false);
        }
        unsafe { std::env::remove_var("VIMA_DIR") };
    }
});
