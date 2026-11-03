//! Git-style subcommands: `zylotek-cli webapp --listen …` runs the `zylotek-webapp` executable.
//!
//! The long-running services stay separate binaries, so the CLI does not link their code; it
//! only finds them and hands over. The executable is looked up next to this binary first (a
//! `cargo install` or a `target/` directory keeps them together), then on `PATH`.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Prefix of every executable the CLI dispatches to.
pub const PREFIX: &str = "zylotek-";

/// The executable name for a subcommand, or `None` if the name could escape the lookup
/// (empty, or containing a path separator).
pub fn program_name(subcommand: &OsStr) -> Option<OsString> {
    let name = subcommand.to_str()?;
    if name.is_empty() || name.starts_with('-') || name.contains(['/', '\\']) {
        return None;
    }
    Some(format!("{PREFIX}{name}{}", env::consts::EXE_SUFFIX).into())
}

/// Finds `program` in `first` and then in each directory of `path`.
pub fn find(program: &OsStr, first: Option<&Path>, path: Option<&OsStr>) -> Option<PathBuf> {
    let from_path = path.into_iter().flat_map(env::split_paths);
    first
        .map(Path::to_path_buf)
        .into_iter()
        .chain(from_path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

pub fn execute(args: &[OsString]) -> Result<(), Error> {
    let (subcommand, rest) = args.split_first().ok_or("missing subcommand")?;
    let display = subcommand.to_string_lossy();
    let program =
        program_name(subcommand).ok_or_else(|| format!("invalid subcommand `{display}`"))?;

    let own_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let found =
        find(&program, own_dir.as_deref(), env::var_os("PATH").as_deref()).ok_or_else(|| {
            format!(
                "unknown command `{display}`: no `{}` executable next to this binary or on PATH",
                program.to_string_lossy()
            )
        })?;

    run(Command::new(found).args(rest))
}

/// Replaces this process, so signals and the exit status reach the service untouched.
#[cfg(unix)]
fn run(command: &mut Command) -> Result<(), Error> {
    use std::os::unix::process::CommandExt;

    let program = command.get_program().to_owned();
    let error = command.exec();
    Err(format!("cannot run {program:?}: {error}").into())
}

#[cfg(not(unix))]
fn run(command: &mut Command) -> Result<(), Error> {
    let status = command.status()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_the_subcommand() {
        let name = program_name(OsStr::new("webapp")).unwrap();
        assert_eq!(
            name,
            OsString::from(format!("zylotek-webapp{}", env::consts::EXE_SUFFIX))
        );
    }

    #[test]
    fn refuses_names_that_could_escape_the_lookup() {
        for bad in ["", "../x", "a/b", "a\\b", "-x"] {
            assert_eq!(program_name(OsStr::new(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn looks_in_the_first_directory_before_path() {
        let first = tempfile::tempdir().unwrap();
        let later = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("zylotek-x"), "").unwrap();
        std::fs::write(later.path().join("zylotek-x"), "").unwrap();
        let path = env::join_paths([later.path()]).unwrap();

        let found = find(OsStr::new("zylotek-x"), Some(first.path()), Some(&path));
        assert_eq!(found, Some(first.path().join("zylotek-x")));

        let found = find(OsStr::new("zylotek-x"), None, Some(&path));
        assert_eq!(found, Some(later.path().join("zylotek-x")));
        assert_eq!(find(OsStr::new("zylotek-y"), None, Some(&path)), None);
        assert_eq!(find(OsStr::new("zylotek-x"), None, None), None);
    }
}
