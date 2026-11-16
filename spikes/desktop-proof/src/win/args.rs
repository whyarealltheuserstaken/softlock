use std::path::PathBuf;

use super::util::absolute_dir;

pub struct Args {
    pub service: bool,
    pub interactive: bool,
    pub allow_local_service: bool,
    pub runner_image: String,
    pub output_dir: PathBuf,
    pub parent_pid: u32,
}

pub fn parse() -> Result<Args, String> {
    let mut service = false;
    let mut interactive = false;
    let mut runner_image = std::env::var("ImageOS").unwrap_or_else(|_| "unknown".to_string());
    let mut output_dir = PathBuf::from("proof-output");
    let mut parent_pid = 0u32;
    let mut allow_local_service = false;
    let mut tokens = std::env::args().skip(1);
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--service" => service = true,
            "--interactive-proof" => interactive = true,
            "--allow-local-service" => allow_local_service = true,
            "--runner-image" => {
                runner_image = tokens
                    .next()
                    .ok_or("--runner-image needs a value")?
                    .to_string();
            }
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--parent-pid" => {
                let text = tokens.next().ok_or("--parent-pid needs a value")?;
                parent_pid = text.parse().map_err(|_| format!("bad pid {text}"))?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if service && interactive {
        return Err("service and interactive proof are different processes".to_string());
    }
    Ok(Args {
        service,
        interactive,
        allow_local_service,
        runner_image,
        output_dir: absolute_dir(&output_dir),
        parent_pid,
    })
}
