//! Process execution helpers for Konvoy.

use std::process::Command;

use crate::error::UtilError;

/// Structured output from a command execution.
#[derive(Debug)]
pub struct CommandOutput {
    /// Standard output as a string.
    pub stdout: String,
    /// Standard error as a string.
    pub stderr: String,
    /// Whether the command exited successfully.
    pub success: bool,
    /// The exit code, if the process was not killed by a signal.
    pub exit_code: Option<i32>,
}

/// Execute a command and capture its output.
///
/// # Errors
/// Returns an error if the command cannot be spawned (e.g. binary not found).
/// A non-zero exit code is **not** an error; check `CommandOutput::success` instead.
pub fn run_command(cmd: &mut Command) -> Result<CommandOutput, UtilError> {
    let output = cmd
        .output()
        .map_err(|source| UtilError::CommandExec { source })?;

    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
        exit_code: output.status.code(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn run_command_success() {
        let result = run_command(&mut Command::new("echo").arg("hello"));
        let output = result.unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.trim(), "hello");
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn run_command_failure() {
        let result = run_command(&mut Command::new("false"));
        let output = result.unwrap();
        assert!(!output.success);
        assert_ne!(output.exit_code, Some(0));
    }

    #[test]
    fn run_command_missing_binary() {
        let result = run_command(&mut Command::new("nonexistent_binary_xyz_123"));
        assert!(result.is_err());
    }

    #[test]
    fn run_command_captures_stderr() {
        let result = run_command(&mut Command::new("sh").arg("-c").arg("echo err >&2"));
        let output = result.unwrap();
        assert!(output.stderr.contains("err"));
    }
}
