use std::env;
use std::io::BufRead;
use std::path::Path;

use crate::store::find_vima_root;

/// Attempt to find and execute a plugin named `vima-{command}`.
/// Returns None if no plugin found on PATH. Never returns on success (process is replaced).
pub fn try_plugin(command: &str, args: &[String]) -> Option<()> {
    let plugin_name = format!("vima-{}", command);
    let path = which::which(&plugin_name).ok()?;

    // Safety: single-threaded at this point, and we're about to exec (replacing the process)
    unsafe {
        if let Ok(vima_dir) = find_vima_root() {
            env::set_var("VIMA_DIR", &vima_dir);
            env::set_var("VIMA_TICKETS_DIR", vima_dir.join("tickets"));
        }
        if let Ok(exe) = env::current_exe() {
            env::set_var("VIMA_BIN", &exe);
        }
    }

    let err = exec::Command::new(&path).args(args).exec();
    eprintln!("vima: failed to execute {}: {}", plugin_name, err);
    std::process::exit(1);
}

/// Discover all `vima-*` plugins on PATH.
/// Returns a sorted list of (command_name, optional_description) pairs.
pub fn discover_plugins() -> Vec<(String, Option<String>)> {
    let path_var = env::var("PATH").unwrap_or_default();
    let mut plugins: Vec<(String, Option<String>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dir in env::split_paths(&path_var) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if !name.starts_with("vima-") {
                continue;
            }
            let command = name["vima-".len()..].to_string();
            if command.is_empty() || seen.contains(&command) {
                continue;
            }
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }
            seen.insert(command.clone());
            let description = read_plugin_description(&path);
            plugins.push((command, description));
        }
    }

    plugins.sort_by(|a, b| a.0.cmp(&b.0));
    plugins
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

fn read_plugin_description(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(10) {
        let Ok(line) = line else { continue };
        if let Some(desc) = line.strip_prefix("# vima-plugin: ") {
            return Some(desc.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    #[serial(env)]
    fn discover_plugins_finds_vima_executables() {
        let tmp = tempdir().unwrap();
        let plugin_path = tmp.path().join("vima-testplugin");
        let mut f = std::fs::File::create(&plugin_path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "# vima-plugin: A test plugin").unwrap();
        writeln!(f, "echo hello").unwrap();
        drop(f);
        make_executable(&plugin_path);

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", tmp.path().display(), original_path);
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found = plugins.iter().find(|(name, _)| name == "testplugin");
        assert!(
            found.is_some(),
            "should find vima-testplugin as 'testplugin'"
        );
        assert_eq!(found.unwrap().1.as_deref(), Some("A test plugin"));
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_ignores_non_executable() {
        let tmp = tempdir().unwrap();
        let plugin_path = tmp.path().join("vima-notexec");
        std::fs::write(&plugin_path, "#!/bin/sh\necho hello\n").unwrap();
        // intentionally NOT setting executable bit

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", tmp.path().display(), original_path);
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found = plugins.iter().find(|(name, _)| name == "notexec");
        assert!(found.is_none(), "non-executable should not be discovered");
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_prefers_executable_over_non_executable_earlier_in_path() {
        let tmp1 = tempdir().unwrap();
        let tmp2 = tempdir().unwrap();

        // dir1 has a non-executable vima-foo
        let non_exec = tmp1.path().join("vima-foo");
        std::fs::write(&non_exec, "#!/bin/sh\necho non-exec\n").unwrap();
        // intentionally NOT setting executable bit

        // dir2 has an executable vima-foo
        let exec_plugin = tmp2.path().join("vima-foo");
        std::fs::write(&exec_plugin, "#!/bin/sh\necho exec\n").unwrap();
        make_executable(&exec_plugin);

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!(
            "{}:{}:{}",
            tmp1.path().display(),
            tmp2.path().display(),
            original_path
        );
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found = plugins.iter().find(|(name, _)| name == "foo");
        assert!(found.is_some(), "should find vima-foo from dir2");
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_deduplicates_across_path_dirs() {
        let tmp1 = tempdir().unwrap();
        let tmp2 = tempdir().unwrap();

        for dir in [tmp1.path(), tmp2.path()] {
            let plugin_path = dir.join("vima-myplugin");
            std::fs::write(&plugin_path, "#!/bin/sh\necho hello\n").unwrap();
            make_executable(&plugin_path);
        }

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!(
            "{}:{}:{}",
            tmp1.path().display(),
            tmp2.path().display(),
            original_path
        );
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found: Vec<_> = plugins
            .iter()
            .filter(|(name, _)| name == "myplugin")
            .collect();
        assert_eq!(found.len(), 1, "should deduplicate across PATH dirs");
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_returns_sorted() {
        let tmp = tempdir().unwrap();
        for name in ["vima-zzz", "vima-aaa", "vima-mmm"] {
            let p = tmp.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            make_executable(&p);
        }

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", tmp.path().display(), original_path);
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let names: Vec<&str> = plugins
            .iter()
            .filter(|(name, _)| matches!(name.as_str(), "aaa" | "mmm" | "zzz"))
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, vec!["aaa", "mmm", "zzz"]);
    }

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &std::path::Path) {}

    #[test]
    #[serial(env)]
    fn discover_plugins_empty_path_returns_empty() {
        let original_path = env::var("PATH").unwrap_or_default();
        unsafe { env::set_var("PATH", "") };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        assert!(plugins.is_empty(), "empty PATH should yield no plugins");
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_names_with_special_characters() {
        let tmp = tempdir().unwrap();
        // Plugins with dashes and underscores in their names
        for name in ["vima-my-plugin", "vima-my_plugin", "vima-a-b-c"] {
            let p = tmp.path().join(name);
            std::fs::write(&p, "#!/bin/sh\necho hi\n").unwrap();
            make_executable(&p);
        }

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", tmp.path().display(), original_path);
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let names: Vec<&str> = plugins
            .iter()
            .filter(|(n, _)| n == "my-plugin" || n == "my_plugin" || n == "a-b-c")
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            names.len(),
            3,
            "should find all three specially-named plugins"
        );
        // They should be sorted
        assert_eq!(names, vec!["a-b-c", "my-plugin", "my_plugin"]);
    }

    #[test]
    fn read_description_no_description_line() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("vima-nodesc");
        std::fs::write(&path, "#!/bin/sh\necho hello\n# just a comment\n").unwrap();

        let desc = read_plugin_description(&path);
        assert!(
            desc.is_none(),
            "should return None when no description marker is present"
        );
    }

    #[test]
    fn read_description_uses_first_match_only() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("vima-multidesc");
        let content = "#!/bin/sh\n# vima-plugin: First description\n# vima-plugin: Second description\necho hello\n";
        std::fs::write(&path, content).unwrap();

        let desc = read_plugin_description(&path);
        assert_eq!(
            desc.as_deref(),
            Some("First description"),
            "should return only the first description line"
        );
    }

    #[test]
    fn read_description_trims_whitespace() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("vima-trimmed");
        std::fs::write(&path, "#!/bin/sh\n# vima-plugin:   padded description   \n").unwrap();

        let desc = read_plugin_description(&path);
        assert_eq!(
            desc.as_deref(),
            Some("padded description"),
            "description should be trimmed"
        );
    }

    #[test]
    fn read_description_empty_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("vima-empty");
        std::fs::write(&path, "").unwrap();

        let desc = read_plugin_description(&path);
        assert!(desc.is_none(), "empty file should return None");
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_deduplicates_symlinks() {
        let tmp_real = tempdir().unwrap();
        let tmp_link_dir = tempdir().unwrap();

        // Create an executable plugin in tmp_real
        let plugin_path = tmp_real.path().join("vima-symtest");
        let mut f = std::fs::File::create(&plugin_path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "# vima-plugin: Sym test plugin").unwrap();
        drop(f);
        make_executable(&plugin_path);

        // Create a symlink to the same plugin in tmp_link_dir
        #[cfg(unix)]
        std::os::unix::fs::symlink(&plugin_path, tmp_link_dir.path().join("vima-symtest")).unwrap();

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!(
            "{}:{}:{}",
            tmp_real.path().display(),
            tmp_link_dir.path().display(),
            original_path
        );
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found: Vec<_> = plugins
            .iter()
            .filter(|(name, _)| name == "symtest")
            .collect();
        assert_eq!(
            found.len(),
            1,
            "symlinked duplicates should be deduplicated"
        );
        assert_eq!(found[0].1.as_deref(), Some("Sym test plugin"));
    }

    #[test]
    fn read_description_marker_after_line_10_is_ignored() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("vima-late");
        // Put 10 filler lines before the description marker (marker on line 11)
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&format!("# line {}\n", i));
        }
        content.push_str("# vima-plugin: Too late\n");
        std::fs::write(&path, &content).unwrap();

        let desc = read_plugin_description(&path);
        assert!(
            desc.is_none(),
            "description marker past the first 10 lines should be ignored"
        );
    }

    #[test]
    #[serial(env)]
    fn discover_plugins_skips_bare_vima_dash_name() {
        // A file named exactly "vima-" (empty command) should be skipped
        let tmp = tempdir().unwrap();
        let p = tmp.path().join("vima-");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        make_executable(&p);

        let original_path = env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", tmp.path().display(), original_path);
        unsafe { env::set_var("PATH", &new_path) };

        let plugins = discover_plugins();

        unsafe { env::set_var("PATH", &original_path) };

        let found = plugins.iter().find(|(name, _)| name.is_empty());
        assert!(
            found.is_none(),
            "vima- with empty command name should be skipped"
        );
    }
}
