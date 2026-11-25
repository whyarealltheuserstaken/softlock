//! Ticket 36 (GitHub #89): the seam 3 display-change step runs on both
//! Windows images, checks every case or its "unproven in CI" line, and never
//! installs a display driver onto the hosted image.

fn ci() -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci")
}

/// The `seam3:` job, up to the next job key (two-space indent).
fn seam3_job(ci: &str) -> String {
    let mut lines = ci.lines().skip_while(|line| *line != "  seam3:");
    let mut job = String::from(lines.next().expect("seam3 job"));
    for line in lines {
        let job_key =
            line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':');
        if job_key {
            break;
        }
        job.push('\n');
        job.push_str(line);
    }
    job
}

fn display_step(job: &str) -> &str {
    let start = job
        .find("- name: Display change while locked")
        .expect("the seam3 job has no display-change step");
    let rest = &job[start + 1..];
    let end = rest.find("\n      - name:").unwrap_or(rest.len());
    &job[start..start + 1 + end]
}

#[test]
fn the_display_step_runs_in_the_seam3_job_on_both_images() {
    let job = seam3_job(&ci());
    display_step(&job);
    assert!(job.contains("image: [windows-2022, windows-2025]"));
}

#[test]
fn the_display_step_checks_every_case_or_its_unproven_line() {
    let job = seam3_job(&ci());
    let step = display_step(&job);
    for needle in [
        "--test display_change",
        "runner product=",
        "test a_resolution_change_while_locked_keeps_every_monitor_covered ...",
        "test a_scale_change_is_covered_or_named_unproven ...",
        "test an_added_display_is_covered_and_a_removed_one_drops_out_or_named_unproven ...",
        "test result: ok. 3 passed; 0 failed",
        "resolution change [0-9]+x[0-9]+ -> [0-9]+x[0-9]+ covered=",
        "display-dpi (covered=|unproven in CI)",
        "display-add (covered=|unproven in CI)",
        "display-remove (covered=|unproven in CI)",
    ] {
        assert!(
            step.contains(needle),
            "display step does not check {needle}"
        );
    }
}

#[test]
fn the_seam3_job_installs_no_display_driver() {
    let job = seam3_job(&ci()).to_ascii_lowercase();
    for tool in [
        "pnputil",
        "devcon",
        "add-driver",
        ".inf",
        "usbmmidd",
        "virtual-display",
    ] {
        assert!(!job.contains(tool), "the seam3 job mentions {tool}");
    }
}
