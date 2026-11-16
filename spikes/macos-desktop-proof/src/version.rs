//! `sw_vers` text. The command itself runs on the runner.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwVers {
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
}

pub fn parse_sw_vers(text: &str) -> Result<SwVers, String> {
    let mut product_name = None;
    let mut product_version = None;
    let mut build_version = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "ProductName" => product_name = Some(value.to_string()),
            "ProductVersion" => product_version = Some(value.to_string()),
            "BuildVersion" => build_version = Some(value.to_string()),
            _ => {}
        }
    }
    match (product_name, product_version, build_version) {
        (Some(product_name), Some(product_version), Some(build_version)) => Ok(SwVers {
            product_name,
            product_version,
            build_version,
        }),
        _ => Err(format!("sw_vers output is missing a field: {text}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sw_vers_fields_are_trimmed() {
        let text = "ProductName:\tmacOS\nProductVersion:    26.0.1\nBuildVersion: 25A362\n";
        let parsed = parse_sw_vers(text).expect("sw_vers");
        assert_eq!(parsed.product_name, "macOS");
        assert_eq!(parsed.product_version, "26.0.1");
        assert_eq!(parsed.build_version, "25A362");
    }

    #[test]
    fn a_missing_build_is_an_error() {
        let error = parse_sw_vers("ProductName: macOS\nProductVersion: 15.5\n").unwrap_err();
        assert!(error.contains("sw_vers"));
    }
}
