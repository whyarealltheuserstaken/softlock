//! Throwaway launchd plists. The probe writes them, and the workflow deletes them.

use std::path::{Path, PathBuf};

use crate::labels::AGENT_PLIST_NAME;

pub struct PlistOpts<'a> {
    pub label: &'a str,
    pub arguments: &'a [String],
    pub stdout_path: &'a str,
    pub stderr_path: &'a str,
    pub session_limit: Option<&'a str>,
}

pub fn job_arguments(exe: &str, kind: &str, output_dir: &str) -> Result<Vec<String>, String> {
    if kind != "agent" && kind != "daemon" {
        return Err(format!("unknown job {kind}"));
    }
    if !exe.starts_with('/') {
        return Err("job executable must be an absolute path".to_string());
    }
    if !output_dir.starts_with('/') {
        return Err("job output directory must be an absolute path".to_string());
    }
    Ok(vec![
        exe.to_string(),
        "--job".to_string(),
        kind.to_string(),
        "--output-dir".to_string(),
        output_dir.to_string(),
    ])
}

pub fn agent_plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents").join(AGENT_PLIST_NAME)
}

pub fn job_plist(opts: &PlistOpts<'_>) -> Result<String, String> {
    if opts.label.is_empty() {
        return Err("launchd label is empty".to_string());
    }
    if opts.arguments.is_empty() || !opts.arguments[0].starts_with('/') {
        return Err("launchd program must be an absolute path".to_string());
    }
    let mut args_xml = String::new();
    for arg in opts.arguments {
        args_xml.push_str("    <string>");
        args_xml.push_str(&xml_escape(arg));
        args_xml.push_str("</string>\n");
    }
    let session = match opts.session_limit {
        Some(limit) if !limit.is_empty() => format!(
            "  <key>LimitLoadToSessionType</key>\n  <string>{}</string>\n",
            xml_escape(limit)
        ),
        _ => String::new(),
    };
    let working = parent_or_root(opts.stdout_path);
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{args}</array>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
  <key>ExitTimeOut</key>
  <integer>5</integer>
  <key>WorkingDirectory</key>
  <string>{working}</string>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
{session}</dict>
</plist>
"#,
        label = xml_escape(opts.label),
        args = args_xml,
        working = xml_escape(&working),
        stdout = xml_escape(opts.stdout_path),
        stderr = xml_escape(opts.stderr_path),
        session = session,
    ))
}

fn parent_or_root(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|parent| {
            let text = parent.to_string_lossy();
            if text.is_empty() {
                "/".to_string()
            } else {
                text.into_owned()
            }
        })
        .unwrap_or_else(|| "/".to_string())
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::{AGENT_LABEL, DAEMON_LABEL};

    fn args(kind: &str) -> Vec<String> {
        job_arguments("/tmp/macos-desktop-proof", kind, "/tmp/macos-proof-out").unwrap()
    }

    #[test]
    fn the_agent_plist_is_keep_alive_and_limited_to_aqua() {
        let arguments = args("agent");
        let body = job_plist(&PlistOpts {
            label: AGENT_LABEL,
            arguments: &arguments,
            stdout_path: "/tmp/macos-proof-out/agent.out",
            stderr_path: "/tmp/macos-proof-out/agent.err",
            session_limit: Some("Aqua"),
        })
        .unwrap();
        assert!(body.contains("<key>KeepAlive</key>"));
        assert!(body.contains("<true/>"));
        assert!(body.contains(AGENT_LABEL));
        assert!(body.contains("<key>LimitLoadToSessionType</key>"));
        assert!(body.contains("<string>Aqua</string>"));
        assert!(body.contains("--job"));
        assert!(body.contains("/tmp/macos-desktop-proof"));
        assert!(
            !body.contains("LimitLoadToSessionType</key>\n  <string></string>"),
            "empty session limit must not be written"
        );
    }

    #[test]
    fn the_daemon_plist_has_no_session_limit() {
        let arguments = args("daemon");
        let body = job_plist(&PlistOpts {
            label: DAEMON_LABEL,
            arguments: &arguments,
            stdout_path: "/tmp/macos-proof-out/daemon.out",
            stderr_path: "/tmp/macos-proof-out/daemon.err",
            session_limit: None,
        })
        .unwrap();
        assert!(body.contains(DAEMON_LABEL));
        assert!(!body.contains("LimitLoadToSessionType"));
        assert!(body.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn program_arguments_escape_xml_and_relative_paths_are_refused() {
        let arguments = vec!["/tmp/probe".to_string(), "a&b<c>".to_string()];
        let body = job_plist(&PlistOpts {
            label: AGENT_LABEL,
            arguments: &arguments,
            stdout_path: "/tmp/out",
            stderr_path: "/tmp/err",
            session_limit: None,
        })
        .unwrap();
        assert!(body.contains("a&amp;b&lt;c&gt;"));
        assert!(
            job_plist(&PlistOpts {
                label: AGENT_LABEL,
                arguments: &["relative".to_string()],
                stdout_path: "/tmp/out",
                stderr_path: "/tmp/err",
                session_limit: None,
            })
            .is_err()
        );
        assert!(job_arguments("relative", "agent", "/tmp/out").is_err());
        assert!(job_arguments("/tmp/probe", "other", "/tmp/out").is_err());
    }

    #[test]
    fn the_agent_plist_lives_in_the_user_launch_agents_directory() {
        let path = agent_plist_path(Path::new("/Users/runner"));
        assert_eq!(
            path,
            PathBuf::from("/Users/runner/Library/LaunchAgents").join(AGENT_PLIST_NAME)
        );
    }
}
