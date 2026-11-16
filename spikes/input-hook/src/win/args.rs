use std::path::PathBuf;

use super::util::absolute_dir;

pub struct Args {
    pub service: bool,
    pub role: Option<String>,
    pub allow_local_service: bool,
    pub runner_image: String,
    pub output_dir: PathBuf,
    pub port: u16,
}

pub fn parse() -> Result<Args, String> {
    let mut service = false;
    let mut role = None;
    let mut runner_image = std::env::var("ImageOS").unwrap_or_else(|_| "unknown".to_string());
    let mut output_dir = PathBuf::from("proof-output");
    let mut allow_local_service = false;
    let mut port = 0u16;
    let mut tokens = std::env::args().skip(1);
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--service" => service = true,
            "--allow-local-service" => allow_local_service = true,
            "--role" => {
                role = Some(tokens.next().ok_or("--role needs a value")?);
            }
            "--runner-image" => {
                runner_image = tokens
                    .next()
                    .ok_or("--runner-image needs a value")?
                    .to_string();
            }
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--port" => {
                let text = tokens.next().ok_or("--port needs a value")?;
                port = text.parse().map_err(|_| format!("bad port {text}"))?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if service && role.is_some() {
        return Err("the service and a hook role are different processes".to_string());
    }
    if role.is_some() && port == 0 {
        return Err("a hook role needs --port".to_string());
    }
    Ok(Args {
        service,
        role,
        allow_local_service,
        runner_image,
        output_dir: absolute_dir(&output_dir),
        port,
    })
}
