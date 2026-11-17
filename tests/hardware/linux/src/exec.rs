//! Process runner. Tests install a stub so a lock request is observable.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct CmdResult {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

type Hook = Arc<dyn Fn(&str, &[String]) -> CmdResult + Send + Sync>;

fn hook() -> &'static Mutex<Option<Hook>> {
    static HOOK: OnceLock<Mutex<Option<Hook>>> = OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

pub struct Stub {
    pub calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl Default for Stub {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl Stub {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn install_stub(stub: Arc<Stub>) {
    let stub = stub.clone();
    let mut slot = hook().lock().expect("command hook");
    *slot = Some(Arc::new(move |program, args| {
        stub.calls
            .lock()
            .expect("stub calls")
            .push((program.to_string(), args.to_vec()));
        CmdResult {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    }));
}

pub fn clear_stub() {
    if let Ok(mut slot) = hook().lock() {
        *slot = None;
    }
}

pub fn run_cmd(program: &str, args: &[&str]) -> CmdResult {
    let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    if let Ok(slot) = hook().lock()
        && let Some(hook) = slot.as_ref()
    {
        return hook(program, &owned);
    }
    let output = Command::new(program)
        .args(&owned)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(out) => CmdResult {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(err) => CmdResult {
            code: None,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}
