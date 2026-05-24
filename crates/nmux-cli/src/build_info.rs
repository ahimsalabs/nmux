#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub channel: &'static str,
    pub commit: &'static str,
    pub build_date: &'static str,
}

impl BuildInfo {
    pub const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            channel: match option_env!("NMUX_BUILD_CHANNEL") {
                Some(value) => value,
                None => "dev",
            },
            commit: match option_env!("NMUX_BUILD_COMMIT") {
                Some(value) => value,
                None => "unknown",
            },
            build_date: match option_env!("NMUX_BUILD_DATE") {
                Some(value) => value,
                None => "unknown",
            },
        }
    }

    pub fn version_line(&self, binary: &str) -> String {
        match (self.short_commit(), self.known_build_date()) {
            (Some(commit), Some(build_date)) => {
                format!(
                    "{binary} {} ({} {commit} {build_date})",
                    self.version, self.channel
                )
            }
            (Some(commit), None) => {
                format!("{binary} {} ({} {commit})", self.version, self.channel)
            }
            (None, Some(build_date)) => {
                format!("{binary} {} ({} {build_date})", self.version, self.channel)
            }
            (None, None) => format!("{binary} {} ({})", self.version, self.channel),
        }
    }

    fn short_commit(&self) -> Option<&str> {
        if self.commit == "unknown" || self.commit.is_empty() {
            return None;
        }
        Some(self.commit.get(..7).unwrap_or(self.commit))
    }

    fn known_build_date(&self) -> Option<&str> {
        if self.build_date == "unknown" || self.build_date.is_empty() {
            return None;
        }
        Some(self.build_date)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_omits_unknown_commit_and_date() {
        let info = BuildInfo {
            version: "1.2.3",
            channel: "dev",
            commit: "unknown",
            build_date: "unknown",
        };

        assert_eq!(info.version_line("nmux"), "nmux 1.2.3 (dev)");
    }

    #[test]
    fn version_line_uses_short_commit_when_known() {
        let info = BuildInfo {
            version: "1.2.3",
            channel: "nightly",
            commit: "abcdef1234567890",
            build_date: "2026-05-24",
        };

        assert_eq!(
            info.version_line("nmux"),
            "nmux 1.2.3 (nightly abcdef1 2026-05-24)"
        );
    }
}
