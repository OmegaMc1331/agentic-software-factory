use factory_types::TaskState;

pub struct Workflow;

impl Workflow {
    pub fn initial_state(has_dependencies: bool) -> TaskState {
        if has_dependencies {
            TaskState::Pending
        } else {
            TaskState::Ready
        }
    }

    pub fn can_transition(from: TaskState, to: TaskState) -> bool {
        if from == to {
            return true;
        }
        matches!(
            (from, to),
            (TaskState::Pending, TaskState::Ready | TaskState::Blocked)
                | (TaskState::Ready, TaskState::Running)
                | (
                    TaskState::Running,
                    TaskState::AwaitingIntegration | TaskState::Completed | TaskState::Failed
                        | TaskState::Blocked
                )
                | (TaskState::AwaitingIntegration, TaskState::Integrating | TaskState::Ready)
                | (
                    TaskState::Integrating,
                    TaskState::Completed | TaskState::Ready | TaskState::Failed | TaskState::Blocked
                )
                | (TaskState::Blocked, TaskState::Ready)
                | (TaskState::Failed, TaskState::Ready)
                | (TaskState::Completed, TaskState::Ready)
        )
    }

    pub fn allowed_targets(from: TaskState) -> Vec<TaskState> {
        match from {
            TaskState::Pending => vec![TaskState::Ready, TaskState::Blocked],
            TaskState::Ready => vec![TaskState::Running],
            TaskState::Running => vec![
                TaskState::AwaitingIntegration,
                TaskState::Completed,
                TaskState::Failed,
                TaskState::Blocked,
            ],
            TaskState::AwaitingIntegration => vec![TaskState::Integrating, TaskState::Ready],
            TaskState::Integrating => vec![
                TaskState::Completed,
                TaskState::Ready,
                TaskState::Failed,
                TaskState::Blocked,
            ],
            TaskState::Blocked => vec![TaskState::Ready],
            TaskState::Failed => vec![TaskState::Ready],
            // Completed → Ready is used by the runtime when a specialized
            // review requests changes: the implementation task is reworked
            // instead of restarting the workflow, represented as fresh
            // attempts rather than a cyclic DAG.
            TaskState::Completed => vec![TaskState::Ready],
        }
    }

    pub fn next_state_for_dependent(dep_states: &[TaskState]) -> TaskState {
        // AwaitingIntegration / Integrating deps must integrate (reach
        // Completed) before their dependents can schedule, just like Running.
        if dep_states
            .iter()
            .any(|s| matches!(s, TaskState::Failed | TaskState::Blocked))
        {
            TaskState::Blocked
        } else if dep_states.iter().all(|s| *s == TaskState::Completed) {
            TaskState::Ready
        } else {
            TaskState::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_types::TaskState::*;

    #[test]
    fn initial_state_depends_on_dependencies() {
        assert_eq!(Workflow::initial_state(false), Ready);
        assert_eq!(Workflow::initial_state(true), Pending);
    }

    #[test]
    fn accepts_valid_transitions() {
        assert!(Workflow::can_transition(Pending, Ready));
        assert!(Workflow::can_transition(Pending, Blocked));
        assert!(Workflow::can_transition(Ready, Running));
        assert!(Workflow::can_transition(Running, Completed));
        assert!(Workflow::can_transition(Running, Failed));
        assert!(Workflow::can_transition(Running, Blocked));
        assert!(Workflow::can_transition(Running, AwaitingIntegration));
        assert!(Workflow::can_transition(AwaitingIntegration, Integrating));
        assert!(Workflow::can_transition(AwaitingIntegration, Ready));
        assert!(Workflow::can_transition(Integrating, Completed));
        assert!(Workflow::can_transition(Integrating, Ready));
        assert!(Workflow::can_transition(Integrating, Failed));
        assert!(Workflow::can_transition(Integrating, Blocked));
        assert!(Workflow::can_transition(Blocked, Ready));
        assert!(Workflow::can_transition(Failed, Ready));
        assert!(Workflow::can_transition(Completed, Completed));
    }

    #[test]
    fn rejects_invalid_transitions() {
        assert!(!Workflow::can_transition(Pending, Running));
        assert!(!Workflow::can_transition(Pending, Completed));
        assert!(!Workflow::can_transition(Ready, Completed));
        assert!(!Workflow::can_transition(Running, Ready));
        assert!(!Workflow::can_transition(Blocked, Running));
        assert!(!Workflow::can_transition(Completed, Pending));
        assert!(!Workflow::can_transition(Failed, Running));
        assert!(!Workflow::can_transition(AwaitingIntegration, Running));
        assert!(!Workflow::can_transition(AwaitingIntegration, Completed));
        assert!(!Workflow::can_transition(Integrating, Running));
        assert!(!Workflow::can_transition(Integrating, AwaitingIntegration));
    }

    #[test]
    fn completed_allows_explicit_rework() {
        // A completed implementation can be reset to Ready when a specialized
        // review requests changes; retries stay bounded by attempt counts.
        assert_eq!(Workflow::allowed_targets(Completed), vec![Ready]);
        assert!(Workflow::can_transition(Completed, Ready));
    }

    #[test]
    fn dependent_state_follows_dependencies() {
        assert_eq!(Workflow::next_state_for_dependent(&[Completed]), Ready);
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, Ready]),
            Pending
        );
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, Failed]),
            Blocked
        );
        assert_eq!(Workflow::next_state_for_dependent(&[Pending]), Pending);
        assert_eq!(
            Workflow::next_state_for_dependent(&[Blocked, Ready]),
            Blocked
        );
        // Not-yet-integrated deps keep dependents pending.
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, AwaitingIntegration]),
            Pending
        );
        assert_eq!(
            Workflow::next_state_for_dependent(&[Completed, Integrating]),
            Pending
        );
    }
}
