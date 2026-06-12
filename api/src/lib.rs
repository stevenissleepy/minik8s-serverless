pub mod eventtrigger;
pub mod revision;
pub mod service;
pub mod workflow;

pub use eventtrigger::{
    EventTrigger, EventTriggerSpec, EventTriggerStatus, TriggerTarget, TriggerTargetKind,
};
pub use revision::{Revision, RevisionSpec, RevisionStatus};
pub use service::{
    ServerlessConcurrency, ServerlessScale, ServerlessService, ServerlessServiceSpec,
    ServerlessServiceStatus,
};
pub use workflow::{
    BranchCondition, Workflow, WorkflowBranch, WorkflowSpec, WorkflowStatus, WorkflowStep,
};

pub const GROUP: &str = "serverless.minik8s.io";
pub const VERSION: &str = "v1alpha1";
pub const DEFAULT_HANDLER: &str = "handler";
