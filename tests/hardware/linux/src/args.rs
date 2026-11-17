//! Command-line flags.

use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Args {
    pub ci: bool,
    pub admin: bool,
    pub interactive: bool,
    pub output_dir: Option<PathBuf>,
    pub help: bool,
}

impl Args {
    pub fn from_args() -> Result<Self, String> {
        Self::parse(env::args().skip(1))
    }

    pub fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut ci = false;
        let mut admin = false;
        let mut interactive = false;
        let mut output_dir = None;
        let mut help = false;
        let mut iter = args.into_iter().peekable();
        while let Some(raw) = iter.next() {
            let arg = raw.as_ref();
            match arg {
                "--ci" => ci = true,
                "--admin" => admin = true,
                "--interactive" => interactive = true,
                "--help" | "-h" => help = true,
                "--output-dir" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| " --output-dir needs a path".to_string())?;
                    output_dir = Some(PathBuf::from(value.as_ref()));
                }
                other if other.starts_with("--output-dir=") => {
                    output_dir = Some(PathBuf::from(&other["--output-dir=".len()..]));
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(Self {
            ci,
            admin,
            interactive,
            output_dir,
            help,
        })
    }

    pub fn mode_label(&self) -> String {
        match (self.ci, self.admin) {
            (true, true) => "ci+admin".to_string(),
            (true, false) => "ci".to_string(),
            (false, true) => "admin".to_string(),
            (false, false) => "interactive".to_string(),
        }
    }

    pub fn ask_physical(&self) -> bool {
        if self.ci {
            return false;
        }
        if self.interactive {
            return true;
        }
        std::io::stdin().is_terminal()
    }

    pub fn allow_root_ops(&self) -> bool {
        self.admin
    }
}

pub const HELP: &str = "\
Linux hardware suite

  ./tests/hardware/linux/run.sh [--ci] [--admin] [--interactive] [--output-dir DIR]

  --ci            no prompts; skip physical and admin rows; ubuntu-runner green
  --admin         run rows that need /dev/uinput, EVIOCGRAB, or passwordless sudo
  --interactive   ask the batched physical rows even if stdin is not a tty
  --output-dir    write result.json and result.txt here (default suite-output/<utc>)
";
