pub mod capacity;
pub mod config;
pub mod factory;
pub mod github;
pub mod mission;
pub mod planner;
pub mod roles;
pub mod workflow;

pub use config::{
    default_config_text, AgentEntry, AgentInfo, AgentResolutionError, Agents, Config, ConfigError,
    RoleAssignment, RoleAssignmentInfo, RoleDefinitionEntry, RoleInfo,
};
pub use factory::{
    Factory, FactoryError, MarkOutcome, RunOutcome, WorkflowResult, FACTORY_DIR, MAX_TASK_ATTEMPTS,
};
pub use factory_agent::AgentStatus;
pub use factory_context::ContextConfig;
pub use factory_policy;
pub use factory_types::WorkflowTeam;
pub use planner::{parse_plan, validate_plan, PlanError, PlanOutcome, Planner};
pub use roles::{
    core_role, core_roles, is_core_role, is_pipeline_role, select_agent_with_capacity, slugify,
    ExecutionClass, RoleCatalog, RoleDefinition, RoleKind, CORE_ROLE_IDS, PIPELINE_ROLE_IDS,
    PLANNER, REVIEWER, SECURITY_AUDITOR, TEST_ENGINEER, WORKER,
};
pub use workflow::Workflow;
