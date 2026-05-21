pub mod generated;

pub use generated::nmux::protocol;

pub const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::protocol;

    #[test]
    fn exposes_workspace_snapshot_body_type() {
        assert_eq!(
            protocol::EnvelopeBody::WorkspaceTreeSnapshot.variant_name(),
            Some("WorkspaceTreeSnapshot")
        );
    }
}
