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
            if command.is_empty() || !seen.insert(command.clone()) {
                continue;
            }
            let path = entry.path();
            if !is_executable(&path) {
                continue;
            }
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
        return false;
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
        assert!(found.is_some(), "should find vima-testplugin as 'testplugin'");
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

        let found: Vec<_> = plugins.iter().filter(|(name, _)| name == "myplugin").collect();
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
}
