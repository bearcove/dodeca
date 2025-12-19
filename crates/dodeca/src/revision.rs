use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionStatus {
    Building,
    Ready,
}

#[derive(Debug, Clone)]
pub struct RevisionState {
    pub generation: u64,
    pub status: RevisionStatus,
    #[allow(dead_code)]
    pub reason: Option<String>,
    #[allow(dead_code)]
    pub started_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
pub struct RevisionToken {
    pub generation: u64,
    pub started_at: Instant,
}
