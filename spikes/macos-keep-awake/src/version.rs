use crate::report::VersionReport;

pub fn parse_sw_vers(text: &str, uname_m: &str) -> VersionReport {
    let mut product_name = String::new();
    let mut product_version = String::new();
    let mut build_version = String::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ProductName:") {
            product_name = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("ProductVersion:") {
            product_version = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("BuildVersion:") {
            build_version = value.trim().to_string();
        }
    }
    VersionReport {
        product_name,
        product_version,
        build_version,
        uname_m: uname_m.trim().to_string(),
        hw_model: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sw_vers_lines_are_split() {
        let report = parse_sw_vers(
            "ProductName:\tmacOS\nProductVersion:\t26.7.2\nBuildVersion:\t25G332\n",
            "arm64\n",
        );
        assert_eq!(report.product_name, "macOS");
        assert_eq!(report.product_version, "26.7.2");
        assert_eq!(report.build_version, "25G332");
        assert_eq!(report.uname_m, "arm64");
    }
}
