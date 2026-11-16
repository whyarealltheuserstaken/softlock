use std::path::PathBuf;

use super::util::absolute_dir;

pub struct Args {
    pub service: bool,
    pub guard: bool,
    pub revert: bool,
    pub heartbeat: bool,
    pub arm_and_wait: bool,
    pub check_restored: bool,
    /// Present only in a build that opts into the panic-after-arm feature.
    #[cfg(feature = "panic-after-arm")]
    pub panic_after_arm: bool,
    pub allow_local_service: bool,
    pub runner_image: String,
    pub output_dir: PathBuf,
    pub parent_pid: u32,
}

pub fn parse() -> Result<Args, String> {
    let mut service = false;
    let mut guard = false;
    let mut revert = false;
    let mut heartbeat = false;
    let mut arm_and_wait = false;
    let mut check_restored = false;
    #[cfg(feature = "panic-after-arm")]
    let mut panic_after_arm = false;
    let mut allow_local_service = false;
    let mut runner_image = std::env::var("ImageOS").unwrap_or_else(|_| "unknown".to_string());
    let mut output_dir = PathBuf::from("proof-output");
    let mut parent_pid = 0u32;
    let mut tokens = std::env::args().skip(1);
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--service" => service = true,
            "--guard" => guard = true,
            "--revert" => revert = true,
            "--heartbeat" => heartbeat = true,
            "--arm-and-wait" => arm_and_wait = true,
            "--check-restored" => check_restored = true,
            #[cfg(feature = "panic-after-arm")]
            "--panic-after-arm" => panic_after_arm = true,
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
    let modes = [
        service,
        guard,
        revert,
        heartbeat,
        arm_and_wait,
        check_restored,
    ]
    .into_iter()
    .filter(|mode| *mode)
    .count();
    if modes > 1 {
        return Err(
            "choose one of --service, --guard, --revert, --heartbeat, --arm-and-wait, or --check-restored"
                .to_string(),
        );
    }
    Ok(Args {
        service,
        guard,
        revert,
        heartbeat,
        arm_and_wait,
        check_restored,
        #[cfg(feature = "panic-after-arm")]
        panic_after_arm,
        allow_local_service,
        runner_image,
        output_dir: absolute_dir(&output_dir),
        parent_pid,
    })
}
