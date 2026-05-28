pub mod generated;
pub mod wire;

pub use generated::nmux::protocol;

pub const PROTOCOL_VERSION: u32 = 2;

#[cfg(test)]
mod tests {
    use super::protocol;

    #[test]
    fn exposes_workspace_snapshot_body_type() {
        assert_eq!(
            protocol::EnvelopeBody::WorkspaceTreeSnapshot.variant_name(),
            Some("WorkspaceTreeSnapshot")
        );
        assert_eq!(
            protocol::EnvelopeBody::AttachRequest.variant_name(),
            Some("AttachRequest")
        );
    }
}
