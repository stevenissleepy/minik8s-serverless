pub mod eventtrigger;
pub mod function;
pub mod workflow;

pub use eventtrigger::{
    EventTrigger, EventTriggerSpec, EventTriggerStatus, TriggerTarget, TriggerTargetKind,
};
pub use function::{
    Function, FunctionConcurrency, FunctionRuntime, FunctionScale, FunctionSource, FunctionSpec,
    FunctionStatus,
};
pub use workflow::{
    BranchCondition, Workflow, WorkflowBranch, WorkflowSpec, WorkflowStatus, WorkflowStep,
};

pub const GROUP: &str = "serverless.minik8s.io";
pub const VERSION: &str = "v1alpha1";
pub const DEFAULT_HANDLER: &str = "handler";
