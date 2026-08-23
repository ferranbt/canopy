//! What goes over the wire, in both directions.

pub mod context;
pub mod encoding;
pub mod message;
pub mod registration;
pub mod results;
pub mod service;
pub mod session;
pub mod step;

pub use context::{GithubContext, JobContext, JobDetails, NeedsResult, StrategyContext};
pub use encoding::normalize;
pub use message::{Envelope, JobMessage, JobOffer, Outcome, Plan, Timeline, Variable};
pub use registration::{
    Agent, AgentAuthorization, AgentRequest, Label, Pool, Pools, PublicKey, PublicKeyRequest,
    Tenant, TenantRequest,
};
pub use service::{
    ACTIONS, AccessMapping, ActionDownload, ActionDownloads, ActionReference, ActionReferences,
    ConnectionData, Identity, Lines, LocationServiceData, LogReference, Record, ResourceLocation,
};

pub use results::{
    LogsMetadata, SignedLogUrl, SignedLogUrlRequest, StepResult, StepsUpdateRequest, timestamp,
};
pub use session::{
    AcquireJobRequest, AgentReference, BrokerAgent, BrokerSessionRequest, CompleteJobRequest,
    EncryptionKey, RenewJobRequest, Session, SessionRequest, Token,
};
pub use step::{PipelineStep, StepReference};

/// A third-party runner has no binaries to swap out, so it claims a version the service
/// accepts and ignores any request to update, which is what other implementations do.
pub const RUNNER_VERSION: &str = "3.0.0";
