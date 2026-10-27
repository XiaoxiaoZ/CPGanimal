//! Finding Python for the Train tab. Computers do not always call it `python`:
//! it may be `py -3` (the launcher of the python.org installer), `python3`, or
//! only a python.exe in its install folder that is not on PATH. Every
//! candidate is run once and must report Python 3.8 or newer; this also skips
//! the Microsoft Store placeholder `python.exe`, which only says "Python was
//! not found".

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The oldest Python the scripts run on.
const OLDEST: (u32, u32) = (3, 8);

/// How long a candidate may take to answer: the first start of Python can be
/// slow, e.g. while a virus scanner checks it.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A command that starts Python 3, and its version.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// What goes in the Python field, e.g. `py -3` or the path to python.exe.
    pub command: String,
    /// e.g. "3.12.10"
    pub version: String,
}

/// Split a Python command into the program and its arguments: `py -3`, a path
/// to python.exe (in quotes if it has spaces and arguments follow), or just a
/// path with spaces.
pub fn split(command: &str) -> Option<(String, Vec<String>)> {
    let command = command.trim();
    let unquoted = command.trim_matches('"');
    if !unquoted.is_empty() && Path::new(unquoted).is_file() {
        return Some((unquoted.to_string(), Vec::new()));
    }
    if let Some(rest) = command.strip_prefix('"') {
        let (program, args) = rest.split_once('"')?;
        return Some((program.to_string(), args.split_whitespace().map(String::from).collect()));
    }
    let mut words = command.split_whitespace().map(String::from);
    let program = words.next()?;
    Some((program, words.collect()))
}

/// The process for a Python command, ready for the script's arguments.
pub fn command(python: &str) -> Option<Command> {
    let (program, args) = split(python)?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    Some(cmd)
}

/// Run a Python command once: its version, or why it cannot be used.
pub fn check(python: &str) -> Result<String, String> {
    let mut cmd = command(python).ok_or("no command")?;
    let mut child = cmd
        .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3])"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + TIMEOUT;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            child.kill().ok();
            child.wait().ok();
            return Err(format!("no answer in {} s", TIMEOUT.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        // e.g. the Store placeholder's "Python was not found; ..."
        let stderr = String::from_utf8_lossy(&out.stderr);
        let first = stderr.lines().chain(stdout.lines()).map(str::trim).find(|l| !l.is_empty());
        return Err(first.map_or_else(|| format!("failed ({})", out.status), String::from));
    }
    let version = stdout.trim();
    usable(version)?;
    Ok(version.to_string())
}

/// Ok for "3.8.0" and newer.
fn usable(version: &str) -> Result<(), String> {
    let mut parts = version.split('.').map(|p| p.parse::<u32>());
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) if (major, minor) >= OLDEST => Ok(()),
        (Some(Ok(_)), Some(Ok(_))) => Err(format!("Python {version} is too old: {}.{} or newer is needed", OLDEST.0, OLDEST.1)),
        _ => Err(format!("not Python: it answered {version:?}")),
    }
}

/// The commands to try, in order: the usual names, then python.exe in the
/// usual install folders.
pub fn candidates() -> Vec<String> {
    let names: &[&str] = if cfg!(windows) { &["python", "py -3", "python3"] } else { &["python3", "python"] };
    let mut list: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    for p in installed() {
        let p = p.display().to_string();
        if !list.contains(&p) {
            list.push(p);
        }
    }
    list
}

/// The first candidate that works, or what each one said.
pub fn find() -> Result<Found, Vec<String>> {
    let mut tried = Vec::new();
    for c in candidates() {
        match check(&c) {
            Ok(version) => return Ok(Found { command: c, version }),
            Err(e) => tried.push(format!("{c}: {e}")),
        }
    }
    Err(tried)
}

/// python.exe in the usual install folders, which need not be on PATH.
fn installed() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if cfg!(windows) {
        let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let local = env("LOCALAPPDATA");
        if let Some(d) = &local {
            // The python.org installer, for this user ...
            found.extend(versions(&d.join("Programs").join("Python"), "python3"));
            // ... and the Python install manager (Python 3.14 and newer).
            found.extend(versions(&d.join("Python"), "pythoncore-3"));
        }
        // The python.org installer for all users, and older ones in C:\.
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(d) = env(var) {
                found.extend(versions(&d, "python3"));
            }
        }
        if let Some(drive) = std::env::var_os("SystemDrive") {
            let mut root = OsString::from(drive);
            root.push("\\");
            found.extend(versions(Path::new(&root), "python3"));
        }
        // Anaconda and Miniconda, for this user or for all users.
        for base in [env("USERPROFILE"), env("ProgramData")].into_iter().flatten() {
            for conda in ["anaconda3", "miniconda3"] {
                found.push(base.join(conda).join("python.exe"));
            }
        }
        // The Python that comes with the Thonny editor.
        for base in [local.map(|d| d.join("Programs")), env("ProgramFiles")].into_iter().flatten() {
            found.push(base.join("Thonny").join("python.exe"));
        }
    } else {
        // Apps started from the desktop may not get the shell's PATH (macOS).
        for p in ["/opt/homebrew/bin/python3", "/usr/local/bin/python3", "/usr/bin/python3", "/Library/Frameworks/Python.framework/Versions/Current/bin/python3"] {
            found.push(PathBuf::from(p));
        }
    }
    found.retain(|p| p.is_file());
    found
}

/// `dir/<prefix>*/python.exe` that exist, newest version first (Python312 before Python39).
fn versions(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(Vec<u32>, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let exe = e.path().join("python.exe");
            (name.starts_with(prefix) && exe.is_file()).then(|| (numbers(&name), exe))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p)| p).collect()
}

/// The numbers in a folder name: "python312" -> [312], "pythoncore-3.14-64" -> [3, 14, 64].
fn numbers(name: &str) -> Vec<u32> {
    name.split(|c: char| !c.is_ascii_digit()).filter_map(|s| s.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_commands_with_arguments_and_paths_with_spaces() {
        assert_eq!(split("  python  "), Some(("python".into(), vec![])));
        assert_eq!(split("py -3"), Some(("py".into(), vec!["-3".into()])));
        assert_eq!(
            split(r#""C:\Program Files\Python312\python.exe" -X utf8"#),
            Some((r"C:\Program Files\Python312\python.exe".into(), vec!["-X".into(), "utf8".into()]))
        );
        assert_eq!(split(""), None);
        // An existing file is one program, even with spaces and without quotes.
        let dir = std::env::temp_dir().join("arena gui python test");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("python.exe");
        std::fs::write(&exe, "").unwrap();
        let path = exe.display().to_string();
        assert_eq!(split(&path), Some((path.clone(), vec![])));
        assert_eq!(split(&format!("\"{path}\"")), Some((path, vec![])));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn needs_python_3_8_or_newer() {
        assert!(usable("3.8.0").is_ok() && usable("3.12.10").is_ok() && usable("3.14.0").is_ok());
        assert!(usable("3.7.9").unwrap_err().contains("too old"));
        assert!(usable("2.7.18").unwrap_err().contains("too old"));
        assert!(usable("hello").unwrap_err().contains("not Python"));
        assert!(usable("").is_err());
    }

    #[test]
    fn newest_install_folder_first() {
        assert_eq!(numbers("python312"), vec![312]);
        assert_eq!(numbers("pythoncore-3.14-64"), vec![3, 14, 64]);
        let dir = std::env::temp_dir().join("arena-gui-python-versions");
        std::fs::remove_dir_all(&dir).ok();
        for (sub, exe) in [("Python39", true), ("Python312", true), ("Python313", false), ("Launcher", true)] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            if exe {
                std::fs::write(dir.join(sub).join("python.exe"), "").unwrap();
            }
        }
        let names: Vec<String> =
            versions(&dir, "python3").iter().map(|p| p.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Python312", "Python39"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_command_is_not_python() {
        assert!(check("arena-certainly-not-python").is_err());
        assert!(check("").is_err());
    }

    #[test]
    #[ignore = "needs Python"]
    fn finds_python() {
        let found = find().unwrap_or_else(|tried| panic!("{tried:#?}"));
        assert!(usable(&found.version).is_ok(), "{found:?}");
        assert_eq!(check(&found.command), Ok(found.version));
        assert!(command(&found.command).unwrap().arg("-c").arg("import arena").current_dir(env!("CARGO_MANIFEST_DIR")).env("PYTHONPATH", "python").status().unwrap().success());
    }
}
